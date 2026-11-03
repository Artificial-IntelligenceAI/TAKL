//! A mistake TAKL found, and how it is laid out for a kid.
//!
//! trace: decision.errors.layout
//! trace: decision.errors.all

use crate::source::{Source, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Head {
    /// What went wrong.
    What(String),
    /// The one fix, as code to copy.
    Fix(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Because {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub span: Span,
    pub head: Head,
    /// The rule in plain words; one line or several.
    pub why: String,
    pub because: Vec<Because>,
}

impl Failure {
    pub fn what(span: Span, what: impl Into<String>, why: impl Into<String>) -> Failure {
        Failure { span, head: Head::What(what.into()), why: why.into(), because: Vec::new() }
    }

    pub fn fix(span: Span, fix: impl Into<String>, why: impl Into<String>) -> Failure {
        Failure { span, head: Head::Fix(fix.into()), why: why.into(), because: Vec::new() }
    }

    pub fn because(mut self, text: impl Into<String>, span: Span) -> Failure {
        self.because.push(Because { text: text.into(), span });
        self
    }

    fn block(&self, source: &Source, kind: &str) -> String {
        let mut out = format!("{} {}\n{}\n", kind, source.place(self.span), source.pointer(self.span));
        match &self.head {
            Head::What(what) => out.push_str(&format!("what: {}\n", what)),
            Head::Fix(fix) => out.push_str(&format!("fix:  {}\n", fix)),
        }
        for (number, line) in self.why.split('\n').enumerate() {
            out.push_str(if number == 0 { "why:  " } else { "      " });
            out.push_str(line);
            out.push('\n');
        }
        for because in &self.because {
            out.push_str(&format!("because: {}, {}\n{}\n", because.text, source.place(because.span), source.pointer(because.span)));
        }
        out
    }

    /// One error that stopped a running program.
    pub fn render_running(&self, source: &Source) -> String {
        self.block(source, "Stopped while running:")
    }
}

/// Every mistake found before running, in the order they are in the program, then the line pointing at the first.
pub fn render_before_running(failures: &[Failure], source: &Source) -> String {
    let mut out = String::new();
    for failure in failures {
        out.push_str(&failure.block(source, "Stopped before running:"));
        out.push('\n');
    }
    let first = source.place(failures[0].span);
    if failures.len() == 1 {
        out.push_str(&format!("1 mistake found: {}.\n", first));
    } else {
        out.push_str(&format!("{} mistakes found. Start with the first one: {}.\n", failures.len(), first));
    }
    out
}
