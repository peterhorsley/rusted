use eframe::egui::{self, TextBuffer};
use std::ops::{Deref, DerefMut, Range};
use std::sync::atomic::{AtomicU64, Ordering};

const LINES_PER_CHECKPOINT: usize = 256;
pub(crate) const MAX_PREVIEW_LINE_BYTES: usize = 2048;
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

struct Checkpoint {
    byte: usize,
    chars: usize,
}

struct LineIndex {
    checkpoints: Vec<Checkpoint>,
    lines: usize,
    long_lines: Vec<(usize, usize)>,
}

fn char_count(text: &str) -> usize {
    if text.is_ascii() { text.len() } else { text.chars().count() }
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut checkpoints = vec![Checkpoint { byte: 0, chars: 0 }];
        let mut long_lines = Vec::new();
        let (mut lines, mut byte, mut chars) = (1, 0, 0);
        for segment in text.split_inclusive('\n') {
            let content_len = segment.len() - usize::from(segment.ends_with('\n'));
            if content_len > MAX_PREVIEW_LINE_BYTES {
                long_lines.push((lines - 1, byte + content_len));
            }
            if !segment.ends_with('\n') {
                break;
            }
            byte += segment.len();
            chars += char_count(segment);
            if lines % LINES_PER_CHECKPOINT == 0 {
                checkpoints.push(Checkpoint { byte, chars });
            }
            lines += 1;
        }
        Self { checkpoints, lines, long_lines }
    }

    fn line_start_char(&self, text: &str, line: usize) -> Option<usize> {
        if line >= self.lines {
            return None;
        }
        let checkpoint = &self.checkpoints[line / LINES_PER_CHECKPOINT];
        let offset: usize = text[checkpoint.byte..].split_inclusive('\n')
            .take(line % LINES_PER_CHECKPOINT).map(char_count).sum();
        Some(checkpoint.chars + offset)
    }

    fn line_start_byte(&self, text: &str, line: usize) -> Option<usize> {
        if line >= self.lines {
            return None;
        }
        let checkpoint = &self.checkpoints[line / LINES_PER_CHECKPOINT];
        let mut byte = checkpoint.byte;
        for current in (line / LINES_PER_CHECKPOINT * LINES_PER_CHECKPOINT)..line {
            byte = match self.long_lines.binary_search_by_key(&current, |&(line, _)| line) {
                Ok(index) => self.long_lines[index].1 + 1,
                Err(_) => byte + text[byte..].find('\n').expect("indexed line has a newline") + 1,
            };
        }
        Some(byte)
    }
}

pub(crate) struct DocumentText {
    text: String,
    index: Option<LineIndex>,
    revision: u64,
}

impl Default for DocumentText {
    fn default() -> Self {
        Self { text: String::new(), index: None, revision: next_revision() }
    }
}

impl DocumentText {
    fn invalidate(&mut self) {
        self.index = None;
        self.revision = next_revision();
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    fn index(&mut self) -> &LineIndex {
        self.index.get_or_insert_with(|| LineIndex::new(&self.text))
    }

    pub(crate) fn line_count(&mut self) -> usize {
        self.index().lines
    }

    pub(crate) fn line_start_char(&mut self, line: usize) -> Option<usize> {
        let (text, index) = (&self.text, &mut self.index);
        index.get_or_insert_with(|| LineIndex::new(text)).line_start_char(text, line)
    }

    pub(crate) fn preview_lines_from(&mut self, line: usize) -> Option<PreviewLines<'_>> {
        let (text, index) = (&self.text, &mut self.index);
        let index = index.get_or_insert_with(|| LineIndex::new(text));
        let byte = index.line_start_byte(text, line)?;
        Some(PreviewLines { text, index, line, byte })
    }
}

pub(crate) struct PreviewLines<'a> {
    text: &'a str,
    index: &'a LineIndex,
    line: usize,
    byte: usize,
}

impl<'a> Iterator for PreviewLines<'a> {
    type Item = (&'a str, bool);

    fn next(&mut self) -> Option<Self::Item> {
        if self.line >= self.index.lines {
            return None;
        }
        let long = self.index.long_lines.binary_search_by_key(&self.line, |&(line, _)| line);
        let end = match long {
            Ok(index) => self.index.long_lines[index].1,
            Err(_) => self.byte + self.text[self.byte..].find('\n').unwrap_or(self.text.len() - self.byte),
        };
        let truncated = long.is_ok();
        let mut preview_end = end.min(self.byte + MAX_PREVIEW_LINE_BYTES);
        while !self.text.is_char_boundary(preview_end) {
            preview_end -= 1;
        }
        let result = &self.text[self.byte..preview_end];
        self.byte = if end < self.text.len() { end + 1 } else { end };
        self.line += 1;
        Some((result, truncated))
    }
}

impl From<String> for DocumentText {
    fn from(text: String) -> Self {
        Self { text, index: None, revision: next_revision() }
    }
}

impl From<&str> for DocumentText {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl Deref for DocumentText {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl DerefMut for DocumentText {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.invalidate();
        &mut self.text
    }
}

impl std::fmt::Debug for DocumentText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}

impl<T: AsRef<str>> PartialEq<T> for DocumentText {
    fn eq(&self, other: &T) -> bool {
        self.text == other.as_ref()
    }
}

impl PartialEq<DocumentText> for String {
    fn eq(&self, other: &DocumentText) -> bool {
        self == &other.text
    }
}

impl TextBuffer for DocumentText {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }

    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        &self.text
    }

    fn insert_text(&mut self, text: &str, char_index: egui::text::CharIndex) -> usize {
        let inserted = self.text.insert_text(text, char_index);
        if inserted != 0 {
            self.invalidate();
        }
        inserted
    }

    fn delete_char_range(&mut self, char_range: Range<egui::text::CharIndex>) {
        if char_range.start != char_range.end {
            self.invalidate();
        }
        self.text.delete_char_range(char_range);
    }

    fn clear(&mut self) {
        self.invalidate();
        self.text.clear();
    }

    fn replace_with(&mut self, text: &str) {
        self.invalidate();
        text.clone_into(&mut self.text);
    }

    fn take(&mut self) -> String {
        self.invalidate();
        std::mem::take(&mut self.text)
    }
}
