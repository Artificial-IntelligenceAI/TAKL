//! A program's text and how to turn byte offsets into the two places an error names.

use unicode_segmentation::UnicodeSegmentation;

/// A run of bytes in the original text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

pub struct Source {
    pub path: String,
    pub text: String,
    line_starts: Vec<usize>,
}

/// Where a span starts, as editors and as people count.
pub struct Place {
    pub line: usize,
    /// trace: decision.errors.layout (the editor's column counts UTF-8 bytes)
    pub byte_column: usize,
    /// trace: decision.errors.layout (the person's column counts grapheme clusters)
    pub letter_column: usize,
}

impl Source {
    pub fn new(path: &str, text: &str) -> Source {
        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Source { path: path.to_string(), text: text.to_string(), line_starts }
    }

    /// The lines of the program, each with the offset it starts at, without their line endings.
    pub fn lines(&self) -> Vec<(usize, &str)> {
        let mut lines = Vec::new();
        for (number, start) in self.line_starts.iter().enumerate() {
            let end = self.line_starts.get(number + 1).map(|next| next - 1).unwrap_or(self.text.len());
            let line = self.text[*start..end].trim_end_matches('\r');
            lines.push((*start, line));
        }
        lines
    }

    pub fn line_number(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index + 1,
            Err(index) => index,
        }
    }

    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self.line_starts.get(line).map(|next| next - 1).unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches('\r')
    }

    pub fn place(&self, offset: usize) -> Place {
        let line = self.line_number(offset);
        let start = self.line_starts[line - 1];
        let before = &self.text[start..offset];
        Place { line, byte_column: before.len() + 1, letter_column: before.graphemes(true).count() + 1 }
    }

    /// How many letters (grapheme clusters) of its own line a span covers, at least 1.
    pub fn letters_covered(&self, span: Span) -> usize {
        let line = self.line_number(span.start);
        let start = self.line_starts[line - 1];
        let line_end = start + self.line_text(line).len();
        let end = span.end.min(line_end).max(span.start);
        self.text[span.start..end].graphemes(true).count().max(1)
    }
}
