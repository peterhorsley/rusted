use crate::editor::*;
use crate::search::*;
use crate::settings::*;
use crate::toolbar::*;
use eframe::egui::{self, Color32, FontId, Key, Modifiers};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_FONT_SIZE: f32 = 14.0;

pub(crate) const APP_ICON: &[u8] = include_bytes!("assets/app.png");

pub(crate) const MAX_FILTER_HISTORY: usize = 10;

pub(crate) const TAIL_POLL: Duration = Duration::from_millis(400);

pub(crate) fn remember_history(history: &mut Vec<String>, value: &str) {
    if !value.is_empty() {
        history.retain(|entry| entry != value);
        history.insert(0, value.to_owned());
        history.truncate(MAX_FILTER_HISTORY);
    }
}

pub(crate) struct Tab {
    pub(crate) id: u64,
    pub(crate) path: Option<PathBuf>,
    pub(crate) text: String,
    pub(crate) saved_text: String,
    pub(crate) crlf: bool,
    // Bytes of the file on disk that are reflected in `text`.
    pub(crate) disk_len: u64,
    pub(crate) dirty: bool,
    pub(crate) filter: FilterState,
    pub(crate) search_highlights: SearchHighlights,
    pub(crate) highlight_color: HighlightColor,
    pub(crate) highlight: Option<usize>,
    pub(crate) scroll_to_highlight: bool,
    pub(crate) goto_open: bool,
    pub(crate) goto_text: String,
    pub(crate) focus_goto: bool,
    pub(crate) goto_line: Option<usize>,
    pub(crate) scroll_to_goto: Option<usize>,
    pub(crate) filter_open: bool,
    pub(crate) filter_text: String,
    pub(crate) replace_text: String,
    pub(crate) case_sensitive: bool,
    pub(crate) applied: (String, bool),
    pub(crate) focus_filter: bool,
    pub(crate) word_wrap: bool,
    pub(crate) col_sel: Option<ColSel>,
    pub(crate) col_drag: bool,
    pub(crate) hidden_chars: bool,
    pub(crate) tail: bool,
    pub(crate) scroll_to_end: bool,
    pub(crate) row_index: RowIndex,
}

impl Tab {
    pub(crate) fn title(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => "Untitled".to_owned(),
        }
    }

    pub(crate) fn tab_title(&self) -> String {
        let title = self.title();
        if self.path.is_some() && self.dirty { format!("{title}*") } else { title }
    }

    pub(crate) fn tooltip(&self) -> String {
        let name = self.path.as_ref().map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.title());
        let prefix = if self.tail { "Tailing " } else { "" };
        let suffix = if self.dirty { " (unsaved)" } else { "" };
        format!("{prefix}{name}{suffix}")
    }
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum BrowserMode {
    Open,
    SaveAs,
}

pub(crate) struct Browser {
    pub(crate) mode: BrowserMode,
    // Empty path means the list of drives (Windows only).
    pub(crate) dir: PathBuf,
    pub(crate) entries: Vec<(String, bool)>,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    pub(crate) focus_name: bool,
}

impl Browser {
    pub(crate) fn new(mode: BrowserMode, dir: PathBuf, name: String) -> Self {
        let mut b = Browser { mode, dir, entries: Vec::new(), name, error: None, focus_name: true };
        b.refresh();
        b
    }

    pub(crate) fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.refresh();
    }

    pub(crate) fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        if self.dir.as_os_str().is_empty() {
            for c in b'A'..=b'Z' {
                let d = format!("{}:\\", c as char);
                if Path::new(&d).exists() {
                    self.entries.push((d, true));
                }
            }
            return;
        }
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let is_dir = match e.file_type() {
                        Ok(t) if t.is_dir() => true,
                        Ok(t) if t.is_symlink() => e.path().is_dir(),
                        _ => false,
                    };
                    self.entries.push((e.file_name().to_string_lossy().into_owned(), is_dir));
                }
                self.entries.sort_by(|a, b| {
                    b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
                });
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub(crate) fn up(&mut self) {
        match self.dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => {
                let p = p.to_path_buf();
                self.go(p)
            }
            _ => {
                if cfg!(windows) {
                    self.go(PathBuf::new())
                }
            }
        }
    }
}

pub(crate) fn format_json(text: &str) -> Result<String, serde_json::Error> {
    let _: &serde_json::value::RawValue = serde_json::from_str(text)?;
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            output.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                output.push(c);
            }
            '{' | '[' => {
                output.push(c);
                depth += 1;
                if !chars.clone().find(|c| !c.is_ascii_whitespace())
                    .is_some_and(|next| matches!((c, next), ('{', '}') | ('[', ']')))
                {
                    output.push('\n');
                    output.push_str(&"  ".repeat(depth));
                }
            }
            '}' | ']' => {
                depth -= 1;
                if !matches!(output.chars().last(), Some('{') | Some('[')) {
                    output.push('\n');
                    output.push_str(&"  ".repeat(depth));
                }
                output.push(c);
            }
            ',' => {
                output.push_str(",\n");
                output.push_str(&"  ".repeat(depth));
            }
            ':' => output.push_str(": "),
            ' ' | '\n' | '\r' | '\t' => {}
            _ => output.push(c),
        }
    }
    Ok(output)
}

pub(crate) struct App {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) next_id: u64,
    pub(crate) recent: Vec<PathBuf>,
    pub(crate) font_size: f32,
    pub(crate) browser: Option<Browser>,
    pub(crate) message: Option<String>,
    pub(crate) focus_editor: bool,
    pub(crate) filter_history: Vec<String>,
    pub(crate) replace_history: Vec<String>,
    pub(crate) toolbar_icons: Option<Result<ToolbarIcons, String>>,
    pub(crate) about_open: bool,
    pub(crate) light_theme: bool,
    pub(crate) about_icon: Option<Result<egui::TextureHandle, String>>,
    pub(crate) editor_selection: Option<(u64, egui::text::CCursorRange)>,
    pub(crate) pending_edit: Option<PendingEdit>,
    pub(crate) edit_menu_open: bool,
    pub(crate) clipboard_status: Result<bool, String>,
    #[cfg(windows)]
    pub(crate) launches: Option<std::sync::mpsc::Receiver<Result<crate::single_instance::Launch, String>>>,
    #[cfg(test)]
    pub(crate) clipboard_test_status: Option<Result<bool, String>>,
    pub(crate) window: WindowState,
    pub(crate) last_poll: Instant,
    pub(crate) status: (usize, usize, usize),
}

// Replaces the tab's contents with the file on disk, discarding unsaved edits.
pub(crate) fn reload_from_disk(tab: &mut Tab, path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let s = String::from_utf8_lossy(&bytes).into_owned();
    tab.crlf = s.contains("\r\n");
    tab.text = if tab.crlf { s.replace("\r\n", "\n") } else { s };
    tab.saved_text = tab.text.clone();
    tab.search_highlights.search(&tab.text, &tab.applied.0, tab.applied.1);
    tab.disk_len = bytes.len() as u64;
    tab.dirty = false;
    tab.highlight = None;
    tab.filter.needs_rescan = true;
    true
}

// Appends any complete lines added to the file on disk since the last read.
// Returns true when `text` changed.
pub(crate) fn read_tail(tab: &mut Tab, path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(meta) = std::fs::metadata(path) else { return false };
    let len = meta.len();
    if len == tab.disk_len {
        return false;
    }
    if len < tab.disk_len {
        // File was truncated or replaced: reload from scratch.
        return reload_from_disk(tab, path);
    }
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    if f.seek(SeekFrom::Start(tab.disk_len)).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if f.take(len - tab.disk_len).read_to_end(&mut buf).is_err() {
        return false;
    }
    // Consume as much as can be decoded so far; a trailing partial line is shown and completed
    // by later reads. A trailing '\r' is held back so CRLF pairs are never split.
    if tab.crlf && buf.last() == Some(&b'\r') {
        buf.pop();
    }
    let s: String = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_owned(),
        Err(e) if e.error_len().is_none() => {
            buf.truncate(e.valid_up_to());
            String::from_utf8_lossy(&buf).into_owned()
        }
        Err(_) => String::from_utf8_lossy(&buf).into_owned(),
    };
    if buf.is_empty() {
        return false;
    }
    let s = if tab.crlf { s.replace("\r\n", "\n") } else { s };
    tab.text.push_str(&s);
    tab.saved_text.push_str(&s);
    tab.disk_len += buf.len() as u64;
    true
}

impl eframe::App for App {
    // egui low-pass filters discrete wheel notches over several frames. Preceding each wheel
    // event with a Start phase makes egui treat it as direct (unsmoothed) input.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let dropped_files = std::mem::take(&mut raw_input.dropped_files);
        if !dropped_files.is_empty() {
            self.open_dropped_files(dropped_files);
        }
        if !raw_input.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })) {
            return;
        }
        let events = std::mem::take(&mut raw_input.events);
        for e in events {
            if let egui::Event::MouseWheel { unit, source, modifiers, phase: egui::TouchPhase::Move, .. } = &e {
                raw_input.events.push(egui::Event::MouseWheel {
                    unit: *unit,
                    delta: egui::Vec2::ZERO,
                    phase: egui::TouchPhase::Start,
                    source: *source,
                    modifiers: *modifiers,
                });
            }
            raw_input.events.push(e);
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        save_window(&self.window);
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        self.handle_launches();
        self.tab_shortcuts(ctx);
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) {
            self.browser = None;
            self.new_tab(None, String::new(), false, 0);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
            self.open_browser();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save();
        }
        self.view_shortcuts(ctx);
        self.goto_shortcut(ctx);
        if ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::F5) | i.consume_key(Modifiers::COMMAND, Key::R)
        }) {
            self.reload();
        }
        if ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::F4) | i.consume_key(Modifiers::COMMAND, Key::F)
        }) {
            self.toggle_filter();
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        ctx.input(|i| {
            let vp = i.viewport();
            let maximized = vp.maximized.unwrap_or(false);
            if vp.minimized.unwrap_or(false) {
                return;
            }
            self.window.maximized = maximized;
            // Keep the last restored geometry so it survives exiting while maximized.
            if !maximized {
                if let (Some(inner), Some(outer)) = (vp.inner_rect, vp.outer_rect) {
                    self.window.size = inner.size();
                    self.window.pos = Some(outer.min);
                }
            }
        });
        if zoom != 1.0 {
            self.font_size = (self.font_size * zoom).clamp(6.0, 96.0);
        }
        if self.browser.is_none() {
            let tab_tail = self.tabs[self.active].tail;
            let poll = self.last_poll.elapsed() >= TAIL_POLL;
            if poll {
                self.last_poll = Instant::now();
            }
            if self.tabs[self.active].filter_open || tab_tail {
                self.update_filter(poll);
            }
            if tab_tail {
                ctx.request_repaint_after(TAIL_POLL);
            }
            if self.tabs.iter().any(|t| t.tail) {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(root, |ui| self.menu_bar(ui));
        egui::Panel::top("tabs").show(root, |ui| self.tab_bar(ui));
        egui::Panel::top("view_toolbar").show(root, |ui| self.view_toolbar(ui));
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                let (total, l, c) = self.status;
                ui.label(format!("Lines: {total}    Ln: {l}    Col: {c}"));
                if let Some(m) = &self.message {
                    ui.colored_label(Color32::LIGHT_RED, m);
                }
            });
        });
        if self.tabs[self.active].goto_open && self.browser.is_none() {
            egui::Panel::bottom(("goto", self.tabs[self.active].id)).show(root, |ui| self.goto_ui(ui));
        }
        if self.tabs[self.active].filter_open && self.browser.is_none() {
            egui::Panel::bottom(("filter", self.tabs[self.active].id))
                .resizable(true)
                .default_size(260.0)
                .min_size(60.0)
                .show(root, |ui| self.filter_ui(ui));
        }
        egui::CentralPanel::default().show(root, |ui| {
            if self.browser.is_some() {
                self.browser_ui(ui);
            } else {
                self.editor_ui(ui);
            }
        });
        self.about_ui(root.ctx());
    }
}

impl App {
    #[cfg(windows)]
    pub(crate) fn handle_launches(&mut self) {
        while let Some(result) = self.launches.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
            match result {
                Ok(launch) => {
                    if let Some(error) = launch.error {
                        self.message = Some(error);
                    } else if let Some(path) = launch.file {
                        self.browser = None;
                        self.open_startup_file(path, launch.tail);
                    }
                    if let Some(error) = launch.focus_error {
                        self.message = Some(match self.message.take() {
                            Some(message) => format!("{message}; {error}"),
                            None => error,
                        });
                    }
                }
                Err(error) => self.message = Some(error),
            }
        }
    }

    pub(crate) fn toggle_tail(&mut self) {
        let tab = &mut self.tabs[self.active];
        if tab.tail {
            tab.tail = false;
        } else {
            if let Some(path) = tab.path.clone() {
                if !reload_from_disk(tab, &path) {
                    self.message = Some(format!("Cannot reload {}", path.display()));
                    return;
                }
            }
            tab.tail = true;
            tab.scroll_to_end = true;
        }
    }

    pub(crate) fn view_shortcuts(&mut self, ctx: &egui::Context) {
        if self.browser.is_none() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W)) {
                let tab = &mut self.tabs[self.active];
                tab.word_wrap = !tab.word_wrap;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
                let tab = &mut self.tabs[self.active];
                tab.hidden_chars = !tab.hidden_chars;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T)) {
                self.toggle_tail();
            }
        }
    }

    pub(crate) fn goto_shortcut(&mut self, ctx: &egui::Context) {
        if self.browser.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::G)) {
            self.open_goto();
        }
    }

    pub(crate) fn open_goto(&mut self) {
        let tab = &mut self.tabs[self.active];
        tab.goto_open = true;
        tab.goto_text.clear();
        tab.focus_goto = true;
        self.focus_editor = false;
    }

    pub(crate) fn tab_shortcuts(&mut self, ctx: &egui::Context) {
        let steps = ctx.input_mut(|input| {
            let mut steps = Vec::new();
            input.events.retain(|event| {
                if let egui::Event::Key { key: Key::Tab, pressed: true, modifiers, .. } = event {
                    if modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd {
                        steps.push(modifiers.shift);
                        return false;
                    }
                }
                true
            });
            steps
        });
        if self.tabs.len() < 2 {
            return;
        }
        for backwards in steps {
            let next = if backwards {
                if self.active == 0 { self.tabs.len() - 1 } else { self.active - 1 }
            } else {
                (self.active + 1) % self.tabs.len()
            };
            self.select_tab(next);
        }
    }

    pub(crate) fn select_tab(&mut self, index: usize) {
        self.active = index;
        self.browser = None;
        self.focus_editor = true;
    }

    pub(crate) fn edit_availability(&self, ui: &egui::Ui) -> (bool, bool, bool) {
        let tab = &self.tabs[self.active];
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", tab.id));
        let state = egui::text_edit::TextEditState::load(ui.ctx(), id);
        let undo = state.as_ref().is_some_and(|s| history_steps(s, &tab.text, false) > 0);
        let redo = state.as_ref().is_some_and(|s| history_steps(s, &tab.text, true) > 0);
        let selected = if let Some(selection) = tab.col_sel {
            let shown: String = if tab.hidden_chars {
                tab.text.chars().map(|c| if c == ' ' { '·' } else { c }).collect()
            } else {
                tab.text.clone()
            };
            let job = egui::text::LayoutJob::simple(
                shown, FontId::monospace(self.font_size), Color32::WHITE, f32::INFINITY,
            );
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            column_text(&tab.text, &selection, &galley).chars().any(|c| c != '\n')
        } else {
            self.editor_selection
                .filter(|(id, _)| *id == tab.id)
                .map(|(_, range)| !range.is_empty())
                .unwrap_or(false)
        };
        (undo, redo, selected)
    }

    pub(crate) fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>, tail: bool) -> Self {
        let light_theme = load_light_theme();
        cc.egui_ctx.set_visuals(theme_visuals(light_theme));
        cc.egui_ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            recent: load_recent(),
            font_size: DEFAULT_FONT_SIZE,
            browser: None,
            message: None,
            focus_editor: true,
            filter_history: Vec::new(),
            replace_history: Vec::new(),
            toolbar_icons: None,
            about_open: false,
            light_theme: light_theme,
            about_icon: None,
            editor_selection: None,
            pending_edit: None,
            edit_menu_open: false,
            clipboard_status: Ok(false),
            #[cfg(windows)]
            launches: None,
            #[cfg(test)]
            clipboard_test_status: None,
            window: load_window().unwrap_or(WindowState {
                pos: None,
                size: egui::vec2(1000.0, 700.0),
                maximized: false,
            }),
            last_poll: Instant::now(),
            status: (1, 1, 1),
        };
        if let Some(p) = file {
            app.open_startup_file(p, tail);
        }
        if app.tabs.is_empty() {
            app.new_tab(None, String::new(), false, 0);
        }
        app
    }

    pub(crate) fn open_startup_file(&mut self, path: PathBuf, tail: bool) {
        self.message = None;
        self.open_path(path);
        if tail && self.message.is_none() && !self.tabs.is_empty() && !self.tabs[self.active].tail {
            self.toggle_tail();
        }
    }

    pub(crate) fn new_tab(&mut self, path: Option<PathBuf>, text: String, crlf: bool, disk_len: u64) {
        self.tabs.push(Tab {
            id: self.next_id,
            path,
            saved_text: text.clone(),
            text,
            crlf,
            disk_len,
            dirty: false,
            filter: FilterState::default(),
            search_highlights: SearchHighlights::default(),
            highlight_color: HighlightColor::default(),
            highlight: None,
            scroll_to_highlight: false,
            goto_open: false,
            goto_text: String::new(),
            focus_goto: false,
            goto_line: None,
            scroll_to_goto: None,
            filter_open: false,
            filter_text: String::new(),
            replace_text: String::new(),
            case_sensitive: false,
            applied: (String::new(), false),
            focus_filter: false,
            word_wrap: false,
            hidden_chars: false,
            tail: false,
            scroll_to_end: false,
            col_sel: None,
            col_drag: false,
            row_index: RowIndex::default(),
        });
        self.next_id += 1;
        self.active = self.tabs.len() - 1;
        self.focus_editor = true;
    }

    pub(crate) fn open_path(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.path.as_ref() == Some(&path)) {
            self.active = i;
            self.focus_editor = true;
            self.add_recent(path);
            return;
        }
        let (text, crlf, len) = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let len = bytes.len() as u64;
                    let s = String::from_utf8_lossy(&bytes).into_owned();
                    if s.contains("\r\n") {
                        (s.replace("\r\n", "\n"), true, len)
                    } else {
                        (s, false, len)
                    }
                }
                Err(e) => {
                    self.message = Some(format!("Cannot open {}: {e}", path.display()));
                    return;
                }
            }
        } else {
            (String::new(), false, 0)
        };
        // Replace a pristine empty untitled tab instead of stacking on it.
        if self.tabs.len() == 1 && self.tabs[0].path.is_none() && self.tabs[0].text.is_empty() {
            self.tabs.clear();
        }
        self.add_recent(path.clone());
        self.new_tab(Some(path), text, crlf, len);
        self.message = None;
    }

    pub(crate) fn open_dropped_files(&mut self, files: Vec<egui::DroppedFileHandle>) {
        let mut errors = Vec::new();
        for file in files {
            let path = file.path();
            if path.as_os_str().is_empty() {
                errors.push("Dropped file has no local path".to_owned());
                continue;
            }
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.is_file() => {
                    self.browser = None;
                    self.message = None;
                    self.open_path(path.to_path_buf());
                    if let Some(error) = self.message.take() {
                        errors.push(error);
                    }
                }
                Ok(_) => errors.push(format!("Cannot open {}: not a regular file", path.display())),
                Err(error) => errors.push(format!("Cannot open {}: {error}", path.display())),
            }
        }
        if !errors.is_empty() {
            self.message = Some(errors.join("; "));
        }
    }

    pub(crate) fn add_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        #[cfg(not(test))]
        if let Some(f) = recent_file() {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let s: Vec<String> = self.recent.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            let _ = std::fs::write(f, s.join("\n"));
        }
    }

    pub(crate) fn write_tab(&mut self, idx: usize, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let tab = &mut self.tabs[idx];
        let data = if tab.crlf { tab.text.replace('\n', "\r\n") } else { tab.text.clone() };
        match std::fs::write(&path, &data) {
            Ok(()) => {
                tab.path = Some(path.clone());
                tab.dirty = false;
                tab.saved_text = tab.text.clone();
                tab.disk_len = data.len() as u64;
                tab.filter.needs_rescan = true;
                self.message = None;
                self.add_recent(path);
            }
            Err(e) => self.message = Some(format!("Cannot save {}: {e}", path.display())),
        }
    }

    pub(crate) fn save(&mut self) {
        match self.tabs[self.active].path.clone() {
            Some(p) => self.write_tab(self.active, p),
            None => self.save_as(),
        }
    }

    pub(crate) fn start_dir(&self) -> PathBuf {
        self.tabs
            .get(self.active)
            .and_then(|t| t.path.as_ref())
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    pub(crate) fn open_browser(&mut self) {
        self.browser = Some(Browser::new(BrowserMode::Open, self.start_dir(), String::new()));
    }

    pub(crate) fn save_as(&mut self) {
        let name = self.tabs[self.active].title();
        self.browser = Some(Browser::new(BrowserMode::SaveAs, self.start_dir(), name));
    }

    pub(crate) fn close_browser(&mut self) {
        self.browser = None;
        self.focus_editor = true;
    }

    pub(crate) fn reload(&mut self) {
        let tab = &mut self.tabs[self.active];
        let Some(path) = tab.path.clone() else { return };
        if !reload_from_disk(tab, &path) {
            self.message = Some(format!("Cannot reload {}", path.display()));
        } else {
            self.message = None;
            self.focus_editor = true;
        }
    }

    pub(crate) fn toggle_filter(&mut self) {
        let tab = &mut self.tabs[self.active];
        tab.filter_open = !tab.filter_open;
        if tab.filter_open {
            tab.focus_filter = true;
        } else {
            self.focus_editor = true;
        }
    }

    pub(crate) fn replace_tab_text(&mut self, ctx: &egui::Context, text: String) {
        let tab = &mut self.tabs[self.active];
        if text == tab.text {
            return;
        }
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", tab.id));
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let old_range = state.cursor.char_range()
            .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
        let cursor = usize::from(old_range.primary.index).min(text.chars().count());
        let new_range = egui::text::CCursorRange::one(egui::text::CCursor::new(cursor));
        let mut undoer = state.undoer();
        undoer.add_undo(&(old_range, tab.text.clone()));
        undoer.feed_state(ctx.input(|input| input.time), &(new_range, text.clone()));
        undoer.add_undo(&(new_range, text.clone()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(new_range));
        state.store(ctx, id);
        tab.text = text;
        tab.search_highlights.edited(&tab.text);
        tab.dirty = tab.text != tab.saved_text;
        tab.highlight = None;
        tab.col_sel = None;
        tab.col_drag = false;
        tab.filter.applied = None;
        self.editor_selection = Some((tab.id, new_range));
        self.focus_editor = true;
    }

    pub(crate) fn replace_all(&mut self, ctx: &egui::Context) {
        let tab = &self.tabs[self.active];
        if tab.tail {
            return;
        }
        if !tab.filter_text.is_empty() {
            remember_history(&mut self.replace_history, &tab.replace_text);
        }
        if let Some(text) = replace_matches(&tab.text, &tab.filter_text, &tab.replace_text, tab.case_sensitive) {
            self.replace_tab_text(ctx, text);
            self.update_filter(false);
        }
    }

    pub(crate) fn format_current_json(&mut self, ctx: &egui::Context) {
        let tab = &self.tabs[self.active];
        match format_json(&tab.text) {
            Ok(formatted) => {
                self.message = None;
                self.replace_tab_text(ctx, formatted);
            }
            Err(error) => self.message = Some(format!("Cannot format JSON: {error}")),
        }
    }

    pub(crate) fn apply_search(&mut self) {
        let tab = &mut self.tabs[self.active];
        let text = tab.filter_text.clone();
        remember_history(&mut self.filter_history, &text);
        tab.applied = (text, tab.case_sensitive);
        tab.search_highlights.search(&tab.text, &tab.applied.0, tab.applied.1);
        tab.filter.applied = None;
        self.update_filter(false);
    }

    // Keeps the active tab's filter results current; with `poll_disk` it also tails the file.
    pub(crate) fn update_filter(&mut self, poll_disk: bool) {
        let tab = &mut self.tabs[self.active];
        let filter_open = tab.filter_open;
        let applied = tab.applied.clone();
        let tailing = tab.tail;

        let mut disk_changed = false;
        if poll_disk && tailing {
            if let Some(path) = tab.path.clone() {
                disk_changed = read_tail(tab, &path);
            }
        }
        if disk_changed {
            tab.scroll_to_end = true;
            tab.search_highlights.tailed(&tab.text, &tab.applied.0, tab.applied.1);
        }
        if !filter_open {
            return;
        }

        let mut full = tab.filter.applied.as_ref() != Some(&applied);
        if tailing && tab.filter.needs_rescan {
            full = true;
        }
        tab.filter.needs_rescan = false;
        if full {
            tab.filter = FilterState { applied: Some(applied.clone()), ..Default::default() };
            tab.filter.scan(&tab.text, &applied.0, applied.1);
        } else if disk_changed {
            tab.filter.scan(&tab.text, &applied.0, applied.1);
        }
    }

    // Closing discards unsaved edits; the last tab is replaced by an empty untitled one.
    pub(crate) fn close_tab(&mut self, i: usize) {
        self.tabs.remove(i);
        self.browser = None;
        if self.tabs.is_empty() {
            self.new_tab(None, String::new(), false, 0);
        } else if i < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_editor = true;
    }
}
