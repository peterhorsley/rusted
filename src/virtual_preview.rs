use crate::app::App;
use eframe::egui::{self, Color32, FontId, Key, Modifiers};

impl App {
    pub(crate) fn virtual_preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Virtual preview: read-only, no wrap, selection, search highlighting or hidden markers.");
        ui.label("Lines longer than 2048 bytes are truncated in this preview.");

        let tab = &mut self.tabs[self.active];
        let total = tab.text.line_count();
        let font = FontId::monospace(self.font_size);
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let gutter_width = char_width * (total.to_string().len().max(3) as f32 + 2.0);
        let area_height = ui.available_height();
        let focus_id = egui::Id::unique(("virtual_preview", tab.id));
        let focused = ui.memory(|memory| memory.has_focus(focus_id));
        let mut delta = 0.0;
        let mut target_offset = None;
        if focused {
            ui.input_mut(|input| {
                if input.consume_key(Modifiers::NONE, Key::ArrowUp) { delta += row_height; }
                if input.consume_key(Modifiers::NONE, Key::ArrowDown) { delta -= row_height; }
                if input.consume_key(Modifiers::NONE, Key::PageUp) { delta += area_height; }
                if input.consume_key(Modifiers::NONE, Key::PageDown) { delta -= area_height; }
                if input.consume_key(Modifiers::COMMAND, Key::Home) { target_offset = Some(0.0); }
                if input.consume_key(Modifiers::COMMAND, Key::End) {
                    target_offset = Some((total as f32 * row_height - area_height).max(0.0));
                }
            });
        }
        let scroll_end = std::mem::take(&mut tab.scroll_to_end);
        if scroll_end {
            target_offset = Some((total as f32 * row_height - area_height).max(0.0));
        }
        if std::mem::take(&mut tab.scroll_to_highlight) {
            if let Some(line) = tab.highlight {
                target_offset = Some((line as f32 * row_height - area_height / 2.0).max(0.0));
            }
        }
        if let Some(line) = tab.scroll_to_goto.take() {
            target_offset = Some((line as f32 * row_height - area_height / 2.0).max(0.0));
        }
        let mut scroll = egui::ScrollArea::both()
            .id_salt(("virtual_scroll", tab.id))
            .auto_shrink([false, false])
            .animated(false);
        if let Some(offset) = target_offset {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        if scroll_end {
            scroll = scroll.horizontal_scroll_offset(0.0);
        }
        let focus = std::mem::take(&mut self.focus_editor);
        let width_id = egui::Id::unique(("virtual_width", tab.id));
        let color = if self.light_theme { Color32::from_gray(50) } else { Color32::from_gray(212) };
        let mut first_visible = 0;
        scroll.show_viewport(ui, |ui, viewport| {
            let first = ((viewport.min.y / row_height).floor() as usize).min(total - 1);
            let end = ((viewport.max.y / row_height).ceil() as usize + 1).min(total);
            first_visible = first;
            let mut max_width = ui.ctx().data_mut(|data| data.get_temp::<f32>(width_id).unwrap_or(0.0));
            let mut rows = Vec::new();
            if let Some(lines) = tab.text.preview_lines_from(first) {
                for (line, (text, truncated)) in lines.take(end.saturating_sub(first)).enumerate() {
                    let shown = if truncated {
                        format!("{text}…")
                    } else {
                        text.to_owned()
                    };
                    let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(shown, font.clone(), color));
                    max_width = max_width.max(galley.size().x);
                    rows.push((first + line, galley));
                }
            }
            ui.ctx().data_mut(|data| data.insert_temp(width_id, max_width));
            ui.set_min_size(egui::vec2(
                (gutter_width + max_width).max(ui.available_width()),
                total as f32 * row_height,
            ));
            let origin = ui.min_rect().min;
            let clip = ui.clip_rect();
            let text_clip = egui::Rect::from_min_max(
                egui::pos2((clip.left() + gutter_width).min(clip.right()), clip.top()), clip.max,
            );
            let painter = ui.painter();
            let text_painter = painter.with_clip_rect(text_clip);
            for (line, galley) in rows {
                let y = origin.y + line as f32 * row_height;
                text_painter.galley(egui::pos2(origin.x + gutter_width, y), galley, color);
            }
            let gutter = egui::Rect::from_min_max(
                clip.min, egui::pos2((clip.left() + gutter_width).min(clip.right()), clip.bottom()),
            );
            painter.rect_filled(gutter, 0.0, ui.visuals().extreme_bg_color);
            for line in first..end {
                let y = origin.y + line as f32 * row_height;
                painter.text(
                    egui::pos2(gutter.right() - char_width, y),
                    egui::Align2::RIGHT_TOP,
                    (line + 1).to_string(),
                    font.clone(),
                    Color32::from_gray(110),
                );
            }
            let response = ui.interact(clip, focus_id, egui::Sense::click());
            if response.clicked() || focus {
                response.request_focus();
            }
            if delta != 0.0 {
                ui.scroll_with_delta(egui::vec2(0.0, delta));
            }
        });
        self.status = (total, first_visible + 1, 1);
    }
}
