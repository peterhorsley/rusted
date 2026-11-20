use crate::app::*;
use crate::search::*;
use crate::toolbar::*;
use eframe::egui::{self, Color32, FontId, Key, Modifiers, TextEdit};
use std::sync::{Arc, Weak};

struct RowInfo {
    line: usize,
    start: usize,
    paragraph_start: usize,
    new_line: bool,
}

#[derive(Default)]
pub(crate) struct RowIndex {
    galley: Weak<egui::Galley>,
    rows: Vec<RowInfo>,
}

impl RowIndex {
    fn update(&mut self, galley: &Arc<egui::Galley>) {
        if self.galley.upgrade().is_some_and(|old| Arc::ptr_eq(&old, galley)) {
            return;
        }
        self.rows.clear();
        let (mut line, mut start, mut paragraph_start, mut new_line) = (0, 0, 0, true);
        for row in &galley.rows {
            if new_line {
                paragraph_start = start;
            }
            self.rows.push(RowInfo { line, start, paragraph_start, new_line });
            start += usize::from(row.char_count_including_newline());
            new_line = row.ends_with_newline;
            line += usize::from(new_line);
        }
        self.galley = Arc::downgrade(galley);
    }

    fn row_for_line(&self, line: usize) -> Option<usize> {
        let index = self.rows.partition_point(|row| row.line < line);
        self.rows.get(index).filter(|row| row.line == line).map(|_| index)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColSel {
    // Line index and horizontal position relative to the text galley.
    pub(crate) anchor: (usize, f32),
    pub(crate) head: (usize, f32),
}

impl ColSel {
    pub(crate) fn bounds(&self) -> (usize, usize, f32, f32) {
        (
            self.anchor.0.min(self.head.0),
            self.anchor.0.max(self.head.0),
            self.anchor.1.min(self.head.1),
            self.anchor.1.max(self.head.1),
        )
    }
}

pub(crate) fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map(|(b, _)| b).unwrap_or(line.len())
}

pub(crate) fn columns_for_row(galley: &egui::Galley, row: usize, x0: f32, x1: f32) -> (usize, usize) {
    let Some(row) = galley.rows.get(row) else { return (0, 0) };
    (
        usize::from(row.char_at(x0 - row.pos.x)),
        usize::from(row.char_at(x1 - row.pos.x)),
    )
}

pub(crate) fn column_text(text: &str, s: &ColSel, galley: &egui::Galley) -> String {
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

pub(crate) fn delete_column(text: &mut String, s: &ColSel, galley: &egui::Galley) {
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

pub(crate) fn char_index_of(text: &str, line: usize, col: usize) -> usize {
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
pub(crate) fn column_keys(ctx: &egui::Context, tab: &mut Tab, galley: &egui::Galley) {
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
        tab.search_highlights.edited(&tab.text);
        let (l0, l1, x0, _) = sel.bounds();
        tab.col_sel = Some(ColSel { anchor: (l0, x0), head: (l1, x0) });
        tab.dirty = tab.text != tab.saved_text;
        tab.highlight = None;
    } else if clear {
        tab.col_sel = None;
    }
}

// True when the user pressed the mouse or scrolled the wheel over `rect`.
pub(crate) fn user_interacted(ui: &egui::Ui, rect: egui::Rect) -> bool {
    let (pos, interacted) = ui.input(|i| {
        let Some(pos) = i.pointer.hover_pos() else { return (None, false) };
        (Some(pos), i.pointer.primary_pressed()
            || (!i.modifiers.command
                && i.raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))))
    });
    interacted && pos.is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()))
}

impl App {
    pub(crate) fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(self.font_size);
        let tab = &mut self.tabs[self.active];
        let wrap = tab.word_wrap;
        let hidden = tab.hidden_chars;
        let scroll_end = std::mem::take(&mut tab.scroll_to_end);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", tab.id));
        if self.pending_edit.as_ref().is_some_and(|pending| pending.tab_id != tab.id) {
            self.pending_edit = None;
        }
        if self.pending_edit.as_ref().is_some_and(|pending| pending.ready) {
            let pending = self.pending_edit.take().unwrap();
            if let Some(range) = pending.selection {
                if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                    state.cursor.set_char_range(Some(range));
                    state.store(ui.ctx(), id);
                }
            }
            ui.memory_mut(|memory| memory.request_focus(id));
            match pending.action {
                EditAction::Paste => ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste),
                EditAction::Copy => ui.input_mut(|input| input.events.push(egui::Event::Copy)),
                EditAction::Cut => ui.input_mut(|input| input.events.push(egui::Event::Cut)),
                EditAction::Undo | EditAction::Redo => {
                    let key = if matches!(pending.action, EditAction::Undo) { Key::Z } else { Key::Y };
                    ui.input_mut(|input| input.events.push(egui::Event::Key {
                        key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND,
                    }));
                }
            }
        }
        if ui.memory(|memory| memory.has_focus(id)) && !tab.tail {
            let redo_z = ui.input_mut(|input| input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z));
            let redo_y = ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Y));
            let undo = ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Z));
            if undo || redo_y || redo_z {
                let redo = redo_y || redo_z;
                let steps = egui::text_edit::TextEditState::load(ui.ctx(), id)
                    .map(|state| history_steps(&state, &tab.text, redo))
                    .unwrap_or(0);
                ui.input_mut(|input| {
                    for _ in 0..steps {
                        input.events.push(egui::Event::Key {
                            key: if redo { Key::Y } else { Key::Z },
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::COMMAND,
                        });
                    }
                });
            }
        }
        let total = tab.text.bytes().filter(|&b| b == b'\n').count() + 1;
        let gutter_digits = total.to_string().len().max(3);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let gutter_w = char_w * (gutter_digits as f32 + 2.0);
        let avail = ui.available_size();
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let focus = std::mem::take(&mut self.focus_editor);

        let mut highlights = std::mem::take(&mut tab.search_highlights);
        let hl = tab.highlight;
        let light = self.light_theme;
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let ww = if wrap { wrap_width } else { f32::INFINITY };
            let selected_line = if highlights.active && text.as_str() != highlights.snapshot { None } else { hl };
            let job = search_layout(text.as_str(), &highlights, selected_line, hidden, &font, ww, light);
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
        let scroll_to = std::mem::take(&mut tab.scroll_to_highlight);
        let mut goto_target = tab.scroll_to_goto.take();
        let scroll_area = egui::ScrollArea::both()
            .id_salt(("scroll", tab.id))
            .auto_shrink([false, false]);
        let scroll_area = if scroll_end { scroll_area.horizontal_scroll_offset(0.0).animated(false) } else { scroll_area };
        scroll_area.show(ui, |ui| {
                let clip = ui.clip_rect();
                let area = egui::Rect::from_min_max(
                    egui::pos2(clip.left() + gutter_w, clip.top()),
                    clip.max,
                );
                let (pressed, down, alt, pos) = ui.input(|i| {
                    (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.alt, i.pointer.latest_pos())
                });
                let over_editor = pos.is_some_and(|p| {
                    area.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
                });
                let starting_column = !tailing && !wrap && pressed && alt
                    && over_editor;
                let column_drag = starting_column || tab.col_drag;
                if column_drag {
                    if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                        if let Some(range) = state.cursor.char_range() {
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(range.primary)));
                            state.store(ui.ctx(), id);
                        }
                    }
                }
                let background = ui.painter().add(egui::Shape::Noop);
                let column_background = ui.painter().add(egui::Shape::Noop);
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
                        if pressed && over_editor {
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
                        egui::Rect::from_min_max(
                            egui::pos2(clip.left(), r.bottom() - 1.0),
                            egui::pos2(clip.right(), r.bottom()),
                        ),
                        None,
                    );
                }

                let galley = &out.galley;
                tab.row_index.update(galley);
                let first_visible = galley.rows.partition_point(|row| {
                    out.galley_pos.y + row.rect().max.y < clip.top()
                });
                let end_visible = galley.rows.partition_point(|row| {
                    out.galley_pos.y + row.rect().min.y <= clip.bottom()
                });
                let visible = first_visible..end_visible.max(first_visible);
                let painter = ui.painter();
                if tab.highlight_color != HighlightColor::DarkGray
                    && highlights.active && tab.text == highlights.snapshot
                {
                    let mut shapes = Vec::new();
                    for ri in visible.clone() {
                        let row = &galley.rows[ri];
                        let line = tab.row_index.rows[ri].line;
                        let y = out.galley_pos.y + row.rect().min.y;
                        if !highlights.lines.get(line).is_none_or(Vec::is_empty)
                            && y + row.rect().height() >= clip.top() && y <= clip.bottom()
                        {
                            let rect = egui::Rect::from_min_max(
                                egui::pos2(clip.left() + gutter_w, y),
                                egui::pos2(clip.right(), y + row.rect().height()),
                            );
                            shapes.push(egui::Shape::rect_filled(rect, 0.0, tab.highlight_color.themed(light)));
                        }
                    }
                    painter.set(background, egui::Shape::Vec(shapes));
                }
                if hidden {
                    let marker_painter = painter.with_clip_rect(out.text_clip_rect.intersect(clip));
                    let stroke = egui::Stroke::new(1.0, Color32::from_gray(140));
                    for row in &galley.rows[visible.clone()] {
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
                        // Painted behind the text, like the normal selection.
                        painter.set(column_background, egui::Shape::rect_filled(
                            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
                            0.0,
                            ui.visuals().selection.bg_fill,
                        ));
                    }
                }
                let gutter = egui::Rect::from_min_max(
                    egui::pos2(clip.left(), clip.top()),
                    egui::pos2(clip.left() + gutter_w, clip.bottom()),
                );
                painter.rect_filled(gutter, 0.0, ui.visuals().extreme_bg_color);
                let cursor_idx = out.cursor_range.map(|r| usize::from(r.primary.index));
                if let Some(ci) = cursor_idx {
                    let ri = tab.row_index.rows.partition_point(|row| row.start <= ci).saturating_sub(1);
                    if let Some(info) = tab.row_index.rows.get(ri) {
                        status = (total, info.line + 1, ci.saturating_sub(info.paragraph_start) + 1);
                    }
                }
                if let Some(target) = goto_target.take() {
                    if let Some(ri) = tab.row_index.row_for_line(target) {
                        let y = out.galley_pos.y + galley.rows[ri].rect().min.y;
                        if y < clip.top() || y + row_h > clip.bottom() {
                            ui.scroll_with_delta(egui::vec2(0.0, clip.center().y - (y + row_h / 2.0)));
                        }
                    }
                }
                if scroll_to {
                    if let Some(ri) = hl.and_then(|line| tab.row_index.row_for_line(line)) {
                        let y = out.galley_pos.y + galley.rows[ri].rect().min.y;
                        ui.scroll_with_delta(egui::vec2(0.0, clip.center().y - (y + row_h / 2.0)));
                    }
                }
                for ri in visible {
                    let row = &galley.rows[ri];
                    let info = &tab.row_index.rows[ri];
                    let line = info.line + 1;
                    let r = row.rect();
                    let y = out.galley_pos.y + r.min.y;
                    if info.new_line && y + row_h >= clip.top() && y <= clip.bottom() {
                        let target = tab.goto_line == Some(line - 1);
                        if target {
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(clip.left(), y), egui::vec2(gutter_w, row_h)),
                                0.0,
                                Color32::from_gray(70),
                            );
                        }
                        painter.text(
                            egui::pos2(clip.left() + gutter_w - char_w, y),
                            egui::Align2::RIGHT_TOP,
                            line.to_string(),
                            font.clone(),
                            if target { Color32::WHITE } else { Color32::from_gray(110) },
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
                }
                if out.response.changed() {
                    tab.dirty = tab.text != tab.saved_text;
                    tab.highlight = None;
                    tab.filter.applied = None;
                }
            });
        if highlights.active && highlights.snapshot != tab.text {
            highlights.edited(&tab.text);
            ui.ctx().request_repaint();
        }
        if hl != tab.highlight {
            ui.ctx().request_repaint();
        }
        tab.search_highlights = highlights;
        self.status = status;
        if ui.memory(|memory| memory.has_focus(id)) {
            self.editor_selection = egui::text_edit::TextEditState::load(ui.ctx(), id)
                .and_then(|state| state.cursor.char_range())
                .map(|range| (tab.id, range));
        }
        if let Some(pending) = &mut self.pending_edit {
            if pending.tab_id == tab.id && !pending.ready {
                pending.ready = true;
                ui.memory_mut(|memory| memory.request_focus(id));
                ui.ctx().request_repaint();
            }
        }
    }
}
