use crate::app::App;
use crate::document::DocumentText;
use eframe::egui::{self, Color32, FontId, Key, Modifiers};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct VirtualPreviewState {
    pub(crate) anchor: usize,
    pub(crate) was_wrapped: bool,
    wrap: Option<WrapIndex>,
}

#[derive(PartialEq)]
struct WrapKey {
    revision: u64,
    lines: usize,
    width: f32,
    font: FontId,
    pixels_per_point: f32,
}

struct WrapIndex {
    key: WrapKey,
    heights: HashMap<usize, usize>,
    // Unknown lines contribute one row; the sparse Fenwick tree stores measured extra rows.
    tree: HashMap<usize, usize>,
}

impl WrapIndex {
    fn new(key: WrapKey) -> Self {
        Self { key, heights: HashMap::new(), tree: HashMap::new() }
    }

    fn set_rows(&mut self, line: usize, rows: usize) {
        let rows = rows.max(1);
        let old = self.heights.insert(line, rows).unwrap_or(1);
        if old == rows {
            return;
        }
        let mut i = line + 1;
        while i <= self.key.lines {
            let value = self.tree.entry(i).or_default();
            *value = *value + rows - old;
            i += i & i.wrapping_neg();
        }
    }

    fn prefix(&self, lines: usize) -> usize {
        let mut i = lines;
        let mut rows = lines;
        while i > 0 {
            rows += self.tree.get(&i).copied().unwrap_or(0);
            i &= i - 1;
        }
        rows
    }

    fn line_at(&self, row: usize) -> usize {
        let mut bit = 1usize;
        while bit <= self.key.lines / 2 {
            bit *= 2;
        }
        let (mut index, mut rows) = (0, 0);
        while bit != 0 {
            let next = index + bit;
            if next <= self.key.lines {
                let candidate = rows + (next & next.wrapping_neg()) + self.tree.get(&next).copied().unwrap_or(0);
                if candidate <= row {
                    index = next;
                    rows = candidate;
                }
            }
            bit /= 2;
        }
        index.min(self.key.lines - 1)
    }
}

fn layout_line(ui: &mut egui::Ui, text: &str, truncated: bool, font: &FontId, width: f32, color: Color32) -> Arc<egui::Galley> {
    let shown = if truncated { format!("{text}…") } else { text.to_owned() };
    ui.fonts_mut(|fonts| fonts.layout(shown, font.clone(), color, width))
}

fn measure_last_page(
    ui: &mut egui::Ui, text: &mut DocumentText, index: &mut WrapIndex,
    font: &FontId, width: f32, color: Color32, rows_needed: usize,
) {
    let mut measured_rows = 0;
    for line in (0..index.key.lines).rev() {
        let (text, truncated) = text.preview_lines_from(line).expect("indexed line").next().expect("indexed line");
        let galley = layout_line(ui, text, truncated, font, width, color);
        let rows = galley.rows.len().max(1);
        index.set_rows(line, rows);
        measured_rows += rows;
        if measured_rows >= rows_needed {
            break;
        }
    }
}

impl App {
    pub(crate) fn wrapped_preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Wrapped scrollbar positions are estimated until lines have been viewed.");
        let tab = &mut self.tabs[self.active];
        let total = tab.text.line_count();
        let font = FontId::monospace(self.font_size);
        // Galleys round row heights to device pixels; the raw font metric does not.
        let row_height = ui.fonts_mut(|fonts| fonts.layout_no_wrap(" ".to_owned(), font.clone(), Color32::TRANSPARENT).size().y);
        let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let gutter_width = char_width * (total.to_string().len().max(3) as f32 + 2.0);
        let area_height = ui.available_height();
        let width = (ui.available_width() - gutter_width - ui.spacing().scroll.allocated_width()).max(1.0);
        let color = if self.light_theme { Color32::from_gray(50) } else { Color32::from_gray(212) };
        let key = WrapKey {
            revision: tab.text.revision(), lines: total, width, font: font.clone(),
            pixels_per_point: ui.ctx().pixels_per_point(),
        };
        let changed = !tab.preview.was_wrapped || tab.preview.wrap.as_ref().is_none_or(|index| index.key != key);
        if changed {
            tab.preview.wrap = Some(WrapIndex::new(key));
        }

        tab.preview.was_wrapped = true;
        let index = tab.preview.wrap.as_mut().expect("wrap index was initialized");
        let mut target_offset = if changed {
            Some(index.prefix(tab.preview.anchor.min(total - 1)) as f32 * row_height)
        } else {
            None
        };
        let focus_id = egui::Id::unique(("virtual_preview", tab.id));
        let focused = ui.memory(|memory| memory.has_focus(focus_id));
        let mut delta = 0.0;
        let mut end = std::mem::take(&mut tab.scroll_to_end);
        if focused {
            ui.input_mut(|input| {
                if input.consume_key(Modifiers::NONE, Key::ArrowUp) { delta += row_height; }
                if input.consume_key(Modifiers::NONE, Key::ArrowDown) { delta -= row_height; }
                if input.consume_key(Modifiers::NONE, Key::PageUp) { delta += area_height; }
                if input.consume_key(Modifiers::NONE, Key::PageDown) { delta -= area_height; }
                if input.consume_key(Modifiers::COMMAND, Key::Home) { target_offset = Some(0.0); }
                if input.consume_key(Modifiers::COMMAND, Key::End) { end = true; }
            });
        }
        let mut jump = None;
        if std::mem::take(&mut tab.scroll_to_highlight) {
            jump = tab.highlight;
        }
        if let Some(line) = tab.scroll_to_goto.take() {
            jump = Some(line);
        }
        let page_rows = (area_height / row_height).ceil() as usize + 1;
        if end || jump.is_some_and(|line| line >= total.saturating_sub(page_rows)) {
            measure_last_page(ui, &mut tab.text, index, &font, width, color, page_rows);
        }
        if end {
            target_offset = Some((index.prefix(total) as f32 * row_height - area_height).max(0.0));
        }
        if let Some(line) = jump {
            target_offset = Some(index.prefix(line.min(total - 1)) as f32 * row_height);
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("wrapped_virtual_scroll", tab.id))
            .content_margin(0)
            .auto_shrink([false, false])
            .animated(false);
        if let Some(offset) = target_offset {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let focus = std::mem::take(&mut self.focus_editor);
        let mut first_visible = 0;
        scroll.show_viewport(ui, |ui, viewport| {
            let row = (viewport.min.y / row_height).max(0.0).floor() as usize;
            let first = index.line_at(row);
            first_visible = first;
            let mut rows = Vec::new();
            if let Some(lines) = tab.text.preview_lines_from(first) {
                for (offset, (text, truncated)) in lines.enumerate() {
                    let line = first + offset;
                    let y = index.prefix(line) as f32 * row_height;
                    if y > viewport.max.y + row_height {
                        break;
                    }
                    let galley = layout_line(ui, text, truncated, &font, width, color);
                    index.set_rows(line, galley.rows.len());
                    rows.push((line, y, galley));
                }
            }
            ui.set_min_size(egui::vec2(ui.available_width(), index.prefix(total) as f32 * row_height));
            let origin = ui.min_rect().min;
            let clip = ui.clip_rect();
            let text_clip = egui::Rect::from_min_max(
                egui::pos2((clip.left() + gutter_width).min(clip.right()), clip.top()), clip.max,
            );
            let painter = ui.painter();
            let text_painter = painter.with_clip_rect(text_clip);
            for (_, y, galley) in &rows {
                text_painter.galley(egui::pos2(origin.x + gutter_width, origin.y + y), galley.clone(), color);
            }
            let gutter = egui::Rect::from_min_max(
                clip.min, egui::pos2((clip.left() + gutter_width).min(clip.right()), clip.bottom()),
            );
            painter.rect_filled(gutter, 0.0, ui.visuals().extreme_bg_color);
            for (line, y, _) in rows {
                painter.text(
                    egui::pos2(gutter.right() - char_width, origin.y + y),
                    egui::Align2::RIGHT_TOP, (line + 1).to_string(), font.clone(), Color32::from_gray(110),
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
        tab.preview.anchor = first_visible;
        self.status = (total, first_visible + 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_wrap_index_maps_rows_after_height_changes() {
        let mut index = WrapIndex::new(WrapKey {
            revision: 1, lines: 257, width: 100.0,
            font: FontId::monospace(14.0), pixels_per_point: 1.0,
        });
        let mut heights = vec![1; 257];
        for (line, height) in [(0, 4), (12, 8), (255, 3), (12, 2), (256, 6)] {
            heights[line] = height;
            index.set_rows(line, height);
            let mut row = 0;
            for (line, height) in heights.iter().copied().enumerate() {
                assert_eq!(index.prefix(line), row);
                for current in row..row + height {
                    assert_eq!(index.line_at(current), line);
                }
                row += height;
            }
            assert_eq!(index.prefix(heights.len()), row);
        }
    }
}
