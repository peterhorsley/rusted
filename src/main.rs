#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, TextEdit};
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 10;
const DEFAULT_FONT_SIZE: f32 = 14.0;

struct Tab {
    id: u64,
    path: Option<PathBuf>,
    text: String,
    crlf: bool,
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
    word_wrap: bool,
    hidden_chars: bool,
    font_size: f32,
    browser: Option<Browser>,
    message: Option<String>,
    focus_editor: bool,
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

impl App {
    fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            recent: load_recent(),
            word_wrap: false,
            hidden_chars: false,
            font_size: DEFAULT_FONT_SIZE,
            browser: None,
            message: None,
            focus_editor: true,
            status: (1, 1, 1),
        };
        if let Some(p) = file {
            app.open_path(p);
        }
        if app.tabs.is_empty() {
            app.new_tab(None, String::new(), false);
        }
        app
    }

    fn new_tab(&mut self, path: Option<PathBuf>, text: String, crlf: bool) {
        self.tabs.push(Tab { id: self.next_id, path, text, crlf });
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
        let (text, crlf) = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let s = String::from_utf8_lossy(&bytes).into_owned();
                    if s.contains("\r\n") { (s.replace("\r\n", "\n"), true) } else { (s, false) }
                }
                Err(e) => {
                    self.message = Some(format!("Cannot open {}: {e}", path.display()));
                    return;
                }
            }
        } else {
            (String::new(), false)
        };
        // Replace a pristine empty untitled tab instead of stacking on it.
        if self.tabs.len() == 1 && self.tabs[0].path.is_none() && self.tabs[0].text.is_empty() {
            self.tabs.clear();
        }
        self.add_recent(path.clone());
        self.new_tab(Some(path), text, crlf);
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
        match std::fs::write(&path, data) {
            Ok(()) => {
                tab.path = Some(path.clone());
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

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
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
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.word_wrap, "Word wrap");
                ui.checkbox(&mut self.hidden_chars, "Hidden characters");
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut select = None;
            for (i, t) in self.tabs.iter().enumerate() {
                let mut r = ui.selectable_label(self.browser.is_none() && i == self.active, t.title());
                if let Some(p) = &t.path {
                    r = r.on_hover_text(p.to_string_lossy());
                }
                if r.clicked() {
                    select = Some(i);
                }
            }
            if let Some(i) = select {
                self.active = i;
                self.browser = None;
                self.focus_editor = true;
            }
        });
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
        let wrap = self.word_wrap;
        let hidden = self.hidden_chars;
        let tab = &mut self.tabs[self.active];
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
                        '\t' => '→',
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
        egui::ScrollArea::both()
            .id_salt(("scroll", tab.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let clip = ui.clip_rect();
                let out = ui
                    .horizontal_top(|ui| {
                        ui.add_space(gutter_w);
                        let width =
                            if wrap { (avail.x - gutter_w - 16.0).max(50.0) } else { f32::INFINITY };
                        TextEdit::multiline(&mut tab.text)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .font(font.clone())
                            .lock_focus(true)
                            .desired_width(width)
                            .desired_rows(((avail.y / row_h) as usize).max(1))
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner;
                if focus {
                    out.response.request_focus();
                }

                let galley = &out.galley;
                let painter = ui.painter();
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
                    new_line = row.ends_with_newline;
                    if new_line {
                        line += 1;
                    }
                }
            });
        self.status = status;
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
            self.open_browser();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save();
        }
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            self.font_size = (self.font_size * zoom).clamp(6.0, 96.0);
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
        egui::CentralPanel::default().show(root, |ui| {
            if self.browser.is_some() {
                self.browser_ui(ui);
            } else {
                self.editor_ui(ui);
            }
        });
    }
}

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]).with_title("rusted"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("rusted", options, Box::new(|cc| Ok(Box::new(App::new(cc, file)))))
}
