//! A program's text and the places in it.
//!
//! trace: decision.errors.layout
//!
//! A place is a line and a byte range inside that line. An error shows the place twice: `file:line:column` with the
//! column in UTF-8 bytes, for editors, and `line N column M` with the column in letters as a kid sees them (grapheme
//! clusters), for people. The `^` run under the code is counted in letters too.

use unicode_segmentation::UnicodeSegmentation;

/// A part of one line: `start` and `end` are byte offsets inside the line, `line` counts from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Span {
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(line: u32, start: usize, end: usize) -> Span {
        Span { line, start: start as u32, end: end as u32 }
    }

    /// From the start of `self` to the end of `other`, which is on the same line or later.
    pub fn to(self, other: Span) -> Span {
        if other.line == self.line { Span { line: self.line, start: self.start, end: other.end.max(self.end) } } else { self }
    }
}

/// The program's file name, as it is written in error messages, and its lines.
pub struct Source {
    pub name: String,
    pub lines: Vec<String>,
}

impl Source {
    pub fn new(name: &str, text: &str) -> Source {
        let lines = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line).to_string()).collect();
        Source { name: name.to_string(), lines }
    }

    pub fn line(&self, number: u32) -> &str {
        self.lines.get(number as usize - 1).map(String::as_str).unwrap_or("")
    }

    /// The text a span covers.
    pub fn text(&self, span: Span) -> &str {
        let line = self.line(span.line);
        line.get(span.start as usize..span.end as usize).unwrap_or("")
    }

    /// `file:line:column, line N column M`.
    pub fn place(&self, span: Span) -> String {
        let line = self.line(span.line);
        let before = line.get(..span.start as usize).unwrap_or(line);
        let letters = before.graphemes(true).count();
        format!("{}:{}:{}, line {} column {}", self.name, span.line, span.start + 1, span.line, letters + 1)
    }

    /// The code line with its number, then the `^` run under the span.
    pub fn pointer(&self, span: Span) -> String {
        let line = self.line(span.line);
        let gutter = format!("  {} | ", span.line);
        let before = line.get(..span.start as usize).unwrap_or(line);
        let covered = line.get(span.start as usize..span.end as usize).unwrap_or("");
        let indent = gutter.chars().count() + before.graphemes(true).count();
        let width = covered.graphemes(true).count().max(1);
        format!("{}{}\n{}{}", gutter, line, " ".repeat(indent), "^".repeat(width))
    }
}
