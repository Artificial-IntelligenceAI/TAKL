//! Splits each line into tokens.
//!
//! trace: decision.spelling.cant_change
//!
//! What autocorrect changes is read as what was typed: `’` as `'`, `“ ”` as `"`, `—` as `-`. The token keeps the
//! place of what was really written, so an error points at the kid's own characters.

use crate::source::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// A name or one of TAKL's words, with `’` read as `'`.
    Word(String),
    /// `@name`, held without its `@`.
    Variable(String),
    /// A written number: its whole digits, and its digits after the `.` if it has one.
    Number { whole: String, after_point: Option<String> },
    /// Text between quotes, without them.
    Text(String),
    Symbol(char),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn is_word(&self, word: &str) -> bool {
        matches!(&self.kind, TokenKind::Word(w) if w == word)
    }

    pub fn is_symbol(&self, symbol: char) -> bool {
        self.kind == TokenKind::Symbol(symbol)
    }
}

/// A problem found while splitting a line; the parser turns it into a message.
#[derive(Clone, Debug)]
pub enum LexProblem {
    // trace: decision.text.quotes
    /// A `\` inside text, which isn't designed.
    Backslash(Span),
    /// Text whose closing quote never comes on its line.
    TextNotClosed(Span),
}

fn word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn word_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || c == '’'
}

/// The tokens of one line, which counts from 1, and what went wrong splitting it.
pub fn lex_line(line_number: u32, line: &str) -> (Vec<Token>, Vec<LexProblem>) {
    let characters: Vec<(usize, char)> = line.char_indices().collect();
    let end_of = |index: usize| characters.get(index).map(|(at, _)| *at).unwrap_or(line.len());
    let mut tokens = Vec::new();
    let mut problems = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let (start, c) = characters[index];
        if c.is_whitespace() {
            index += 1;
            continue;
        }
        if c == '#' {
            break;
        }
        if c == '"' || c == '“' || c == '”' {
            let mut text = String::new();
            let mut closed = false;
            index += 1;
            while index < characters.len() {
                let (at, inside) = characters[index];
                if inside == '"' || inside == '“' || inside == '”' {
                    closed = true;
                    index += 1;
                    break;
                }
                if inside == '\\' && !text.contains('\\') {
                    problems.push(LexProblem::Backslash(Span::new(line_number, at, at + 1)));
                }
                text.push(inside);
                index += 1;
            }
            let span = Span::new(line_number, start, end_of(index));
            if !closed {
                problems.push(LexProblem::TextNotClosed(span));
            }
            tokens.push(Token { kind: TokenKind::Text(text), span });
            continue;
        }
        if c.is_ascii_digit() {
            let mut whole = String::new();
            while index < characters.len() && characters[index].1.is_ascii_digit() {
                whole.push(characters[index].1);
                index += 1;
            }
            let mut after_point = None;
            if index + 1 < characters.len() && characters[index].1 == '.' && characters[index + 1].1.is_ascii_digit() {
                index += 1;
                let mut digits = String::new();
                while index < characters.len() && characters[index].1.is_ascii_digit() {
                    digits.push(characters[index].1);
                    index += 1;
                }
                after_point = Some(digits);
            }
            tokens.push(Token { kind: TokenKind::Number { whole, after_point }, span: Span::new(line_number, start, end_of(index)) });
            continue;
        }
        if c == '@' && index + 1 < characters.len() && word_start(characters[index + 1].1) {
            index += 1;
            let mut name = String::new();
            while index < characters.len() && word_continue(characters[index].1) {
                let inside = characters[index].1;
                name.push(if inside == '’' { '\'' } else { inside });
                index += 1;
            }
            tokens.push(Token { kind: TokenKind::Variable(name), span: Span::new(line_number, start, end_of(index)) });
            continue;
        }
        if word_start(c) {
            let mut word = String::new();
            while index < characters.len() && word_continue(characters[index].1) {
                let inside = characters[index].1;
                word.push(if inside == '’' { '\'' } else { inside });
                index += 1;
            }
            tokens.push(Token { kind: TokenKind::Word(word), span: Span::new(line_number, start, end_of(index)) });
            continue;
        }
        let symbol = if c == '—' { '-' } else { c };
        index += 1;
        tokens.push(Token { kind: TokenKind::Symbol(symbol), span: Span::new(line_number, start, end_of(index)) });
    }
    (tokens, problems)
}
