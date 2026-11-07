#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, TextEdit};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_RECENT: usize = 10;
const DEFAULT_FONT_SIZE: f32 = 14.0;

const MAX_FILTER_HISTORY: usize = 10;
const TAIL_POLL: Duration = Duration::from_millis(400);

#[derive(Default)]
struct FilterState {
    // (0-based line number, line text)
    results: Vec<(usize, String)>,
    // Byte offset and line number of the first line not yet fully scanned.
    offset: usize,
    line: usize,
    applied: Option<(String, bool)>,
    selected: Option<usize>,
    needs_rescan: bool,
}

impl FilterState {
    fn scan(&mut self, text: &str, pat: &str, case: bool) {
        let keep = self.line;
        self.results.retain(|r| r.0 < keep);
        if self.offset > text.len() || !text.is_char_boundary(self.offset) {
            self.results.clear();
            self.offset = 0;
            self.line = 0;
        }
        let needle = if case { pat.to_owned() } else { pat.to_lowercase() };
        let (mut off, mut line) = (self.offset, self.line);
        while off < text.len() {
            let rest = &text[off..];
            let (l, terminated) = match rest.find('\n') {
                Some(i) => (&rest[..i], true),
                None => (rest, false),
            };
            let hit = !needle.is_empty()
                && if case { l.contains(&needle) } else { l.to_lowercase().contains(&needle) };
            if hit {
                self.results.push((line, l.to_owned()));
            }
            if !terminated {
                break;
            }
            off += l.len() + 1;
            line += 1;
        }
        self.offset = off;
        self.line = line;
    }
}

#[derive(Clone, Copy)]
struct ColSel {
    // Line index and horizontal position relative to the text galley.
    anchor: (usize, f32),
    head: (usize, f32),
}

impl ColSel {
    fn bounds(&self) -> (usize, usize, f32, f32) {
        (
            self.anchor.0.min(self.head.0),
            self.anchor.0.max(self.head.0),
            self.anchor.1.min(self.head.1),
            self.anchor.1.max(self.head.1),
        )
    }
}

fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map(|(b, _)| b).unwrap_or(line.len())
}

fn columns_for_row(galley: &egui::Galley, row: usize, x0: f32, x1: f32) -> (usize, usize) {
    let Some(row) = galley.rows.get(row) else { return (0, 0) };
    (
        usize::from(row.char_at(x0 - row.pos.x)),
        usize::from(row.char_at(x1 - row.pos.x)),
    )
}

fn column_text(text: &str, s: &ColSel, galley: &egui::Galley) -> String {
    let (l0, l1, x0, x1) = s.bounds();
    text.split('\n')
        .enumerate()
        .skip(l0)
        .take(l1 - l0 + 1)
        .map(|(i, l)| {
            let (c0, c1) = columns_for_row(galley, i, x0, x1);
            &l[byte_at(l, c0)..byte_at(l, c1)]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn delete_column(text: &mut String, s: &ColSel, galley: &egui::Galley) {
    let (l0, l1, x0, x1) = s.bounds();
    let mut out = String::with_capacity(text.len());
    for (i, l) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if i >= l0 && i <= l1 {
            let (c0, c1) = columns_for_row(galley, i, x0, x1);
            out.push_str(&l[..byte_at(l, c0)]);
            out.push_str(&l[byte_at(l, c1)..]);
        } else {
            out.push_str(l);
        }
    }
    *text = out;
}

fn char_index_of(text: &str, line: usize, col: usize) -> usize {
    let mut idx = 0;
    for (i, l) in text.split('\n').enumerate() {
        let n = l.chars().count();
        if i == line {
            return idx + col.min(n);
        }
        idx += n + 1;
    }
    idx.saturating_sub(1)
}

// Handles copy/cut/delete for the rectangular selection before the text edit can act on them.
fn column_keys(ctx: &egui::Context, tab: &mut Tab, galley: &egui::Galley) {
    let Some(sel) = tab.col_sel else { return };
    let (copy, cut, del, clear) = ctx.input_mut(|i| {
        let (mut copy, mut cut) = (false, false);
        i.events.retain(|e| match e {
            egui::Event::Copy => {
                copy = true;
                false
            }
            egui::Event::Cut => {
                cut = true;
                false
            }
            _ => true,
        });
        let del = i.consume_key(Modifiers::NONE, Key::Delete) | i.consume_key(Modifiers::NONE, Key::Backspace);
        let clear = i.events.iter().any(|e| match e {
            egui::Event::Text(_) | egui::Event::Paste(_) => true,
            egui::Event::Key { key, pressed: true, .. } => matches!(
                key,
                Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::ArrowUp
                    | Key::ArrowDown
                    | Key::Home
                    | Key::End
                    | Key::PageUp
                    | Key::PageDown
                    | Key::Enter
                    | Key::Tab
                    | Key::Escape
            ),
            _ => false,
        });
        (copy, cut, del, clear)
    });
    if copy || cut {
        ctx.copy_text(column_text(&tab.text, &sel, galley));
    }
    if cut || del {
        delete_column(&mut tab.text, &sel, galley);
        let (l0, l1, x0, _) = sel.bounds();
        tab.col_sel = Some(ColSel { anchor: (l0, x0), head: (l1, x0) });
        tab.dirty = true;
        tab.highlight = None;
    } else if clear {
        tab.col_sel = None;
    }
}

struct Tab {
    id: u64,
    path: Option<PathBuf>,
    text: String,
    crlf: bool,
    // Bytes of the file on disk that are reflected in `text`.
    disk_len: u64,
    dirty: bool,
    filter: FilterState,
    highlight: Option<usize>,
    scroll_to_highlight: bool,
    filter_open: bool,
    filter_text: String,
    case_sensitive: bool,
    applied: (String, bool),
    focus_filter: bool,
    word_wrap: bool,
    col_sel: Option<ColSel>,
    col_drag: bool,
    hidden_chars: bool,
    tail: bool,
    scroll_to_end: bool,
}

impl Tab {
    fn title(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => "Untitled".to_owned(),
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum BrowserMode {
    Open,
    SaveAs,
}

struct Browser {
    mode: BrowserMode,
    // Empty path means the list of drives (Windows only).
    dir: PathBuf,
    entries: Vec<(String, bool)>,
    name: String,
    error: Option<String>,
    focus_name: bool,
}

impl Browser {
    fn new(mode: BrowserMode, dir: PathBuf, name: String) -> Self {
        let mut b = Browser { mode, dir, entries: Vec::new(), name, error: None, focus_name: true };
        b.refresh();
        b
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.refresh();
    }

    fn refresh(&mut self) {
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

    fn up(&mut self) {
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

struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    recent: Vec<PathBuf>,
    font_size: f32,
    browser: Option<Browser>,
    message: Option<String>,
    focus_editor: bool,
    filter_history: Vec<String>,
    window: WindowState,
    last_poll: Instant,
    status: (usize, usize, usize),
}

fn recent_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        Some(PathBuf::from(x))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }?;
    Some(base.join("rusted").join("recent.txt"))
}

fn load_recent() -> Vec<PathBuf> {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).take(MAX_RECENT).collect())
        .unwrap_or_default()
}

// "Live" indicator at the left edge of a tab label: a solid dot with a ring that expands and fades.
fn paint_spinner(ui: &egui::Ui, label: &egui::Response) {
    let size = egui::TextStyle::Body.resolve(ui.style()).size * 0.8;
    let center = egui::pos2(
        label.rect.left() + ui.spacing().button_padding.x + size / 2.0,
        label.rect.center().y,
    );
    let color = Color32::from_rgb(90, 200, 120);
    let phase = (ui.input(|i| i.time) % 1.4 / 1.4) as f32;
    let painter = ui.painter();
    painter.circle_filled(center, size * 0.22, color);
    painter.circle_stroke(
        center,
        size * (0.22 + 0.28 * phase),
        egui::Stroke::new(1.5, color.gamma_multiply(1.0 - phase)),
    );
}

// True when the user pressed the mouse or scrolled the wheel over `rect`.
fn user_interacted(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.input(|i| {
        let Some(pos) = i.pointer.hover_pos() else { return false };
        if !rect.contains(pos) {
            return false;
        }
        i.pointer.primary_pressed()
            || (!i.modifiers.command
                && i.raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })))
    })
}

// Replaces the tab's contents with the file on disk, discarding unsaved edits.
fn reload_from_disk(tab: &mut Tab, path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let s = String::from_utf8_lossy(&bytes).into_owned();
    tab.crlf = s.contains("\r\n");
    tab.text = if tab.crlf { s.replace("\r\n", "\n") } else { s };
    tab.disk_len = bytes.len() as u64;
    tab.dirty = false;
    tab.highlight = None;
    tab.filter.needs_rescan = true;
    true
}

// Appends any complete lines added to the file on disk since the last read.
// Returns true when `text` changed.
fn read_tail(tab: &mut Tab, path: &Path) -> bool {
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
    if tab.crlf {
        tab.text.push_str(&s.replace("\r\n", "\n"));
    } else {
        tab.text.push_str(&s);
    }
    tab.disk_len += buf.len() as u64;
    true
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
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
            window: load_window().unwrap_or(WindowState {
                pos: None,
                size: egui::vec2(1000.0, 700.0),
                maximized: false,
            }),
            last_poll: Instant::now(),
            status: (1, 1, 1),
        };
        if let Some(p) = file {
            app.open_path(p);
        }
        if app.tabs.is_empty() {
            app.new_tab(None, String::new(), false, 0);
        }
        app
    }

    fn new_tab(&mut self, path: Option<PathBuf>, text: String, crlf: bool, disk_len: u64) {
        self.tabs.push(Tab {
            id: self.next_id,
            path,
            text,
            crlf,
            disk_len,
            dirty: false,
            filter: FilterState::default(),
            highlight: None,
            scroll_to_highlight: false,
            filter_open: false,
            filter_text: String::new(),
            case_sensitive: false,
            applied: (String::new(), false),
            focus_filter: false,
            word_wrap: false,
            hidden_chars: false,
            tail: false,
            scroll_to_end: false,
            col_sel: None,
            col_drag: false,
        });
        self.next_id += 1;
        self.active = self.tabs.len() - 1;
        self.focus_editor = true;
    }

    fn open_path(&mut self, path: PathBuf) {
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

    fn add_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        if let Some(f) = recent_file() {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let s: Vec<String> = self.recent.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            let _ = std::fs::write(f, s.join("\n"));
        }
    }

    fn write_tab(&mut self, idx: usize, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let tab = &mut self.tabs[idx];
        let data = if tab.crlf { tab.text.replace('\n', "\r\n") } else { tab.text.clone() };
        match std::fs::write(&path, &data) {
            Ok(()) => {
                tab.path = Some(path.clone());
                tab.dirty = false;
                tab.disk_len = data.len() as u64;
                tab.filter.needs_rescan = true;
                self.message = None;
                self.add_recent(path);
            }
            Err(e) => self.message = Some(format!("Cannot save {}: {e}", path.display())),
        }
    }

    fn save(&mut self) {
        match self.tabs[self.active].path.clone() {
            Some(p) => self.write_tab(self.active, p),
            None => self.save_as(),
        }
    }

    fn start_dir(&self) -> PathBuf {
        self.tabs
            .get(self.active)
            .and_then(|t| t.path.as_ref())
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn open_browser(&mut self) {
        self.browser = Some(Browser::new(BrowserMode::Open, self.start_dir(), String::new()));
    }

    fn save_as(&mut self) {
        let name = self.tabs[self.active].title();
        self.browser = Some(Browser::new(BrowserMode::SaveAs, self.start_dir(), name));
    }

    fn close_browser(&mut self) {
        self.browser = None;
        self.focus_editor = true;
    }

    fn reload(&mut self) {
        let tab = &mut self.tabs[self.active];
        let Some(path) = tab.path.clone() else { return };
        if !reload_from_disk(tab, &path) {
            self.message = Some(format!("Cannot reload {}", path.display()));
        } else {
            self.message = None;
            self.focus_editor = true;
        }
    }

    fn toggle_filter(&mut self) {
        let tab = &mut self.tabs[self.active];
        tab.filter_open = !tab.filter_open;
        if tab.filter_open {
            tab.focus_filter = true;
        } else {
            self.focus_editor = true;
        }
    }

    fn apply_search(&mut self) {
        let tab = &mut self.tabs[self.active];
        let text = tab.filter_text.clone();
        if !text.is_empty() {
            self.filter_history.retain(|h| h != &text);
            self.filter_history.insert(0, text.clone());
            self.filter_history.truncate(MAX_FILTER_HISTORY);
        }
        tab.applied = (text, tab.case_sensitive);
        tab.filter.applied = None;
        self.update_filter(false);
    }

    // Keeps the active tab's filter results current; with `poll_disk` it also tails the file.
    fn update_filter(&mut self, poll_disk: bool) {
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

    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut search = false;
        let history = self.filter_history.clone();
        let tab = &mut self.tabs[self.active];
        ui.horizontal(|ui| {
            ui.label("Filter:");
            let r = ui.add(TextEdit::singleline(&mut tab.filter_text).desired_width(320.0));
            if std::mem::take(&mut tab.focus_filter) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                search = true;
            }
            ui.add_enabled_ui(!history.is_empty(), |ui| {
                ui.menu_button("▾", |ui| {
                    for h in history {
                        if ui.button(&h).clicked() {
                            tab.filter_text = h;
                            search = true;
                            ui.close();
                        }
                    }
                });
            });
            if ui.button("Search").clicked() {
                search = true;
            }
            if ui.checkbox(&mut tab.case_sensitive, "Case sensitive").changed() {
                search = true;
            }
        });
        if search {
            self.apply_search();
        }
        ui.separator();

        let font = FontId::monospace(self.font_size);
        let row_h = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().item_spacing.y;
        let tab = &mut self.tabs[self.active];
        if tab.tail && user_interacted(ui, ui.available_rect_before_wrap()) {
            tab.tail = false;
        }
        let mut jump = None;
        let results = &tab.filter.results;
        let mut selected = tab.filter.selected;
        egui::ScrollArea::both()
            .id_salt(("filter_results", tab.id))
            .auto_shrink([false, false])
            .stick_to_bottom(tab.tail)
            .show_rows(
            ui,
            row_h,
            results.len(),
            |ui, range| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for i in range {
                        let (line, text) = &results[i];
                        let label = RichText::new(format!("{:>6}  {}", line + 1, text)).font(font.clone());
                        let r = ui.selectable_label(selected == Some(i), label);
                        if r.clicked() {
                            selected = Some(i);
                        }
                        if r.double_clicked() {
                            selected = Some(i);
                            jump = Some(*line);
                        }
                    }
                });
            },
        );
        tab.filter.selected = selected;
        if let Some(line) = jump {
            tab.highlight = Some(line);
            tab.scroll_to_highlight = true;
            tab.scroll_to_end = false;
            tab.tail = false;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                    self.browser = None;
                    self.new_tab(None, String::new(), false, 0);
                    ui.close();
                }
                if ui.add(egui::Button::new("Open").shortcut_text("Ctrl+O")).clicked() {
                    self.open_browser();
                    ui.close();
                }
                ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
                    ui.menu_button("Open Recent", |ui| {
                        let mut chosen = None;
                        for p in &self.recent {
                            if ui.button(p.to_string_lossy()).clicked() {
                                chosen = Some(p.clone());
                                ui.close();
                            }
                        }
                        if let Some(p) = chosen {
                            self.browser = None;
                            self.open_path(p);
                        }
                    });
                });
                if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    self.save();
                    ui.close();
                }
                if ui.button("Save As").clicked() {
                    self.save_as();
                    ui.close();
                }
                if ui.add(egui::Button::new("Reload").shortcut_text("F5 / Ctrl+R")).clicked() {
                    self.reload();
                    ui.close();
                }
                if ui.add(egui::Button::new("Filter / find").shortcut_text("F4 / Ctrl+F")).clicked() {
                    self.toggle_filter();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                let tab = &mut self.tabs[self.active];
                ui.checkbox(&mut tab.word_wrap, "Word wrap");
                ui.checkbox(&mut tab.hidden_chars, "Hidden characters");
                if ui.checkbox(&mut tab.tail, "Tail").changed() && tab.tail {
                                    if let Some(p) = tab.path.clone() {
                                        reload_from_disk(tab, &p);
                                    }
                                    tab.scroll_to_end = true;
                }
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut select = None;
            let mut close = None;
            for (i, t) in self.tabs.iter().enumerate() {
                let title = if t.tail {
                    // Leave room for the spinner painted over the start of the label.
                    let body = egui::TextStyle::Body.resolve(ui.style());
                    let space_w = ui.fonts_mut(|f| f.glyph_width(&body, ' ')).max(1.0);
                    let n = (body.size / space_w).ceil() as usize + 1;
                    format!("{}{}", " ".repeat(n), t.title())
                } else {
                    t.title()
                };
                let mut r = ui.selectable_label(self.browser.is_none() && i == self.active, title);
                if t.tail {
                    paint_spinner(ui, &r);
                }
                if let Some(p) = &t.path {
                    let path = p.to_string_lossy();
                    r = r.on_hover_text(if t.tail { format!("Tailing {path}") } else { path.into_owned() });
                }
                if r.clicked() {
                    select = Some(i);
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
            }
            if let Some(i) = close {
                self.close_tab(i);
            } else if let Some(i) = select {
                self.active = i;
                self.browser = None;
                self.focus_editor = true;
            }
        });
    }

    // Closing discards unsaved edits; the last tab is replaced by an empty untitled one.
    fn close_tab(&mut self, i: usize) {
        self.tabs.remove(i);
        self.browser = None;
        if self.tabs.is_empty() {
            self.new_tab(None, String::new(), false, 0);
        } else if i < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_editor = true;
    }

    fn browser_ui(&mut self, ui: &mut egui::Ui) {
        let mut b = self.browser.take().unwrap();
        let mut cancel = false;
        let mut open_file: Option<PathBuf> = None;
        let mut nav: Option<PathBuf> = None;

        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                b.up();
            }
            let title = if b.dir.as_os_str().is_empty() {
                "Drives".to_owned()
            } else {
                b.dir.to_string_lossy().into_owned()
            };
            ui.label(RichText::new(title).strong());
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if b.mode == BrowserMode::SaveAs {
            ui.horizontal(|ui| {
                ui.label("Name:");
                let r = ui.add(TextEdit::singleline(&mut b.name).desired_width(300.0));
                if b.focus_name {
                    r.request_focus();
                    b.focus_name = false;
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.button("Save").clicked() || enter)
                    && !b.name.trim().is_empty()
                    && !b.dir.as_os_str().is_empty()
                {
                    open_file = Some(b.dir.join(b.name.trim()));
                }
            });
        }
        if let Some(e) = &b.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        ui.separator();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y + 4.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            row_h,
            b.entries.len(),
            |ui, range| {
                for i in range {
                    let (name, is_dir) = b.entries[i].clone();
                    let label = if is_dir { format!("[{name}]") } else { name.clone() };
                    if ui.selectable_label(false, label).clicked() {
                        if is_dir {
                            nav = Some(if b.dir.as_os_str().is_empty() {
                                PathBuf::from(&name)
                            } else {
                                b.dir.join(&name)
                            });
                        } else if b.mode == BrowserMode::Open {
                            open_file = Some(b.dir.join(&name));
                        } else {
                            b.name = name;
                        }
                    }
                }
            },
        );
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }

        if let Some(d) = nav {
            b.go(d);
        }
        if cancel {
            self.close_browser();
        } else if let Some(p) = open_file {
            match b.mode {
                BrowserMode::Open => self.open_path(p),
                BrowserMode::SaveAs => self.write_tab(self.active, p),
            }
            self.close_browser();
        } else {
            self.browser = Some(b);
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(self.font_size);
        let tab = &mut self.tabs[self.active];
        if tab.tail && user_interacted(ui, ui.available_rect_before_wrap()) {
            tab.tail = false;
        }
        let wrap = tab.word_wrap;
        let hidden = tab.hidden_chars;
        let mut scroll_end = std::mem::take(&mut tab.scroll_to_end);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", tab.id));
        let total = tab.text.bytes().filter(|&b| b == b'\n').count() + 1;
        let gutter_digits = total.to_string().len().max(3);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let gutter_w = char_w * (gutter_digits as f32 + 2.0);
        let avail = ui.available_size();
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let focus = std::mem::take(&mut self.focus_editor);

        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let s = text.as_str();
            let shown: String = if hidden {
                s.chars()
                    .map(|c| match c {
                        ' ' => '·',
                        c => c,
                    })
                    .collect()
            } else {
                s.to_owned()
            };
            let ww = if wrap { wrap_width } else { f32::INFINITY };
            let job = egui::text::LayoutJob::simple(shown, font.clone(), Color32::from_gray(212), ww);
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let mut status = (total, 1, 1);
        let tailing = tab.tail;
        if tailing || wrap {
            tab.col_sel = None;
            tab.col_drag = false;
        }
        if tab.col_sel.is_some() && ui.ctx().memory(|m| m.has_focus(id)) {
            let galley = layouter(ui, &tab.text, f32::INFINITY);
            column_keys(ui.ctx(), tab, &galley);
        }
        let hl = tab.highlight;
        let mut scroll_to = std::mem::take(&mut tab.scroll_to_highlight);
        egui::ScrollArea::both()
            .id_salt(("scroll", tab.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let clip = ui.clip_rect();
                let area = egui::Rect::from_min_max(
                    egui::pos2(clip.left() + gutter_w, clip.top()),
                    clip.max,
                );
                let (pressed, down, alt, pos) = ui.input(|i| {
                    (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.alt, i.pointer.latest_pos())
                });
                let starting_column = !tailing && !wrap && pressed && alt
                    && pos.is_some_and(|p| area.contains(p));
                let column_drag = starting_column || tab.col_drag;
                if column_drag {
                    if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                        if let Some(range) = state.cursor.char_range() {
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(range.primary)));
                            state.store(ui.ctx(), id);
                        }
                    }
                }
                let out = ui
                    .horizontal_top(|ui| {
                        ui.add_space(gutter_w);
                        let width =
                            if wrap { (avail.x - gutter_w - 16.0).max(50.0) } else { f32::INFINITY };
                        let mut ro: &str = &tab.text;
                        let edit = if tailing {
                            TextEdit::multiline(&mut ro)
                        } else {
                            TextEdit::multiline(&mut tab.text)
                        };
                        ui.scope(|ui| {
                            if column_drag {
                                // TextEdit paints its linear drag selection before we can
                                // collapse it; only the rectangular selection should be visible.
                                ui.visuals_mut().selection.bg_fill = Color32::TRANSPARENT;
                                ui.visuals_mut().selection.stroke.color = Color32::from_gray(212);
                            }
                            edit
                                .id(id)
                                .frame(egui::Frame::NONE)
                                .font(font.clone())
                                .lock_focus(true)
                                .desired_width(width)
                                .desired_rows(((avail.y / row_h) as usize).max(1))
                                .layouter(&mut layouter)
                                .show(ui)
                        }).inner
                    })
                    .inner;
                if focus || column_drag {
                    out.response.request_focus();
                }
                if out.response.changed() {
                    tab.col_sel = None;
                    tab.col_drag = false;
                }
                if !tailing && !wrap {
                    let to_lc = |p: egui::Pos2| {
                        let y = p.y - out.galley_pos.y;
                        let line = out.galley.rows.partition_point(|row| row.rect().min.y <= y)
                            .saturating_sub(1)
                            .min(total - 1);
                        let x = p.x - out.galley_pos.x + out.galley.rect.left();
                        let row = &out.galley.rows[line];
                        let local_x = x - row.pos.x;
                        let x = if local_x <= row.size.x {
                            row.pos.x + row.x_offset(row.char_at(local_x))
                        } else {
                            x
                        };
                        (line, x.max(0.0))
                    };
                    if let Some(p) = pos {
                        if pressed && area.contains(p) {
                            if alt {
                                let lc = to_lc(p);
                                tab.col_sel = Some(ColSel { anchor: lc, head: lc });
                                tab.col_drag = true;
                            } else {
                                tab.col_sel = None;
                                tab.col_drag = false;
                            }
                        } else if tab.col_drag && down {
                            if let Some(s) = &mut tab.col_sel {
                                s.head = to_lc(p);
                            }
                        }
                    }
                    if !down {
                        tab.col_drag = false;
                    }
                    // Keep the normal text cursor collapsed so it doesn't show a competing selection.
                    if let Some(s) = tab.col_sel {
                        let col = columns_for_row(&out.galley, s.head.0, s.head.1, s.head.1).0;
                        let idx = char_index_of(&tab.text, s.head.0, col);
                        let mut st = out.state.clone();
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(idx))));
                        st.store(ui.ctx(), id);
                    }
                }
                if scroll_end {
                    let r = out.response.rect;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 1.0), r.max),
                        Some(egui::Align::BOTTOM),
                    );
                    scroll_end = false;
                }

                let galley = &out.galley;
                let painter = ui.painter();
                if hidden {
                    let marker_painter = painter.with_clip_rect(out.text_clip_rect.intersect(clip));
                    let stroke = egui::Stroke::new(1.0, Color32::from_gray(140));
                    for row in &galley.rows {
                        let y = out.galley_pos.y + row.rect().center().y;
                        if y < clip.top() || y > clip.bottom() {
                            continue;
                        }
                        for glyph in &row.glyphs {
                            if glyph.chr != '\t' {
                                continue;
                            }
                            let x = out.galley_pos.x - galley.rect.left() + row.pos.x + glyph.pos.x;
                            let cell = glyph.advance_width / 4.0;
                            for i in 0..3 {
                                let left = x + (i as f32 + 0.2) * cell;
                                marker_painter.line_segment(
                                    [egui::pos2(left, y), egui::pos2(left + 0.6 * cell, y)],
                                    stroke,
                                );
                            }
                            let left = x + 3.15 * cell;
                            let right = x + 3.8 * cell;
                            let tip = egui::pos2(right, y);
                            marker_painter.line_segment([egui::pos2(left, y), tip], stroke);
                            marker_painter.line_segment(
                                [egui::pos2(right - 0.3 * cell, y - 0.25 * cell), tip],
                                stroke,
                            );
                            marker_painter.line_segment(
                                [egui::pos2(right - 0.3 * cell, y + 0.25 * cell), tip],
                                stroke,
                            );
                        }
                    }
                }
                if let Some(s) = &tab.col_sel {
                    let (l0, l1, left, right) = s.bounds();
                    let x0 = out.galley_pos.x - galley.rect.left() + left;
                    let x1 = if right == left { x0 + 2.0 } else { out.galley_pos.x - galley.rect.left() + right };
                    if let (Some(first), Some(last)) = (galley.rows.get(l0), galley.rows.get(l1)) {
                        let y0 = out.galley_pos.y + first.rect().min.y;
                        let y1 = out.galley_pos.y + last.rect().max.y;
                        painter.rect_filled(
                            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
                            0.0,
                            ui.visuals().selection.bg_fill.gamma_multiply(0.8),
                        );
                    }
                }
                let gutter = egui::Rect::from_min_max(
                    egui::pos2(clip.left(), clip.top()),
                    egui::pos2(clip.left() + gutter_w, clip.bottom()),
                );
                painter.rect_filled(gutter, 0.0, ui.visuals().extreme_bg_color);
                let cursor_idx = out.cursor_range.map(|r| usize::from(r.primary.index));
                let mut line = 1usize;
                let mut new_line = true;
                let mut start = 0usize;
                let mut para_start = 0usize;
                let last = galley.rows.len().saturating_sub(1);
                for (ri, row) in galley.rows.iter().enumerate() {
                    let r = row.rect();
                    let y = out.galley_pos.y + r.min.y;
                    if new_line {
                        para_start = start;
                    }
                    let count = usize::from(row.char_count_including_newline());
                    if let Some(ci) = cursor_idx {
                        if ci >= start && (ci < start + count || ri == last) {
                            status = (total, line, ci - para_start + 1);
                        }
                    }
                    start += count;
                    if new_line && y + row_h >= clip.top() && y <= clip.bottom() {
                        painter.text(
                            egui::pos2(clip.left() + gutter_w - char_w, y),
                            egui::Align2::RIGHT_TOP,
                            line.to_string(),
                            font.clone(),
                            Color32::from_gray(110),
                        );
                    }
                    if hidden && row.ends_with_newline && y + row_h >= clip.top() && y <= clip.bottom() {
                        painter.text(
                            egui::pos2(out.galley_pos.x + r.max.x, y),
                            egui::Align2::LEFT_TOP,
                            "¶",
                            font.clone(),
                            Color32::from_gray(90),
                        );
                    }
                    if let Some(h) = hl {
                        if line - 1 == h {
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(clip.left() + gutter_w, y),
                                egui::vec2(clip.width() - gutter_w, row_h),
                            );
                            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(255, 214, 0, 55));
                            if scroll_to {
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                scroll_to = false;
                            }
                        }
                    }
                    new_line = row.ends_with_newline;
                    if new_line {
                        line += 1;
                    }
                }
                if out.response.changed() {
                    tab.dirty = true;
                    tab.highlight = None;
                }
            });
        self.status = status;
    }
}

impl eframe::App for App {
    // egui low-pass filters discrete wheel notches over several frames. Preceding each wheel
    // event with a Start phase makes egui treat it as direct (unsmoothed) input.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
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
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                let (total, l, c) = self.status;
                ui.label(format!("Lines: {total}    Ln: {l}    Col: {c}"));
                if let Some(m) = &self.message {
                    ui.colored_label(Color32::LIGHT_RED, m);
                }
            });
        });
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
    }
}

struct WindowState {
    pos: Option<egui::Pos2>,
    size: egui::Vec2,
    maximized: bool,
}

fn window_file() -> Option<PathBuf> {
    recent_file().map(|f| f.with_file_name("window.txt"))
}

fn load_window() -> Option<WindowState> {
    let s = std::fs::read_to_string(window_file()?).ok()?;
    let v: Vec<f32> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
    if v.len() != 5 || v[2] < 200.0 || v[3] < 150.0 || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    // Reject positions that are clearly off any screen (e.g. minimized sentinel values).
    let pos = (v[0] > -10000.0 && v[1] > -10000.0 && v[0] < 20000.0 && v[1] < 20000.0)
        .then(|| egui::pos2(v[0], v[1]));
    Some(WindowState { pos, size: egui::vec2(v[2], v[3]), maximized: v[4] != 0.0 })
}

fn save_window(w: &WindowState) {
    let Some(f) = window_file() else { return };
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let p = w.pos.unwrap_or(egui::pos2(-99999.0, -99999.0));
    let _ = std::fs::write(
        f,
        format!("{} {} {} {} {}", p.x, p.y, w.size.x, w.size.y, w.maximized as u8),
    );
}

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]).with_title("RustEd");
    let mut centered = true;
    if let Some(w) = load_window() {
        viewport = viewport.with_inner_size(w.size).with_maximized(w.maximized);
        if let Some(p) = w.pos {
            viewport = viewport.with_position(p);
            centered = false;
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        centered,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("rusted", options, Box::new(|cc| Ok(Box::new(App::new(cc, file)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            recent: Vec::new(),
            font_size: DEFAULT_FONT_SIZE,
            browser: None,
            message: None,
            focus_editor: true,
            filter_history: Vec::new(),
            window: WindowState {
                pos: None,
                size: egui::vec2(800.0, 600.0),
                maximized: false,
            },
            last_poll: Instant::now(),
            status: (1, 1, 1),
        };
        app.new_tab(None, "abcdefghij\nabcdefghij\nabcdefghij\nabcdefghij".into(), false, 0);
        app
    }

    fn frame(ctx: &egui::Context, app: &mut App, mut events: Vec<egui::Event>, alt: bool) -> egui::FullOutput {
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() }));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.editor_ui(ui));
        output.textures_delta.clear();
        output
    }

    fn has_linear_selection(output: &egui::FullOutput, color: Color32) -> bool {
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => text.galley.rows.iter().any(|row| {
                row.visuals.mesh.vertices.iter().any(|vertex| vertex.color == color)
            }),
            _ => false,
        })
    }

    #[test]
    fn hidden_tabs_keep_layout_and_draw_four_cell_markers() {
        for font_size in [14.0, 32.0, 60.0] {
            for scale in [1.0, 1.25] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(scale);
                let mut app = test_app();
                app.font_size = font_size;
                app.tabs[0].text = "a\tb\tc\n\tdef".into();
                let normal = frame(&ctx, &mut app, vec![], false);
                app.tabs[0].hidden_chars = true;
                let hidden = frame(&ctx, &mut app, vec![], false);
                let get_galley = |output: &egui::FullOutput| {
                    output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text.galley.clone()),
                        _ => None,
                    }).unwrap()
                };
                let normal_galley = get_galley(&normal);
                let hidden_galley = get_galley(&hidden);
                for (normal_row, hidden_row) in normal_galley.rows.iter().zip(&hidden_galley.rows) {
                    assert_eq!(normal_row.glyphs.len(), hidden_row.glyphs.len());
                    for (normal, hidden) in normal_row.glyphs.iter().zip(&hidden_row.glyphs) {
                        assert_eq!(normal.chr, hidden.chr);
                        assert!((normal.pos.x - hidden.pos.x).abs() < 0.01);
                        assert!((normal.advance_width - hidden.advance_width).abs() < 0.01);
                    }

                }
                let markers = hidden.shapes.iter().filter(|shape| match &shape.shape {
                    egui::Shape::LineSegment { stroke, .. } => stroke.color == Color32::from_gray(140),
                    _ => false,
                }).count();
                assert_eq!(markers, 18, "six strokes for each of three tabs at font size {font_size}");
            }
        }
    }

    #[test]
    fn alt_drag_does_not_draw_linear_selection() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        frame(&ctx, &mut app, vec![], false);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", 0_u64));
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(25),
        )));
        state.store(&ctx, id);

        let color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill;
        let start = egui::pos2(75.0, 10.0);
        let end = egui::pos2(95.0, 55.0);
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        let output = frame(&ctx, &mut app, vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
        ], true);
        assert!(app.tabs[0].col_drag);
        assert!(!has_linear_selection(&output, color));
        let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
        assert!(!has_linear_selection(&output, color));
        let rectangle_color = color.gamma_multiply(0.8);
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == rectangle_color)
        }));
        let selection = app.tabs[0].col_sel.unwrap();
        assert_ne!(selection.anchor.0, selection.head.0);
        assert_ne!(selection.anchor.1, selection.head.1);
        let output = frame(&ctx, &mut app, vec![
            egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
        ], true);
        assert!(!has_linear_selection(&output, color));
        assert!(!app.tabs[0].col_drag);
        assert!(ctx.memory(|m| m.has_focus(id)), "editor lost focus after drag");
        let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
            _ => None,
        }).unwrap();
        let copied = column_text(&app.tabs[0].text, &selection, &galley);
        let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == &copied)
        }));
        let original = app.tabs[0].text.clone();
        let output = frame(&ctx, &mut app, vec![egui::Event::Cut], false);
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == &copied)
        }));
        let mut expected = original;
        delete_column(&mut expected, &selection, &galley);
        assert_eq!(app.tabs[0].text, expected);

        app.tabs[0].col_sel = Some(selection);
        let output = frame(&ctx, &mut app, vec![], false);
        let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
            _ => None,
        }).unwrap();
        delete_column(&mut expected, &selection, &galley);
        frame(&ctx, &mut app, vec![egui::Event::Key {
            key: Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }], false);
        assert_eq!(app.tabs[0].text, expected);
    }

    #[test]
    fn normal_mouse_drag_still_draws_linear_selection() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        frame(&ctx, &mut app, vec![], false);
        let start = egui::pos2(75.0, 10.0);
        let end = egui::pos2(95.0, 55.0);
        let modifiers = egui::Modifiers::NONE;
        frame(&ctx, &mut app, vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers },
        ], false);
        let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], false);
        assert!(has_linear_selection(&output, ctx.style_of(ctx.theme()).visuals.selection.bg_fill));
        assert!(app.tabs[0].col_sel.is_none());
        frame(&ctx, &mut app, vec![
            egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers },
        ], false);
        let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if !text.is_empty())
        }), "ordinary selection was not copied");
    }

    #[test]
    fn column_selection_copies_visible_characters_at_scaled_width() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.25);
        let mut app = test_app();
        app.tabs[0].text = format!("{}\n{}", "0123456789".repeat(12), "abcdefghij".repeat(12));
        let output = frame(&ctx, &mut app, vec![], false);
        let text = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text),
            _ => None,
        }).unwrap();
        let row0 = &text.galley.rows[0];
        let row1 = &text.galley.rows[1];
        let start = text.pos + egui::vec2(row0.pos.x + row0.glyphs[50].pos.x, row0.rect().center().y);
        let end = text.pos + egui::vec2(row1.pos.x + row1.glyphs[55].pos.x, row1.rect().center().y);
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        frame(&ctx, &mut app, vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
        ], true);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
        frame(&ctx, &mut app, vec![
            egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
        ], true);
        let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == "01234\nabcde")
        }), "incorrect block copied: {:?}", output.platform_output.commands);
    }

    #[test]
    fn column_selection_matches_glyph_positions_after_tab() {
        for scale in [1.0, 1.25, 1.5] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut app = test_app();
            app.tabs[0].text = "prefix\tABCDEFG\nprefix\tABCDEFG".into();
            let output = frame(&ctx, &mut app, vec![], false);
            let text = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text),
                _ => None,
            }).unwrap();
            let row0 = &text.galley.rows[0];
            let row1 = &text.galley.rows[1];
            let start = text.pos + egui::vec2(row0.pos.x + row0.glyphs[8].pos.x, row0.rect().center().y);
            let end = text.pos + egui::vec2(row1.pos.x + row1.glyphs[11].pos.x, row1.rect().center().y);
            let alt = egui::Modifiers { alt: true, ..Default::default() };
            frame(&ctx, &mut app, vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
            ], true);
            let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
            let selection_color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill.gamma_multiply(0.8);
            let rect = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == selection_color => Some(rect.rect),
                _ => None,
            }).unwrap();
            assert!((rect.left() - start.x).abs() < 0.01);
            assert!((rect.right() - end.x).abs() < 0.01);
            frame(&ctx, &mut app, vec![
                egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
            ], true);
            let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
            assert!(output.platform_output.commands.iter().any(|command| {
                matches!(command, egui::OutputCommand::CopyText(text) if text == "BCD\nBCD")
            }), "scale {scale}: {:?}", output.platform_output.commands);
        }
    }

    #[test]
    fn column_selection_uses_visible_rows() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.25);
        let mut app = test_app();
        app.tabs[0].text = (0..100).map(|i| format!("{i:03}-abcdefgh")).collect::<Vec<_>>().join("\n");
        let output = frame(&ctx, &mut app, vec![], false);
        let text = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 100 => Some(text),
            _ => None,
        }).unwrap();
        let start = text.pos + text.galley.rows[30].rect().center().to_vec2();
        let end = text.pos + text.galley.rows[32].rect().center().to_vec2() + egui::vec2(12.0, 0.0);
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        frame(&ctx, &mut app, vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
        ], true);
        let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
        let selection = app.tabs[0].col_sel.unwrap();
        assert_eq!((selection.anchor.0, selection.head.0), (30, 32));
        let text = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 100 => Some(text),
            _ => None,
        }).unwrap();
        let selection_color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill.gamma_multiply(0.8);
        let rect = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == selection_color => Some(rect.rect),
            _ => None,
        }).unwrap();
        assert!((rect.top() - (text.pos.y + text.galley.rows[30].rect().top())).abs() < 0.01);
        assert!((rect.bottom() - (text.pos.y + text.galley.rows[32].rect().bottom())).abs() < 0.01);
        frame(&ctx, &mut app, vec![
            egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
        ], true);
        let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
        let (l0, l1, _, _) = selection.bounds();
        let expected = (l0..=l1)
            .map(|line| {
                let (c0, c1) = columns_for_row(&text.galley, line, selection.bounds().2, selection.bounds().3);
                format!("{line:03}-abcdefgh").chars().skip(c0).take(c1 - c0).collect::<String>()
            })
            .collect::<Vec<_>>().join("\n");
        assert!(!expected.is_empty());
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == &expected)
        }));
    }
}
