//! Runs a checked program by walking its parsed source.

use crate::ast::*;
use crate::check::Facts;
use crate::diagnostic::{Diagnostic, Stage};
use crate::messages;
use crate::numbers::{self, Exact, Fast, FastOp, Size, Trouble};
use crate::source::Span;
use crate::types::{Rules, Ty};
use crate::value::{Table, Value};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

type Run<T> = Result<T, Diagnostic>;

/// How deep procedures may call each other before TAKL stops; what TAKL should do there is not designed.
const DEEPEST: usize = 10_000;

enum Flow {
    Next,
    Stop,
    Finish,
    Return(Value),
}

struct Slot {
    name: String,
    value: Value,
    rules: Rules,
    set_at: Span,
    set_step: u64,
}

struct Machine<'a> {
    procedures: HashMap<&'a str, &'a Procedure>,
    facts: &'a Facts,
    output: &'a mut String,
    frames: Vec<Vec<Vec<Slot>>>,
    step: u64,
}

/// Runs `start_running` and gives the exit code.
/// trace: decision.procedures.start_running
/// trace: decision.procedures.start_running_required
/// trace: decision.procedures.exit_code
pub fn run(program: &Program, facts: &Facts, output: &mut String) -> Run<u8> {
    let procedures = program.procedures.iter().map(|procedure| (procedure.name.as_str(), procedure)).collect();
    let mut machine = Machine { procedures, facts, output, frames: Vec::new(), step: 0 };
    let start = machine.procedures["start_running"];
    match machine.call(start, Vec::new(), start.name_span)? {
        None => Ok(0),
        Some(Value::Whole { n, .. }) => Ok(n.to_u8().unwrap_or(0)),
        Some(_) => Ok(0),
    }
}

enum Passed {
    Value(Value),
    Shared(Value),
}

impl<'a> Machine<'a> {
    // ---- variables --------------------------------------------------------------------------------------------

    fn scopes(&mut self) -> &mut Vec<Vec<Slot>> {
        self.frames.last_mut().expect("a procedure is always running")
    }

    fn slot(&mut self, name: &str) -> &mut Slot {
        self.scopes()
            .iter_mut()
            .rev()
            .flat_map(|scope| scope.iter_mut().rev())
            .find(|slot| slot.name == name)
            .expect("the checker made sure every variable is declared")
    }

    fn find(&self, name: &str) -> Option<&Slot> {
        self.frames.last()?.iter().rev().flat_map(|scope| scope.iter().rev()).find(|slot| slot.name == name)
    }

    fn declare(&mut self, name: &str, value: Value, rules: Rules, at: Span) {
        self.step += 1;
        let step = self.step;
        let slot = Slot { name: name.to_string(), value, rules, set_at: at, set_step: step };
        self.scopes().last_mut().expect("a block is always open").push(slot);
    }

    /// A value made into a variable's type and checked against its bounds before it is stored.
    /// trace: decision.types.bounds
    fn fit(&self, value: Value, rules: &Rules, span: Span) -> Run<Value> {
        let value = self.make(value, &rules.ty, span)?;
        if let Value::Whole { n, .. } = &value {
            let below = rules.at_least.as_ref().is_some_and(|least| n < least);
            let above = rules.at_most.as_ref().is_some_and(|most| n > most);
            if below || above {
                return Err(messages::outside_bounds(span, &n.to_string(), &rules.at_least, &rules.at_most, false));
            }
        }
        Ok(value)
    }

    /// Makes a written number the type it is used as; a number that doesn't fit stops the program.
    /// trace: decision.numbers.range
    fn make(&self, value: Value, ty: &Ty, span: Span) -> Run<Value> {
        match (value, ty) {
            (Value::Whole { size: None, n, .. }, Ty::Whole { size, never_negative }) => {
                if !numbers::whole_fits(&n, *size, *never_negative) {
                    let (least, most) = numbers::whole_range(*size, *never_negative);
                    return Err(messages::too_big_for(span, &n.to_string(), &ty.name(), &least, &most, *size, false));
                }
                Ok(Value::Whole { size: Some(*size), never_negative: *never_negative, n })
            }
            (Value::Whole { size: None, n, .. }, Ty::Exact(size)) => {
                let value = Exact::whole(n).fit(Some(*size)).map_err(|t| self.trouble(span, t, ty, ""))?;
                Ok(Value::Exact { size: Some(*size), value })
            }
            (Value::Exact { size: None, value }, Ty::Exact(size)) => {
                let value = value.fit(Some(*size)).map_err(|t| self.trouble(span, t, ty, ""))?;
                Ok(Value::Exact { size: Some(*size), value })
            }
            (Value::Whole { size: None, n, .. }, Ty::Fast(size)) => self.fast_from(&n.to_string(), *size, span, ty),
            (Value::Exact { size: None, value }, Ty::Fast(size)) => self.fast_from(&value.to_text(), *size, span, ty),
            (Value::Fraction { size: None, top, bottom }, Ty::Fraction(size)) => {
                if !numbers::fraction_fits(&top, &bottom, *size) {
                    let written = format!("{} over {}", top, bottom);
                    return Err(messages::fraction_too_big(span, &written, &ty.name(), *size, Stage::WhileRunning));
                }
                Ok(Value::Fraction { size: Some(*size), top, bottom })
            }
            (value, _) => Ok(value),
        }
    }

    fn fast_from(&self, text: &str, size: Size, span: Span, ty: &Ty) -> Run<Value> {
        match Fast::parse(text, size) {
            Ok((fast, true)) => Ok(Value::Fast(fast)),
            Ok((_, false)) => Err(messages::fast_cant_hold(
                span,
                text,
                &Fast::nearest_shown(text, size),
                &ty.name(),
                Stage::WhileRunning,
            )),
            Err(_) => Err(messages::fast_too_big(span, text, &ty.name(), Stage::WhileRunning)),
        }
    }

    fn trouble(&self, span: Span, trouble: Trouble, ty: &Ty, written: &str) -> Diagnostic {
        match trouble {
            Trouble::DividedByZero => messages::divided_by_zero(span, Stage::WhileRunning),
            Trouble::Halfway => messages::round_halfway_not_designed(span, Stage::WhileRunning),
            _ if !written.is_empty() && matches!(ty, Ty::Whole { .. }) => {
                messages::uneven_whole_division(span, written, Stage::WhileRunning)
            }
            trouble => messages::decimal_trouble(span, trouble, &ty.name(), Stage::WhileRunning),
        }
    }

    // ---- procedures -------------------------------------------------------------------------------------------

    fn call(&mut self, procedure: &'a Procedure, args: Vec<Passed>, span: Span) -> Run<Option<Value>> {
        if self.frames.len() >= DEEPEST {
            return Err(messages::too_deep_not_designed(span));
        }
        let facts = &self.facts.procedures[procedure.name.as_str()];
        self.frames.push(vec![Vec::new()]);
        let mut args = args.into_iter();
        for (param, rules) in procedure.params.iter().zip(&facts.params) {
            let value = match args.next() {
                // trace: decision.procedures.share
                Some(Passed::Shared(value)) => value,
                Some(Passed::Value(value)) => match self.fit(value, rules, span) {
                    Ok(value) => value,
                    Err(error) => {
                        self.frames.pop();
                        return Err(error);
                    }
                },
                None => {
                    // trace: decision.procedures.defaults
                    let default =
                        param.default.as_ref().expect("the checker made sure a missing argument has a default");
                    let value = self.eval(default).and_then(|value| self.fit(value, rules, default.span));
                    match value {
                        Ok(value) => value,
                        Err(error) => {
                            self.frames.pop();
                            return Err(error);
                        }
                    }
                }
            };
            self.declare(&param.name, value, rules.clone(), param.span);
        }
        let flow = self.block(&procedure.body);
        self.frames.pop();
        match flow? {
            Flow::Return(value) => Ok(Some(value)),
            _ => Ok(None),
        }
    }

    fn pass(&mut self, args: &[Arg]) -> Run<Vec<Passed>> {
        let mut passed = Vec::new();
        for arg in args {
            passed.push(match arg {
                Arg::Value(expr) => Passed::Value(self.eval(expr)?.copy()),
                Arg::Share(name, _) => Passed::Shared(self.slot(name).value.clone()),
            });
        }
        Ok(passed)
    }

    // ---- statements -------------------------------------------------------------------------------------------

    fn block(&mut self, body: &[Stmt]) -> Run<Flow> {
        self.scopes().push(Vec::new());
        let mut flow = Ok(Flow::Next);
        for stmt in body {
            match self.stmt(stmt) {
                Ok(Flow::Next) => {}
                other => {
                    flow = other;
                    break;
                }
            }
        }
        self.scopes().pop();
        flow
    }

    fn truth(&mut self, test: &Expr) -> Run<bool> {
        match self.eval(test)? {
            Value::TrueOrFalse(truth) => Ok(truth),
            _ => Ok(false),
        }
    }

    fn stmt(&mut self, stmt: &Stmt) -> Run<Flow> {
        if let Some(test) = &stmt.only_if {
            // trace: decision.conditions.trailing_if
            if !self.truth(test)? {
                return Ok(Flow::Next);
            }
        }
        match &stmt.kind {
            StmtKind::Let { name, name_span, value, .. } => {
                let rules = self.facts.lets[&name_span.start].clone();
                let value = self.let_value(value, &rules, *name_span)?;
                self.declare(name, value, rules, stmt.span);
            }
            StmtKind::BrokenLet { .. } | StmtKind::Broken => {}
            StmtKind::Assign { name, value, .. } => {
                let rules = self.slot(name).rules.clone();
                let value = self.let_value(value, &rules, stmt.span)?;
                self.step += 1;
                let step = self.step;
                let slot = self.slot(name);
                slot.value = value;
                slot.set_at = stmt.span;
                slot.set_step = step;
            }
            StmtKind::Show(pieces) => {
                // trace: decision.show.pieces
                let mut line = String::new();
                for piece in pieces {
                    let value = self.eval(piece)?;
                    line.push_str(&self.shown(&value, piece.span)?);
                }
                self.output.push_str(&line);
                self.output.push_str(messages::SHOW_ENDS_WITH);
            }
            StmtKind::If { branches, otherwise } => {
                for (test, body) in branches {
                    if self.truth(test)? {
                        return self.block(body);
                    }
                }
                if let Some(otherwise) = otherwise {
                    return self.block(otherwise);
                }
            }
            StmtKind::RepeatTimes { count, body } => {
                // trace: decision.loops.repeat
                let count = self.whole(count)?;
                if count.is_negative() {
                    return Err(messages::negative_repeat_not_designed(stmt.span, &count.to_string()));
                }
                let mut done = BigInt::zero();
                while done < count {
                    match self.block(body)? {
                        Flow::Next => {}
                        Flow::Stop => break,
                        other => return Ok(other),
                    }
                    done += 1;
                }
            }
            StmtKind::RepeatUntil { test, body } => loop {
                // trace: decision.loops.until (checked before each round)
                if self.truth(test)? {
                    break;
                }
                match self.block(body)? {
                    Flow::Next => {}
                    Flow::Stop => break,
                    other => return Ok(other),
                }
            },
            StmtKind::RepeatForever { body } => loop {
                // trace: decision.loops.forever
                match self.block(body)? {
                    Flow::Next => {}
                    Flow::Stop => break,
                    other => return Ok(other),
                }
            },
            StmtKind::ForEach { key, item, table, body } => {
                // The loop goes through the table as it was when the loop began, since a table is a value.
                let Value::Table(table) = self.eval(table)?.copy() else { return Ok(Flow::Next) };
                let pairs: Vec<(Option<Value>, Value)> = match &*table.borrow() {
                    Table::Numbered(items) => items.iter().map(|v| (None, v.clone())).collect(),
                    Table::Keyed(pairs) => pairs.iter().map(|(k, v)| (Some(k.clone()), v.clone())).collect(),
                };
                for (key_value, value) in pairs {
                    self.scopes().push(Vec::new());
                    if let (Some((name, span)), Some(key_value)) = (key, key_value) {
                        self.declare(name, key_value, Rules::of(Ty::Unknown), *span);
                    }
                    self.declare(&item.0, value, Rules::of(Ty::Unknown), item.1);
                    let flow = self.block(body);
                    self.scopes().pop();
                    match flow? {
                        Flow::Next => {}
                        Flow::Stop => break,
                        other => return Ok(other),
                    }
                }
            }
            StmtKind::Stop => return Ok(Flow::Stop),
            StmtKind::Finish => return Ok(Flow::Finish),
            StmtKind::Return(value) => {
                // trace: decision.procedures.return
                return Ok(Flow::Return(self.eval(value)?.copy()));
            }
            StmtKind::Add { value, key, table, at } => self.add(value, key.as_ref(), table, at.as_ref(), stmt.span)?,
            StmtKind::Call { name, name_span, args } => {
                let passed = self.pass(args)?;
                let procedure = self.procedures[name.as_str()];
                self.call(procedure, passed, *name_span)?;
            }
        }
        Ok(Flow::Next)
    }

    fn let_value(&mut self, value: &LetValue, rules: &Rules, span: Span) -> Run<Value> {
        match value {
            LetValue::Expr(expr) => {
                let value = self.eval(expr)?.copy();
                self.fit(value, rules, expr.span)
            }
            // trace: decision.memory.share
            LetValue::Share(name, _) => Ok(self.slot(name).value.clone()),
            LetValue::Block(items, _) => {
                let _ = span;
                let mut numbered = Vec::new();
                let mut keyed = Vec::new();
                for item in items {
                    match item {
                        BlockItem::Item(expr) => numbered.push(self.eval(expr)?.copy()),
                        BlockItem::Keyed(key, expr) => keyed.push((self.eval(key)?.copy(), self.eval(expr)?.copy())),
                    }
                }
                let keyed_table = matches!(rules.ty, Ty::Table { key: Some(_), .. });
                Ok(Value::new_table(if keyed_table { Table::Keyed(keyed) } else { Table::Numbered(numbered) }))
            }
        }
    }

    /// trace: decision.tables.add
    /// trace: decision.tables.no_holes
    fn add(
        &mut self,
        value: &Expr,
        key: Option<&Expr>,
        table: &(String, Span),
        at: Option<&(IndexKind, Expr)>,
        span: Span,
    ) -> Run<()> {
        let value = self.eval(value)?.copy();
        let key = match key {
            Some(key) => Some(self.eval(key)?.copy()),
            None => None,
        };
        let at = match at {
            Some((kind, index)) => Some((*kind, self.whole(index)?, index.span)),
            None => None,
        };
        let Value::Table(shared) = self.slot(&table.0).value.clone() else { return Ok(()) };
        let mut contents = shared.borrow_mut();
        match (&mut *contents, key) {
            (Table::Numbered(items), None) => match at {
                None => items.push(value),
                Some((kind, index, index_span)) => {
                    let place = match kind {
                        IndexKind::Position => index.clone() - 1,
                        IndexKind::Offset => index.clone(),
                    };
                    let length = items.len();
                    match place.to_usize().filter(|place| *place <= length) {
                        Some(place) => items.insert(place, value),
                        None => {
                            let written = match kind {
                                IndexKind::Position => format!("position {}", index),
                                IndexKind::Offset => format!("offset {}", index),
                            };
                            return Err(messages::would_leave_hole(index_span, &table.0, length, &written));
                        }
                    }
                }
            },
            (Table::Keyed(pairs), Some(key)) => {
                if let Some(existing) = pairs.iter_mut().find(|(k, _)| same(k, &key)) {
                    let _ = existing;
                    return Err(messages::key_already_there_not_designed(span, &self.shown_plain(&key)));
                }
                pairs.push((key, value));
            }
            _ => {}
        }
        drop(contents);
        self.step += 1;
        let step = self.step;
        let slot = self.slot(&table.0);
        slot.set_at = span;
        slot.set_step = step;
        Ok(())
    }

    // ---- values -----------------------------------------------------------------------------------------------

    fn whole(&mut self, expr: &Expr) -> Run<BigInt> {
        match self.eval(expr)? {
            Value::Whole { n, .. } => Ok(n),
            _ => Ok(BigInt::zero()),
        }
    }

    fn shown_plain(&self, value: &Value) -> String {
        match value {
            Value::Text(text) => format!("\"{}\"", text),
            Value::Whole { n, .. } => n.to_string(),
            _ => String::new(),
        }
    }

    /// trace: decision.show.numbers
    fn shown(&self, value: &Value, span: Span) -> Run<String> {
        Ok(match value {
            Value::Whole { n, .. } => n.to_string(),
            Value::Exact { value, .. } => value.to_text(),
            Value::Fast(fast) => fast.to_text(),
            Value::Fraction { top, bottom, .. } => {
                // trace: decision.types.over
                if !numbers::in_lowest_terms(top, bottom) {
                    return Err(messages::fraction_not_lowest_not_designed(span, &format!("{} over {}", top, bottom)));
                }
                format!("{} over {}", top, bottom)
            }
            Value::Text(text) => text.clone(),
            Value::TrueOrFalse(_) | Value::Table(_) => String::new(),
        })
    }

    fn eval(&mut self, expr: &Expr) -> Run<Value> {
        let value = self.eval_inner(expr)?;
        match self.facts.targets.get(&expr.id) {
            Some(ty) if is_written(&value) => self.make(value, ty, expr.span),
            _ => Ok(value),
        }
    }

    fn eval_inner(&mut self, expr: &Expr) -> Run<Value> {
        match &expr.kind {
            ExprKind::Whole(n) => Ok(Value::Whole { size: None, never_negative: false, n: n.clone() }),
            ExprKind::Decimal { digits, places, .. } => Ok(Value::Exact {
                size: None,
                value: Exact { coefficient: digits.clone(), exponent: -(*places as i64) },
            }),
            ExprKind::About { written } => {
                // trace: decision.types.approx (the nearest a fast decimal holds)
                let Some(Ty::Fast(size)) = self.facts.targets.get(&expr.id) else {
                    return Ok(Value::Text(String::new()));
                };
                match Fast::parse(written, *size) {
                    Ok((fast, _)) => Ok(Value::Fast(fast)),
                    Err(_) => {
                        Err(messages::fast_too_big(expr.span, written, &Ty::Fast(*size).name(), Stage::WhileRunning))
                    }
                }
            }
            ExprKind::Fraction(top, bottom) => {
                Ok(Value::Fraction { size: None, top: top.clone(), bottom: bottom.clone() })
            }
            ExprKind::Text(text) => Ok(Value::Text(text.clone())),
            ExprKind::True => Ok(Value::TrueOrFalse(true)),
            ExprKind::False => Ok(Value::TrueOrFalse(false)),
            ExprKind::Empty => Ok(match self.facts.targets.get(&expr.id) {
                Some(Ty::Table { key: Some(_), .. }) => Value::new_table(Table::Keyed(Vec::new())),
                Some(Ty::Table { key: None, .. }) => Value::new_table(Table::Numbered(Vec::new())),
                _ => Value::Text(String::new()),
            }),
            ExprKind::Variable(name) => Ok(self.slot(name).value.clone()),
            ExprKind::Brackets(inner) => self.eval(inner),
            ExprKind::Maths(op, left, right) => {
                let a = self.eval(left)?;
                let b = self.eval(right)?;
                self.maths(expr, *op, a, b, false)
            }
            ExprKind::Compare(op, left, right) => {
                let a = self.eval(left)?;
                let b = self.eval(right)?;
                let ordering = compare(&a, &b);
                let truth = match op {
                    CompareOp::Is => ordering == Some(Ordering::Equal),
                    CompareOp::Isnt => ordering != Some(Ordering::Equal),
                    CompareOp::MoreThan => ordering == Some(Ordering::Greater),
                    CompareOp::LessThan => ordering == Some(Ordering::Less),
                    CompareOp::AtLeast => matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
                    CompareOp::AtMost => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                };
                Ok(Value::TrueOrFalse(truth))
            }
            ExprKind::Logic(op, left, right) => {
                // trace: decision.conditions.short_circuit
                let a = self.truth(left)?;
                let truth = match op {
                    LogicOp::And => a && self.truth(right)?,
                    LogicOp::Or => a || self.truth(right)?,
                    LogicOp::AndAlso => {
                        let b = self.truth(right)?;
                        a && b
                    }
                    LogicOp::OrAlso => {
                        let b = self.truth(right)?;
                        a || b
                    }
                };
                Ok(Value::TrueOrFalse(truth))
            }
            ExprKind::Not(inner) => Ok(Value::TrueOrFalse(!self.truth(inner)?)),
            ExprKind::Table(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.eval(item)?.copy());
                }
                Ok(Value::new_table(Table::Numbered(values)))
            }
            ExprKind::KeyedTable(pairs) => {
                let mut values = Vec::new();
                for (key, value) in pairs {
                    values.push((self.eval(key)?.copy(), self.eval(value)?.copy()));
                }
                Ok(Value::new_table(Table::Keyed(values)))
            }
            ExprKind::Of(word, target) => {
                let target_value = self.eval(target)?;
                self.of(expr, *word, target_value)
            }
            ExprKind::Index(kind, index, target) => {
                // trace: decision.lists.index
                let index_value = self.whole(index)?;
                let target_value = self.eval(target)?;
                let place = match kind {
                    IndexKind::Position => index_value.clone() - 1,
                    IndexKind::Offset => index_value.clone(),
                };
                let written = match kind {
                    IndexKind::Position => format!("position {}", index_value),
                    IndexKind::Offset => format!("offset {}", index_value),
                };
                match target_value {
                    Value::Table(table) => {
                        let table = table.borrow();
                        let Table::Numbered(items) = &*table else { return Ok(Value::Text(String::new())) };
                        match place.to_usize().and_then(|place| items.get(place)) {
                            Some(item) => Ok(item.clone()),
                            None => Err(messages::index_outside(expr.span, &written, items.len(), "items")),
                        }
                    }
                    Value::Text(text) => {
                        // trace: decision.text.letters
                        let letters: Vec<&str> = text.graphemes(true).collect();
                        match place.to_usize().and_then(|place| letters.get(place)) {
                            Some(letter) => Ok(Value::Text(letter.to_string())),
                            None => Err(messages::index_outside(expr.span, &written, letters.len(), "letters")),
                        }
                    }
                    _ => Ok(Value::Text(String::new())),
                }
            }
            ExprKind::Round(inner) => {
                // trace: decision.decimals.rounding
                let unbracketed = match &inner.kind {
                    ExprKind::Brackets(inner) => inner,
                    _ => inner,
                };
                if let ExprKind::Maths(MathsOp::DividedBy, left, right) = &unbracketed.kind {
                    let mut a = self.eval(left)?;
                    let mut b = self.eval(right)?;
                    // Written numbers take the type the rounded answer goes into, so it rounds to that type's digits.
                    if let Some(ty) = self.facts.targets.get(&expr.id) {
                        a = self.make(a, ty, left.span)?;
                        b = self.make(b, ty, right.span)?;
                    }
                    let value = self.maths(unbracketed, MathsOp::DividedBy, a, b, true)?;
                    return Ok(value);
                }
                self.eval(inner)
            }
            ExprKind::Remainder(left, right) => {
                // trace: decision.maths.remainder
                let a = self.eval(left)?;
                let b = self.eval(right)?;
                let (Value::Whole { size, never_negative, n: a }, Value::Whole { n: b, .. }) = (a, b) else {
                    return Ok(Value::Text(String::new()));
                };
                if b.is_zero() {
                    return Err(self.because(expr, messages::divided_by_zero(expr.span, Stage::WhileRunning)));
                }
                if a.is_negative() || b.is_negative() {
                    return Err(self.because(expr, messages::negative_remainder_not_designed(expr.span)));
                }
                Ok(Value::Whole { size, never_negative, n: a % b })
            }
            ExprKind::Call(name, args) => {
                let passed = self.pass(args)?;
                let procedure = self.procedures[name.as_str()];
                Ok(self.call(procedure, passed, expr.span)?.unwrap_or(Value::Text(String::new())))
            }
            ExprKind::BareName(_) => Ok(Value::Text(String::new())),
        }
    }

    /// Maths on two values of one type. Whole numbers stop past their range; exact decimals never round on their own.
    /// trace: decision.maths.words
    /// trace: decision.numbers.range
    /// trace: decision.maths.whole_divide
    fn maths(&self, expr: &Expr, op: MathsOp, a: Value, b: Value, round: bool) -> Run<Value> {
        match (a, b) {
            (Value::Whole { size, never_negative, n: a }, Value::Whole { n: b, .. }) => {
                let n = match op {
                    MathsOp::Plus => a + b,
                    MathsOp::Minus => a - b,
                    MathsOp::Times => a * b,
                    MathsOp::DividedBy => {
                        // Every maths step that stops says where its values came from, as `because:`.
                        if b.is_zero() {
                            return Err(self.because(expr, messages::divided_by_zero(expr.span, Stage::WhileRunning)));
                        }
                        if round {
                            numbers::round_nearest(&a, &b).map_err(|_| {
                                self.because(expr, messages::round_halfway_not_designed(expr.span, Stage::WhileRunning))
                            })?
                        } else if (&a % &b).is_zero() {
                            a / b
                        } else {
                            let written = format!("{} divided by {}", a, b);
                            let error = messages::uneven_whole_division(expr.span, &written, Stage::WhileRunning);
                            return Err(self.because(expr, error));
                        }
                    }
                };
                if let Some(size) = size
                    && !numbers::whole_fits(&n, size, never_negative)
                {
                    let ty = Ty::Whole { size, never_negative };
                    let (least, most) = numbers::whole_range(size, never_negative);
                    let error =
                        messages::too_big_for(expr.span, &n.to_string(), &ty.name(), &least, &most, size, false);
                    return Err(self.because(expr, error));
                }
                Ok(Value::Whole { size, never_negative, n })
            }
            (Value::Exact { size, value: a }, Value::Exact { value: b, .. }) => {
                let result = match op {
                    MathsOp::Plus => a.plus(&b, size),
                    MathsOp::Minus => a.minus(&b, size),
                    MathsOp::Times => a.times(&b, size),
                    MathsOp::DividedBy if round => a.round_division(&b, size),
                    MathsOp::DividedBy => a.divided_by(&b, size),
                };
                let ty = size.map(Ty::Exact).unwrap_or(Ty::WrittenDecimal);
                let value = result.map_err(|trouble| self.because(expr, self.trouble(expr.span, trouble, &ty, "")))?;
                Ok(Value::Exact { size, value })
            }
            (Value::Whole { size: None, n, .. }, Value::Exact { size, value: b }) => self.maths(
                expr,
                op,
                Value::Exact { size, value: Exact::whole(n) },
                Value::Exact { size, value: b },
                round,
            ),
            (Value::Exact { size, value: a }, Value::Whole { size: None, n, .. }) => self.maths(
                expr,
                op,
                Value::Exact { size, value: a },
                Value::Exact { size, value: Exact::whole(n) },
                round,
            ),
            (Value::Fast(a), Value::Fast(b)) => {
                let op = match op {
                    MathsOp::Plus => FastOp::Plus,
                    MathsOp::Minus => FastOp::Minus,
                    MathsOp::Times => FastOp::Times,
                    MathsOp::DividedBy => FastOp::DividedBy,
                };
                let ty = Ty::Fast(a.size);
                let fast =
                    a.apply(&b, op).map_err(|trouble| self.because(expr, self.trouble(expr.span, trouble, &ty, "")))?;
                Ok(Value::Fast(fast))
            }
            (a, _) => Ok(a),
        }
    }

    /// Adds where each variable in a step got its value, oldest first.
    /// Adds where each variable in the failing step got its value, oldest first, nested ones included.
    /// trace: decision.errors.layout (`because:`, the earlier steps that led here)
    fn because(&self, expr: &Expr, mut error: Diagnostic) -> Diagnostic {
        let mut names = Vec::new();
        variables_in(expr, &mut names);
        let mut sets: Vec<(u64, Span, String)> = Vec::new();
        for name in names {
            let Some(slot) = self.find(&name) else { continue };
            if sets.iter().any(|(_, span, _)| *span == slot.set_at) {
                continue;
            }
            let fact = match (&slot.value, self.shown(&slot.value, slot.set_at)) {
                (Value::Table(_), _) | (_, Err(_)) => format!("{} was last set here", name),
                (_, Ok(shown)) => format!("{} was {}", name, shown),
            };
            sets.push((slot.set_step, slot.set_at, fact));
        }
        sets.sort_by_key(|(step, ..)| *step);
        for (_, span, fact) in sets {
            error = error.because(span, fact);
        }
        error
    }

    fn of(&self, expr: &Expr, word: OfWord, target: Value) -> Run<Value> {
        // trace: decision.tables.of_family
        let items: Vec<Value> = match &target {
            Value::Text(text) if word == OfWord::Length => {
                return Ok(written_whole(BigInt::from(text.graphemes(true).count())));
            }
            Value::Table(table) => match &*table.borrow() {
                Table::Numbered(items) => {
                    if word == OfWord::Length {
                        return Ok(written_whole(BigInt::from(items.len())));
                    }
                    items.clone()
                }
                Table::Keyed(pairs) => return Ok(written_whole(BigInt::from(pairs.len()))),
            },
            _ => return Ok(written_whole(BigInt::zero())),
        };
        let item_type = self.facts.of_types.get(&expr.id).cloned().unwrap_or(Ty::WrittenWhole);
        match word {
            OfWord::Length => unreachable!("length returned above"),
            OfWord::Sum | OfWord::Average => {
                if word == OfWord::Average && items.is_empty() {
                    return Err(messages::no_items(expr.span, "average"));
                }
                let mut total = self.make(written_whole(BigInt::zero()), &item_type, expr.span)?;
                for item in &items {
                    total = self.maths(expr, MathsOp::Plus, total, item.clone(), false)?;
                }
                if word == OfWord::Sum {
                    return Ok(total);
                }
                let count = self.make(written_whole(BigInt::from(items.len())), &item_type, expr.span)?;
                self.maths(expr, MathsOp::DividedBy, total, count, false)
            }
            OfWord::Biggest | OfWord::Smallest => {
                let mut best: Option<Value> = None;
                for item in items {
                    best = Some(match best {
                        None => item,
                        Some(current) => {
                            let ordering = compare(&item, &current);
                            let better = if word == OfWord::Biggest {
                                ordering == Some(Ordering::Greater)
                            } else {
                                ordering == Some(Ordering::Less)
                            };
                            if better { item } else { current }
                        }
                    });
                }
                best.ok_or_else(|| {
                    messages::no_items(expr.span, if word == OfWord::Biggest { "biggest" } else { "smallest" })
                })
            }
        }
    }
}

/// Every variable named in an expression, in the order they are written, each once.
fn variables_in(expr: &Expr, names: &mut Vec<String>) {
    let visit = |inner: &Expr, names: &mut Vec<String>| variables_in(inner, names);
    match &expr.kind {
        ExprKind::Variable(name) => {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        ExprKind::Brackets(inner) | ExprKind::Not(inner) | ExprKind::Round(inner) | ExprKind::Of(_, inner) => {
            visit(inner, names)
        }
        ExprKind::Maths(_, left, right)
        | ExprKind::Compare(_, left, right)
        | ExprKind::Logic(_, left, right)
        | ExprKind::Remainder(left, right)
        | ExprKind::Index(_, left, right) => {
            visit(left, names);
            visit(right, names);
        }
        ExprKind::Table(items) => items.iter().for_each(|item| visit(item, names)),
        ExprKind::KeyedTable(pairs) => pairs.iter().for_each(|(key, value)| {
            visit(key, names);
            visit(value, names);
        }),
        ExprKind::Call(_, args) => {
            for arg in args {
                match arg {
                    Arg::Value(value) => visit(value, names),
                    Arg::Share(name, _) => {
                        if !names.contains(name) {
                            names.push(name.clone());
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn written_whole(n: BigInt) -> Value {
    Value::Whole { size: None, never_negative: false, n }
}

fn is_written(value: &Value) -> bool {
    matches!(
        value,
        Value::Whole { size: None, .. } | Value::Exact { size: None, .. } | Value::Fraction { size: None, .. }
    )
}

fn same(a: &Value, b: &Value) -> bool {
    compare(a, b) == Some(Ordering::Equal)
}

/// How two values of one type compare; `None` when they can't be ordered.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Whole { n: a, .. }, Value::Whole { n: b, .. }) => Some(a.cmp(b)),
        (Value::Exact { value: a, .. }, Value::Exact { value: b, .. }) => Some(a.compare(b)),
        (Value::Whole { n, .. }, Value::Exact { value, .. }) => Some(Exact::whole(n.clone()).compare(value)),
        (Value::Exact { value, .. }, Value::Whole { n, .. }) => Some(value.compare(&Exact::whole(n.clone()))),
        (Value::Fast(a), Value::Fast(b)) => Some(a.compare(b)),
        (Value::Fraction { top: a, bottom: b, .. }, Value::Fraction { top: c, bottom: d, .. }) => {
            Some((a * d).cmp(&(c * b)))
        }
        (Value::Text(a), Value::Text(b)) => (a == b).then_some(Ordering::Equal).or(Some(Ordering::Less)),
        (Value::TrueOrFalse(a), Value::TrueOrFalse(b)) => (a == b).then_some(Ordering::Equal).or(Some(Ordering::Less)),
        _ => None,
    }
}
