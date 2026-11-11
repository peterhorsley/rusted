#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, TextEdit};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_RECENT: usize = 10;
const DEFAULT_FONT_SIZE: f32 = 14.0;
const APP_ICON: &[u8] = include_bytes!("assets/app.png");

const MAX_FILTER_HISTORY: usize = 10;
const TAIL_POLL: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum HighlightColor {
    DarkGray,
    DarkRed,
    #[default]
    DarkGreen,
    DarkBlue,
    DarkPurple,
}

impl HighlightColor {
    const ALL: [Self; 5] = [Self::DarkGray, Self::DarkRed, Self::DarkGreen, Self::DarkBlue, Self::DarkPurple];

    fn name(self) -> &'static str {
        match self {
            Self::DarkGray => "Dark Gray",
            Self::DarkRed => "Dark Red",
            Self::DarkGreen => "Dark Green",
            Self::DarkBlue => "Dark Blue",
            Self::DarkPurple => "Dark Purple",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::DarkGray => Color32::from_rgb(27, 27, 27),
            Self::DarkRed => Color32::from_rgb(85, 30, 30),
            Self::DarkGreen => Color32::from_rgb(25, 70, 40),
            Self::DarkBlue => Color32::from_rgb(25, 45, 80),
            Self::DarkPurple => Color32::from_rgb(65, 35, 75),
        }
    }
}

fn match_ranges(line: &str, pattern: &str, case: bool) -> Vec<std::ops::Range<usize>> {
    if pattern.is_empty() {
        return Vec::new();
    }
    if case {
        return line.match_indices(pattern).map(|(start, s)| start..start + s.len()).collect();
    }
    let lower = line.to_lowercase();
    let needle = pattern.to_lowercase();
    let mut boundaries = Vec::new();
    let mut offset = 0;
    for (index, ch) in line.char_indices() {
        let next = offset + ch.to_lowercase().map(char::len_utf8).sum::<usize>();
        boundaries.push((offset, next, index, index + ch.len_utf8()));
        offset = next;
    }
    lower.match_indices(&needle).filter_map(|(start, s)| {
        let end = start + s.len();
        let first = boundaries.get(boundaries.partition_point(|(_, to, _, _)| *to <= start))?;
        let last = boundaries.get(boundaries.partition_point(|(_, to, _, _)| *to < end))?;
        Some(first.2..last.3)
    }).collect()
}

fn search_layout(text: &str, highlights: &SearchHighlights, selected_line: Option<usize>, hidden: bool, font: &FontId, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let append = |job: &mut egui::text::LayoutJob, s: &str, color| {
        let shown = if hidden { s.replace(' ', "·") } else { s.to_owned() };
        job.append(&shown, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
    };
    let matches_current_text = highlights.active && text == highlights.snapshot;
    if !matches_current_text && selected_line.is_none() {
        append(&mut job, text, Color32::from_gray(212));
        return job;
    }
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if selected_line == Some(i) {
            append(&mut job, line, Color32::YELLOW);
            continue;
        }
        let content = line.strip_suffix('\n').unwrap_or(line);
        let mut start = 0;
        if let Some(ranges) = matches_current_text.then(|| highlights.lines.get(i)).flatten() {
            for range in ranges {
                if range.start < start {
                    continue;
                }
                append(&mut job, &content[start..range.start], Color32::from_gray(212));
                append(&mut job, &content[range.clone()], Color32::WHITE);
                start = range.end;
            }
        }
        append(&mut job, &line[start..], Color32::from_gray(212));
    }
    if text.is_empty() {
        append(&mut job, "", Color32::from_gray(212));
    }
    job
}

#[derive(Default)]
struct SearchHighlights {
    snapshot: String,
    lines: Vec<Vec<std::ops::Range<usize>>>,
    active: bool,
}

impl SearchHighlights {
    fn search(&mut self, text: &str, pattern: &str, case: bool) {
        self.active = !pattern.is_empty();
        if self.active {
            self.snapshot = text.to_owned();
            self.lines = text.split('\n').map(|line| match_ranges(line, pattern, case)).collect();
        } else {
            self.snapshot.clear();
            self.lines.clear();
        }
    }

    fn edited(&mut self, text: &str) {
        if !self.active || self.snapshot == text {
            return;
        }
        let old: Vec<_> = self.snapshot.split('\n').collect();
        let new: Vec<_> = text.split('\n').collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b).count();
        let end = self.lines.len().saturating_sub(suffix);
        self.lines.splice(prefix..end, std::iter::repeat_with(Vec::new).take(new.len() - prefix - suffix));
        self.snapshot = text.to_owned();
    }

    fn tailed(&mut self, text: &str, pattern: &str, case: bool) {
        if !self.active {
            return;
        }
        if !text.starts_with(&self.snapshot) {
            self.search(text, pattern, case);
            return;
        }
        let start = self.snapshot.rfind('\n').map_or(0, |i| i + 1);
        let line = self.snapshot[..start].bytes().filter(|b| *b == b'\n').count();
        self.lines.truncate(line);
        self.lines.extend(text[start..].split('\n').map(|s| match_ranges(s, pattern, case)));
        self.snapshot = text.to_owned();
    }
}

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
        tab.search_highlights.edited(&tab.text);
        let (l0, l1, x0, _) = sel.bounds();
        tab.col_sel = Some(ColSel { anchor: (l0, x0), head: (l1, x0) });
        tab.dirty = tab.text != tab.saved_text;
        tab.highlight = None;
    } else if clear {
        tab.col_sel = None;
    }
}

struct Tab {
    id: u64,
    path: Option<PathBuf>,
    text: String,
    saved_text: String,
    crlf: bool,
    // Bytes of the file on disk that are reflected in `text`.
    disk_len: u64,
    dirty: bool,
    filter: FilterState,
    search_highlights: SearchHighlights,
    highlight_color: HighlightColor,
    highlight: Option<usize>,
    scroll_to_highlight: bool,
    goto_open: bool,
    goto_text: String,
    focus_goto: bool,
    goto_line: Option<usize>,
    scroll_to_goto: Option<usize>,
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

    fn tab_title(&self) -> String {
        let title = self.title();
        if self.path.is_some() && self.dirty { format!("{title}*") } else { title }
    }

    fn tooltip(&self) -> String {
        let name = self.path.as_ref().map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.title());
        let prefix = if self.tail { "Tailing " } else { "" };
        let suffix = if self.dirty { " (unsaved)" } else { "" };
        format!("{prefix}{name}{suffix}")
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

struct ToolbarIcons {
    find: egui::TextureHandle,
    wrap: egui::TextureHandle,
    hidden: egui::TextureHandle,
    tail: egui::TextureHandle,
    format_json: egui::TextureHandle,
}

enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
}

fn history_steps(state: &egui::text_edit::TextEditState, text: &str, redo: bool) -> usize {
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

fn clipboard_has_text() -> Result<bool, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    match clipboard.get_text() {
        Ok(text) => Ok(!text.is_empty()),
        Err(arboard::Error::ContentNotAvailable) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

struct PendingEdit {
    tab_id: u64,
    action: EditAction,
    selection: Option<egui::text::CCursorRange>,
    ready: bool,
}

fn load_icon(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Result<egui::TextureHandle, String> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot load {name} icon: {error}"))?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Ok(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

impl ToolbarIcons {
    fn load(ctx: &egui::Context) -> Result<Self, String> {
        Ok(Self {
            find: load_icon(ctx, "find", include_bytes!("assets/find.png"))?,
            wrap: load_icon(ctx, "wordwrap", include_bytes!("assets/wordwrap.png"))?,
            hidden: load_icon(ctx, "hidden", include_bytes!("assets/hidden-characters.png"))?,
            tail: load_icon(ctx, "tail", include_bytes!("assets/tail.png"))?,
            format_json: load_icon(ctx, "format-json", include_bytes!("assets/copy.png"))?,
        })
    }
}

fn toolbar_button(icon: &egui::TextureHandle, label: &str, selected: bool) -> egui::Button<'static> {
    let native = icon.size_vec2();
    let size = native * (20.0 / native.x.max(native.y));
    let image = egui::Image::new((icon.id(), size));
    egui::Button::image_and_text(image, label.to_owned()).selected(selected)
}

fn format_json(text: &str) -> Result<String, serde_json::Error> {
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
    toolbar_icons: Option<Result<ToolbarIcons, String>>,
    about_open: bool,
    about_icon: Option<Result<egui::TextureHandle, String>>,
    editor_selection: Option<(u64, egui::text::CCursorRange)>,
    pending_edit: Option<PendingEdit>,
    edit_menu_open: bool,
    clipboard_status: Result<bool, String>,
    #[cfg(test)]
    clipboard_test_status: Option<Result<bool, String>>,
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
    let (pos, interacted) = ui.input(|i| {
        let Some(pos) = i.pointer.hover_pos() else { return (None, false) };
        (Some(pos), i.pointer.primary_pressed()
            || (!i.modifiers.command
                && i.raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))))
    });
    interacted && pos.is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()))
}

// Replaces the tab's contents with the file on disk, discarding unsaved edits.
fn reload_from_disk(tab: &mut Tab, path: &Path) -> bool {
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
    let s = if tab.crlf { s.replace("\r\n", "\n") } else { s };
    tab.text.push_str(&s);
    tab.saved_text.push_str(&s);
    tab.disk_len += buf.len() as u64;
    true
}

impl App {
    fn toggle_tail(&mut self) {
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

    fn view_shortcuts(&mut self, ctx: &egui::Context) {
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

    fn goto_shortcut(&mut self, ctx: &egui::Context) {
        if self.browser.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::G)) {
            self.open_goto();
        }
    }

    fn open_goto(&mut self) {
        let tab = &mut self.tabs[self.active];
        tab.goto_open = true;
        tab.goto_text.clear();
        tab.focus_goto = true;
        self.focus_editor = false;
    }

    fn goto_ui(&mut self, ui: &mut egui::Ui) {
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

    fn tab_shortcuts(&mut self, ctx: &egui::Context) {
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

    fn select_tab(&mut self, index: usize) {
        self.active = index;
        self.browser = None;
        self.focus_editor = true;
    }

    fn edit_availability(&self, ui: &egui::Ui) -> (bool, bool, bool) {
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
            toolbar_icons: None,
            about_open: false,
            about_icon: None,
            editor_selection: None,
            pending_edit: None,
            edit_menu_open: false,
            clipboard_status: Ok(false),
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

    fn open_dropped_files(&mut self, files: Vec<egui::DroppedFileHandle>) {
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

    fn add_recent(&mut self, path: PathBuf) {
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

    fn write_tab(&mut self, idx: usize, path: PathBuf) {
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

    fn format_current_json(&mut self, ctx: &egui::Context) {
        let tab = &mut self.tabs[self.active];
        match format_json(&tab.text) {
            Ok(formatted) => {
                self.message = None;
                if formatted == tab.text {
                    return;
                }
                #[allow(deprecated)]
                let id = egui::Id::new(("editor", tab.id));
                let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
                let old_range = state.cursor.char_range()
                    .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
                let cursor = usize::from(old_range.primary.index).min(formatted.chars().count());
                let new_range = egui::text::CCursorRange::one(egui::text::CCursor::new(cursor));
                let mut undoer = state.undoer();
                undoer.add_undo(&(old_range, tab.text.clone()));
                undoer.feed_state(ctx.input(|input| input.time), &(new_range, formatted.clone()));
                undoer.add_undo(&(new_range, formatted.clone()));
                state.set_undoer(undoer);
                state.cursor.set_char_range(Some(new_range));
                state.store(ctx, id);
                tab.text = formatted;
                tab.search_highlights.edited(&tab.text);
                tab.dirty = tab.text != tab.saved_text;
                tab.highlight = None;
                tab.col_sel = None;
                tab.col_drag = false;
                tab.filter.applied = None;
                self.editor_selection = Some((tab.id, new_range));
                self.focus_editor = true;
            }
            Err(error) => self.message = Some(format!("Cannot format JSON: {error}")),
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
        tab.search_highlights.search(&tab.text, &tab.applied.0, tab.applied.1);
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

    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut search = false;
        let history = self.filter_history.clone();
        let tab = &mut self.tabs[self.active];
        ui.horizontal(|ui| {
            ui.label("Find:");
            let r = ui.add(TextEdit::singleline(&mut tab.filter_text).desired_width(320.0));
            if std::mem::take(&mut tab.focus_filter) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                search = true;
            }
            ui.add_enabled_ui(!history.is_empty(), |ui| {
                let arrow = egui::Atom::paint(egui::vec2(12.0, 12.0), |ui, args| {
                    ui.painter().add(egui::Shape::rotated_triangle(
                        egui::Rect::from_center_size(args.rect.center(), egui::vec2(8.0, 6.0)),
                        0.0,
                        args.fallback_text_color,
                    ));
                });
                ui.menu_button(arrow, |ui| {
                    for h in history {
                        if ui.button(&h).clicked() {
                            tab.filter_text = h;
                            search = true;
                            ui.close();
                        }
                    }
                }).response.on_hover_text("Find history");
            });
            if ui.button("Search").clicked() {
                search = true;
            }
            if ui.checkbox(&mut tab.case_sensitive, "Case sensitive").changed() {
                search = true;
            }
            ui.scope(|ui| {
                let selected = tab.highlight_color.color();
                let widgets = &mut ui.visuals_mut().widgets;
                for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
                    visuals.bg_fill = selected;
                    visuals.weak_bg_fill = selected;
                }
                egui::ComboBox::from_id_salt(("highlight_color", tab.id))
                    .selected_text(RichText::new(tab.highlight_color.name()).color(Color32::WHITE))
                    .show_ui(ui, |ui| {
                        for color in HighlightColor::ALL {
                            egui::Frame::NONE.fill(color.color()).show(ui, |ui| {
                                ui.set_min_width(105.0);
                                ui.visuals_mut().selection.bg_fill = color.color();
                                if ui.selectable_label(tab.highlight_color == color, RichText::new(color.name()).color(Color32::WHITE)).clicked() {
                                    tab.highlight_color = color;
                                    ui.close();
                                }
                            });
                        }
                    });
            });
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
                if ui.add(egui::Button::new("Find").shortcut_text("F4 / Ctrl+F")).clicked() {
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
                if ui.button("About").clicked() {
                    self.about_open = true;
                    ui.close();
                }
            });
        });
    }

    fn about_ui(&mut self, ctx: &egui::Context) {
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

    fn view_toolbar(&mut self, ui: &mut egui::Ui) {
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

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
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
                ui.push_id(t.id, |ui| {
                    let r = ui.selectable_label(self.browser.is_none() && i == self.active, title)
                        .on_hover_text(t.tooltip());
                    if t.tail {
                        paint_spinner(ui, &r);
                    }
                    r.context_menu(|ui| {
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
                });
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
        let wrap = tab.word_wrap;
        let hidden = tab.hidden_chars;
        let mut scroll_end = std::mem::take(&mut tab.scroll_to_end);
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
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let ww = if wrap { wrap_width } else { f32::INFINITY };
            let selected_line = if highlights.active && text.as_str() != highlights.snapshot { None } else { hl };
            let job = search_layout(text.as_str(), &highlights, selected_line, hidden, &font, ww);
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
        let mut scroll_to = std::mem::take(&mut tab.scroll_to_highlight);
        let mut goto_target = tab.scroll_to_goto.take();
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
                        egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 1.0), r.max),
                        Some(egui::Align::BOTTOM),
                    );
                    scroll_end = false;
                }

                let galley = &out.galley;
                let painter = ui.painter();
                if tab.highlight_color != HighlightColor::DarkGray
                    && highlights.active && tab.text == highlights.snapshot
                {
                    let mut line = 0usize;
                    let mut shapes = Vec::new();
                    for row in &galley.rows {
                        let y = out.galley_pos.y + row.rect().min.y;
                        if !highlights.lines.get(line).is_none_or(Vec::is_empty)
                            && y + row.rect().height() >= clip.top() && y <= clip.bottom()
                        {
                            let rect = egui::Rect::from_min_max(
                                egui::pos2(clip.left() + gutter_w, y),
                                egui::pos2(clip.right(), y + row.rect().height()),
                            );
                            shapes.push(egui::Shape::rect_filled(rect, 0.0, tab.highlight_color.color()));
                        }
                        if row.ends_with_newline {
                            line += 1;
                        }
                    }
                    painter.set(background, egui::Shape::Vec(shapes));
                }
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
                            if tab.goto_line == Some(line - 1) { Color32::WHITE } else { Color32::from_gray(110) },
                        );
                    }
                    if new_line && goto_target == Some(line - 1) {
                        if y < clip.top() || y + row_h > clip.bottom() {
                            ui.scroll_with_delta(egui::vec2(0.0, clip.center().y - (y + row_h / 2.0)));
                        }
                        goto_target = None;
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
                            if scroll_to {
                                ui.scroll_with_delta(egui::vec2(0.0, clip.center().y - rect.center().y));
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
    let icon = eframe::icon_data::from_png_bytes(APP_ICON).expect("Cannot load application icon");
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]).with_title("RustEd").with_icon(icon).with_drag_and_drop(true);
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
            toolbar_icons: None,
            about_open: false,
            about_icon: None,
            editor_selection: None,
            pending_edit: None,
            edit_menu_open: false,
            clipboard_status: Ok(false),
            clipboard_test_status: Some(Ok(true)),
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

    #[derive(Debug)]
    struct TestDrop(PathBuf);

    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|error| error.to_string())
        }
    }

    #[test]
    fn dropped_files_open_once_in_tabs_and_preserve_existing_edits() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "unsaved".into();
        app.tabs[0].dirty = true;
        app.browser = Some(Browser {
            mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
            name: String::new(), error: None, focus_name: false,
        });
        let dir = std::env::temp_dir();
        let id = format!("rusted-drop-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let first = dir.join(format!("{id}-1.txt"));
        let second = dir.join(format!("{id}-2.txt"));
        std::fs::write(&first, "first\r\nline").unwrap();
        std::fs::write(&second, "second").unwrap();
        let mut input = egui::RawInput::default();
        input.dropped_files = vec![std::sync::Arc::new(TestDrop(first.clone())), std::sync::Arc::new(TestDrop(second.clone()))];
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert!(input.dropped_files.is_empty());
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.tabs[0].text, "unsaved");
        assert_eq!(app.tabs[1].text, "first\nline");
        assert_eq!(app.tabs[2].text, "second");
        assert_eq!(app.active, 2);
        assert!(app.browser.is_none());
        assert_eq!(app.recent[0], second);
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(app.tabs.len(), 3);
        app.tabs[1].text.push_str(" modified");
        app.tabs[1].dirty = true;
        input.dropped_files.push(std::sync::Arc::new(TestDrop(first.clone())));
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active, 1);
        assert_eq!(app.tabs[1].text, "first\nline modified");
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn dropped_invalid_paths_are_reported_without_hiding_successes() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let missing = std::env::temp_dir().join(format!(
            "rusted-drop-missing-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let valid = missing.with_extension("txt");
        std::fs::write(&valid, "valid").unwrap();
        let mut input = egui::RawInput::default();
        input.dropped_files = vec![
            std::sync::Arc::new(TestDrop(missing.clone())),
            std::sync::Arc::new(TestDrop(std::env::temp_dir())),
            std::sync::Arc::new(TestDrop(PathBuf::new())),
            std::sync::Arc::new(TestDrop(valid.clone())),
        ];
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(app.tabs[app.active].text, "valid");
        let message = app.message.as_deref().unwrap();
        assert!(message.contains(&missing.to_string_lossy().to_string()));
        assert!(message.contains("not a regular file"));
        assert!(message.contains("no local path"));
        std::fs::remove_file(valid).unwrap();
    }

    #[test]
    fn dropped_file_replaces_empty_untitled_tab() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text.clear();
        let path = std::env::temp_dir().join(format!(
            "rusted-drop-empty-{}-{}.txt", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::write(&path, "content").unwrap();
        let mut input = egui::RawInput::default();
        input.dropped_files.push(std::sync::Arc::new(TestDrop(path.clone())));
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].text, "content");
        std::fs::remove_file(path).unwrap();
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

    fn filter_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.filter_ui(ui));
        output.textures_delta.clear();
        output
    }

    #[test]
    fn double_click_search_result_preserves_horizontal_scroll() {
        for font_size in [14.0, 30.0] {
            for horizontal in [0.0, 120.0] {
                let ctx = egui::Context::default();
                ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
                let mut app = test_app();
                app.font_size = font_size;
                let long_line = "abcdefghij".repeat(80);
                app.tabs[0].text = (0..100).map(|i| {
                    if i == 70 { format!("needle {long_line}") } else { format!("line {i} {long_line}") }
                }).collect::<Vec<_>>().join("\n");
                app.tabs[0].filter_open = true;
                app.tabs[0].filter_text = "needle".into();
                app.apply_search();
                app.tabs[0].highlight_color = HighlightColor::DarkRed;
                let mut time = 0.0;
                let mut run = |app: &mut App, events: Vec<egui::Event>| {
                    time += 0.05;
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    };
                    let mut offset = egui::Vec2::ZERO;
                    let mut editor_pos = None;
                    let mut output = ctx.run_ui(input, |ui| {
                        egui::Panel::bottom("test_filter").exact_size(150.0).show(ui, |ui| app.filter_ui(ui));
                        egui::CentralPanel::default().show(ui, |ui| {
                            editor_pos = Some(ui.min_rect().min);
                            app.editor_ui(ui);
                        });
                    });
                    if let Some(pos) = editor_pos {
                        let text = output.shapes.iter().find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t),
                            _ => None,
                        }).unwrap();
                        offset = egui::vec2(text.pos.x - pos.x, text.pos.y - pos.y);
                    }
                    output.textures_delta.clear();
                    (output, offset)
                };
                if horizontal > 0.0 {
                    // Set the offset using the actual editor scroll-area ID in the same panel.
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                        ..Default::default()
                    };
                    let mut output = ctx.run_ui(input, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            egui::ScrollArea::both().id_salt(("scroll", app.tabs[0].id))
                                .horizontal_scroll_offset(horizontal).show(ui, |ui| ui.set_min_size(egui::vec2(8000.0, 3000.0)));
                        });
                    });
                    output.textures_delta.clear();
                }
                let (output, before) = run(&mut app, vec![]);
                assert!(before.x.is_finite(), "{before:?}");
                if horizontal > 0.0 {
                    assert!(before.x < -20.0, "horizontal scroll was not established: {before:?}");
                }
                let label = format!("{:>6}  needle {long_line}", 71);
                let pos = output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + egui::vec2(60.0, t.galley.rect.center().y)),
                    _ => None,
                }).unwrap();
                let button = |pressed| egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                };
                run(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
                run(&mut app, vec![button(false)]);
                run(&mut app, vec![button(true)]);
                let (_, after) = run(&mut app, vec![button(false)]);
                let (_, settled) = run(&mut app, vec![]);
                let (output, settled2) = run(&mut app, vec![]);
                assert_eq!(app.tabs[0].highlight, Some(70));
                let text = output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t),
                    _ => None,
                }).unwrap();
                assert!(text.galley.job.sections.iter().any(|s| {
                    s.format.color == Color32::YELLOW
                        && text.galley.job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)].starts_with("needle ")
                }));
                assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == Color32::from_rgba_unmultiplied(255, 214, 0, 55))));
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())))));
                assert!((after.x - before.x).abs() < 0.1, "{before:?} -> {after:?}");
                assert!((settled.x - before.x).abs() < 0.1, "{before:?} -> {settled:?}");
                assert!((settled2.x - before.x).abs() < 0.1, "{before:?} -> {settled2:?}");
                assert!(settled2.y < before.y - 100.0, "{before:?} -> {after:?} -> {settled:?} -> {settled2:?}");
            }
        }
    }

    #[test]
    fn find_history_arrow_is_drawn_and_history_remains_selectable() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].filter_open = true;
        app.filter_history.push("history entry".into());
        let output = filter_frame(&ctx, &mut app, vec![]);
        let triangle = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Path(path) if path.closed && path.points.len() == 3 => Some(path),
            _ => None,
        }).expect("find history arrow was not painted");
        let rect = egui::Rect::from_points(&triangle.points);
        assert!(rect.width() > 0.0 && rect.height() > 0.0);
        assert!(triangle.points.iter().any(|point| point.y > rect.center().y));
        let pos = rect.center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };
        filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        filter_frame(&ctx, &mut app, vec![button(pos, false)]);
        let output = filter_frame(&ctx, &mut app, vec![]);
        let pos = find_label(&output, "history entry");
        filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        filter_frame(&ctx, &mut app, vec![button(pos, false)]);
        assert_eq!(app.tabs[0].filter_text, "history entry");
        assert_eq!(app.tabs[0].applied.0, "history entry");
    }

    #[test]
    fn search_highlighting_tracks_color_searches_and_editing() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "alpha one\nbeta alpha\nother".into();
        app.tabs[0].filter_text = "alpha".into();
        app.apply_search();
        assert_eq!(app.tabs[0].highlight_color, HighlightColor::DarkGreen);
        assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 1, 0]);
        let output = frame(&ctx, &mut app, vec![], false);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().filter(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color())).count() == 2)));
        app.tabs[0].highlight_color = HighlightColor::DarkRed;
        let output = frame(&ctx, &mut app, vec![], false);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().filter(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())).count() == 2)));
        let text = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text.contains("alpha one") => Some(t),
            _ => None,
        }).unwrap();
        assert!(text.galley.job.sections.iter().any(|s| s.format.color == Color32::WHITE && &text.galley.job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)] == "alpha"));
        app.tabs[0].highlight_color = HighlightColor::DarkGray;
        let output = frame(&ctx, &mut app, vec![], false);
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())))));
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color())))));
        app.tabs[0].text = "alpha changed\nbeta alpha\nother".into();
        let tab = &mut app.tabs[0];
        tab.search_highlights.edited(&tab.text);
        assert!(app.tabs[0].search_highlights.lines[0].is_empty());
        assert!(!app.tabs[0].search_highlights.lines[1].is_empty());
        app.tabs[0].filter_text = "other".into();
        app.apply_search();
        assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 1]);
    }

    #[test]
    fn search_highlighting_handles_unicode_wrap_and_tail_updates() {
        assert_eq!(match_ranges("İx İX", "i\u{307}x", false), vec![0..3, 4..7]);
        assert_eq!(match_ranges("Alpha alpha", "alpha", true), vec![6..11]);
        let mut highlights = SearchHighlights::default();
        highlights.search("hit\nother\nhit", "hit", false);
        highlights.edited("inserted\nhit\nother\nhit");
        assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 1, 0, 1]);
        highlights.edited("inserted\nmodified hit\nother\nhit");
        assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 0, 1]);
        highlights.tailed("inserted\nmodified hit\nother\nhit\nHIT", "hit", false);
        assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 0, 1, 1]);
        let font = FontId::monospace(14.0);
        let job = search_layout("inserted\nmodified hit\nother\nhit\nHIT", &highlights, None, true, &font, 60.0);
        assert_eq!(job.wrap.max_width, 60.0);
        assert!(job.sections.iter().any(|s| s.format.color == Color32::WHITE && &job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)] == "HIT"));
        assert!(job.text.contains("modified·hit"));
    }

    #[test]
    fn selected_search_line_uses_yellow_text_without_overriding_other_matches() {
        let mut highlights = SearchHighlights::default();
        let text = "hit first\nhit second\nother";
        highlights.search(text, "hit", false);
        for hidden in [false, true] {
            for width in [60.0, f32::INFINITY] {
                let job = search_layout(text, &highlights, Some(1), hidden, &FontId::monospace(14.0), width);
                let sections: Vec<_> = job.sections.iter().map(|s| (
                    s.format.color,
                    &job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)],
                )).collect();
                assert!(sections.iter().any(|(color, text)| *color == Color32::WHITE && *text == "hit"));
                assert!(sections.iter().any(|(color, text)| *color == Color32::YELLOW && text.starts_with("hit")));
                assert!(!sections.iter().any(|(color, text)| *color == Color32::WHITE && text.contains("second")));
            }
        }
        highlights.active = false;
        let job = search_layout(text, &highlights, Some(1), false, &FontId::monospace(14.0), f32::INFINITY);
        assert!(job.sections.iter().any(|s| s.format.color == Color32::YELLOW));
    }

    #[test]
    fn highlight_color_selector_changes_current_matches() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].filter_text = "abc".into();
        app.apply_search();
        let output = filter_frame(&ctx, &mut app, vec![]);
        let pos = find_label(&output, "Dark Green");
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color() && r.rect.contains(pos))));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };
        filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        filter_frame(&ctx, &mut app, vec![button(pos, false)]);
        let output = filter_frame(&ctx, &mut app, vec![]);
        for color in HighlightColor::ALL {
            find_label(&output, color.name());
        }
        let pos = find_label(&output, "Dark Blue");
        filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        filter_frame(&ctx, &mut app, vec![button(pos, false)]);
        assert_eq!(app.tabs[0].highlight_color, HighlightColor::DarkBlue);
        assert_eq!(app.tabs[0].search_highlights.lines.len(), 4);
        let output = filter_frame(&ctx, &mut app, vec![]);
        let pos = find_label(&output, "Dark Blue");
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkBlue.color() && r.rect.contains(pos))));
        filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        filter_frame(&ctx, &mut app, vec![button(pos, false)]);
        let mut output = filter_frame(&ctx, &mut app, vec![]);
        for _ in 0..30 {
            output = filter_frame(&ctx, &mut app, vec![]);
        }
        for color in HighlightColor::ALL {
            let texts: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == color.name() => Some(t.pos + t.galley.rect.center().to_vec2()),
                _ => None,
            }).collect();
            assert_eq!(texts.len(), if color == HighlightColor::DarkBlue { 2 } else { 1 });
            for pos in texts {
                let fills: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.rect.contains(pos) && r.fill != Color32::TRANSPARENT => Some(r.fill),
                    _ => None,
                }).collect();
                assert_eq!(fills.last(), Some(&color.color()), "{}: {fills:?}", color.name());
            }
        }
    }

    #[test]
    fn actual_editor_edits_clear_only_the_edited_line_highlight() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "hit\nhit\nhit".into();
        app.tabs[0].filter_text = "hit".into();
        app.apply_search();
        frame(&ctx, &mut app, vec![], false);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", app.tabs[0].id));
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(5))));
        state.store(&ctx, id);
        frame(&ctx, &mut app, vec![egui::Event::Text("x".into())], false);
        assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 0, 1]);
    }

    #[test]
    fn tail_poll_highlights_appended_matches() {
        let path = std::env::temp_dir().join(format!(
            "rusted-highlight-{}-{}.txt", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::write(&path, "hit\n").unwrap();
        let mut app = test_app();
        app.tabs[0].path = Some(path.clone());
        assert!(reload_from_disk(&mut app.tabs[0], &path));
        app.tabs[0].filter_open = true;
        app.tabs[0].filter_text = "hit".into();
        app.apply_search();
        app.tabs[0].tail = true;
        std::fs::write(&path, "hit\nmiss\nHIT\n").unwrap();
        app.update_filter(true);
        assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 0, 1, 0]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn tail_editor_stays_read_only_when_clicked_and_typed() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].tail = true;
        let output = frame(&ctx, &mut app, vec![], false);
        let text = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.rows.len() == 4 => Some(t),
            _ => None,
        }).unwrap();
        let pos = text.pos + text.galley.rows[0].rect().center().to_vec2();
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(true)], false);
        frame(&ctx, &mut app, vec![button(false), egui::Event::Text("x".into()), egui::Event::Paste("y".into())], false);
        assert!(app.tabs[0].tail);
        assert_eq!(app.tabs[0].text, "abcdefghij\nabcdefghij\nabcdefghij\nabcdefghij");
    }

    fn toolbar_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.menu_bar(ui);
            app.view_toolbar(ui);
            ui.separator();
            app.editor_ui(ui);
            app.about_ui(ui.ctx());
        });
        output.textures_delta.clear();
        output
    }

    fn find_label(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        }).unwrap_or_else(|| panic!("missing {label}: {:?}", output.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        }).collect::<Vec<_>>()))
    }

    fn goto_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.goto_shortcut(ui.ctx());
            egui::Panel::top("test_menu").show(ui, |ui| app.menu_bar(ui));
            egui::Panel::bottom("test_status").show(ui, |ui| { ui.label("Status"); });
            if app.tabs[app.active].goto_open && app.browser.is_none() {
                egui::Panel::bottom(("goto", app.tabs[app.active].id)).show(ui, |ui| app.goto_ui(ui));
            }
            if app.tabs[app.active].filter_open && app.browser.is_none() {
                egui::Panel::bottom("test_goto_filter").exact_size(150.0).show(ui, |ui| app.filter_ui(ui));
            }
            egui::CentralPanel::default().show(ui, |ui| {
                if app.browser.is_some() { app.browser_ui(ui); } else { app.editor_ui(ui); }
            });
        });
        output.textures_delta.clear();
        output
    }

    fn goto_click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) -> egui::FullOutput {
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
        };
        goto_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
        goto_frame(ctx, app, vec![button(false)]);
        goto_frame(ctx, app, vec![])
    }

    #[test]
    fn goto_line_shortcut_validation_cancel_and_tab_state() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let key = egui::Event::Key {
            key: Key::G, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND,
        };
        let output = goto_frame(&ctx, &mut app, vec![key.clone()]);
        assert!(app.tabs[0].goto_open);
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(("goto_input", app.tabs[0].id)))));
        assert!(find_label(&output, "Line:").y < find_label(&output, "Status").y);
        for invalid in ["", "0", "5", "1x", "-1", "999999999999999999999999999999999"] {
            app.tabs[0].goto_text = invalid.into();
            let output = goto_frame(&ctx, &mut app, vec![]);
            goto_click(&ctx, &mut app, find_label(&output, "Go"));
            assert!(app.tabs[0].goto_open, "{invalid:?}");
            assert_eq!(app.tabs[0].goto_line, None);
        }
        let output = goto_frame(&ctx, &mut app, vec![]);
        goto_click(&ctx, &mut app, find_label(&output, "Cancel"));
        assert!(!app.tabs[0].goto_open);
        goto_frame(&ctx, &mut app, vec![key.clone()]);
        goto_frame(&ctx, &mut app, vec![egui::Event::Text("4".into())]);
        assert_eq!(app.tabs[0].goto_text, "4");
        let output = goto_frame(&ctx, &mut app, vec![]);
        goto_click(&ctx, &mut app, find_label(&output, "Go"));
        assert!(!app.tabs[0].goto_open);
        assert_eq!(app.tabs[0].goto_line, Some(3));
        app.tabs[0].text = "first\n".into();
        goto_frame(&ctx, &mut app, vec![key.clone()]);
        goto_frame(&ctx, &mut app, vec![egui::Event::Text("2".into())]);
        let enter = egui::Event::Key {
            key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE,
        };
        goto_frame(&ctx, &mut app, vec![enter]);
        assert!(!app.tabs[0].goto_open);
        assert_eq!(app.tabs[0].goto_line, Some(1));
        app.new_tab(None, "other".into(), false, 0);
        assert_eq!(app.tabs[1].goto_line, None);
        app.open_browser();
        goto_frame(&ctx, &mut app, vec![key]);
        assert!(!app.tabs[1].goto_open);
    }

    #[test]
    fn goto_line_menu_scrolls_and_paints_white_with_wrap_and_tail() {
        for wrap in [false, true] {
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
            let mut app = test_app();
            app.tabs[0].text = (1..=100).map(|i| format!("line {i} {}", "abcdefghij".repeat(12)))
                .collect::<Vec<_>>().join("\n");
            app.tabs[0].word_wrap = wrap;
            app.tabs[0].tail = true;
            app.tabs[0].filter_open = true;
            let output = goto_frame(&ctx, &mut app, vec![]);
            let output = goto_click(&ctx, &mut app, find_label(&output, "File"));
            let output = goto_click(&ctx, &mut app, find_label(&output, "Go to line"));
            assert!(app.tabs[0].goto_open);
            assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(("goto_input", app.tabs[0].id)))));
            app.tabs[0].goto_text = "71".into();
            goto_click(&ctx, &mut app, find_label(&output, "Go"));
            let output = goto_frame(&ctx, &mut app, vec![]);
            assert!(app.tabs[0].tail);
            assert_eq!(app.tabs[0].goto_line, Some(70));
            let number = output.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == "71" => Some(t),
                _ => None,
            }).expect("target line number is not visible");
            assert_eq!(number.galley.job.sections[0].format.color, Color32::WHITE);
            assert!(number.pos.y >= 0.0 && number.pos.y < find_label(&output, "Status").y);
            let other = output.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == "70" => Some(t),
                _ => None,
            });
            if let Some(other) = other {
                assert_eq!(other.galley.job.sections[0].format.color, Color32::from_gray(110));
            }
        }
    }

    #[test]
    fn goto_line_preserves_horizontal_scroll_and_search_highlight() {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        let mut app = test_app();
        app.tabs[0].text = (0..100).map(|i| format!("line {i} {}", "abcdefghij".repeat(80)))
            .collect::<Vec<_>>().join("\n");
        app.tabs[0].filter_text = "line".into();
        app.apply_search();
        app.tabs[0].highlight = Some(2);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::Panel::top("test_menu").show(ui, |ui| app.menu_bar(ui));
            egui::Panel::bottom("test_status").show(ui, |ui| { ui.label("Status"); });
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::both().id_salt(("scroll", app.tabs[0].id))
                    .horizontal_scroll_offset(120.0).show(ui, |ui| ui.set_min_size(egui::vec2(8000.0, 3000.0)));
            });
        });
        output.textures_delta.clear();
        let output = goto_frame(&ctx, &mut app, vec![]);
        let text_x = |output: &egui::FullOutput| output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text.starts_with("line 0 ") => Some(t.pos.x),
            _ => None,
        }).unwrap();
        let before = text_x(&output);
        assert!(before < -20.0);
        app.open_goto();
        app.tabs[0].goto_text = "71".into();
        let output = goto_frame(&ctx, &mut app, vec![]);
        goto_click(&ctx, &mut app, find_label(&output, "Go"));
        let output = goto_frame(&ctx, &mut app, vec![]);
        assert!((text_x(&output) - before).abs() < 0.1);
        assert_eq!(app.tabs[0].highlight, Some(2));
        assert!(app.tabs[0].search_highlights.active);
    }

    fn tab_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.tab_bar(ui);
            ui.separator();
            app.editor_ui(ui);
        });
        output.textures_delta.clear();
        output
    }

    fn tab_shortcut_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.tab_shortcuts(ui.ctx());
            app.editor_ui(ui);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn tab_shortcuts_cycle_wrap_and_focus_active_editor() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "first".into();
        app.new_tab(None, "second".into(), false, 0);
        app.new_tab(None, "third".into(), false, 0);
        app.tabs[1].word_wrap = true;
        app.active = 0;
        tab_shortcut_frame(&ctx, &mut app, vec![]);
        let forward = || key_event(Key::Tab, Modifiers::CTRL);
        let backward = || key_event(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT);
        tab_shortcut_frame(&ctx, &mut app, vec![forward()]);
        assert_eq!(app.active, 1);
        assert!(app.tabs[1].word_wrap);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", app.tabs[1].id));
        assert!(ctx.memory(|memory| memory.has_focus(id)));
        assert_eq!(app.tabs[1].text, "second");
        tab_shortcut_frame(&ctx, &mut app, vec![forward(), forward()]);
        assert_eq!(app.active, 0);
        tab_shortcut_frame(&ctx, &mut app, vec![backward()]);
        assert_eq!(app.active, 2);
        tab_shortcut_frame(&ctx, &mut app, vec![backward(), forward()]);
        assert_eq!(app.active, 2);
    }

    #[test]
    fn tab_shortcuts_preserve_plain_tab_and_handle_browser_and_single_tab() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "text".into();
        tab_shortcut_frame(&ctx, &mut app, vec![]);
        tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::CTRL)]);
        assert_eq!(app.tabs[0].text, "text");
        tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::NONE)]);
        assert!(app.tabs[0].text.contains('\t'));
        app.new_tab(None, "other".into(), false, 0);
        app.active = 0;
        app.browser = Some(Browser {
            mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
            name: String::new(), error: None, focus_name: false,
        });
        tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT)]);
        assert_eq!(app.active, 1);
        assert!(app.browser.is_none());
    }

    fn tab_click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2, button: egui::PointerButton) -> egui::FullOutput {
        let event = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE };
        tab_frame(ctx, app, vec![egui::Event::PointerMoved(pos), event(true)]);
        tab_frame(ctx, app, vec![event(false)])
    }

    fn tab_context(ctx: &egui::Context, app: &mut App, tab: &str, action: &str) -> egui::FullOutput {
        let output = tab_frame(ctx, app, vec![]);
        let pos = find_label(&output, tab);
        tab_click(ctx, app, pos, egui::PointerButton::Secondary);
        let output = tab_frame(ctx, app, vec![]);
        let pos = find_label(&output, action);
        tab_click(ctx, app, pos, egui::PointerButton::Primary)
    }

    #[test]
    fn tab_unsaved_indicator_tracks_edits_undo_and_save() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let path = std::env::temp_dir().join(format!(
            "rusted-tabs-{}-{}.txt", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        app.tabs.clear();
        app.new_tab(Some(path.clone()), "original".into(), false, 0);
        let output = tab_frame(&ctx, &mut app, vec![]);
        find_label(&output, path.file_name().unwrap().to_str().unwrap());
        assert_eq!(app.tabs[0].tooltip(), path.to_string_lossy());
        tab_frame(&ctx, &mut app, vec![egui::Event::Text("x".into())]);
        assert!(app.tabs[0].dirty);
        let title = format!("{}*", path.file_name().unwrap().to_str().unwrap());
        find_label(&tab_frame(&ctx, &mut app, vec![]), &title);
        assert_eq!(app.tabs[0].tooltip(), format!("{} (unsaved)", path.display()));
        app.tabs[0].tail = true;
        assert_eq!(app.tabs[0].tooltip(), format!("Tailing {} (unsaved)", path.display()));
        app.tabs[0].tail = false;
        tab_frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)]);
        assert!(!app.tabs[0].dirty);
        assert_eq!(app.tabs[0].text, "original");
        tab_frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)]);
        assert!(app.tabs[0].dirty);
        app.write_tab(0, path.clone());
        assert!(!app.tabs[0].dirty);
        assert_eq!(app.tabs[0].tooltip(), path.to_string_lossy());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn tab_context_actions_target_clicked_tab() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs.clear();
        app.new_tab(Some(PathBuf::from("first.txt")), "first".into(), false, 0);
        app.new_tab(Some(PathBuf::from("second.txt")), "second".into(), false, 0);
        app.new_tab(Some(PathBuf::from("third.txt")), "third".into(), false, 0);
        app.tabs[0].dirty = true;
        app.tabs[2].dirty = true;
        let output = tab_context(&ctx, &mut app, "first.txt*", "Copy file path");
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "first.txt")));
        assert_eq!(app.active, 2);
        tab_context(&ctx, &mut app, "first.txt*", "Close all others");
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].text, "first");
        assert!(app.tabs[0].dirty);
        assert_eq!(app.active, 0);
        tab_context(&ctx, &mut app, "first.txt*", "Close");
        assert_eq!(app.tabs.len(), 1);
        assert!(app.tabs[0].path.is_none());
        assert!(app.tabs[0].text.is_empty());
    }

    #[test]
    fn middle_click_closes_dirty_inactive_and_active_tabs() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs.clear();
        app.new_tab(Some(PathBuf::from("first.txt")), "first".into(), false, 0);
        app.new_tab(Some(PathBuf::from("second.txt")), "second".into(), false, 0);
        app.tabs[0].dirty = true;
        app.tabs[1].dirty = true;
        let output = tab_frame(&ctx, &mut app, vec![]);
        tab_click(&ctx, &mut app, find_label(&output, "first.txt*"), egui::PointerButton::Middle);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[app.active].text, "second");
        let output = tab_frame(&ctx, &mut app, vec![]);
        tab_click(&ctx, &mut app, find_label(&output, "second.txt*"), egui::PointerButton::Middle);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].tab_title(), "Untitled");
    }

    #[test]
    fn untitled_tab_has_no_copy_path_and_keeps_plain_save_name() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].dirty = true;
        assert_eq!(app.tabs[0].tab_title(), "Untitled");
        assert_eq!(app.tabs[0].tooltip(), "Untitled (unsaved)");
        app.save_as();
        assert_eq!(app.browser.as_ref().unwrap().name, "Untitled");
        app.browser = None;
        let output = tab_context(&ctx, &mut app, "Untitled", "Copy file path");
        assert!(!output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
        assert_eq!(app.tabs.len(), 1);
    }

    #[test]
    fn saved_file_named_untitled_shows_unsaved_marker() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].dirty = true;
        app.tabs[0].path = Some(PathBuf::from("Untitled"));
        let output = tab_frame(&ctx, &mut app, vec![]);
        find_label(&output, "Untitled*");
        assert_eq!(app.tabs[0].tooltip(), "Untitled (unsaved)");
    }

    fn open_edit(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
        let output = toolbar_frame(ctx, app, vec![]);
        let pos = find_label(&output, "Edit");
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
        toolbar_frame(ctx, app, vec![button(false)]);
        toolbar_frame(ctx, app, vec![])
    }

    fn click_edit(ctx: &egui::Context, app: &mut App, label: &str) -> egui::FullOutput {
        let output = open_edit(ctx, app);
        let pos = find_label(&output, label);
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
        toolbar_frame(ctx, app, vec![button(false)]);
        toolbar_frame(ctx, app, vec![])
    }

    fn click_at(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) -> egui::FullOutput {
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
        toolbar_frame(ctx, app, vec![button(false)]);
        toolbar_frame(ctx, app, vec![])
    }

    #[test]
    fn json_format_preserves_values_order_duplicates_and_escapes() {
        let input = r#" { "z":[1e400,{},[],true,null],"a":"a \"quoted\" : [x]", "z":123456789012345678901234567890 } "#;
        let expected = "{\n  \"z\": [\n    1e400,\n    {},\n    [],\n    true,\n    null\n  ],\n  \"a\": \"a \\\"quoted\\\" : [x]\",\n  \"z\": 123456789012345678901234567890\n}";
        assert_eq!(format_json(input).unwrap(), expected);
        assert_eq!(format_json(expected).unwrap(), expected);
        assert_eq!(format_json(" [ ] ").unwrap(), "[]");
        assert_eq!(format_json(r#""hello ☃""#).unwrap(), r#""hello ☃""#);
        for invalid in ["", "{\"a\":}", "[1,]", "true false", "{\"a\":1}\u{a0}"] {
            assert!(format_json(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn json_toolbar_formats_active_file_and_supports_undo_redo() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].path = Some(PathBuf::from("first.JSON"));
        app.tabs[0].text = r#"{"x":[1,2]}"#.into();
        app.tabs[0].filter_open = true;
        app.tabs[0].applied = ("2".into(), false);
        app.tabs[0].filter.applied = Some(("2".into(), false));
        app.new_tab(Some(PathBuf::from("other.txt")), "other".into(), false, 0);
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "Format JSON")));
        app.active = 0;
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        assert!(find_label(&output, "Tail").x < find_label(&output, "Format JSON").x);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        let formatted = "{\n  \"x\": [\n    1,\n    2\n  ]\n}";
        assert_eq!(app.tabs[0].text, formatted);
        assert!(app.tabs[0].dirty);
        assert!(app.tabs[0].filter.applied.is_none());
        assert_eq!(app.tabs[1].text, "other");
        app.update_filter(false);
        assert_eq!(app.tabs[0].filter.results[0].0, 3);
        click_edit(&ctx, &mut app, "Undo");
        assert_eq!(app.tabs[0].text, r#"{"x":[1,2]}"#);
        app.update_filter(false);
        assert_eq!(app.tabs[0].filter.results[0].0, 0);
        click_edit(&ctx, &mut app, "Redo");
        assert_eq!(app.tabs[0].text, formatted);
        app.update_filter(false);
        assert_eq!(app.tabs[0].filter.results[0].0, 3);
    }

    #[test]
    fn json_toolbar_handles_invalid_json_and_disables_during_tail_or_browser() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].path = Some(PathBuf::from("test.json"));
        app.tabs[0].text = "{invalid}".into();
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        assert_eq!(app.tabs[0].text, "{invalid}");
        assert!(!app.tabs[0].dirty);
        assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot format JSON:")));
        app.tabs[0].text = r#"{"x":1}"#.into();
        app.tabs[0].tail = true;
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        assert_eq!(app.tabs[0].text, r#"{"x":1}"#);
        app.tabs[0].tail = false;
        app.browser = Some(Browser {
            mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
            name: String::new(), error: None, focus_name: false,
        });
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        assert_eq!(app.tabs[0].text, r#"{"x":1}"#);
    }

    #[test]
    fn json_format_noop_preserves_redo_and_changed_format_clears_redo() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].path = Some(PathBuf::from("test.json"));
        app.tabs[0].text = "{}".into();
        toolbar_frame(&ctx, &mut app, vec![]);
        toolbar_frame(&ctx, &mut app, vec![egui::Event::Text(" ".into())]);
        let edited = app.tabs[0].text.clone();
        click_edit(&ctx, &mut app, "Undo");
        assert_eq!(app.tabs[0].text, "{}");
        app.tabs[0].dirty = false;
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        assert!(!app.tabs[0].dirty);
        click_edit(&ctx, &mut app, "Redo");
        assert_eq!(app.tabs[0].text, edited);
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        assert_eq!(app.tabs[0].text, "{}");
        click_edit(&ctx, &mut app, "Undo");
        assert_eq!(app.tabs[0].text, edited);
        click_edit(&ctx, &mut app, "Undo");
        assert_eq!(app.tabs[0].text, "{}");
        toolbar_frame(&ctx, &mut app, vec![egui::Event::Text("  ".into())]);
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
        click_edit(&ctx, &mut app, "Redo");
        assert_eq!(app.tabs[0].text, "{}");
    }

    #[test]
    fn about_program_opens_and_closes_dialog() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        assert!(find_label(&output, "File").x < find_label(&output, "About").x);
        assert!(find_label(&output, "About").x < find_label(&output, "Edit").x);
        let output = click_at(&ctx, &mut app, find_label(&output, "About"));
        let output = click_at(&ctx, &mut app, find_label(&output, "Program"));
        assert!(app.about_open);
        find_label(&output, "rusted");
        find_label(&output, "Version 1.0.0");
        assert!(app.about_icon.as_ref().is_some_and(Result::is_ok));
        click_at(&ctx, &mut app, find_label(&output, "Close"));
        assert!(!app.about_open);
    }

    #[test]
    fn find_toolbar_toggles_active_tab_filter() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.new_tab(None, "second".into(), false, 0);
        app.active = 0;
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        let find = find_label(&output, "Find");
        assert!(find.x < find_label(&output, "Word wrap").x);
        let output = click_at(&ctx, &mut app, find);
        assert!(app.tabs[0].filter_open);
        assert!(app.tabs[0].focus_filter);
        assert!(!app.tabs[1].filter_open);
        click_at(&ctx, &mut app, find_label(&output, "Find"));
        assert!(!app.tabs[0].filter_open);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", app.tabs[0].id));
        assert!(ctx.memory(|memory| memory.has_focus(id)));
        app.browser = Some(Browser {
            mode: BrowserMode::Open,
            dir: PathBuf::new(),
            entries: Vec::new(),
            name: String::new(),
            error: None,
            focus_name: false,
        });
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        click_at(&ctx, &mut app, find_label(&output, "Find"));
        assert!(!app.tabs[0].filter_open);
    }

    #[test]
    fn edit_menu_clipboard_actions_use_editor_selection() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        let text = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text),
            _ => None,
        }).unwrap();
        let row = &text.galley.rows[0];
        let start = text.pos + egui::vec2(row.pos.x + row.glyphs[1].pos.x, row.rect().center().y);
        let end = text.pos + egui::vec2(row.pos.x + row.glyphs[4].pos.x, row.rect().center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        toolbar_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start), button(start, true)]);
        toolbar_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)]);
        toolbar_frame(&ctx, &mut app, vec![button(end, false)]);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", 0_u64));
        let range = egui::text_edit::TextEditState::load(&ctx, id).unwrap().cursor.char_range().unwrap();
        assert!(!range.is_empty(), "selection missing after drag: {range:?}");
        let output = click_edit(&ctx, &mut app, "Copy");
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd")), "{:?}", output.platform_output.commands);
        let output = click_edit(&ctx, &mut app, "Cut");
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd")));
        assert!(app.tabs[0].text.starts_with("aefghij"));
        let output = click_edit(&ctx, &mut app, "Paste");
        assert!(output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))));
        toolbar_frame(&ctx, &mut app, vec![egui::Event::Paste("XYZ".into())]);
        assert!(app.tabs[0].text.starts_with("aXYZefghij"));
    }

    #[test]
    fn edit_menu_clipboard_respects_rectangular_selection_and_tail() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let output = toolbar_frame(&ctx, &mut app, vec![]);
        let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
            _ => None,
        }).unwrap();
        let row = &galley.rows[0];
        app.tabs[0].col_sel = Some(ColSel {
            anchor: (0, row.glyphs[1].pos.x),
            head: (1, row.glyphs[4].pos.x),
        });
        let output = click_edit(&ctx, &mut app, "Copy");
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd\nbcd")));
        let output = click_edit(&ctx, &mut app, "Cut");
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd\nbcd")));
        assert!(app.tabs[0].text.starts_with("aefghij\naefghij"));
        app.tabs[0].tail = true;
        let text = app.tabs[0].text.clone();
        click_edit(&ctx, &mut app, "Cut");
        let output = click_edit(&ctx, &mut app, "Paste");
        assert_eq!(app.tabs[0].text, text);
        assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))));
        assert!(app.tabs[0].tail);
    }

    #[test]
    fn edit_menu_undo_and_redo() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "original".into();
        toolbar_frame(&ctx, &mut app, vec![]);
        toolbar_frame(&ctx, &mut app, vec![egui::Event::Text("X".into())]);
        let edited = app.tabs[0].text.clone();
        assert_ne!(edited, "original");
        click_edit(&ctx, &mut app, "Undo");
        assert_eq!(app.tabs[0].text, "original");
        click_edit(&ctx, &mut app, "Redo");
        assert_eq!(app.tabs[0].text, edited);
    }

    fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn keyboard_undo_ignores_cursor_only_history_and_preserves_position() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        frame(&ctx, &mut app, vec![], false);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", app.tabs[0].id));
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        let first = egui::text::CCursorRange::one(egui::text::CCursor::new(0));
        let last = egui::text::CCursorRange::one(egui::text::CCursor::new(app.tabs[0].text.len()));
        let mut undoer = egui::util::undoer::Undoer::default();
        undoer.add_undo(&(last, app.tabs[0].text.clone()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(first));
        state.store(&ctx, id);
        let before = frame(&ctx, &mut app, vec![], false);
        assert!(ctx.memory(|m| m.has_focus(id)), "editor lacks focus");
        assert_eq!(egui::text_edit::TextEditState::load(&ctx, id).unwrap().cursor.char_range(), Some(first), "before undo");
        let text_y = |output: &egui::FullOutput| output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t.pos.y),
            _ => None,
        }).unwrap();
        let after = frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
        let state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        assert_eq!(state.cursor.char_range(), Some(first));
        assert_eq!(text_y(&before), text_y(&after));
        assert_eq!(history_steps(&state, &app.tabs[0].text, false), 0);
    }

    #[test]
    fn keyboard_undo_and_shift_redo_skip_cursor_only_history() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.tabs[0].text = "changed".into();
        frame(&ctx, &mut app, vec![], false);
        #[allow(deprecated)]
        let id = egui::Id::new(("editor", app.tabs[0].id));
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        let at = |i| egui::text::CCursorRange::one(egui::text::CCursor::new(i));
        let mut undoer = egui::util::undoer::Undoer::default();
        undoer.add_undo(&(at(0), "original".into()));
        undoer.add_undo(&(at(1), "changed".into()));
        undoer.add_undo(&(at(2), "changed".into()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(at(2)));
        assert_eq!(history_steps(&state, "changed", false), 2);
        state.store(&ctx, id);
        assert!(ctx.memory(|m| m.has_focus(id)), "editor lacks focus");
        frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
        assert_eq!(app.tabs[0].text, "original");
        frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)], false);
        assert_eq!(app.tabs[0].text, "changed");
        frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
        assert_eq!(app.tabs[0].text, "original");
        frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)], false);
        assert_eq!(app.tabs[0].text, "changed");
    }

    #[test]
    fn edit_menu_disables_unavailable_actions_and_reports_clipboard_errors() {
        for (status, label) in [
            (Ok(false), "Paste"),
            (Err("test clipboard error".to_owned()), "Paste"),
            (Ok(true), "Undo"),
            (Ok(true), "Redo"),
            (Ok(true), "Cut"),
            (Ok(true), "Copy"),
        ] {
            let ctx = egui::Context::default();
            let mut app = test_app();
            app.clipboard_test_status = Some(status.clone());
            let before = app.tabs[0].text.clone();
            let output = click_edit(&ctx, &mut app, label);
            assert_eq!(app.tabs[0].text, before, "{label}");
            assert!(app.pending_edit.is_none(), "{label}");
            assert!(!output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))), "{label}");
            assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))), "{label}");
            if status.is_err() {
                let output = open_edit(&ctx, &mut app);
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains("test clipboard error"))));
            }
        }
    }

    #[test]
    fn view_shortcuts_toggle_only_the_active_tab() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.new_tab(None, "second".into(), false, 0);
        app.active = 0;
        for key in [Key::W, Key::E, Key::T] {
            let input = egui::RawInput {
                events: vec![key_event(key, Modifiers::COMMAND)],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
            output.textures_delta.clear();
        }
        assert!(app.tabs[0].word_wrap && app.tabs[0].hidden_chars && app.tabs[0].tail);
        assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
        let input = egui::RawInput { events: vec![key_event(Key::T, Modifiers::COMMAND)], ..Default::default() };
        let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
        output.textures_delta.clear();
        assert!(!app.tabs[0].tail);
    }

    #[test]
    fn view_shortcuts_respect_browser_and_failed_tail_reload() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.browser = Some(Browser {
            mode: BrowserMode::Open,
            dir: PathBuf::new(),
            entries: Vec::new(),
            name: String::new(),
            error: None,
            focus_name: false,
        });
        for key in [Key::W, Key::E, Key::T] {
            let input = egui::RawInput { events: vec![key_event(key, Modifiers::COMMAND)], ..Default::default() };
            let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
            output.textures_delta.clear();
        }
        assert!(!app.tabs[0].word_wrap && !app.tabs[0].hidden_chars && !app.tabs[0].tail);
        app.browser = None;
        app.tabs[0].path = Some(std::env::temp_dir().join(format!("rusted-missing-{}", std::process::id())).join("file"));
        let input = egui::RawInput { events: vec![key_event(Key::T, Modifiers::COMMAND)], ..Default::default() };
        let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
        output.textures_delta.clear();
        assert!(!app.tabs[0].tail);
        assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot reload ")));
    }

    #[test]
    fn toolbar_controls_current_tab_and_tail_reloads() {
        let ctx = egui::Context::default();
        let mut app = test_app();
        let path = std::env::temp_dir().join(format!(
            "rusted-toolbar-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::write(&path, "disk content").unwrap();
        app.tabs[0].path = Some(path.clone());
        app.tabs[0].text = "unsaved content".into();
        app.tabs[0].dirty = true;
        app.new_tab(None, "second tab".into(), false, 0);
        app.active = 0;

        let run = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.view_toolbar(ui));
            output.textures_delta.clear();
            output
        };
        for label in ["Word wrap", "Hidden characters", "Tail"] {
            let output = run(&mut app, vec![]);
            let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                _ => None,
            }).unwrap();
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            run(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
            run(&mut app, vec![button(false)]);
        }
        assert!(app.tabs[0].word_wrap);
        assert!(app.tabs[0].hidden_chars);
        assert!(app.tabs[0].tail);
        assert_eq!(app.tabs[0].text, "disk content");
        assert!(!app.tabs[0].dirty);
        assert!(app.tabs[0].scroll_to_end);
        assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
        app.active = 1;
        run(&mut app, vec![]);
        assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
        std::fs::remove_file(path).unwrap();
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
