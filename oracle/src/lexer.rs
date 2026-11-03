//! Turns each line of a program into words, names, numbers, text and marks.

use crate::diagnostic::Diagnostic;
use crate::messages;
use crate::source::{Source, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// A word: one of TAKL's own or a name a kid made, such as `show` or `small_whole`.
    Word(String),
    /// A variable, with its `@`.
    Variable(String),
    /// A written number: its whole part, and the part after the point if there is one.
    Number(String, Option<String>),
    Text(String),
    Colon,
    Comma,
    Bar,
    LeftBracket,
    RightBracket,
    LeftSquare,
    RightSquare,
    Equals,
    Tilde,
    /// A mark TAKL has no use for, kept so the parser can say what to write instead.
    Mark(char),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn is_word(&self, word: &str) -> bool {
        matches!(&self.kind, TokenKind::Word(w) if w == word)
    }
}

/// One line that has something on it, comments left out.
#[derive(Clone, Debug)]
pub struct Line {
    pub tokens: Vec<Token>,
    pub span: Span,
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_word_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_apostrophe(c: char) -> bool {
    // trace: decision.spelling.cant_change (autocorrect's curly apostrophe is read as typed)
    c == '\'' || c == '\u{2019}' || c == '\u{2018}'
}

fn is_open_quote(c: char) -> bool {
    c == '"' || c == '\u{201C}'
}

fn is_close_quote(c: char) -> bool {
    c == '"' || c == '\u{201D}'
}

/// Words a kid may spell without the apostrophe, read as if it were there.
fn spelling(word: String) -> String {
    // trace: decision.spelling.cant_change
    // trace: decision.conditions.isnt
    match word.as_str() {
        "cant_change" => "can't_change".to_string(),
        "isnt" => "isn't".to_string(),
        _ => word,
    }
}

pub fn lex(source: &Source) -> (Vec<Line>, Vec<Diagnostic>) {
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    for (line_start, text) in source.lines() {
        let chars: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (line_start + i, c)).collect();
        let end_of = |index: usize| chars.get(index).map(|(offset, _)| *offset).unwrap_or(line_start + text.len());
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let (offset, c) = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            // trace: decision.syntax.let (`#` starts a comment)
            if c == '#' {
                break;
            }
            let start = i;
            let kind = if is_word_start(c) {
                let mut word = String::new();
                while i < chars.len() {
                    let c = chars[i].1;
                    if is_word_part(c) {
                        word.push(c);
                        i += 1;
                    } else if is_apostrophe(c)
                        && !word.is_empty()
                        && chars.get(i + 1).is_some_and(|(_, next)| next.is_alphabetic())
                    {
                        word.push('\'');
                        i += 1;
                    } else {
                        break;
                    }
                }
                TokenKind::Word(spelling(word))
            } else if c == '@' {
                i += 1;
                let mut name = String::from("@");
                while i < chars.len() && is_word_part(chars[i].1) {
                    name.push(chars[i].1);
                    i += 1;
                }
                if name.len() == 1 {
                    errors.push(messages::lone_at(Span::new(offset, end_of(i))));
                    continue;
                }
                TokenKind::Variable(name)
            } else if c.is_ascii_digit() {
                let mut whole = String::new();
                while i < chars.len() && chars[i].1.is_ascii_digit() {
                    whole.push(chars[i].1);
                    i += 1;
                }
                let mut fraction = None;
                if i + 1 < chars.len() && chars[i].1 == '.' && chars[i + 1].1.is_ascii_digit() {
                    i += 1;
                    let mut digits = String::new();
                    while i < chars.len() && chars[i].1.is_ascii_digit() {
                        digits.push(chars[i].1);
                        i += 1;
                    }
                    fraction = Some(digits);
                }
                TokenKind::Number(whole, fraction)
            } else if is_open_quote(c) {
                // trace: decision.text.quotes
                i += 1;
                let mut content = String::new();
                let mut closed = false;
                let mut backslash = None;
                while i < chars.len() {
                    let c = chars[i].1;
                    if is_close_quote(c) {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if c == '\\' && backslash.is_none() {
                        backslash = Some(chars[i].0);
                    }
                    content.push(c);
                    i += 1;
                }
                if !closed {
                    // Kept as text, so the rest of the line is read without a second error.
                    errors.push(messages::text_not_closed(Span::new(offset, end_of(i))));
                }
                if let Some(at) = backslash {
                    errors.push(messages::backslash_in_text(Span::new(at, at + 1)));
                }
                TokenKind::Text(content)
            } else {
                i += 1;
                match c {
                    ':' => TokenKind::Colon,
                    ',' => TokenKind::Comma,
                    '|' => TokenKind::Bar,
                    '(' => TokenKind::LeftBracket,
                    ')' => TokenKind::RightBracket,
                    '[' => TokenKind::LeftSquare,
                    ']' => TokenKind::RightSquare,
                    '=' => TokenKind::Equals,
                    '~' => TokenKind::Tilde,
                    // trace: decision.spelling.cant_change (autocorrect's long dash is read as `-`)
                    '\u{2014}' => TokenKind::Mark('-'),
                    other => TokenKind::Mark(other),
                }
            };
            tokens.push(Token { kind, span: Span::new(chars[start].0, end_of(i)) });
        }
        if let (Some(first), Some(last)) = (tokens.first(), tokens.last()) {
            let span = Span::new(first.span.start, last.span.end);
            lines.push(Line { tokens, span });
        }
    }
    (lines, errors)
}
