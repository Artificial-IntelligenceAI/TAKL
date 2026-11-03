//! Reads lines into procedures, statements and expressions.
//!
//! A line that can't be read gives one error and is skipped, so every separate mistake is found in one go.

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::lexer::{Line, Token, TokenKind, lex};
use crate::messages;
use crate::source::{Source, Span};
use num_bigint::BigInt;

/// TAKL's own words. None of them may name a procedure.
/// trace: decision.names.underscore
/// trace: decision.names.no_grammar_names
pub const GRAMMAR_WORDS: &[&str] = &[
    "let",
    "show",
    "of",
    "in",
    "for",
    "each",
    "repeat",
    "times",
    "until",
    "forever",
    "stop",
    "if",
    "else",
    "end",
    "procedure",
    "returns",
    "return",
    "just",
    "finishes",
    "finish",
    "position",
    "offset",
    "add",
    "to",
    "at",
    "least",
    "most",
    "more",
    "less",
    "than",
    "is",
    "isn't",
    "not",
    "and",
    "or",
    "also",
    "plus",
    "minus",
    "divided",
    "by",
    "remainder",
    "sum",
    "length",
    "biggest",
    "smallest",
    "average",
    "round",
    "over",
    "table",
    "true",
    "false",
    "empty",
    "share",
];

/// trace: decision.loops.no_shortcut (there is no `50x`, so it is read as a mistake)
pub fn is_grammar(word: &str) -> bool {
    GRAMMAR_WORDS.contains(&word)
}

type Parsed<T> = Result<T, ()>;

struct Parser<'a> {
    source: &'a Source,
    lines: Vec<Line>,
    line: usize,
    errors: Vec<Diagnostic>,
    next_id: ExprId,
}

/// A cursor over one line's tokens.
struct Cursor<'a> {
    tokens: &'a [Token],
    at: usize,
    line_span: Span,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token], line_span: Span) -> Cursor<'a> {
        Cursor { tokens, at: 0, line_span }
    }

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

    fn at_end(&self) -> bool {
        self.at >= self.tokens.len()
    }

    fn peek_word(&self, word: &str) -> bool {
        self.peek().is_some_and(|token| token.is_word(word))
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.peek_word(word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn peek_kind(&self, kind: &TokenKind) -> bool {
        self.peek().is_some_and(|token| &token.kind == kind)
    }

    fn eat_kind(&mut self, kind: &TokenKind) -> bool {
        if self.peek_kind(kind) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Where the next token is, or the end of the line.
    fn here(&self) -> Span {
        match self.peek() {
            Some(token) => token.span,
            None => Span::new(self.line_span.end, self.line_span.end),
        }
    }

    fn last_end(&self) -> usize {
        if self.at == 0 { self.line_span.start } else { self.tokens[self.at - 1].span.end }
    }
}

fn word_of(token: &Token) -> Option<&str> {
    match &token.kind {
        TokenKind::Word(word) => Some(word),
        _ => None,
    }
}

/// Splits tokens at commas that are not inside brackets.
fn split_top_level<'t>(tokens: &'t [Token], at: &TokenKind) -> Vec<&'t [Token]> {
    let mut pieces = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LeftBracket | TokenKind::LeftSquare => depth += 1,
            TokenKind::RightBracket | TokenKind::RightSquare => depth -= 1,
            _ => {}
        }
        if depth == 0 && &token.kind == at {
            pieces.push(&tokens[start..index]);
            start = index + 1;
        }
    }
    pieces.push(&tokens[start..]);
    pieces
}

fn tokens_span(tokens: &[Token]) -> Span {
    match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => Span::new(first.span.start, last.span.end),
        _ => Span::default(),
    }
}

pub fn parse(source: &Source) -> (Program, Vec<Diagnostic>) {
    let (lines, errors) = lex(source);
    let mut parser = Parser { source, lines, line: 0, errors, next_id: 0 };
    let mut procedures = Vec::new();
    while parser.line < parser.lines.len() {
        let line = parser.lines[parser.line].clone();
        if line.tokens[0].is_word("procedure") {
            if let Some(procedure) = parser.procedure() {
                procedures.push(procedure);
            }
        } else {
            // trace: decision.procedures.start_running_required (every line runs from inside a procedure)
            parser.errors.push(messages::outside_procedure(line.span));
            parser.line += 1;
        }
    }
    (Program { procedures }, parser.errors)
}

impl<'a> Parser<'a> {
    fn id(&mut self) -> ExprId {
        self.next_id += 1;
        self.next_id
    }

    fn expr(&mut self, kind: ExprKind, span: Span) -> Expr {
        Expr { id: self.id(), kind, span }
    }

    fn text(&self, span: Span) -> &str {
        &self.source.text[span.start..span.end]
    }

    fn fail<T>(&mut self, error: Diagnostic) -> Parsed<T> {
        self.errors.push(error);
        Err(())
    }

    // ---- procedures -------------------------------------------------------------------------------------------

    /// trace: decision.procedures.shape
    fn procedure(&mut self) -> Option<Procedure> {
        let line = self.lines[self.line].clone();
        self.line += 1;
        let header = self.procedure_header(&line);
        let (body, end) = self.block(line.span);
        let end_span = match end {
            Some(end_line) => {
                let end_tokens = &end_line.tokens;
                let closes_procedure = end_tokens.get(1).is_some_and(|token| token.is_word("procedure"));
                if !closes_procedure {
                    self.errors.push(messages::wrong_end(end_line.span, "end procedure"));
                } else if let Ok((name, ..)) = &header {
                    let written = end_tokens.get(2).and_then(word_of);
                    if written != Some(name.as_str()) || end_tokens.len() != 3 {
                        self.errors.push(messages::end_names_wrong(end_line.span, &format!("end procedure {}", name)));
                    }
                }
                end_line.span
            }
            None => {
                if let Ok((name, ..)) = &header {
                    self.errors.push(messages::no_end(line.span, &format!("end procedure {}", name)));
                }
                line.span
            }
        };
        match header {
            Ok((name, name_span, params, ending, ending_span)) => {
                Some(Procedure { name, name_span, params, ending, ending_span, body, end_span, broken_header: false })
            }
            Err(()) => {
                // Known by name, so calls to it and `start_running` don't give errors of their own.
                let name_token = line.tokens.get(1)?;
                let name = word_of(name_token).filter(|word| !is_grammar(word))?.to_string();
                Some(Procedure {
                    name,
                    name_span: name_token.span,
                    params: Vec::new(),
                    ending: Ending::JustFinishes,
                    ending_span: line.span,
                    body,
                    end_span,
                    broken_header: true,
                })
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn procedure_header(&mut self, line: &Line) -> Parsed<(String, Span, Vec<Param>, Ending, Span)> {
        let mut cursor = Cursor::new(&line.tokens, line.span);
        cursor.next();
        let Some(name_token) = cursor.next() else {
            return self.fail(messages::procedure_needs_name(line.span));
        };
        let name = match &name_token.kind {
            TokenKind::Word(word) => word.clone(),
            TokenKind::Variable(variable) => {
                return self.fail(messages::procedure_name_has_at(name_token.span, &variable[1..]));
            }
            _ => return self.fail(messages::procedure_needs_name(name_token.span)),
        };
        if is_grammar(&name) {
            return self.fail(messages::grammar_word_as_name(name_token.span, &name));
        }
        let mut params = Vec::new();
        if cursor.peek_kind(&TokenKind::LeftBracket) {
            let open = cursor.at;
            let mut depth = 0;
            let mut close = None;
            for (index, token) in line.tokens.iter().enumerate().skip(open) {
                match token.kind {
                    TokenKind::LeftBracket => depth += 1,
                    TokenKind::RightBracket => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else {
                return self.fail(messages::bracket_not_closed(line.tokens[open].span));
            };
            let inside = &line.tokens[open + 1..close];
            if !inside.is_empty() {
                for piece in split_top_level(inside, &TokenKind::Bar) {
                    params.push(self.param(piece, line.tokens[open].span)?);
                }
            }
            cursor.at = close + 1;
        }
        let ending_start = cursor.here();
        let ending = if cursor.eat_word("returns") {
            let ty = self.type_expr(&mut cursor)?;
            let modifiers = self.modifiers(&mut cursor, &ty)?;
            Ending::Returns(ty, modifiers)
        } else if cursor.peek_word("just") && cursor.peek_at(1).is_some_and(|token| token.is_word("finishes")) {
            cursor.at += 2;
            // trace: decision.procedures.just_finishes
            Ending::JustFinishes
        } else {
            return self.fail(messages::procedure_needs_ending(ending_start, &name));
        };
        let ending_span = Span::new(ending_start.start, cursor.last_end());
        if !cursor.eat_kind(&TokenKind::Colon) || !cursor.at_end() {
            return self.fail(messages::header_needs_colon(cursor.here()));
        }
        Ok((name, name_token.span, params, ending, ending_span))
    }

    fn param(&mut self, tokens: &[Token], bracket: Span) -> Parsed<Param> {
        let span = if tokens.is_empty() { bracket } else { tokens_span(tokens) };
        let mut cursor = Cursor::new(tokens, span);
        let name = self.variable_name(&mut cursor)?;
        if !cursor.eat_kind(&TokenKind::Colon) {
            return self.fail(messages::needs_type(cursor.here(), &name.0));
        }
        let ty = self.type_expr(&mut cursor)?;
        let modifiers = self.modifiers(&mut cursor, &ty)?;
        let default = if cursor.eat_kind(&TokenKind::Equals) { Some(self.test(&mut cursor)?) } else { None };
        if !cursor.at_end() {
            return self.fail(messages::params_split_by_bar(cursor.here()));
        }
        Ok(Param { name: name.0, span: name.1, ty, modifiers, default })
    }

    /// A variable's name with its `@`; a name without one gets a fix.
    fn variable_name(&mut self, cursor: &mut Cursor) -> Parsed<(String, Span)> {
        match cursor.next() {
            Some(Token { kind: TokenKind::Variable(name), span }) => Ok((name.clone(), *span)),
            Some(Token { kind: TokenKind::Word(word), span }) if !is_grammar(word) => {
                // trace: decision.names.at_mark
                self.fail(messages::variable_needs_at(*span, word))
            }
            Some(token) => self.fail(messages::expected_variable(token.span)),
            None => self.fail(messages::expected_variable(cursor.here())),
        }
    }

    // ---- types ------------------------------------------------------------------------------------------------

    fn type_expr(&mut self, cursor: &mut Cursor) -> Parsed<TypeExpr> {
        let Some(token) = cursor.next() else {
            return self.fail(messages::expected_type(cursor.here()));
        };
        match &token.kind {
            TokenKind::Word(word) if word == "table" => {
                // trace: decision.tables.shape
                if !cursor.eat_word("of") {
                    return self.fail(messages::table_needs_of(cursor.here()));
                }
                let first = self.item_type(cursor)?;
                let (key, item) = if cursor.eat_word("to") {
                    (Some(Box::new(first)), self.item_type(cursor)?)
                } else {
                    (None, first)
                };
                let span = Span::new(token.span.start, cursor.last_end());
                Ok(TypeExpr::Table { key, item: Box::new(item), span })
            }
            TokenKind::Word(word) if !is_grammar(word) => Ok(TypeExpr::Named(word.clone(), token.span)),
            _ => self.fail(messages::expected_type(token.span)),
        }
    }

    /// trace: decision.tables.item_modifiers
    fn item_type(&mut self, cursor: &mut Cursor) -> Parsed<ItemType> {
        if cursor.eat_kind(&TokenKind::LeftBracket) {
            let ty = self.type_expr(cursor)?;
            let modifiers = self.modifiers(cursor, &ty)?;
            if !cursor.eat_kind(&TokenKind::RightBracket) {
                return self.fail(messages::bracket_not_closed(cursor.here()));
            }
            Ok(ItemType { ty, modifiers })
        } else {
            Ok(ItemType { ty: self.type_expr(cursor)?, modifiers: Vec::new() })
        }
    }

    fn modifiers(&mut self, cursor: &mut Cursor, ty: &TypeExpr) -> Parsed<Vec<Modifier>> {
        let mut modifiers = Vec::new();
        while cursor.peek_kind(&TokenKind::Comma) {
            // A comma followed by another param belongs to the param list, which `|` splits.
            if matches!(cursor.peek_at(1).map(|token| &token.kind), Some(TokenKind::Variable(_))) {
                return self.fail(messages::params_split_by_bar(cursor.here()));
            }
            cursor.next();
            let Some(token) = cursor.next() else {
                return self.fail(messages::expected_modifier(cursor.here()));
            };
            let start = token.span;
            let kind = match &token.kind {
                TokenKind::Word(word) => match word.as_str() {
                    "can't_change" => ModifierKind::CantChange,
                    // trace: decision.types.never_negative
                    "never_negative" => ModifierKind::NeverNegative,
                    // trace: decision.types.fast_but_inexact
                    "fast_but_inexact" => ModifierKind::FastButInexact,
                    "share" => ModifierKind::Share,
                    // trace: decision.types.bounds
                    "at" if cursor.peek_word("least") || cursor.peek_word("most") => {
                        let least = cursor.eat_word("least");
                        if !least {
                            cursor.eat_word("most");
                        }
                        let number = self.bound_number(cursor)?;
                        if least { ModifierKind::AtLeast(number) } else { ModifierKind::AtMost(number) }
                    }
                    "fast" => return self.fail(messages::fast_is_now(token.span)),
                    "normal" => return self.fail(messages::normal_not_a_word(token.span)),
                    other => return self.fail(messages::unknown_modifier(token.span, other)),
                },
                _ => return self.fail(messages::expected_modifier(token.span)),
            };
            let span = Span::new(start.start, cursor.last_end());
            // trace: decision.tables.item_modifiers (an item's modifier after a table type must be bracketed)
            if let TypeExpr::Table { .. } = ty
                && matches!(
                    kind,
                    ModifierKind::NeverNegative
                        | ModifierKind::FastButInexact
                        | ModifierKind::AtLeast(_)
                        | ModifierKind::AtMost(_)
                )
            {
                let written = self.text(Span::new(ty.span().start, span.end)).to_string();
                return self.fail(messages::item_modifier_needs_brackets(span, &written));
            }
            modifiers.push(Modifier { kind, span });
        }
        Ok(modifiers)
    }

    fn bound_number(&mut self, cursor: &mut Cursor) -> Parsed<BigInt> {
        match cursor.next() {
            Some(Token { kind: TokenKind::Number(whole, None), .. }) => Ok(whole.parse().unwrap_or_default()),
            Some(Token { kind: TokenKind::Mark('-'), span }) => {
                self.fail(messages::negative_numbers_not_designed(*span))
            }
            Some(token) => self.fail(messages::bound_needs_number(token.span)),
            None => self.fail(messages::bound_needs_number(cursor.here())),
        }
    }

    // ---- blocks and statements --------------------------------------------------------------------------------

    /// Reads statements until a line that starts with `end` or `else`, which is left for the caller.
    /// Returns the `end` line when there is one.
    fn block(&mut self, opener: Span) -> (Vec<Stmt>, Option<Line>) {
        let mut body = Vec::new();
        loop {
            let Some(line) = self.lines.get(self.line).cloned() else {
                return (body, None);
            };
            let first = &line.tokens[0];
            if first.is_word("end") {
                self.line += 1;
                return (body, Some(line));
            }
            if first.is_word("else") {
                return (body, None);
            }
            if first.is_word("procedure") {
                let _ = opener;
                return (body, None);
            }
            if first.is_word("end_of") {
                // trace: decision.blocks.end
                self.errors.push(messages::end_of_is_end(first.span));
                self.line += 1;
                return (body, Some(line));
            }
            self.line += 1;
            if let Some(stmt) = self.statement(&line) {
                body.push(stmt);
            }
        }
    }

    /// A block that must close with `end <what>`.
    fn closed_block(&mut self, opener: &Line, what: &str) -> Vec<Stmt> {
        let (body, end) = self.block(opener.span);
        self.check_end(opener, end, what);
        body
    }

    fn check_end(&mut self, opener: &Line, end: Option<Line>, what: &str) {
        match end {
            Some(end_line) => {
                let written = end_line.tokens.get(1).and_then(word_of);
                if end_line.tokens[0].is_word("end_of") {
                    return;
                }
                if written != Some(what) || end_line.tokens.len() != 2 {
                    self.errors.push(messages::wrong_end(end_line.span, &format!("end {}", what)));
                }
            }
            None => self.errors.push(messages::no_end(opener.span, &format!("end {}", what))),
        }
    }

    fn statement(&mut self, line: &Line) -> Option<Stmt> {
        let first = &line.tokens[0];
        let starts_block = matches!(word_of(first), Some("if" | "repeat" | "for"));
        // trace: decision.conditions.trailing_if
        let (tokens, only_if) = if starts_block {
            (&line.tokens[..], None)
        } else {
            match self.trailing_if(line) {
                Ok(split) => split,
                Err(()) => return Some(Stmt { kind: StmtKind::Broken, span: line.span, only_if: None }),
            }
        };
        let span = Span::new(line.span.start, tokens_span(tokens).end.max(line.span.start));
        let kind = self.statement_kind(line, tokens, span).unwrap_or(StmtKind::Broken);
        Some(Stmt { kind, span, only_if })
    }

    fn trailing_if<'l>(&mut self, line: &'l Line) -> Parsed<(&'l [Token], Option<Expr>)> {
        let mut depth = 0;
        for (index, token) in line.tokens.iter().enumerate() {
            match token.kind {
                TokenKind::LeftBracket | TokenKind::LeftSquare => depth += 1,
                TokenKind::RightBracket | TokenKind::RightSquare => depth -= 1,
                _ => {}
            }
            if depth == 0 && index > 0 && token.is_word("if") {
                let test_tokens = &line.tokens[index + 1..];
                let mut cursor = Cursor::new(test_tokens, line.span);
                let test = self.test(&mut cursor)?;
                self.expect_end_of_line(&cursor)?;
                return Ok((&line.tokens[..index], Some(test)));
            }
        }
        Ok((&line.tokens[..], None))
    }

    fn expect_end_of_line(&mut self, cursor: &Cursor) -> Parsed<()> {
        match cursor.peek() {
            None => Ok(()),
            Some(token) => match token.kind {
                TokenKind::Mark(';') => self.fail(messages::semicolon(token.span)),
                TokenKind::Mark(mark) => self.fail(messages::mark(token.span, mark)),
                _ => self.fail(messages::unexpected_here(token.span, self.text(token.span))),
            },
        }
    }

    fn statement_kind(&mut self, line: &Line, tokens: &[Token], span: Span) -> Parsed<StmtKind> {
        let mut cursor = Cursor::new(tokens, line.span);
        let first = &tokens[0];
        match &first.kind {
            TokenKind::Word(word) => match word.as_str() {
                "let" => self.let_statement(line, &mut cursor),
                "show" => {
                    cursor.next();
                    // trace: decision.show.pieces
                    let rest = &tokens[1..];
                    if rest.is_empty() {
                        return self.fail(messages::show_needs_something(span));
                    }
                    let mut pieces = Vec::new();
                    for piece in split_top_level(rest, &TokenKind::Comma) {
                        pieces.push(self.whole_expression(piece, span)?);
                    }
                    Ok(StmtKind::Show(pieces))
                }
                "if" => self.if_statement(line),
                "repeat" => self.repeat_statement(line),
                "for" => self.for_statement(line),
                "stop" | "finish" => {
                    cursor.next();
                    self.expect_end_of_line(&cursor)?;
                    Ok(if word == "stop" { StmtKind::Stop } else { StmtKind::Finish })
                }
                "return" => {
                    cursor.next();
                    if cursor.peek_word("nothing") {
                        // trace: decision.procedures.finish
                        return self.fail(messages::return_nothing_is_finish(span));
                    }
                    if cursor.at_end() {
                        return self.fail(messages::return_needs_value(span));
                    }
                    let value = self.test(&mut cursor)?;
                    self.expect_end_of_line(&cursor)?;
                    // trace: decision.procedures.return
                    Ok(StmtKind::Return(value))
                }
                "add" => self.add_statement(&mut cursor),
                "else" => self.fail(messages::else_without_if(first.span)),
                other if is_grammar(other) => self.fail(messages::cannot_start_line(first.span, other)),
                name => {
                    // trace: decision.procedures.nested_calls (a call standing alone is bare)
                    let name = name.to_string();
                    cursor.next();
                    if cursor.peek_kind(&TokenKind::LeftBracket) {
                        let fixed = format!("{} {}", name, self.bracket_contents_text(&cursor));
                        return self.fail(messages::standalone_call_is_bare(span, fixed.trim_end()));
                    }
                    let args = self.args(&tokens[1..], span)?;
                    Ok(StmtKind::Call { name, name_span: first.span, args })
                }
            },
            TokenKind::Variable(name) => {
                cursor.next();
                if !cursor.eat_kind(&TokenKind::Equals) {
                    return self.fail(messages::line_does_nothing(span));
                }
                let value = self.let_value(&mut cursor)?;
                Ok(StmtKind::Assign { name: name.clone(), name_span: first.span, value })
            }
            TokenKind::Mark(mark) => self.fail(messages::mark(first.span, *mark)),
            _ => self.fail(messages::line_does_nothing(span)),
        }
    }

    fn bracket_contents_text(&self, cursor: &Cursor) -> String {
        let tokens = &cursor.tokens[cursor.at..];
        if tokens.len() >= 2 {
            let inner = Span::new(tokens[0].span.end, tokens[tokens.len() - 1].span.start);
            self.text(inner).trim().to_string()
        } else {
            String::new()
        }
    }

    fn args(&mut self, tokens: &[Token], span: Span) -> Parsed<Vec<Arg>> {
        let mut args = Vec::new();
        if tokens.is_empty() {
            return Ok(args);
        }
        for piece in split_top_level(tokens, &TokenKind::Comma) {
            if piece.first().is_some_and(|token| token.is_word("share")) {
                let mut cursor = Cursor::new(&piece[1..], span);
                let name = self.variable_name(&mut cursor)?;
                self.expect_end_of_line(&cursor)?;
                args.push(Arg::Share(name.0, tokens_span(piece)));
            } else {
                args.push(Arg::Value(self.whole_expression(piece, span)?));
            }
        }
        Ok(args)
    }

    /// An expression that must take up all of the given tokens.
    fn whole_expression(&mut self, tokens: &[Token], span: Span) -> Parsed<Expr> {
        if tokens.is_empty() {
            return self.fail(messages::expected_value(span));
        }
        let mut cursor = Cursor::new(tokens, tokens_span(tokens));
        let expr = self.test(&mut cursor)?;
        self.expect_end_of_line(&cursor)?;
        Ok(expr)
    }

    fn let_value(&mut self, cursor: &mut Cursor) -> Parsed<LetValue> {
        if cursor.peek_word("share") {
            // trace: decision.memory.share
            let start = cursor.here();
            cursor.next();
            let name = self.variable_name(cursor)?;
            self.expect_end_of_line(cursor)?;
            return Ok(LetValue::Share(name.0, Span::new(start.start, name.1.end)));
        }
        if cursor.at_end() {
            return self.fail(messages::expected_value(cursor.here()));
        }
        let value = self.test(cursor)?;
        self.expect_end_of_line(cursor)?;
        Ok(LetValue::Expr(value))
    }

    /// trace: decision.syntax.let
    fn let_statement(&mut self, line: &Line, cursor: &mut Cursor) -> Parsed<StmtKind> {
        cursor.next();
        let (name, name_span) = self.variable_name(cursor)?;
        let broken = StmtKind::BrokenLet { name: name.clone() };
        if !cursor.eat_kind(&TokenKind::Colon) {
            self.errors.push(messages::needs_type(cursor.here(), &name));
            return Ok(broken);
        }
        let Ok(ty) = self.type_expr(cursor) else { return Ok(broken) };
        let Ok(modifiers) = self.modifiers(cursor, &ty) else { return Ok(broken) };
        if cursor.eat_kind(&TokenKind::Colon) && cursor.at_end() {
            // trace: decision.tables.shape (a table written as a block, one item a line)
            let items = self.table_block(line, &name);
            return Ok(StmtKind::Let { name, name_span, ty, modifiers, value: LetValue::Block(items, line.span) });
        }
        if !cursor.eat_kind(&TokenKind::Equals) {
            if cursor.at_end() {
                self.errors.push(messages::let_needs_value(line.span, &name));
            } else {
                self.errors.push(messages::unexpected_here(cursor.here(), self.text(cursor.here())));
            }
            return Ok(broken);
        }
        let Ok(value) = self.let_value(cursor) else { return Ok(broken) };
        Ok(StmtKind::Let { name, name_span, ty, modifiers, value })
    }

    fn table_block(&mut self, opener: &Line, name: &str) -> Vec<BlockItem> {
        let mut items = Vec::new();
        loop {
            let Some(line) = self.lines.get(self.line).cloned() else {
                self.errors.push(messages::no_end(opener.span, &format!("end table {}", name)));
                return items;
            };
            self.line += 1;
            if line.tokens[0].is_word("end") {
                let closes = line.tokens.get(1).is_some_and(|token| token.is_word("table"))
                    && matches!(line.tokens.get(2).map(|token| &token.kind), Some(TokenKind::Variable(n)) if n == name)
                    && line.tokens.len() == 3;
                if !closes {
                    self.errors.push(messages::wrong_end(line.span, &format!("end table {}", name)));
                }
                return items;
            }
            let pieces = split_top_level(&line.tokens, &TokenKind::Colon);
            let item = match pieces.as_slice() {
                [value] => self.whole_expression(value, line.span).map(BlockItem::Item),
                [key, value] => {
                    let key = self.whole_expression(key, line.span);
                    let value = self.whole_expression(value, line.span);
                    match (key, value) {
                        (Ok(key), Ok(value)) => Ok(BlockItem::Keyed(key, value)),
                        _ => Err(()),
                    }
                }
                _ => self.fail(messages::unexpected_here(line.span, self.text(line.span))),
            };
            if let Ok(item) = item {
                items.push(item);
            }
        }
    }

    /// trace: decision.conditions.if
    fn if_statement(&mut self, line: &Line) -> Parsed<StmtKind> {
        let test = self.block_header_test(line, 1);
        let (body, mut end) = self.block(line.span);
        let mut branches = Vec::new();
        if let Ok(test) = test {
            branches.push((test, body));
        }
        let mut otherwise = None;
        while end.is_none() {
            let Some(next) = self.lines.get(self.line).cloned() else { break };
            if !next.tokens[0].is_word("else") {
                break;
            }
            self.line += 1;
            if next.tokens.get(1).is_some_and(|token| token.is_word("if")) {
                let test = self.block_header_test(&next, 2);
                let (body, next_end) = self.block(next.span);
                if let Ok(test) = test {
                    branches.push((test, body));
                }
                end = next_end;
            } else {
                if next.tokens.len() != 2 || !matches!(next.tokens[1].kind, TokenKind::Colon) {
                    self.errors.push(messages::header_needs_colon(next.span));
                }
                let (body, next_end) = self.block(next.span);
                otherwise = Some(body);
                end = next_end;
                break;
            }
        }
        self.check_end(line, end, "if");
        if branches.is_empty() {
            return Err(());
        }
        Ok(StmtKind::If { branches, otherwise })
    }

    /// The test in a block's first line, between `skip` words and the closing `:`.
    fn block_header_test(&mut self, line: &Line, skip: usize) -> Parsed<Expr> {
        let tokens = &line.tokens;
        if !matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Colon)) {
            return self.fail(messages::header_needs_colon(Span::new(line.span.end, line.span.end)));
        }
        let inner = &tokens[skip..tokens.len() - 1];
        self.whole_expression(inner, line.span)
    }

    fn repeat_statement(&mut self, line: &Line) -> Parsed<StmtKind> {
        let tokens = &line.tokens;
        let header: Parsed<StmtKind> = (|| {
            if !matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Colon)) {
                return self.fail(messages::header_needs_colon(Span::new(line.span.end, line.span.end)));
            }
            let inner = &tokens[1..tokens.len() - 1];
            let mut cursor = Cursor::new(inner, line.span);
            if cursor.eat_word("forever") {
                // trace: decision.loops.forever
                self.expect_end_of_line(&cursor)?;
                return Ok(StmtKind::RepeatForever { body: Vec::new() });
            }
            if cursor.eat_word("until") {
                // trace: decision.loops.until
                let test = self.test(&mut cursor)?;
                self.expect_end_of_line(&cursor)?;
                return Ok(StmtKind::RepeatUntil { test, body: Vec::new() });
            }
            // trace: decision.loops.repeat
            let count = self.bare_or_bracketed(&mut cursor, "repeat", "times")?;
            if !cursor.eat_word("times") {
                return self.fail(messages::repeat_needs_times(cursor.here()));
            }
            self.expect_end_of_line(&cursor)?;
            Ok(StmtKind::RepeatTimes { count, body: Vec::new() })
        })();
        let body = self.closed_block(line, "repeat");
        Ok(match header? {
            StmtKind::RepeatForever { .. } => StmtKind::RepeatForever { body },
            StmtKind::RepeatUntil { test, .. } => StmtKind::RepeatUntil { test, body },
            StmtKind::RepeatTimes { count, .. } => StmtKind::RepeatTimes { count, body },
            _ => unreachable!("the header is one of the three repeats"),
        })
    }

    /// trace: decision.tables.of_family (`for each`, in order)
    fn for_statement(&mut self, line: &Line) -> Parsed<StmtKind> {
        let tokens = &line.tokens;
        let header: Parsed<_> = (|| {
            if !matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Colon)) {
                return self.fail(messages::header_needs_colon(Span::new(line.span.end, line.span.end)));
            }
            let inner = &tokens[1..tokens.len() - 1];
            let mut cursor = Cursor::new(inner, line.span);
            if !cursor.eat_word("each") {
                return self.fail(messages::for_needs_each(cursor.here()));
            }
            let first = self.variable_name(&mut cursor)?;
            let (key, item) =
                if cursor.eat_word("and") { (Some(first), self.variable_name(&mut cursor)?) } else { (None, first) };
            if !cursor.eat_word("in") {
                return self.fail(messages::for_needs_in(cursor.here()));
            }
            let table = self.of_target(&mut cursor, "in")?;
            self.expect_end_of_line(&cursor)?;
            Ok((key, item, table))
        })();
        let body = self.closed_block(line, "for");
        let (key, item, table) = header?;
        Ok(StmtKind::ForEach { key, item, table, body })
    }

    /// trace: decision.tables.add
    fn add_statement(&mut self, cursor: &mut Cursor) -> Parsed<StmtKind> {
        cursor.next();
        let first = self.sum(cursor)?;
        let (key, value) =
            if cursor.eat_kind(&TokenKind::Colon) { (Some(first), self.sum(cursor)?) } else { (None, first) };
        if !cursor.eat_word("to") {
            return self.fail(messages::add_needs_to(cursor.here()));
        }
        let table = self.variable_name(cursor)?;
        let at = if cursor.eat_word("at") {
            let kind = if cursor.eat_word("position") {
                IndexKind::Position
            } else if cursor.eat_word("offset") {
                IndexKind::Offset
            } else {
                return self.fail(messages::add_at_needs_position(cursor.here()));
            };
            let word = if kind == IndexKind::Position { "position" } else { "offset" };
            Some((kind, self.bare_or_bracketed(cursor, word, "")?))
        } else {
            None
        };
        self.expect_end_of_line(cursor)?;
        Ok(StmtKind::Add { value, key, table, at })
    }

    // ---- expressions ------------------------------------------------------------------------------------------

    /// Tests joined by `and` or `or`; mixing the two needs brackets.
    /// trace: decision.conditions.and_or
    /// trace: decision.conditions.short_circuit
    fn test(&mut self, cursor: &mut Cursor) -> Parsed<Expr> {
        let mut left = self.not_or_compare(cursor)?;
        let mut family: Option<bool> = None;
        loop {
            let op_span = cursor.here();
            let op = if cursor.eat_word("and") {
                if cursor.eat_word("also") { LogicOp::AndAlso } else { LogicOp::And }
            } else if cursor.eat_word("or") {
                if cursor.eat_word("also") { LogicOp::OrAlso } else { LogicOp::Or }
            } else {
                return Ok(left);
            };
            let is_and = matches!(op, LogicOp::And | LogicOp::AndAlso);
            if family.is_some_and(|seen| seen != is_and) {
                return self.fail(messages::mix_and_or(op_span));
            }
            family = Some(is_and);
            let right = self.not_or_compare(cursor)?;
            let span = left.span.to(right.span);
            left = self.expr(ExprKind::Logic(op, Box::new(left), Box::new(right)), span);
        }
    }

    fn not_or_compare(&mut self, cursor: &mut Cursor) -> Parsed<Expr> {
        if cursor.peek_word("not") {
            // trace: decision.conditions.not
            let start = cursor.here();
            cursor.next();
            let operand_start = cursor.at;
            let operand = self.atom(cursor)?;
            if cursor.peek_word("is") || cursor.peek_word("isn't") {
                // `not @done is true` could be `not (@done is true)` or `(not @done) is true`.
                cursor.at = operand_start;
                let test = self.not_or_compare(cursor)?;
                let written = self.text(test.span).to_string();
                return self.fail(messages::not_needs_brackets(test.span, &written));
            }
            if !matches!(operand.kind, ExprKind::Variable(_) | ExprKind::Brackets(_)) {
                let written = self.text(operand.span).to_string();
                return self.fail(messages::not_needs_brackets(operand.span, &written));
            }
            let span = start.to(operand.span);
            return Ok(self.expr(ExprKind::Not(Box::new(operand)), span));
        }
        let left = self.sum(cursor)?;
        let op_span = cursor.here();
        // trace: decision.conditions.compare
        let op = if cursor.eat_word("is") {
            if cursor.peek_word("not") {
                // trace: decision.conditions.isnt
                return self.fail(messages::is_not_is_isnt(Span::new(op_span.start, cursor.here().end)));
            }
            if cursor.eat_word("more") {
                self.expect_word(cursor, "than")?;
                CompareOp::MoreThan
            } else if cursor.eat_word("less") {
                self.expect_word(cursor, "than")?;
                CompareOp::LessThan
            } else if cursor.peek_word("at") && cursor.peek_at(1).is_some_and(|t| t.is_word("least")) {
                cursor.at += 2;
                CompareOp::AtLeast
            } else if cursor.peek_word("at") && cursor.peek_at(1).is_some_and(|t| t.is_word("most")) {
                cursor.at += 2;
                CompareOp::AtMost
            } else {
                CompareOp::Is
            }
        } else if cursor.eat_word("isn't") {
            CompareOp::Isnt
        } else if let Some(Token { kind: TokenKind::Mark(mark @ ('<' | '>')), span }) = cursor.peek() {
            return self.fail(messages::mark(*span, *mark));
        } else {
            return Ok(left);
        };
        let right = self.sum(cursor)?;
        let span = left.span.to(right.span);
        Ok(self.expr(ExprKind::Compare(op, Box::new(left), Box::new(right)), span))
    }

    fn expect_word(&mut self, cursor: &mut Cursor, word: &str) -> Parsed<()> {
        if cursor.eat_word(word) { Ok(()) } else { self.fail(messages::expected_word(cursor.here(), word)) }
    }

    /// trace: decision.maths.words
    /// trace: decision.maths.order
    fn sum(&mut self, cursor: &mut Cursor) -> Parsed<Expr> {
        let mut left = self.term(cursor)?;
        loop {
            let op = if cursor.eat_word("plus") {
                MathsOp::Plus
            } else if cursor.eat_word("minus") {
                MathsOp::Minus
            } else if let Some(Token { kind: TokenKind::Mark(mark @ ('+' | '-')), span }) = cursor.peek() {
                return self.fail(messages::mark(*span, *mark));
            } else {
                return Ok(left);
            };
            let right = self.term(cursor)?;
            let span = left.span.to(right.span);
            left = self.expr(ExprKind::Maths(op, Box::new(left), Box::new(right)), span);
        }
    }

    fn term(&mut self, cursor: &mut Cursor) -> Parsed<Expr> {
        let mut left = self.atom(cursor)?;
        loop {
            let op = if cursor.eat_word("times") {
                MathsOp::Times
            } else if cursor.peek_word("divided") {
                cursor.next();
                self.expect_word(cursor, "by")?;
                MathsOp::DividedBy
            } else if let Some(Token { kind: TokenKind::Mark(mark @ ('*' | '/' | '\u{00D7}' | '\u{00F7}')), span }) =
                cursor.peek()
            {
                return self.fail(messages::mark(*span, *mark));
            } else {
                return Ok(left);
            };
            let right = self.atom(cursor)?;
            let span = left.span.to(right.span);
            left = self.expr(ExprKind::Maths(op, Box::new(left), Box::new(right)), span);
        }
    }

    /// One number or one name, or anything bracketed: what `position`, `offset` and `repeat` take.
    /// trace: decision.lists.index_bare
    fn bare_or_bracketed(&mut self, cursor: &mut Cursor, word: &str, after: &str) -> Parsed<Expr> {
        let start = cursor.at;
        let atom = match cursor.peek().map(|token| &token.kind) {
            Some(TokenKind::Number(_, None) | TokenKind::Variable(_) | TokenKind::LeftBracket) => self.atom(cursor)?,
            Some(_) => {
                let here = cursor.here();
                return self.fail(messages::expected_value(here));
            }
            None => return self.fail(messages::expected_value(cursor.here())),
        };
        // The word that ends the count: a `repeat`'s last `times`, or `of` after a position.
        let ends_at = if after == "times" {
            cursor.tokens.iter().rposition(|token| token.is_word("times"))
        } else if after == "of" {
            cursor.tokens[cursor.at..].iter().position(|token| token.is_word("of")).map(|index| index + cursor.at)
        } else {
            None
        };
        let continues = cursor.at != ends_at.unwrap_or(usize::MAX)
            && cursor
                .peek()
                .is_some_and(|token| ["plus", "minus", "times", "divided"].iter().any(|op| token.is_word(op)));
        if continues {
            let end = ends_at.unwrap_or(cursor.tokens.len());
            let span = Span::new(cursor.tokens[start].span.start, cursor.tokens[end - 1].span.end);
            let written = self.text(span).to_string();
            return self.fail(messages::needs_brackets_after(span, word, &written));
        }
        Ok(atom)
    }

    /// What `of` and `in` take: one name, a table written in square brackets, or anything bracketed.
    fn of_target(&mut self, cursor: &mut Cursor, word: &str) -> Parsed<Expr> {
        match cursor.peek().map(|token| &token.kind) {
            Some(TokenKind::Variable(_) | TokenKind::LeftBracket | TokenKind::LeftSquare) => {
                let start = cursor.at;
                let target = self.atom(cursor)?;
                let continues = cursor
                    .peek()
                    .is_some_and(|token| ["plus", "minus", "times", "divided"].iter().any(|op| token.is_word(op)));
                if continues {
                    let span = Span::new(cursor.tokens[start].span.start, cursor.line_span.end);
                    let end = cursor.tokens.iter().take_while(|t| t.span.end <= span.end).last().map(|t| t.span.end);
                    let span = Span::new(span.start, end.unwrap_or(span.end));
                    let written = self.text(span).to_string();
                    return self.fail(messages::needs_brackets_after(span, word, &written));
                }
                Ok(target)
            }
            _ => {
                let here = cursor.here();
                self.fail(messages::of_needs_name_or_brackets(here, word))
            }
        }
    }

    fn atom(&mut self, cursor: &mut Cursor) -> Parsed<Expr> {
        let Some(token) = cursor.next() else {
            return self.fail(messages::expected_value(cursor.here()));
        };
        let span = token.span;
        match &token.kind {
            TokenKind::Number(whole, fraction) => {
                if cursor.peek_word("over") {
                    // trace: decision.types.over
                    cursor.next();
                    let Some(Token { kind: TokenKind::Number(bottom, None), span: bottom_span }) = cursor.next() else {
                        return self.fail(messages::over_needs_whole_numbers(Span::new(span.start, cursor.last_end())));
                    };
                    if fraction.is_some() {
                        return self.fail(messages::over_needs_whole_numbers(span));
                    }
                    let top: BigInt = whole.parse().unwrap_or_default();
                    let bottom: BigInt = bottom.parse().unwrap_or_default();
                    return Ok(self.expr(ExprKind::Fraction(top, bottom), span.to(*bottom_span)));
                }
                Ok(self.number(whole, fraction.as_deref(), span))
            }
            TokenKind::Tilde => {
                // trace: decision.types.approx
                let Some(Token { kind: TokenKind::Number(whole, fraction), span: number_span }) = cursor.next() else {
                    return self.fail(messages::tilde_needs_number(span));
                };
                let written = match fraction {
                    Some(fraction) => format!("{}.{}", whole, fraction),
                    None => whole.clone(),
                };
                Ok(self.expr(ExprKind::About { written }, span.to(*number_span)))
            }
            TokenKind::Text(text) => {
                if text.is_empty() {
                    // trace: decision.values.empty
                    return self.fail(messages::empty_text_is_empty(span));
                }
                Ok(self.expr(ExprKind::Text(text.clone()), span))
            }
            TokenKind::Variable(name) => Ok(self.expr(ExprKind::Variable(name.clone()), span)),
            TokenKind::LeftBracket => {
                let inner = self.test(cursor)?;
                if !cursor.eat_kind(&TokenKind::RightBracket) {
                    return self.fail(messages::bracket_not_closed(span));
                }
                let span = Span::new(span.start, cursor.last_end());
                Ok(self.expr(ExprKind::Brackets(Box::new(inner)), span))
            }
            TokenKind::LeftSquare => self.table_literal(cursor, span),
            TokenKind::Word(word) => self.word_atom(cursor, word, span),
            TokenKind::Mark(mark) => self.fail(messages::mark(span, *mark)),
            _ => self.fail(messages::unexpected_here(span, self.text(span))),
        }
    }

    fn number(&mut self, whole: &str, fraction: Option<&str>, span: Span) -> Expr {
        match fraction {
            None => {
                let value: BigInt = whole.parse().unwrap_or_default();
                self.expr(ExprKind::Whole(value), span)
            }
            Some(fraction) => {
                let digits: BigInt = format!("{}{}", whole, fraction).parse().unwrap_or_default();
                let written = format!("{}.{}", whole, fraction);
                self.expr(ExprKind::Decimal { digits, places: fraction.len() as u32, written }, span)
            }
        }
    }

    /// trace: decision.tables.shape (`[10, 20, 30]` stays on one line)
    fn table_literal(&mut self, cursor: &mut Cursor, open: Span) -> Parsed<Expr> {
        if cursor.eat_kind(&TokenKind::RightSquare) {
            // trace: decision.values.empty
            return self.fail(messages::empty_table_is_empty(Span::new(open.start, cursor.last_end())));
        }
        let mut items = Vec::new();
        let mut keyed = Vec::new();
        loop {
            if cursor.at_end() {
                return self.fail(messages::square_not_closed(open));
            }
            let first = self.test(cursor)?;
            if cursor.eat_kind(&TokenKind::Colon) {
                let value = self.test(cursor)?;
                keyed.push((first, value));
            } else {
                items.push(first);
            }
            if cursor.eat_kind(&TokenKind::Comma) {
                continue;
            }
            if cursor.eat_kind(&TokenKind::RightSquare) {
                break;
            }
            if cursor.at_end() {
                return self.fail(messages::square_not_closed(open));
            }
            let here = cursor.here();
            return self.fail(messages::unexpected_here(here, self.text(here)));
        }
        let span = Span::new(open.start, cursor.last_end());
        if !keyed.is_empty() && !items.is_empty() {
            return self.fail(messages::mixed_keys(span));
        }
        if keyed.is_empty() {
            Ok(self.expr(ExprKind::Table(items), span))
        } else {
            Ok(self.expr(ExprKind::KeyedTable(keyed), span))
        }
    }

    fn word_atom(&mut self, cursor: &mut Cursor, word: &str, span: Span) -> Parsed<Expr> {
        match word {
            "true" => Ok(self.expr(ExprKind::True, span)),
            "false" => Ok(self.expr(ExprKind::False, span)),
            "empty" => Ok(self.expr(ExprKind::Empty, span)),
            "length" | "sum" | "biggest" | "smallest" | "average" => {
                // trace: decision.tables.of_family
                if !cursor.eat_word("of") {
                    return self.fail(messages::needs_of(cursor.here(), word));
                }
                let target = self.of_target(cursor, "of")?;
                let which = match word {
                    "length" => OfWord::Length,
                    "sum" => OfWord::Sum,
                    "biggest" => OfWord::Biggest,
                    "smallest" => OfWord::Smallest,
                    _ => OfWord::Average,
                };
                let span = span.to(target.span);
                Ok(self.expr(ExprKind::Of(which, Box::new(target)), span))
            }
            "position" | "offset" => {
                // trace: decision.lists.index
                let kind = if word == "position" { IndexKind::Position } else { IndexKind::Offset };
                let index = self.bare_or_bracketed(cursor, word, "of")?;
                if !cursor.eat_word("of") {
                    return self.fail(messages::needs_of(cursor.here(), word));
                }
                let target = self.of_target(cursor, "of")?;
                let span = span.to(target.span);
                Ok(self.expr(ExprKind::Index(kind, Box::new(index), Box::new(target)), span))
            }
            "round" => {
                // trace: decision.decimals.rounding
                if !cursor.peek_kind(&TokenKind::LeftBracket) {
                    return self.fail(messages::round_needs_brackets(span));
                }
                let inner = self.atom(cursor)?;
                let span = span.to(inner.span);
                Ok(self.expr(ExprKind::Round(Box::new(inner)), span))
            }
            "remainder" => {
                // trace: decision.maths.remainder
                if !cursor.eat_word("of") {
                    return self.fail(messages::needs_of(cursor.here(), word));
                }
                let inner = if cursor.peek_kind(&TokenKind::LeftBracket) { Some(self.atom(cursor)?) } else { None };
                match inner {
                    Some(Expr { kind: ExprKind::Brackets(inner), span: inner_span, .. }) => match inner.kind {
                        ExprKind::Maths(MathsOp::DividedBy, left, right) => {
                            let span = span.to(inner_span);
                            Ok(self.expr(ExprKind::Remainder(left, right), span))
                        }
                        _ => self.fail(messages::remainder_needs_division(inner_span)),
                    },
                    _ => self.fail(messages::remainder_needs_division(cursor.here())),
                }
            }
            "nothing" => self.fail(messages::nothing_is_not_a_value(span)),
            other if is_grammar(other) => self.fail(messages::unexpected_here(span, other)),
            name => {
                let name = name.to_string();
                if cursor.peek_kind(&TokenKind::LeftBracket) {
                    // trace: decision.procedures.nested_calls
                    let open = cursor.at;
                    let mut depth = 0;
                    let mut close = None;
                    for (index, token) in cursor.tokens.iter().enumerate().skip(open) {
                        match token.kind {
                            TokenKind::LeftBracket => depth += 1,
                            TokenKind::RightBracket => {
                                depth -= 1;
                                if depth == 0 {
                                    close = Some(index);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    let Some(close) = close else {
                        return self.fail(messages::bracket_not_closed(cursor.tokens[open].span));
                    };
                    let inside = &cursor.tokens[open + 1..close];
                    let args = self.args(inside, span)?;
                    cursor.at = close + 1;
                    let span = Span::new(span.start, cursor.last_end());
                    return Ok(self.expr(ExprKind::Call(name, args), span));
                }
                // A name with something after it is a call missing its brackets.
                let next_starts_value = cursor.peek().is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Variable(_) | TokenKind::Number(..) | TokenKind::Text(_) | TokenKind::LeftSquare
                    ) || token.is_word("share")
                });
                if next_starts_value {
                    let rest = &cursor.tokens[cursor.at..];
                    let end = split_top_level(rest, &TokenKind::Comma)[0];
                    let args_span = tokens_span(end);
                    let fixed = format!("{}({})", name, self.text(args_span));
                    let whole = span.to(args_span);
                    return self.fail(messages::nested_call_needs_brackets(whole, &fixed));
                }
                Ok(self.expr(ExprKind::Call(name, Vec::new()), span).with_bare())
            }
        }
    }
}

trait Bare {
    fn with_bare(self) -> Self;
}

impl Bare for Expr {
    /// A lone name in a value: the checker decides whether it is a variable missing its `@` or a call missing `()`.
    fn with_bare(mut self) -> Self {
        if let ExprKind::Call(name, _) = self.kind {
            self.kind = ExprKind::BareName(name);
        }
        self
    }
}
