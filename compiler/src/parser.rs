//! Reads tokens into a program, one line at a time.
//!
//! trace: decision.syntax.let
//! trace: decision.syntax.no_semicolons
//!
//! A line TAKL can't read is reported once and skipped, so every separate mistake in a program is found in one go.

use crate::failure::Failure;
use crate::lexer::{LexProblem, Token, TokenKind, lex_line};
use crate::messages;
use crate::source::{Source, Span};
use crate::syntax::*;

/// TAKL's own words, which never name a procedure or start an argument.
pub const GRAMMAR_WORDS: &[&str] = &[
    "let", "show", "of", "in", "for", "each", "repeat", "times", "position", "offset", "procedure", "returns", "return",
    "end", "add", "to", "at", "if", "else", "until", "forever", "stop", "finish", "just", "finishes", "and", "or",
    "also", "not", "is", "isn't", "isnt", "more", "less", "than", "least", "most", "plus", "minus", "divided", "by",
    "over", "round", "remainder", "sum", "length", "biggest", "smallest", "average", "share", "empty", "true", "false",
    "table", "nothing",
];

pub fn is_grammar_word(word: &str) -> bool {
    GRAMMAR_WORDS.contains(&word)
}

struct Line {
    tokens: Vec<Token>,
    span: Span,
}

/// The tokens of one line and how far reading has got.
struct Cursor<'a> {
    tokens: &'a [Token],
    at: usize,
    /// Set once the line can't be read further; nothing more is reported for it.
    broken: bool,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.at)
    }

    fn peek_at(&self, ahead: usize) -> Option<&'a Token> {
        self.tokens.get(self.at + ahead)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.at);
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    fn at_word(&self, word: &str) -> bool {
        self.peek().is_some_and(|token| token.is_word(word))
    }

    fn at_symbol(&self, symbol: char) -> bool {
        self.peek().is_some_and(|token| token.is_symbol(symbol))
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.at_word(word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn eat_symbol(&mut self, symbol: char) -> bool {
        if self.at_symbol(symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn done(&self) -> bool {
        self.at >= self.tokens.len()
    }

    fn last_span(&self) -> Span {
        self.tokens[self.at.saturating_sub(1).min(self.tokens.len() - 1)].span
    }

    fn here(&self) -> Span {
        self.peek().map(|token| token.span).unwrap_or_else(|| self.last_span())
    }
}

pub struct Parser<'s> {
    source: &'s Source,
    lines: Vec<Line>,
    at: usize,
    pub failures: Vec<Failure>,
}

/// What a block's closing line was.
enum Closing {
    End { word: Option<String>, line: usize },
    ElseIf { line: usize },
    Else { line: usize },
    Procedure { line: usize },
    EndOfFile,
}

fn token_text(token: &Token) -> String {
    match &token.kind {
        TokenKind::Word(word) => word.clone(),
        TokenKind::Variable(name) => format!("@{}", name),
        TokenKind::Number { whole, after_point: Some(digits) } => format!("{}.{}", whole, digits),
        TokenKind::Number { whole, after_point: None } => whole.clone(),
        TokenKind::Text(text) => format!("\"{}\"", text),
        TokenKind::Symbol(symbol) => symbol.to_string(),
    }
}

/// Words that start a value, so a name followed by one is a call without brackets.
fn starts_value(token: &Token) -> bool {
    match &token.kind {
        TokenKind::Number { .. } | TokenKind::Text(_) | TokenKind::Variable(_) => true,
        TokenKind::Symbol(symbol) => matches!(symbol, '[' | '(' | '~'),
        TokenKind::Word(word) => {
            matches!(word.as_str(), "true" | "false" | "empty" | "position" | "offset" | "round" | "remainder" | "share")
                || Family::from_word(word).is_some()
                || !is_grammar_word(word)
        }
    }
}

impl<'s> Parser<'s> {
    pub fn new(source: &'s Source) -> Parser<'s> {
        let mut failures = Vec::new();
        let mut lines = Vec::new();
        for (index, text) in source.lines.iter().enumerate() {
            let number = index as u32 + 1;
            let (tokens, problems) = lex_line(number, text);
            for problem in problems {
                failures.push(match problem {
                    LexProblem::Backslash(span) => messages::not_designed_yet(span, "what `\\` does inside text"),
                    LexProblem::TextNotClosed(span) => {
                        messages::not_designed_yet(span, "this text has no closing `\"`")
                    }
                });
            }
            if let (Some(first), Some(last)) = (tokens.first(), tokens.last()) {
                let span = first.span.to(last.span);
                lines.push(Line { tokens, span });
            }
        }
        Parser { source, lines, at: 0, failures }
    }

    fn fail(&mut self, failure: Failure) {
        self.failures.push(failure);
    }

    fn text(&self, span: Span) -> String {
        self.source.text(span).to_string()
    }

    pub fn program(mut self) -> (Program, Vec<Failure>) {
        let mut procedures = Vec::new();
        while self.at < self.lines.len() {
            let line = &self.lines[self.at];
            if line.tokens[0].is_word("procedure") {
                if let Some(procedure) = self.procedure() {
                    procedures.push(procedure);
                }
            } else {
                let span = line.span;
                self.fail(messages::outside_procedures(span));
                self.at += 1;
            }
        }
        (Program { procedures }, self.failures)
    }

    // trace: decision.procedures.shape
    fn procedure(&mut self) -> Option<Procedure> {
        let line_index = self.at;
        self.at += 1;
        let tokens = std::mem::take(&mut self.lines[line_index].tokens);
        let header_span = self.lines[line_index].span;
        let mut cursor = Cursor { tokens: &tokens, at: 1, broken: false };
        let (name, name_span) = match cursor.next() {
            Some(Token { kind: TokenKind::Word(word), span }) => (word.clone(), *span),
            Some(token) => {
                self.fail(messages::does_not_belong(token.span, &token_text(token)));
                (String::new(), token.span)
            }
            None => {
                self.fail(messages::not_designed_yet(header_span, "a procedure with no name"));
                (String::new(), header_span)
            }
        };
        if is_grammar_word(&name) {
            if name == "add" {
                self.fail(messages::grammar_word_as_name(name_span, &name));
            } else {
                self.fail(messages::not_designed_yet(name_span, format!("`{}` as a procedure's name", name)));
            }
        }
        let mut params = Vec::new();
        if cursor.at_symbol('(') {
            cursor.next();
            loop {
                if let Some(param) = self.param(&mut cursor) {
                    params.push(param);
                }
                if cursor.broken {
                    break;
                }
                if cursor.eat_symbol('|') {
                    continue;
                }
                if cursor.eat_symbol(')') {
                    break;
                }
                self.unreadable(&mut cursor);
                break;
            }
        }
        let ending = if cursor.broken {
            Ending::JustFinishes
        } else if cursor.at_word("returns") {
            let returns_span = cursor.here();
            cursor.next();
            let written = self.written_type(&mut cursor, false);
            let span = returns_span.to(written.span);
            Ending::Returns(written, span)
        } else if cursor.at_word("just") && cursor.peek_at(1).is_some_and(|token| token.is_word("finishes")) {
            cursor.next();
            cursor.next();
            Ending::JustFinishes
        } else if cursor.at_symbol(':') {
            Ending::Missing(cursor.here())
        } else {
            self.unreadable(&mut cursor);
            Ending::JustFinishes
        };
        if !cursor.broken && !cursor.eat_symbol(':') {
            self.unreadable(&mut cursor);
        }
        if !cursor.broken && !cursor.done() {
            self.unreadable(&mut cursor);
        }
        let (body, closing) = self.block();
        let mut end_span = None;
        match closing {
            Closing::End { word, line } => {
                let span = self.lines[line].span;
                end_span = Some(span);
                let tokens = &self.lines[line].tokens;
                let named = tokens.get(2).and_then(|token| match &token.kind {
                    TokenKind::Word(word) => Some(word.clone()),
                    _ => None,
                });
                if word.as_deref() != Some("procedure") {
                    let failure = messages::not_designed_yet(span, "a block closed with the wrong `end`");
                    self.fail(failure);
                } else if named.as_deref() != Some(name.as_str()) || tokens.len() != 3 {
                    self.fail(messages::end_names(span, &name));
                }
                self.at = line + 1;
            }
            Closing::Procedure { line } | Closing::ElseIf { line } | Closing::Else { line } => {
                self.fail(messages::not_designed_yet(
                    header_span,
                    format!("this block never ends: `end procedure {}` is missing", name),
                ));
                self.at = line;
                if !self.lines[line].tokens.first().is_some_and(|token| token.is_word("procedure")) {
                    self.at = line + 1;
                }
            }
            Closing::EndOfFile => {
                self.fail(messages::not_designed_yet(
                    header_span,
                    format!("this block never ends: `end procedure {}` is missing", name),
                ));
            }
        }
        self.lines[line_index].tokens = tokens;
        Some(Procedure { name, name_span, header_span, params, ending, body, end_span })
    }

    fn param(&mut self, cursor: &mut Cursor) -> Option<Param> {
        let (name, name_span) = match cursor.next() {
            Some(Token { kind: TokenKind::Variable(name), span }) => (name.clone(), *span),
            Some(Token { kind: TokenKind::Word(word), span }) => {
                self.fail(messages::needs_at(*span, word));
                (word.clone(), *span)
            }
            _ => {
                cursor.at = cursor.at.saturating_sub(1);
                self.unreadable(cursor);
                return None;
            }
        };
        if !cursor.eat_symbol(':') {
            self.unreadable(cursor);
            return None;
        }
        let written_type = self.written_type(cursor, false);
        let default = if cursor.eat_symbol('=') { Some(self.test(cursor)) } else { None };
        Some(Param { name, name_span, written_type, default })
    }

    /// `small_whole, never_negative`, `table of (tiny_whole, never_negative)`, `table of text to small_whole`.
    fn written_type(&mut self, cursor: &mut Cursor, inside_brackets: bool) -> WrittenType {
        let start = cursor.here();
        let base = self.type_base(cursor);
        let is_table = matches!(base, Base::Table { .. });
        let base_end = cursor.last_span();
        let mut modifiers = Vec::new();
        while !cursor.broken && cursor.at_symbol(',') {
            // A `,` that isn't followed by a modifier word belongs to whatever comes after the type.
            match cursor.peek_at(1) {
                Some(Token { kind: TokenKind::Word(_), .. }) => {}
                _ => break,
            }
            cursor.next();
            let Some(modifier) = self.modifier(cursor) else { break };
            let belongs_to_items = matches!(
                modifier.kind,
                ModifierKind::NeverNegative
                    | ModifierKind::FastButInexact
                    | ModifierKind::AtLeast(_)
                    | ModifierKind::AtMost(_)
            );
            if is_table && belongs_to_items && !inside_brackets {
                // trace: decision.tables.item_modifiers
                if let Base::Table { item, .. } = &base {
                    let fix = format!(
                        "table of ({}, {})",
                        self.text(item.span),
                        self.text(modifier.span)
                    );
                    self.fail(messages::item_modifier(modifier.span, fix));
                }
                continue;
            }
            modifiers.push(modifier);
        }
        let end = modifiers.last().map(|modifier| modifier.span).unwrap_or(base_end);
        WrittenType { base, modifiers, span: start.to(end) }
    }

    fn type_base(&mut self, cursor: &mut Cursor) -> Base {
        match cursor.next() {
            Some(token) if token.is_word("table") && cursor.at_word("of") => {
                cursor.next();
                let mut item = self.table_item(cursor);
                let mut key = None;
                if cursor.eat_word("to") {
                    key = Some(Box::new(item));
                    item = self.table_item(cursor);
                }
                Base::Table { item: Box::new(item), key }
            }
            Some(Token { kind: TokenKind::Word(word), span }) => Base::Named(word.clone(), *span),
            Some(token) => {
                cursor.at -= 1;
                let span = token.span;
                self.unreadable(cursor);
                Base::Named(String::new(), span)
            }
            None => {
                let span = cursor.last_span();
                self.unreadable(cursor);
                Base::Named(String::new(), span)
            }
        }
    }

    fn table_item(&mut self, cursor: &mut Cursor) -> WrittenType {
        if cursor.at_symbol('(') {
            let open = cursor.here();
            cursor.next();
            let inner = self.written_type(cursor, true);
            if !cursor.eat_symbol(')') {
                self.unreadable(cursor);
            }
            WrittenType { span: open.to(cursor.last_span()), ..inner }
        } else {
            let start = cursor.here();
            let base = self.type_base(cursor);
            WrittenType { base, modifiers: Vec::new(), span: start.to(cursor.last_span()) }
        }
    }

    fn modifier(&mut self, cursor: &mut Cursor) -> Option<Modifier> {
        let token = cursor.next()?;
        let TokenKind::Word(word) = &token.kind else { return None };
        let kind = match word.as_str() {
            "never_negative" => ModifierKind::NeverNegative,
            "can't_change" | "cant_change" => ModifierKind::CantChange,
            "fast_but_inexact" => ModifierKind::FastButInexact,
            "share" => ModifierKind::Share,
            "at" if cursor.at_word("least") || cursor.at_word("most") => {
                let least = cursor.at_word("least");
                cursor.next();
                // trace: decision.types.bounds
                match cursor.peek() {
                    Some(Token { kind: TokenKind::Number { whole, after_point: None }, .. }) => {
                        cursor.next();
                        let value: num_bigint::BigInt = whole.parse().unwrap_or_default();
                        if least { ModifierKind::AtLeast(value) } else { ModifierKind::AtMost(value) }
                    }
                    Some(minus) if minus.is_symbol('-') => {
                        self.fail(messages::not_designed_yet(minus.span, "how a number below zero is written"));
                        cursor.next();
                        if matches!(cursor.peek(), Some(Token { kind: TokenKind::Number { .. }, .. })) {
                            cursor.next();
                        }
                        ModifierKind::BrokenBound
                    }
                    _ => {
                        self.unreadable(cursor);
                        ModifierKind::BrokenBound
                    }
                }
            }
            other => ModifierKind::Unknown(other.to_string()),
        };
        Some(Modifier { kind, span: token.span.to(cursor.last_span()) })
    }

    /// Reports the token where reading stopped, and stops reading the line.
    fn unreadable(&mut self, cursor: &mut Cursor) {
        if cursor.broken {
            return;
        }
        cursor.broken = true;
        match cursor.peek() {
            Some(token) if token.is_symbol(';') && cursor.at + 1 == cursor.tokens.len() => {
                self.fail(messages::semicolon(token.span))
            }
            Some(token) => self.fail(messages::does_not_belong(token.span, &token_text(token))),
            None => {
                let span = cursor.last_span();
                self.fail(messages::not_designed_yet(span, "a line that stops before it's finished"));
            }
        }
    }

    /// Statements until a line that closes the block.
    fn block(&mut self) -> (Vec<Statement>, Closing) {
        let mut statements = Vec::new();
        while self.at < self.lines.len() {
            let first = &self.lines[self.at].tokens[0];
            let second = self.lines[self.at].tokens.get(1);
            if first.is_word("end") {
                let word = second.and_then(|token| match &token.kind {
                    TokenKind::Word(word) => Some(word.clone()),
                    _ => None,
                });
                return (statements, Closing::End { word, line: self.at });
            }
            if first.is_word("end_of") {
                // trace: decision.blocks.end
                let span = first.span;
                let word = second.and_then(|token| match &token.kind {
                    TokenKind::Word(word) => Some(word.clone()),
                    _ => None,
                });
                self.fail(messages::end_of(span));
                return (statements, Closing::End { word, line: self.at });
            }
            if first.is_word("else") {
                if second.is_some_and(|token| token.is_word("if")) {
                    return (statements, Closing::ElseIf { line: self.at });
                }
                return (statements, Closing::Else { line: self.at });
            }
            if first.is_word("procedure") {
                return (statements, Closing::Procedure { line: self.at });
            }
            if let Some(statement) = self.statement() {
                statements.push(statement);
            }
        }
        (statements, Closing::EndOfFile)
    }

    /// Checks that a block closed with `end <word>`, and moves past the closing line.
    fn close(&mut self, closing: Closing, word: &str) {
        match closing {
            Closing::End { word: Some(found), line } if found == word => {
                if self.lines[line].tokens.len() > 2 && word != "table" {
                    let mut cursor = Cursor { tokens: &self.lines[line].tokens, at: 2, broken: false };
                    let token = cursor.next().cloned();
                    if let Some(token) = token {
                        self.fail(messages::does_not_belong(token.span, &token_text(&token)));
                    }
                }
                self.at = line + 1;
            }
            Closing::End { line, .. } | Closing::ElseIf { line } | Closing::Else { line } => {
                let span = self.lines[line].span;
                self.fail(messages::not_designed_yet(span, format!("a block closed with something other than `end {}`", word)));
                self.at = line + 1;
            }
            Closing::Procedure { line } => {
                let span = self.lines[line].span;
                self.fail(messages::not_designed_yet(span, format!("a block with no `end {}`", word)));
            }
            Closing::EndOfFile => {}
        }
    }

    fn statement(&mut self) -> Option<Statement> {
        let line_index = self.at;
        self.at += 1;
        let tokens = std::mem::take(&mut self.lines[line_index].tokens);
        let span = self.lines[line_index].span;
        let mut cursor = Cursor { tokens: &tokens, at: 0, broken: false };
        let statement = self.statement_in(&mut cursor, span);
        self.lines[line_index].tokens = tokens;
        statement
    }

    fn statement_in(&mut self, cursor: &mut Cursor, span: Span) -> Option<Statement> {
        let first = cursor.peek()?.clone();
        let kind = match &first.kind {
            TokenKind::Word(word) => match word.as_str() {
                "let" => return Some(self.let_statement(cursor, span)),
                "show" => {
                    cursor.next();
                    // trace: decision.show.pieces
                    let mut pieces = Vec::new();
                    if cursor.done() || cursor.at_word("if") {
                        self.fail(messages::not_designed_yet(first.span, "a `show` with nothing to show"));
                    } else {
                        loop {
                            pieces.push(self.test(cursor));
                            if !cursor.eat_symbol(',') {
                                break;
                            }
                        }
                    }
                    StatementKind::Show(pieces)
                }
                "if" => return Some(self.if_statement(cursor, span)),
                "repeat" => return Some(self.repeat_statement(cursor, span)),
                "for" => return Some(self.for_each(cursor, span)),
                "stop" => {
                    cursor.next();
                    StatementKind::Stop
                }
                "finish" => {
                    cursor.next();
                    StatementKind::Finish
                }
                "return" => {
                    cursor.next();
                    if cursor.at_word("nothing") {
                        cursor.next();
                        // trace: decision.procedures.finish
                        self.fail(messages::return_nothing(span));
                        StatementKind::Finish
                    } else {
                        StatementKind::Return(self.test(cursor))
                    }
                }
                "add" => self.add_statement(cursor),
                _ if !is_grammar_word(word) => self.call_statement(cursor),
                _ => {
                    self.unreadable(cursor);
                    StatementKind::Broken
                }
            },
            TokenKind::Variable(name) => {
                cursor.next();
                if cursor.eat_symbol('=') {
                    let value = self.test(cursor);
                    StatementKind::Assign { name: name.clone(), name_span: first.span, value }
                } else {
                    self.unreadable(cursor);
                    StatementKind::Broken
                }
            }
            _ => {
                self.unreadable(cursor);
                StatementKind::Broken
            }
        };
        // trace: decision.conditions.trailing_if
        let mut only_if = None;
        if !cursor.broken && cursor.eat_word("if") {
            only_if = Some(self.test(cursor));
        }
        if !cursor.broken && !cursor.done() {
            self.unreadable(cursor);
        }
        let kind = if cursor.broken && matches!(kind, StatementKind::Show(_)) { StatementKind::Broken } else { kind };
        Some(Statement { kind, span, only_if })
    }

    fn let_statement(&mut self, cursor: &mut Cursor, span: Span) -> Statement {
        cursor.next();
        let (name, name_span) = match cursor.next() {
            Some(Token { kind: TokenKind::Variable(name), span }) => (name.clone(), *span),
            Some(Token { kind: TokenKind::Word(word), span }) => {
                // trace: decision.names.at_mark
                self.fail(messages::needs_at(*span, word));
                (word.clone(), *span)
            }
            _ => {
                cursor.at = cursor.at.saturating_sub(1);
                self.unreadable(cursor);
                return Statement { kind: StatementKind::Broken, span, only_if: None };
            }
        };
        if !cursor.eat_symbol(':') {
            self.unreadable(cursor);
            let kind = StatementKind::Let { name, name_span, written_type: None, value: LetValue::Missing };
            return Statement { kind, span, only_if: None };
        }
        let written_type = self.written_type(cursor, false);
        let value = if cursor.eat_symbol('=') {
            let value = self.test(cursor);
            if !cursor.broken && !cursor.done() {
                self.unreadable(cursor);
            }
            LetValue::Expression(value)
        } else if cursor.at_symbol(':') && cursor.at + 1 == cursor.tokens.len() {
            return self.let_block(name, name_span, written_type, span);
        } else if cursor.done() {
            LetValue::Missing
        } else {
            self.unreadable(cursor);
            LetValue::Expression(Expression::broken(span))
        };
        Statement { kind: StatementKind::Let { name, name_span, written_type: Some(written_type), value }, span, only_if: None }
    }

    // trace: decision.tables.shape
    fn let_block(&mut self, name: String, name_span: Span, written_type: WrittenType, span: Span) -> Statement {
        let mut items = Vec::new();
        loop {
            if self.at >= self.lines.len() {
                self.fail(messages::not_designed_yet(span, format!("this block never ends: `end table @{}` is missing", name)));
                break;
            }
            let tokens = std::mem::take(&mut self.lines[self.at].tokens);
            let line_span = self.lines[self.at].span;
            let line_index = self.at;
            self.at += 1;
            let mut cursor = Cursor { tokens: &tokens, at: 0, broken: false };
            if cursor.at_word("end") {
                let names_it = tokens.len() == 3
                    && tokens[1].is_word("table")
                    && matches!(&tokens[2].kind, TokenKind::Variable(found) if *found == name);
                if !names_it {
                    self.fail(messages::not_designed_yet(line_span, format!("a table block closed with something other than `end table @{}`", name)));
                }
                self.lines[line_index].tokens = tokens;
                break;
            }
            let first = self.test(&mut cursor);
            let item = if !cursor.broken && cursor.eat_symbol(':') {
                let value = self.test(&mut cursor);
                BlockItem { key: Some(first), value }
            } else {
                BlockItem { key: None, value: first }
            };
            if !cursor.broken && !cursor.done() {
                self.unreadable(&mut cursor);
            }
            items.push(item);
            self.lines[line_index].tokens = tokens;
        }
        let kind = StatementKind::Let { name, name_span, written_type: Some(written_type), value: LetValue::Block(items) };
        Statement { kind, span, only_if: None }
    }

    fn header_end(&mut self, cursor: &mut Cursor) {
        if !cursor.broken && !cursor.eat_symbol(':') {
            self.unreadable(cursor);
        }
        if !cursor.broken && !cursor.done() {
            self.unreadable(cursor);
        }
    }

    // trace: decision.conditions.if
    fn if_statement(&mut self, cursor: &mut Cursor, span: Span) -> Statement {
        cursor.next();
        let test = self.test(cursor);
        self.header_end(cursor);
        let mut branches = Vec::new();
        let mut otherwise = None;
        let (body, mut closing) = self.block();
        branches.push((test, body));
        loop {
            match closing {
                Closing::ElseIf { line } => {
                    self.at = line + 1;
                    let tokens = std::mem::take(&mut self.lines[line].tokens);
                    let mut cursor = Cursor { tokens: &tokens, at: 2, broken: false };
                    let test = self.test(&mut cursor);
                    self.header_end(&mut cursor);
                    self.lines[line].tokens = tokens;
                    let (body, next) = self.block();
                    branches.push((test, body));
                    closing = next;
                }
                Closing::Else { line } if otherwise.is_none() => {
                    self.at = line + 1;
                    let tokens = std::mem::take(&mut self.lines[line].tokens);
                    let mut cursor = Cursor { tokens: &tokens, at: 1, broken: false };
                    self.header_end(&mut cursor);
                    self.lines[line].tokens = tokens;
                    let (body, next) = self.block();
                    otherwise = Some(body);
                    closing = next;
                }
                other => {
                    self.close(other, "if");
                    break;
                }
            }
        }
        Statement { kind: StatementKind::If { branches, otherwise }, span, only_if: None }
    }

    // trace: decision.loops.repeat
    // trace: decision.loops.until
    // trace: decision.loops.forever
    fn repeat_statement(&mut self, cursor: &mut Cursor, span: Span) -> Statement {
        cursor.next();
        let kind_of = if cursor.eat_word("forever") {
            0
        } else if cursor.eat_word("until") {
            1
        } else {
            2
        };
        let mut test = None;
        let mut count = None;
        match kind_of {
            1 => test = Some(self.test(cursor)),
            2 => count = Some(self.bare_or_bracketed(cursor, "repeat", "times")),
            _ => {}
        }
        if kind_of == 2 && !cursor.broken && !cursor.eat_word("times") {
            self.unreadable(cursor);
        }
        self.header_end(cursor);
        let (body, closing) = self.block();
        self.close(closing, "repeat");
        let kind = match kind_of {
            0 => StatementKind::RepeatForever { body },
            1 => StatementKind::RepeatUntil { test: test.unwrap_or_else(|| Expression::broken(span)), body },
            _ => StatementKind::RepeatTimes { count: count.unwrap_or_else(|| Expression::broken(span)), body },
        };
        Statement { kind, span, only_if: None }
    }

    /// One number or one name standing alone, or anything in brackets; anything more is answered with brackets.
    fn bare_or_bracketed(&mut self, cursor: &mut Cursor, word: &str, closer: &str) -> Expression {
        let start = cursor.at;
        let simple = match cursor.peek() {
            Some(Token { kind: TokenKind::Number { .. } | TokenKind::Variable(_), .. }) => true,
            Some(token) if token.is_symbol('(') => true,
            _ => false,
        };
        let value = if simple { self.primary(cursor) } else { Expression::broken(cursor.here()) };
        if !simple {
            self.unreadable(cursor);
            return value;
        }
        if cursor.broken {
            return value;
        }
        // `repeat`'s count ends at the line's last `times`, since `times` is maths too; an index ends at the first
        // `of` outside brackets.
        let closing_at = if closer == "times" {
            (cursor.at..cursor.tokens.len()).rev().find(|&index| cursor.tokens[index].is_word(closer))
        } else {
            let mut depth = 0i32;
            (cursor.at..cursor.tokens.len()).find(|&index| {
                let token = &cursor.tokens[index];
                if token.is_symbol('(') {
                    depth += 1;
                } else if token.is_symbol(')') {
                    depth -= 1;
                }
                depth == 0 && token.is_word(closer)
            })
        };
        if closing_at == Some(cursor.at) {
            return value;
        }
        // trace: decision.lists.index_bare
        // More than one number or name: read it all, to say where the brackets go.
        let Some(closing_at) = closing_at.filter(|&index| index > cursor.at) else {
            self.unreadable(cursor);
            return value;
        };
        cursor.at = start;
        let inner_tokens = &cursor.tokens[..closing_at];
        let mut inner = Cursor { tokens: inner_tokens, at: start, broken: false };
        let whole = self.sum(&mut inner);
        let span = cursor.tokens[start].span.to(cursor.tokens[closing_at - 1].span);
        self.fail(messages::needs_brackets(span, word, format!("{} ({})", word, self.text(span))));
        cursor.at = closing_at;
        Expression { span: whole.span, kind: whole.kind }
    }

    // trace: decision.tables.of_family
    fn for_each(&mut self, cursor: &mut Cursor, span: Span) -> Statement {
        cursor.next();
        if !cursor.eat_word("each") {
            self.unreadable(cursor);
        }
        let mut names = Vec::new();
        while !cursor.broken {
            match cursor.next() {
                Some(Token { kind: TokenKind::Variable(name), span }) => names.push((name.clone(), *span)),
                Some(Token { kind: TokenKind::Word(word), span }) if !is_grammar_word(word) => {
                    self.fail(messages::needs_at(*span, word));
                    names.push((word.clone(), *span));
                }
                _ => {
                    cursor.at -= 1;
                    self.unreadable(cursor);
                    break;
                }
            }
            if !cursor.eat_word("and") {
                break;
            }
        }
        let table = if !cursor.broken && cursor.eat_word("in") {
            self.test(cursor)
        } else {
            self.unreadable(cursor);
            Expression::broken(span)
        };
        self.header_end(cursor);
        let (body, closing) = self.block();
        self.close(closing, "for");
        Statement { kind: StatementKind::ForEach { names, table, body }, span, only_if: None }
    }

    // trace: decision.tables.add
    fn add_statement(&mut self, cursor: &mut Cursor) -> StatementKind {
        cursor.next();
        let first = self.test(cursor);
        let (key, item) = if !cursor.broken && cursor.eat_symbol(':') {
            (Some(first), self.test(cursor))
        } else {
            (None, first)
        };
        if cursor.broken || !cursor.eat_word("to") {
            self.unreadable(cursor);
            return StatementKind::Broken;
        }
        let (table, table_span) = match cursor.next() {
            Some(Token { kind: TokenKind::Variable(name), span }) => (name.clone(), *span),
            Some(Token { kind: TokenKind::Word(word), span }) if !is_grammar_word(word) => {
                self.fail(messages::needs_at(*span, word));
                (word.clone(), *span)
            }
            _ => {
                cursor.at = cursor.at.saturating_sub(1);
                self.unreadable(cursor);
                return StatementKind::Broken;
            }
        };
        let mut at = None;
        if cursor.at_word("at") {
            cursor.next();
            let counting = if cursor.eat_word("position") {
                Counting::Position
            } else if cursor.eat_word("offset") {
                Counting::Offset
            } else {
                self.unreadable(cursor);
                return StatementKind::Broken;
            };
            let index = self.bare_index(cursor, counting);
            at = Some((counting, index));
        }
        StatementKind::Add { key, item, table, table_span, at }
    }

    fn bare_index(&mut self, cursor: &mut Cursor, counting: Counting) -> Expression {
        match cursor.peek() {
            Some(Token { kind: TokenKind::Number { .. } | TokenKind::Variable(_), .. }) => self.primary(cursor),
            Some(token) if token.is_symbol('(') => self.primary(cursor),
            _ => {
                let _ = counting;
                self.unreadable(cursor);
                Expression::broken(cursor.here())
            }
        }
    }

    fn call_statement(&mut self, cursor: &mut Cursor) -> StatementKind {
        let Some(Token { kind: TokenKind::Word(name), span: name_span }) = cursor.next() else {
            return StatementKind::Broken;
        };
        let name_span = *name_span;
        // trace: decision.procedures.nested_calls
        if let Some(open) = cursor.peek()
            && open.is_symbol('(')
            && open.span.start == name_span.end
        {
            let arguments = self.bracketed_arguments(cursor);
            let span = name_span.to(cursor.last_span());
            let inside: Vec<String> = arguments.iter().map(|argument| self.text(argument.span)).collect();
            let fix = if inside.is_empty() { name.clone() } else { format!("{} {}", name, inside.join(", ")) };
            self.fail(messages::call_without_brackets(span, fix));
            return StatementKind::Call { name: name.clone(), name_span, arguments };
        }
        let mut arguments = Vec::new();
        if !cursor.done() && !cursor.at_word("if") {
            loop {
                arguments.push(self.argument(cursor));
                if cursor.broken || !cursor.eat_symbol(',') {
                    break;
                }
            }
        }
        StatementKind::Call { name: name.clone(), name_span, arguments }
    }

    fn argument(&mut self, cursor: &mut Cursor) -> Argument {
        let start = cursor.here();
        let share = cursor.eat_word("share");
        let value = self.test(cursor);
        Argument { span: start.to(value.span), value, share }
    }

    fn bracketed_arguments(&mut self, cursor: &mut Cursor) -> Vec<Argument> {
        let open = cursor.here();
        cursor.next();
        let mut arguments = Vec::new();
        if cursor.eat_symbol(')') {
            return arguments;
        }
        loop {
            arguments.push(self.argument(cursor));
            if cursor.broken {
                return arguments;
            }
            if cursor.eat_symbol(',') {
                continue;
            }
            if cursor.eat_symbol(')') {
                return arguments;
            }
            if cursor.done() {
                self.fail(messages::not_designed_yet(open, "this `(` is never closed"));
                cursor.broken = true;
            } else {
                self.unreadable(cursor);
            }
            return arguments;
        }
    }

    /// A test or any value: the loosest level, where `and` and `or` join.
    // trace: decision.conditions.and_or
    // trace: decision.conditions.short_circuit
    fn test(&mut self, cursor: &mut Cursor) -> Expression {
        let mut left = self.not_level(cursor);
        let mut joined_by: Option<bool> = None;
        let mut reported = false;
        loop {
            if cursor.broken {
                return left;
            }
            let is_and = if cursor.at_word("and") {
                true
            } else if cursor.at_word("or") {
                false
            } else {
                return left;
            };
            let word_span = cursor.here();
            cursor.next();
            let also = cursor.eat_word("also");
            if let Some(before) = joined_by
                && before != is_and
                && !reported
            {
                reported = true;
                self.fail(messages::and_with_or(word_span));
            }
            joined_by = Some(is_and);
            let operator = match (is_and, also) {
                (true, false) => Logic::And,
                (true, true) => Logic::AndAlso,
                (false, false) => Logic::Or,
                (false, true) => Logic::OrAlso,
            };
            let right = self.not_level(cursor);
            let span = left.span.to(right.span);
            left = Expression { kind: ExpressionKind::Logic { operator, left: Box::new(left), right: Box::new(right) }, span };
        }
    }

    // trace: decision.conditions.not
    fn not_level(&mut self, cursor: &mut Cursor) -> Expression {
        if !cursor.at_word("not") {
            return self.comparison(cursor);
        }
        let not_span = cursor.here();
        cursor.next();
        let operand = match cursor.peek() {
            Some(Token { kind: TokenKind::Variable(_), .. }) => self.primary(cursor),
            Some(token) if token.is_symbol('(') => self.primary(cursor),
            _ => {
                self.unreadable(cursor);
                return Expression::broken(not_span);
            }
        };
        if !cursor.broken && Self::at_comparison(cursor) {
            let compared = self.comparison_after(cursor, operand);
            let span = not_span.to(compared.span);
            let fix = format!("not ({})", self.text(compared.span));
            self.fail(messages::not_needs_brackets(compared.span, fix));
            return Expression { kind: ExpressionKind::Not(Box::new(compared)), span };
        }
        let span = not_span.to(operand.span);
        Expression { kind: ExpressionKind::Not(Box::new(operand)), span }
    }

    fn at_comparison(cursor: &Cursor) -> bool {
        cursor.at_word("is") || cursor.at_word("isn't") || cursor.at_word("isnt")
    }

    // trace: decision.conditions.compare
    // trace: decision.conditions.isnt
    fn comparison(&mut self, cursor: &mut Cursor) -> Expression {
        let left = self.sum(cursor);
        if cursor.broken || !Self::at_comparison(cursor) {
            return left;
        }
        self.comparison_after(cursor, left)
    }

    fn comparison_after(&mut self, cursor: &mut Cursor, left: Expression) -> Expression {
        let word = cursor.next().cloned();
        let operator = match word {
            Some(token) if token.is_word("is") => {
                if cursor.at_word("not") {
                    let span = token.span.to(cursor.here());
                    cursor.next();
                    self.fail(messages::is_not(span));
                    Compare::IsNot
                } else if cursor.at_word("more") && cursor.peek_at(1).is_some_and(|token| token.is_word("than")) {
                    cursor.at += 2;
                    Compare::MoreThan
                } else if cursor.at_word("less") && cursor.peek_at(1).is_some_and(|token| token.is_word("than")) {
                    cursor.at += 2;
                    Compare::LessThan
                } else if cursor.at_word("at") && cursor.peek_at(1).is_some_and(|token| token.is_word("least")) {
                    cursor.at += 2;
                    Compare::AtLeast
                } else if cursor.at_word("at") && cursor.peek_at(1).is_some_and(|token| token.is_word("most")) {
                    cursor.at += 2;
                    Compare::AtMost
                } else {
                    Compare::Is
                }
            }
            _ => Compare::IsNot,
        };
        let right = self.sum(cursor);
        let span = left.span.to(right.span);
        Expression { kind: ExpressionKind::Compare { operator, left: Box::new(left), right: Box::new(right) }, span }
    }

    // trace: decision.maths.order
    fn sum(&mut self, cursor: &mut Cursor) -> Expression {
        let mut left = self.product(cursor);
        loop {
            if cursor.broken {
                return left;
            }
            let operator = match cursor.peek() {
                Some(token) if token.is_word("plus") => Maths::Plus,
                Some(token) if token.is_word("minus") => Maths::Minus,
                Some(token) if token.is_symbol('+') || token.is_symbol('-') => {
                    let symbol = if token.is_symbol('+') { '+' } else { '-' };
                    self.fail(messages::maths_symbol(token.span, symbol));
                    if symbol == '+' { Maths::Plus } else { Maths::Minus }
                }
                _ => return left,
            };
            cursor.next();
            let right = self.product(cursor);
            let span = left.span.to(right.span);
            left = Expression { kind: ExpressionKind::Maths { operator, left: Box::new(left), right: Box::new(right) }, span };
        }
    }

    fn product(&mut self, cursor: &mut Cursor) -> Expression {
        let mut left = self.over(cursor);
        loop {
            if cursor.broken {
                return left;
            }
            let operator = match cursor.peek() {
                Some(token) if token.is_word("times") => Maths::Times,
                Some(token) if token.is_word("divided") && cursor.peek_at(1).is_some_and(|by| by.is_word("by")) => {
                    cursor.next();
                    Maths::DividedBy
                }
                Some(token) if token.is_symbol('*') || token.is_symbol('/') => {
                    let symbol = if token.is_symbol('*') { '*' } else { '/' };
                    self.fail(messages::maths_symbol(token.span, symbol));
                    if symbol == '*' { Maths::Times } else { Maths::DividedBy }
                }
                _ => return left,
            };
            cursor.next();
            let right = self.over(cursor);
            let span = left.span.to(right.span);
            left = Expression { kind: ExpressionKind::Maths { operator, left: Box::new(left), right: Box::new(right) }, span };
        }
    }

    // trace: decision.types.over
    fn over(&mut self, cursor: &mut Cursor) -> Expression {
        let top = self.unary(cursor);
        if cursor.broken || !cursor.at_word("over") {
            return top;
        }
        let top_is_number = matches!(top.kind, ExpressionKind::Number { after_point: None, .. });
        let bottom_is_number =
            matches!(cursor.peek_at(1), Some(Token { kind: TokenKind::Number { after_point: None, .. }, .. }));
        if !top_is_number || !bottom_is_number {
            self.unreadable(cursor);
            return top;
        }
        cursor.next();
        let bottom = self.primary(cursor);
        let span = top.span.to(bottom.span);
        Expression { kind: ExpressionKind::Over { top: Box::new(top), bottom: Box::new(bottom) }, span }
    }

    fn unary(&mut self, cursor: &mut Cursor) -> Expression {
        let Some(token) = cursor.peek() else {
            self.unreadable(cursor);
            return Expression::broken(cursor.last_span());
        };
        let start = token.span;
        if let TokenKind::Word(word) = &token.kind {
            if let Some(family) = Family::from_word(word)
                && cursor.peek_at(1).is_some_and(|of| of.is_word("of"))
            {
                cursor.at += 2;
                let operand = self.unary(cursor);
                let span = start.to(operand.span);
                return Expression { kind: ExpressionKind::Family { family, operand: Box::new(operand) }, span };
            }
            match word.as_str() {
                // trace: decision.maths.remainder
                "remainder" if cursor.peek_at(1).is_some_and(|of| of.is_word("of")) => {
                    cursor.at += 2;
                    let operand = self.unary(cursor);
                    let span = start.to(operand.span);
                    let is_division = matches!(&operand.kind, ExpressionKind::Bracketed(inner)
                        if matches!(inner.unbracketed().kind, ExpressionKind::Maths { operator: Maths::DividedBy, .. }));
                    if !is_division && !matches!(operand.kind, ExpressionKind::Broken) {
                        self.fail(messages::not_designed_yet(
                            operand.span,
                            "`remainder of` without a division in brackets",
                        ));
                        return Expression::broken(span);
                    }
                    return Expression { kind: ExpressionKind::Remainder(Box::new(operand)), span };
                }
                // trace: decision.lists.index
                "position" | "offset" => {
                    let counting = if word == "position" { Counting::Position } else { Counting::Offset };
                    cursor.next();
                    let index = self.bare_or_bracketed(cursor, counting.word(), "of");
                    if cursor.broken || !cursor.eat_word("of") {
                        self.unreadable(cursor);
                        return Expression::broken(start);
                    }
                    let table = self.unary(cursor);
                    let span = start.to(table.span);
                    return Expression {
                        kind: ExpressionKind::Index { counting, index: Box::new(index), table: Box::new(table) },
                        span,
                    };
                }
                // trace: decision.decimals.rounding
                "round" => {
                    cursor.next();
                    if !cursor.at_symbol('(') {
                        let operand = self.unary(cursor);
                        let span = start.to(operand.span);
                        self.fail(messages::not_designed_yet(span, "`round` without brackets around what it rounds"));
                        return Expression::broken(span);
                    }
                    let operand = self.primary(cursor);
                    let span = start.to(operand.span);
                    return Expression { kind: ExpressionKind::Round(Box::new(operand)), span };
                }
                // trace: decision.memory.share
                "share" => {
                    cursor.next();
                    let operand = self.primary(cursor);
                    let span = start.to(operand.span);
                    return Expression { kind: ExpressionKind::Share(Box::new(operand)), span };
                }
                _ => {}
            }
        }
        self.primary(cursor)
    }

    fn primary(&mut self, cursor: &mut Cursor) -> Expression {
        if cursor.broken {
            return Expression::broken(cursor.here());
        }
        let Some(token) = cursor.next() else {
            cursor.at = cursor.tokens.len();
            self.unreadable(cursor);
            return Expression::broken(cursor.last_span());
        };
        let span = token.span;
        let kind = match &token.kind {
            TokenKind::Number { whole, after_point } => {
                ExpressionKind::Number { whole: whole.clone(), after_point: after_point.clone() }
            }
            TokenKind::Text(text) => {
                if text.is_empty() {
                    // trace: decision.values.empty
                    self.fail(messages::empty_text(span));
                    ExpressionKind::Empty
                } else {
                    ExpressionKind::Text(text.clone())
                }
            }
            TokenKind::Variable(name) => ExpressionKind::Variable(name.clone()),
            TokenKind::Symbol('~') => match cursor.peek() {
                Some(Token { kind: TokenKind::Number { whole, after_point }, span: number_span }) => {
                    cursor.next();
                    return Expression {
                        kind: ExpressionKind::About { whole: whole.clone(), after_point: after_point.clone() },
                        span: span.to(*number_span),
                    };
                }
                _ => {
                    self.unreadable(cursor);
                    ExpressionKind::Broken
                }
            },
            TokenKind::Symbol('-') => {
                self.fail(messages::not_designed_yet(span, "how a number below zero is written"));
                cursor.broken = true;
                ExpressionKind::Broken
            }
            TokenKind::Symbol('(') => {
                if cursor.done() {
                    self.fail(messages::not_designed_yet(span, "this `(` is never closed"));
                    cursor.broken = true;
                    return Expression::broken(span);
                }
                let inner = self.test(cursor);
                if cursor.broken {
                    return Expression { kind: ExpressionKind::Bracketed(Box::new(inner)), span };
                }
                if !cursor.eat_symbol(')') {
                    if cursor.done() {
                        self.fail(messages::not_designed_yet(span, "this `(` is never closed"));
                        cursor.broken = true;
                    } else {
                        self.unreadable(cursor);
                    }
                    return Expression { kind: ExpressionKind::Bracketed(Box::new(inner)), span };
                }
                return Expression { kind: ExpressionKind::Bracketed(Box::new(inner)), span: span.to(cursor.last_span()) };
            }
            TokenKind::Symbol('[') => return self.square_table(cursor, span),
            TokenKind::Word(word) => match word.as_str() {
                "true" => ExpressionKind::True,
                "false" => ExpressionKind::False,
                "empty" => ExpressionKind::Empty,
                _ if is_grammar_word(word) => {
                    cursor.at -= 1;
                    self.unreadable(cursor);
                    ExpressionKind::Broken
                }
                _ => return self.name_in_line(cursor, word.clone(), span),
            },
            _ => {
                cursor.at -= 1;
                self.unreadable(cursor);
                ExpressionKind::Broken
            }
        };
        Expression { kind, span }
    }

    /// A name inside a line: a call in brackets, a call without them, or a name standing alone.
    fn name_in_line(&mut self, cursor: &mut Cursor, name: String, name_span: Span) -> Expression {
        if let Some(open) = cursor.peek()
            && open.is_symbol('(')
            && open.span.start == name_span.end
        {
            let arguments = self.bracketed_arguments(cursor);
            let span = name_span.to(cursor.last_span());
            return Expression { kind: ExpressionKind::Call { name, name_span, arguments, bracketed: true }, span };
        }
        if cursor.peek().is_some_and(starts_value) {
            // trace: decision.procedures.nested_calls
            let mut arguments = Vec::new();
            loop {
                arguments.push(self.argument(cursor));
                if cursor.broken || !cursor.eat_symbol(',') {
                    break;
                }
            }
            let span = name_span.to(cursor.last_span());
            let inside: Vec<String> = arguments.iter().map(|argument| self.text(argument.span)).collect();
            self.fail(messages::call_needs_brackets(span, format!("{}({})", name, inside.join(", "))));
            return Expression { kind: ExpressionKind::Call { name, name_span, arguments, bracketed: false }, span };
        }
        Expression { kind: ExpressionKind::Bare(name), span: name_span }
    }

    // trace: decision.tables.shape
    fn square_table(&mut self, cursor: &mut Cursor, open: Span) -> Expression {
        if cursor.at_symbol(']') {
            let span = open.to(cursor.here());
            cursor.next();
            self.fail(messages::empty_table(span));
            return Expression { kind: ExpressionKind::Empty, span };
        }
        let mut items = Vec::new();
        let mut keyed = Vec::new();
        loop {
            if cursor.done() {
                self.fail(messages::square_not_closed(open));
                cursor.broken = true;
                return Expression::broken(open);
            }
            let item = self.test(cursor);
            if cursor.broken {
                return Expression::broken(open);
            }
            if cursor.eat_symbol(':') {
                let value = self.test(cursor);
                keyed.push((item, value));
            } else {
                items.push(item);
            }
            if cursor.broken {
                return Expression::broken(open);
            }
            if cursor.eat_symbol(',') {
                continue;
            }
            if cursor.eat_symbol(']') {
                break;
            }
            if cursor.done() {
                self.fail(messages::square_not_closed(open));
                cursor.broken = true;
                return Expression::broken(open);
            }
            self.unreadable(cursor);
            return Expression::broken(open);
        }
        let span = open.to(cursor.last_span());
        if !keyed.is_empty() && items.is_empty() {
            return Expression { kind: ExpressionKind::KeyedTable(keyed), span };
        }
        if !keyed.is_empty() {
            self.fail(messages::not_designed_yet(span, "a table with some named keys and some not"));
            return Expression::broken(span);
        }
        Expression { kind: ExpressionKind::Table(items), span }
    }
}

pub fn parse(source: &Source) -> (Program, Vec<Failure>) {
    Parser::new(source).program()
}
