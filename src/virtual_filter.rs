use crate::document::MAX_PREVIEW_LINE_BYTES;
use crate::search::FilterState;
use crate::sparse_heights::SparseHeights;
use crate::toolbar::filter_result_label;
use eframe::egui::{self, Color32, FontId, RichText};

#[derive(PartialEq)]
struct WrapKey {
    count: usize,
    width: f32,
    font: FontId,
    hidden: bool,
    padding: egui::Vec2,
    spacing: f32,
    min_height: f32,
    pixels_per_point: f32,
}

pub(crate) struct PreviewFilterWrapCache {
    key: WrapKey,
    heights: SparseHeights,
    anchor: usize,
}

#[cfg(test)]
impl PreviewFilterWrapCache {
    pub(crate) fn measured_count(&self) -> usize {
        self.heights.measured_count()
    }
}

fn result_label(line: usize, text: &str, terminated: bool, hidden: bool) -> String {
    let mut end = text.len().min(MAX_PREVIEW_LINE_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < text.len();
    let mut label = filter_result_label(line, &text[..end], terminated && !truncated, hidden);
    if truncated {
        label.push('…');
    }
    label
}

pub(crate) fn show_wrapped_results(
    ui: &mut egui::Ui, filter: &mut FilterState, tab_id: u64,
    tail: bool, font: &FontId, hidden: bool,
) -> Option<usize> {
    let mut jump = None;
    let results = &filter.results;
    let selected = &mut filter.selected;
    egui::ScrollArea::vertical()
        .id_salt(("filter_results", tab_id))
        .horizontal_scroll_offset(0.0)
        .auto_shrink([false, false])
        .stick_to_bottom(tail)
        .animated(false)
        .show_viewport(ui, |ui, viewport| {
            let width = ui.available_width();
            let padding = ui.spacing().button_padding;
            let spacing = ui.spacing().item_spacing.y;
            let min_height = ui.spacing().interact_size.y;
            let key = WrapKey {
                count: results.len(), width, font: font.clone(), hidden, padding,
                spacing, min_height, pixels_per_point: ui.ctx().pixels_per_point(),
            };
            let changed = filter.preview_wrap_cache.as_ref().is_none_or(|cache| cache.key != key);
            let anchor = filter.preview_wrap_cache.as_ref().map_or(0, |cache| cache.anchor);
            if changed {
                filter.preview_wrap_cache = Some(PreviewFilterWrapCache {
                    key,
                    heights: SparseHeights::new(results.len(), f64::from(min_height + spacing)),
                    anchor: anchor.min(results.len().saturating_sub(1)),
                });
            }
            let cache = filter.preview_wrap_cache.as_mut().expect("filter wrap cache was initialized");
            let origin = ui.min_rect().min;
            if changed && tail {
                let mut measured_height = 0.0;
                for i in (0..results.len()).rev() {
                    let (line, text, terminated) = &results[i];
                    let label = result_label(*line, text, *terminated, hidden);
                    let galley = ui.fonts_mut(|fonts| fonts.layout(
                        label, font.clone(), Color32::WHITE, (width - 2.0 * padding.x).max(0.0),
                    ));
                    let height = (galley.size().y + 2.0 * padding.y).max(min_height) + spacing;
                    cache.heights.set(i, f64::from(height));
                    measured_height += height;
                    if measured_height >= viewport.height() + min_height {
                        break;
                    }
                }
            }
            let first = cache.heights.index_at(f64::from(viewport.min.y.max(0.0)));
            if let Some(first) = first {
                cache.anchor = first;
                for i in first..results.len() {
                    let y = cache.heights.prefix(i) as f32;
                    if y > viewport.max.y + min_height {
                        break;
                    }
                    let (line, text, terminated) = &results[i];
                    let label = RichText::new(result_label(*line, text, *terminated, hidden)).font(font.clone());
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x, origin.y + y), egui::vec2(width, viewport.height()),
                    );
                    let mut row_ui = ui.new_child(egui::UiBuilder::new()
                        .id_salt(("wrapped_filter_row", i))
                        .max_rect(rect)
                        .layout(egui::Layout::top_down_justified(egui::Align::LEFT)));
                    row_ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    let response = row_ui.selectable_label(*selected == Some(i), label);
                    cache.heights.set(i, f64::from(response.rect.height() + spacing));
                    if response.clicked() {
                        *selected = Some(i);
                    }
                    if response.double_clicked() {
                        *selected = Some(i);
                        jump = Some(*line);
                    }
                }
            }
            let height = (cache.heights.prefix(results.len()) as f32 - spacing).max(0.0);
            ui.set_height(height);
            if changed && tail && !results.is_empty() {
                ui.scroll_to_rect(egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + height), egui::Vec2::ZERO,
                ), Some(egui::Align::BOTTOM));
            } else if changed && anchor > 0 && !results.is_empty() {
                let y = cache.heights.prefix(anchor.min(results.len() - 1)) as f32;
                ui.scroll_to_rect(egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + y), egui::Vec2::ZERO,
                ), Some(egui::Align::TOP));
            }
        });
    jump
}
