use eframe::egui::{self, Color32, FontId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum HighlightColor {
    DarkGray,
    DarkRed,
    #[default]
    DarkGreen,
    DarkBlue,
    DarkPurple,
}

impl HighlightColor {
    pub(crate) const ALL: [Self; 5] = [Self::DarkGray, Self::DarkRed, Self::DarkGreen, Self::DarkBlue, Self::DarkPurple];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::DarkGray => "Dark Gray",
            Self::DarkRed => "Dark Red",
            Self::DarkGreen => "Dark Green",
            Self::DarkBlue => "Dark Blue",
            Self::DarkPurple => "Dark Purple",
        }
    }

    pub(crate) fn label(self, light: bool) -> &'static str {
        match (self, light) {
            (Self::DarkGray, true) => "White",
            (Self::DarkRed, true) => "Light Red",
            (Self::DarkGreen, true) => "Light Green",
            (Self::DarkBlue, true) => "Light Blue",
            (Self::DarkPurple, true) => "Light Purple",
            _ => self.name(),
        }
    }

    pub(crate) fn themed(self, light: bool) -> Color32 {
        match (self, light) {
            (Self::DarkGray, true) => Color32::WHITE,
            (Self::DarkRed, true) => Color32::from_rgb(255, 205, 205),
            (Self::DarkGreen, true) => Color32::from_rgb(200, 240, 205),
            (Self::DarkBlue, true) => Color32::from_rgb(200, 220, 255),
            (Self::DarkPurple, true) => Color32::from_rgb(232, 210, 245),
            _ => self.color(),
        }
    }

    pub(crate) fn color(self) -> Color32 {
        match self {
            Self::DarkGray => Color32::from_rgb(27, 27, 27),
            Self::DarkRed => Color32::from_rgb(85, 30, 30),
            Self::DarkGreen => Color32::from_rgb(25, 70, 40),
            Self::DarkBlue => Color32::from_rgb(25, 45, 80),
            Self::DarkPurple => Color32::from_rgb(65, 35, 75),
        }
    }
}

pub(crate) fn match_ranges(line: &str, pattern: &str, case: bool) -> Vec<std::ops::Range<usize>> {
    if pattern.is_empty() {
        return Vec::new();
    }
    if case {
        return line.match_indices(pattern).map(|(start, s)| start..start + s.len()).collect();
    }
    let lower = line.to_lowercase();
    let needle = pattern.to_lowercase();
    if line.is_ascii() {
        return lower.match_indices(&needle).map(|(start, s)| start..start + s.len()).collect();
    }
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

pub(crate) fn replace_matches(text: &str, pattern: &str, replacement: &str, case: bool) -> Option<String> {
    if pattern.is_empty() {
        return None;
    }
    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let mut start = 0;
        for range in match_ranges(content, pattern, case) {
            if range.start < start {
                continue;
            }
            output.push_str(&content[start..range.start]);
            output.push_str(replacement);
            changed |= &content[range.clone()] != replacement;
            start = range.end;
        }
        output.push_str(&line[start..]);
    }
    changed.then_some(output)
}

pub(crate) fn search_layout(text: &str, highlights: &SearchHighlights, selected_line: Option<usize>, hidden: bool, font: &FontId, width: f32, light: bool) -> egui::text::LayoutJob {
    let (normal, hit, selected) = if light {
        (Color32::from_gray(50), Color32::BLACK, Color32::from_rgb(0, 38, 255))
    } else {
        (Color32::from_gray(212), Color32::WHITE, Color32::from_rgb(0, 255, 255))
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let append = |job: &mut egui::text::LayoutJob, s: &str, color| {
        let shown = if hidden { std::borrow::Cow::Owned(s.replace(' ', "·")) } else { std::borrow::Cow::Borrowed(s) };
        job.append(&shown, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
    };
    let matches_current_text = highlights.active && text == highlights.snapshot;
    if !matches_current_text && selected_line.is_none() {
        append(&mut job, text, normal);
        return job;
    }
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if selected_line == Some(i) {
            append(&mut job, line, selected);
            continue;
        }
        let content = line.strip_suffix('\n').unwrap_or(line);
        let mut start = 0;
        if let Some(ranges) = matches_current_text.then(|| highlights.lines.get(i)).flatten() {
            for range in ranges {
                if range.start < start {
                    continue;
                }
                append(&mut job, &content[start..range.start], normal);
                append(&mut job, &content[range.clone()], hit);
                start = range.end;
            }
        }
        append(&mut job, &line[start..], normal);
    }
    if text.is_empty() {
        append(&mut job, "", normal);
    }
    job
}

#[derive(Default)]
pub(crate) struct SearchHighlights {
    pub(crate) snapshot: String,
    pub(crate) lines: Vec<Vec<std::ops::Range<usize>>>,
    pub(crate) active: bool,
}

impl SearchHighlights {
    pub(crate) fn search(&mut self, text: &str, pattern: &str, case: bool) {
        self.active = !pattern.is_empty();
        if self.active {
            self.snapshot = text.to_owned();
            self.lines = text.split('\n').map(|line| match_ranges(line, pattern, case)).collect();
        } else {
            self.snapshot.clear();
            self.lines.clear();
        }
    }

    pub(crate) fn edited(&mut self, text: &str) {
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

    pub(crate) fn tailed(&mut self, text: &str, pattern: &str, case: bool) {
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
pub(crate) struct FilterState {
    // (0-based line number, line text, terminated by a newline)
    pub(crate) results: Vec<(usize, String, bool)>,
    // Byte offset and line number of the first line not yet fully scanned.
    pub(crate) offset: usize,
    pub(crate) line: usize,
    pub(crate) applied: Option<(String, bool)>,
    pub(crate) selected: Option<usize>,
    pub(crate) needs_rescan: bool,
    pub(crate) wrap_cache: Option<FilterWrapCache>,
}

pub(crate) struct FilterWrapCache {
    pub(crate) width: f32,
    pub(crate) font: FontId,
    pub(crate) hidden: bool,
    pub(crate) padding: egui::Vec2,
    pub(crate) spacing: f32,
    pub(crate) min_height: f32,
    pub(crate) pixels_per_point: f32,
    pub(crate) offsets: Vec<f32>,
}

impl FilterState {
    pub(crate) fn scan(&mut self, text: &str, pat: &str, case: bool) {
        let keep = self.line;
        let retained = self.results.partition_point(|r| r.0 < keep);
        self.results.truncate(retained);
        if let Some(cache) = &mut self.wrap_cache {
            cache.offsets.truncate(retained + 1);
        }
        if self.offset > text.len() || !text.is_char_boundary(self.offset) {
            self.results.clear();
            self.wrap_cache = None;
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
                self.results.push((line, l.to_owned(), terminated));
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
