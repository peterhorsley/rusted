use eframe::egui::{self, TextBuffer};
use std::ops::{Deref, DerefMut, Range};

const LINES_PER_CHECKPOINT: usize = 256;

struct Checkpoint {
    byte: usize,
    chars: usize,
}

struct LineIndex {
    checkpoints: Vec<Checkpoint>,
    lines: usize,
}

fn char_count(text: &str) -> usize {
    if text.is_ascii() { text.len() } else { text.chars().count() }
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut checkpoints = vec![Checkpoint { byte: 0, chars: 0 }];
        let (mut lines, mut byte, mut chars) = (1, 0, 0);
        for segment in text.split_inclusive('\n') {
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
        Self { checkpoints, lines }
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
}

#[derive(Default)]
pub(crate) struct DocumentText {
    text: String,
    index: Option<LineIndex>,
}

impl DocumentText {
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
}

impl From<String> for DocumentText {
    fn from(text: String) -> Self {
        Self { text, index: None }
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
        self.index = None;
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
            self.index = None;
        }
        inserted
    }

    fn delete_char_range(&mut self, char_range: Range<egui::text::CharIndex>) {
        if char_range.start != char_range.end {
            self.index = None;
        }
        self.text.delete_char_range(char_range);
    }

    fn clear(&mut self) {
        self.index = None;
        self.text.clear();
    }

    fn replace_with(&mut self, text: &str) {
        self.index = None;
        text.clone_into(&mut self.text);
    }

    fn take(&mut self) -> String {
        self.index = None;
        std::mem::take(&mut self.text)
    }
}
