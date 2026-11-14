use crate::app::*;
use crate::editor::*;
use crate::search::*;
use crate::settings::*;
use eframe::egui::{self, Color32, FontId, Key, RichText, TextEdit};
use std::path::PathBuf;

pub(crate) fn history_menu(ui: &mut egui::Ui, history: &[String], tooltip: &str) -> Option<String> {
    let mut selected = None;
    ui.add_enabled_ui(!history.is_empty(), |ui| {
        let arrow = egui::Atom::paint(egui::vec2(12.0, 12.0), |ui, args| {
            ui.painter().add(egui::Shape::rotated_triangle(
                egui::Rect::from_center_size(args.rect.center(), egui::vec2(8.0, 6.0)),
                0.0,
                args.fallback_text_color,
            ));
        });
        ui.menu_button(arrow, |ui| {
            for entry in history {
                if ui.button(entry).clicked() {
                    selected = Some(entry.clone());
                    ui.close();
                }
            }
        }).response.on_hover_text(tooltip);
    });
    selected
}

pub(crate) struct ToolbarIcons {
    pub(crate) find: egui::TextureHandle,
    pub(crate) wrap: egui::TextureHandle,
    pub(crate) hidden: egui::TextureHandle,
    pub(crate) tail: egui::TextureHandle,
    pub(crate) format_json: egui::TextureHandle,
}

pub(crate) enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
}

pub(crate) fn history_steps(state: &egui::text_edit::TextEditState, text: &str, redo: bool) -> usize {
    let Some(range) = state.cursor.char_range() else { return 0 };
    let mut undoer = state.undoer();
    let mut current = (range, text.to_owned());
    let mut steps = 0;
    loop {
        let next = if redo { undoer.redo(&current) } else { undoer.undo(&current) };
        let Some(next) = next.cloned() else { return 0 };
        steps += 1;
        if next.1 != text {
            return steps;
        }
        current = next;
    }
}

pub(crate) fn clipboard_has_text() -> Result<bool, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    match clipboard.get_text() {
        Ok(text) => Ok(!text.is_empty()),
        Err(arboard::Error::ContentNotAvailable) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) struct PendingEdit {
    pub(crate) tab_id: u64,
    pub(crate) action: EditAction,
    pub(crate) selection: Option<egui::text::CCursorRange>,
    pub(crate) ready: bool,
}

pub(crate) fn load_icon(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Result<egui::TextureHandle, String> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot load {name} icon: {error}"))?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Ok(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

impl ToolbarIcons {
    pub(crate) fn load(ctx: &egui::Context) -> Result<Self, String> {
        Ok(Self {
            find: load_icon(ctx, "find", include_bytes!("assets/find.png"))?,
            wrap: load_icon(ctx, "wordwrap", include_bytes!("assets/wordwrap.png"))?,
            hidden: load_icon(ctx, "hidden", include_bytes!("assets/hidden-characters.png"))?,
            tail: load_icon(ctx, "tail", include_bytes!("assets/tail.png"))?,
            format_json: load_icon(ctx, "format-json", include_bytes!("assets/copy.png"))?,
        })
    }
}

pub(crate) fn toolbar_button(icon: &egui::TextureHandle, label: &str, selected: bool) -> egui::Button<'static> {
    let native = icon.size_vec2();
    let size = native * (20.0 / native.x.max(native.y));
    let image = egui::Image::new((icon.id(), size));
    egui::Button::image_and_text(image, label.to_owned()).selected(selected)
}

// "Live" indicator at the left edge of a tab label: a solid dot with a ring that expands and fades.
pub(crate) fn paint_spinner(ui: &egui::Ui, label: &egui::Response) {
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

impl App {
    pub(crate) fn goto_ui(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.active];
        let mut go = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label("Line:");
            let response = ui.add(TextEdit::singleline(&mut tab.goto_text)
                .id(egui::Id::unique(("goto_input", tab.id))).desired_width(100.0));
            if std::mem::take(&mut tab.focus_goto) {
                response.request_focus();
            }
            go |= response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            go |= ui.button("Go").clicked();
            cancel |= ui.button("Cancel").clicked();
        });
        if go && !tab.goto_text.is_empty() && tab.goto_text.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(line) = tab.goto_text.parse::<usize>() {
                let total = tab.text.bytes().filter(|&b| b == b'\n').count() + 1;
                if (1..=total).contains(&line) {
                    tab.goto_line = Some(line - 1);
                    let offset: usize = tab.text.split_inclusive('\n').take(line - 1).map(|l| l.chars().count()).sum();
                    #[allow(deprecated)]
                    let id = egui::Id::new(("editor", tab.id));
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(offset))));
                    state.store(ui.ctx(), id);
                    tab.col_sel = None;
                    tab.scroll_to_goto = Some(line - 1);
                    tab.scroll_to_highlight = false;
                    tab.scroll_to_end = false;
                    tab.goto_open = false;
                    self.focus_editor = true;
                }
            }
        }
        if cancel {
            tab.goto_open = false;
            self.focus_editor = true;
        }
    }

    pub(crate) fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut search = false;
        let mut replace = false;
        let history = &self.filter_history;
        let replace_history = &self.replace_history;
        let tab = &mut self.tabs[self.active];
        let label_font = egui::TextStyle::Body.resolve(ui.style());
        let label_width = ui.fonts_mut(|fonts| {
            ["Find:", "Replace:"].iter()
                .map(|label| fonts.layout_no_wrap((*label).to_owned(), label_font.clone(), Color32::WHITE).size().x)
                .fold(0.0_f32, f32::max)
        });
        ui.horizontal(|ui| {
            let (_, label_rect) = ui.allocate_space(egui::vec2(label_width, 0.0));
            let r = ui.add(TextEdit::singleline(&mut tab.filter_text).desired_width(320.0));
            let label_rect = egui::Rect::from_min_max(
                egui::pos2(label_rect.left(), r.rect.top()),
                egui::pos2(label_rect.right(), r.rect.bottom()),
            );
            ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(egui::Layout::right_to_left(egui::Align::Center)))
                .label("Find:");
            if std::mem::take(&mut tab.focus_filter) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                search = true;
            }
            if let Some(entry) = history_menu(ui, history, "Find history") {
                tab.filter_text = entry;
                search = true;
            }
            if ui.button("Search").clicked() {
                search = true;
            }
            if ui.checkbox(&mut tab.case_sensitive, "Case sensitive").changed() {
                search = true;
            }
            ui.scope(|ui| {
                let light = self.light_theme;
                let text_color = if light { Color32::from_gray(30) } else { Color32::WHITE };
                let selected = tab.highlight_color.themed(light);
                let widgets = &mut ui.visuals_mut().widgets;
                for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
                    visuals.bg_fill = selected;
                    visuals.weak_bg_fill = selected;
                }
                egui::ComboBox::from_id_salt(("highlight_color", tab.id))
                    .selected_text(RichText::new(tab.highlight_color.label(light)).color(text_color))
                    .show_ui(ui, |ui| {
                        for color in HighlightColor::ALL {
                            egui::Frame::NONE.fill(color.themed(light)).show(ui, |ui| {
                                ui.set_min_width(105.0);
                                ui.visuals_mut().selection.bg_fill = color.themed(light);
                                if ui.selectable_label(tab.highlight_color == color, RichText::new(color.label(light)).color(text_color)).clicked() {
                                    tab.highlight_color = color;
                                    ui.close();
                                }
                            });
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            let (_, label_rect) = ui.allocate_space(egui::vec2(label_width, 0.0));
            let r = ui.add(TextEdit::singleline(&mut tab.replace_text).desired_width(320.0));
            let label_rect = egui::Rect::from_min_max(
                egui::pos2(label_rect.left(), r.rect.top()),
                egui::pos2(label_rect.right(), r.rect.bottom()),
            );
            ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(egui::Layout::right_to_left(egui::Align::Center)))
                .label("Replace:");
            if let Some(entry) = history_menu(ui, replace_history, "Replace history") {
                tab.replace_text = entry;
            }
            replace = ui.add_enabled(!tab.tail, egui::Button::new("Replace all")).clicked();
        });
        if search {
            self.apply_search();
        }
        if replace {
            self.replace_all(ui.ctx());
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

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
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
                if ui.add(egui::Button::new("Find / Replace").shortcut_text("F4 / Ctrl+F")).clicked() {
                    self.toggle_filter();
                    ui.close();
                }
                if ui.add_enabled(self.browser.is_none(), egui::Button::new("Go to line").shortcut_text("Ctrl+G")).clicked() {
                    self.open_goto();
                    ui.close();
                }
            });
            let editable = self.browser.is_none() && !self.tabs[self.active].tail;
            let mut action = None;
            let menu = ui.menu_button("Edit", |ui| {
                let (can_undo, can_redo, selected) = self.edit_availability(ui);
                if !self.edit_menu_open {
                    #[cfg(test)]
                    let status = self.clipboard_test_status.clone().unwrap_or_else(clipboard_has_text);
                    #[cfg(not(test))]
                    let status = clipboard_has_text();
                    self.clipboard_status = status;
                }
                self.edit_menu_open = true;
                for (label, shortcut, enabled, command) in [
                    ("Undo", "Ctrl+Z", editable && can_undo, EditAction::Undo),
                    ("Redo", "Ctrl+Y", editable && can_redo, EditAction::Redo),
                    ("Cut", "Ctrl+X", editable && selected, EditAction::Cut),
                    ("Copy", "Ctrl+C", self.browser.is_none() && selected, EditAction::Copy),
                    ("Paste", "Ctrl+V", editable && matches!(self.clipboard_status, Ok(true)), EditAction::Paste),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                        action = Some(command);
                        ui.close();
                    }
                    if label == "Redo" {
                        ui.separator();
                    }
                }
                if let Err(error) = &self.clipboard_status {
                    ui.colored_label(Color32::LIGHT_RED, format!("Clipboard unavailable: {error}"));
                }
            });
            if menu.inner.is_none() {
                self.edit_menu_open = false;
            }
            if let Some(action) = action {
                let tab_id = self.tabs[self.active].id;
                let selection = self.editor_selection.take()
                    .and_then(|(id, range)| if id == tab_id { Some(range) } else { None });
                self.pending_edit = Some(PendingEdit { tab_id, action, selection, ready: false });
                ui.ctx().request_repaint();
            }
            ui.menu_button("Program", |ui| {
                if ui.button(if self.light_theme { "Dark theme" } else { "Light theme" }).clicked() {
                    self.light_theme = !self.light_theme;
                    ui.ctx().set_visuals(theme_visuals(self.light_theme));
                    save_light_theme(self.light_theme);
                    ui.close();
                }
                if ui.button("About").clicked() {
                    self.about_open = true;
                    ui.close();
                }
            });
        });
    }

    pub(crate) fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let icon = self.about_icon.get_or_insert_with(|| load_icon(ctx, "application", APP_ICON));
        let mut close = false;
        egui::Window::new("About")
            .open(&mut self.about_open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match icon {
                        Ok(icon) => { ui.image((icon.id(), egui::vec2(48.0, 48.0))); }
                        Err(error) => { ui.colored_label(Color32::LIGHT_RED, error.as_str()); }
                    }
                    ui.heading("RustEd");
                });
                ui.label("Version 1.0.0");
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
        if close {
            self.about_open = false;
        }
    }

    pub(crate) fn view_toolbar(&mut self, ui: &mut egui::Ui) {
        let icons = self.toolbar_icons.get_or_insert_with(|| ToolbarIcons::load(ui.ctx()));
        let icons = match icons {
            Ok(icons) => icons,
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, error.as_str());
                return;
            }
        };
        let mut toggle_tail = false;
        let mut toggle_filter = false;
        let mut format_json = false;
        let tab = &mut self.tabs[self.active];
        let is_json = tab.path.as_ref().and_then(|path| path.extension())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        ui.horizontal(|ui| {
            if ui.add_enabled(self.browser.is_none(), toolbar_button(&icons.find, "Find", tab.filter_open))
                .on_hover_text("Find (F4 / Ctrl+F)")
                .clicked()
            {
                toggle_filter = true;
            }
            if ui.add(toolbar_button(&icons.wrap, "Word wrap", tab.word_wrap))
                .on_hover_text("Toggle word wrap (Ctrl+W)")
                .clicked()
            {
                tab.word_wrap = !tab.word_wrap;
            }
            if ui.add(toolbar_button(&icons.hidden, "Hidden characters", tab.hidden_chars))
                .on_hover_text("Toggle hidden characters (Ctrl+E)")
                .clicked()
            {
                tab.hidden_chars = !tab.hidden_chars;
            }
            if ui.add(toolbar_button(&icons.tail, "Tail", tab.tail))
                .on_hover_text("Toggle file tailing (Ctrl+T)")
                .clicked()
            {
                toggle_tail = true;
            }
            if is_json && ui.add_enabled(
                self.browser.is_none() && !tab.tail,
                toolbar_button(&icons.format_json, "Format JSON", false),
            ).on_hover_text("Format JSON with two-space indentation").clicked() {
                format_json = true;
            }
        });
        if toggle_filter {
            self.toggle_filter();
        }
        if toggle_tail {
            self.toggle_tail();
        }
        if format_json {
            self.format_current_json(ui.ctx());
        }
    }

    pub(crate) fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut select = None;
            let mut close = None;
            let mut close_others = None;
            for (i, t) in self.tabs.iter().enumerate() {
                let title = if t.tail {
                    // Leave room for the spinner painted over the start of the label.
                    let body = egui::TextStyle::Body.resolve(ui.style());
                    let space_w = ui.fonts_mut(|f| f.glyph_width(&body, ' ')).max(1.0);
                    let n = (body.size / space_w).ceil() as usize + 1;
                    format!("{}{}", " ".repeat(n), t.tab_title())
                } else {
                    t.tab_title()
                };
                let r = ui.selectable_label(self.browser.is_none() && i == self.active, title)
                    .on_hover_text(t.tooltip());
                if t.tail {
                    paint_spinner(ui, &r);
                }
                egui::Popup::context_menu(&r).id(ui.make_persistent_id(("tab_context", t.id))).show(|ui| {
                    if ui.button("Close").clicked() {
                        close = Some(i);
                        ui.close();
                    }
                    if ui.button("Close all others").clicked() {
                        close_others = Some(i);
                        ui.close();
                    }
                    if ui.add_enabled(t.path.is_some(), egui::Button::new("Copy file path")).clicked() {
                        if let Some(path) = &t.path {
                            ui.ctx().copy_text(path.to_string_lossy().into_owned());
                        }
                        ui.close();
                    }
                });
                if r.clicked() {
                    select = Some(i);
                }
                if r.middle_clicked() {
                    close = Some(i);
                }
            }
            if let Some(i) = close {
                self.close_tab(i);
            } else if let Some(i) = close_others {
                let keep = self.tabs.remove(i);
                self.tabs.clear();
                self.tabs.push(keep);
                self.active = 0;
                self.browser = None;
                self.focus_editor = true;
            } else if let Some(i) = select {
                self.select_tab(i);
            }
        });
    }

    pub(crate) fn browser_ui(&mut self, ui: &mut egui::Ui) {
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
}
