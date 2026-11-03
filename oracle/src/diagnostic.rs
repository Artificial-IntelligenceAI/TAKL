//! The error layout, from Radium's and made for kids.
//!
//! trace: decision.errors.layout
//! trace: decision.errors.all

use crate::messages;
use crate::source::{Source, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    BeforeRunning,
    WhileRunning,
}

/// The first label: `what:` says what is wrong; `fix:` replaces it when exactly one fix is certain.
#[derive(Clone, Debug)]
pub enum Headline {
    What(String),
    Fix(String),
}

/// An earlier step that led to the error, with one fact about it.
#[derive(Clone, Debug)]
pub struct Because {
    pub span: Span,
    pub fact: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub stage: Stage,
    pub span: Span,
    pub headline: Headline,
    pub why: String,
    pub when: Option<String>,
    pub because: Vec<Because>,
}

impl Diagnostic {
    pub fn before(span: Span, headline: Headline, why: impl Into<String>) -> Diagnostic {
        Diagnostic { stage: Stage::BeforeRunning, span, headline, why: why.into(), when: None, because: Vec::new() }
    }

    pub fn while_running(span: Span, headline: Headline, why: impl Into<String>) -> Diagnostic {
        Diagnostic { stage: Stage::WhileRunning, span, headline, why: why.into(), when: None, because: Vec::new() }
    }

    pub fn when(mut self, when: impl Into<String>) -> Diagnostic {
        self.when = Some(when.into());
        self
    }

    pub fn because(mut self, span: Span, fact: impl Into<String>) -> Diagnostic {
        self.because.push(Because { span, fact: fact.into() });
        self
    }
}

fn place_text(source: &Source, span: Span) -> String {
    let place = source.place(span.start);
    format!("{}:{}:{}, line {} column {}", source.path, place.line, place.byte_column, place.line, place.letter_column)
}

fn code_lines(source: &Source, span: Span, margin: usize) -> String {
    let place = source.place(span.start);
    let line = source.line_text(place.line);
    let pointer = "^".repeat(source.letters_covered(span));
    let indent = " ".repeat(place.letter_column - 1);
    // The pointer line has no bar, so the `^` run stands alone under the code it points at.
    format!("  {:>margin$} | {}\n  {}   {}{}\n", place.line, line, " ".repeat(margin), indent, pointer, margin = margin)
}

fn label(name: &str, text: &str) -> String {
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 {
            out.push_str(&format!("{:<6}{}\n", format!("{}:", name), line));
        } else {
            out.push_str(&format!("      {}\n", line));
        }
    }
    out
}

/// One error, in the decided layout.
pub fn render(source: &Source, diagnostic: &Diagnostic) -> String {
    let mut lines = vec![source.place(diagnostic.span.start).line];
    lines.extend(diagnostic.because.iter().map(|because| source.place(because.span.start).line));
    let margin = lines.iter().map(|line| line.to_string().len()).max().unwrap_or(1);

    let kind = match diagnostic.stage {
        Stage::BeforeRunning => "Stopped before running:",
        Stage::WhileRunning => "Stopped while running:",
    };
    let mut out = format!("{} {}\n", kind, place_text(source, diagnostic.span));
    out.push_str(&code_lines(source, diagnostic.span, margin));
    match &diagnostic.headline {
        Headline::What(what) => out.push_str(&label("what", what)),
        Headline::Fix(fix) => out.push_str(&label("fix", fix)),
    }
    out.push_str(&label("why", &diagnostic.why));
    if let Some(when) = &diagnostic.when {
        out.push_str(&label("when", when));
    }
    for because in &diagnostic.because {
        out.push_str(&format!("because: {}, {}\n", because.fact, place_text(source, because.span)));
        out.push_str(&code_lines(source, because.span, margin));
    }
    out
}

/// Every compile error, in the order they appear, then a last line pointing at the first.
pub fn render_compile_errors(source: &Source, mut errors: Vec<Diagnostic>) -> String {
    errors.sort_by_key(|error| (error.span.start, error.span.end));
    errors.dedup_by_key(|error| (error.span.start, error.span.end));
    let mut out = String::new();
    for error in &errors {
        out.push_str(&render(source, error));
        out.push('\n');
    }
    out.push_str(&messages::mistakes_found(errors.len(), &place_text(source, errors[0].span)));
    out
}
