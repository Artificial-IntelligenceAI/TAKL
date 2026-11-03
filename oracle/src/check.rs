//! Finds every mistake that can be known before running: names, types, endings, and numbers past their range.

use crate::ast::*;
use crate::diagnostic::{Diagnostic, Stage};
use crate::messages;
use crate::numbers::{self, Exact, Fast, Size, Trouble};
use crate::source::{Source, Span};
use crate::types::{Rules, Ty, number_type};
use num_bigint::BigInt;
use num_traits::{One, Zero};
use std::collections::HashMap;

/// What the interpreter needs from checking.
#[derive(Default)]
pub struct Facts {
    /// The type each written number, or value worked out from written numbers, is made into when it runs.
    pub targets: HashMap<ExprId, Ty>,
    /// Each `let`'s rules, keyed by where its name starts.
    pub lets: HashMap<usize, Rules>,
    pub procedures: HashMap<String, ProcedureFacts>,
    /// The item type each `sum of`, `average of`, `biggest of` and `smallest of` works in.
    pub of_types: HashMap<ExprId, Ty>,
}

#[derive(Clone)]
pub struct ProcedureFacts {
    pub params: Vec<Rules>,
    /// `None` when the procedure just finishes.
    pub returns: Option<Ty>,
}

pub struct Checked {
    pub errors: Vec<Diagnostic>,
    pub facts: Facts,
}

struct Variable {
    rules: Rules,
    declared: Span,
    /// Its `let` could not be read, so nothing more is said about it.
    broken: bool,
    loop_item: bool,
}

struct Checker<'a> {
    source: &'a Source,
    errors: Vec<Diagnostic>,
    facts: Facts,
    scopes: Vec<Vec<(String, Variable)>>,
    ended: HashMap<String, Span>,
    returns: Option<Ty>,
    procedure_name: String,
    loops: usize,
    /// Which params of each procedure have a default.
    defaults: HashMap<String, Vec<bool>>,
}

/// A value known before running, worked out from written numbers.
#[derive(Clone, Debug)]
enum Known {
    Whole(BigInt),
    Decimal(Exact),
    Fraction(BigInt, BigInt),
}

pub fn check(program: &Program, source: &Source) -> Checked {
    let mut checker = Checker {
        source,
        errors: Vec::new(),
        facts: Facts::default(),
        scopes: Vec::new(),
        ended: HashMap::new(),
        returns: None,
        procedure_name: String::new(),
        loops: 0,
        defaults: HashMap::new(),
    };
    checker.procedures(program);
    for procedure in &program.procedures {
        checker.procedure(procedure);
    }
    Checked { errors: checker.errors, facts: checker.facts }
}

/// Whether a list of statements always reaches its ending word.
fn ends_with(body: &[Stmt], finish: bool) -> bool {
    match body.last() {
        Some(Stmt { kind: StmtKind::Finish, only_if: None, .. }) => finish,
        Some(Stmt { kind: StmtKind::Return(_), only_if: None, .. }) => !finish,
        Some(Stmt { kind: StmtKind::If { branches, otherwise: Some(otherwise) }, only_if: None, .. }) => {
            branches.iter().all(|(_, body)| ends_with(body, finish)) && ends_with(otherwise, finish)
        }
        _ => false,
    }
}

impl<'a> Checker<'a> {
    fn error(&mut self, error: Diagnostic) {
        self.errors.push(error);
    }

    fn text(&self, span: Span) -> String {
        self.source.text[span.start..span.end].to_string()
    }

    // ---- procedures -------------------------------------------------------------------------------------------

    fn procedures(&mut self, program: &Program) {
        let mut seen: HashMap<String, Span> = HashMap::new();
        for procedure in &program.procedures {
            if let Some(first) = seen.get(&procedure.name) {
                let first = *first;
                self.error(messages::procedure_twice(procedure.name_span, &procedure.name, first));
                continue;
            }
            seen.insert(procedure.name.clone(), procedure.name_span);
            let mut params = Vec::new();
            for param in &procedure.params {
                let rules = self.rules(&param.ty, &param.modifiers, Place::Param).unwrap_or(Rules::of(Ty::Unknown));
                params.push(rules);
            }
            let returns = match &procedure.ending {
                Ending::JustFinishes => None,
                Ending::Returns(ty, modifiers) => {
                    Some(self.rules(ty, modifiers, Place::Returns).map(|rules| rules.ty).unwrap_or(Ty::Unknown))
                }
            };
            self.defaults
                .insert(procedure.name.clone(), procedure.params.iter().map(|p| p.default.is_some()).collect());
            self.facts.procedures.insert(procedure.name.clone(), ProcedureFacts { params, returns });
        }
        // trace: decision.procedures.start_running_required
        match program.procedures.iter().find(|procedure| procedure.name == "start_running") {
            None => {
                let span = Span::new(0, self.source.text.find('\n').unwrap_or(self.source.text.len()));
                self.error(messages::no_start_running(span));
            }
            Some(start) => {
                if !start.params.is_empty() {
                    let span = start.params[0].span.to(start.params[start.params.len() - 1].span);
                    self.error(messages::start_running_params_not_designed(span));
                }
                // trace: decision.procedures.exit_code
                if let Some(Some(ty)) = self.facts.procedures.get("start_running").map(|facts| facts.returns.clone()) {
                    let exit_code = Ty::Whole { size: Size::Tiny, never_negative: true };
                    let bounded = matches!(&start.ending, Ending::Returns(_, modifiers)
                        if modifiers.iter().any(|m| matches!(m.kind, ModifierKind::AtLeast(_) | ModifierKind::AtMost(_))));
                    if (ty != exit_code && !ty.is_unknown()) || bounded {
                        self.error(messages::exit_code_type(start.ending_span, &ty.name()));
                    }
                }
            }
        }
    }

    fn procedure(&mut self, procedure: &Procedure) {
        let Some(facts) = self.facts.procedures.get(&procedure.name).cloned() else { return };
        self.returns = facts.returns.clone();
        self.procedure_name = procedure.name.clone();
        self.scopes.push(Vec::new());
        for (param, rules) in procedure.params.iter().zip(&facts.params) {
            if let Some(default) = &param.default {
                // trace: decision.procedures.defaults
                let ty = self.expr(default, Some(&rules.ty));
                self.expect_type(default, &ty, &rules.ty);
                self.check_known_bounds(default, rules);
            }
            self.declare(&param.name, param.span, rules.clone(), false);
        }
        self.block(&procedure.body, false);
        self.scopes.pop();
        // trace: decision.procedures.return_nothing_required (now `finish`, decision.procedures.finish)
        let finish = facts.returns.is_none();
        if !ends_with(&procedure.body, finish) {
            if finish {
                self.error(messages::needs_finish(procedure.end_span, &procedure.name));
            } else if let Some(ty) = &facts.returns {
                self.error(messages::needs_return(procedure.end_span, &procedure.name, &ty.name()));
            }
        }
    }

    // ---- types and their rules --------------------------------------------------------------------------------

    fn named_type(&mut self, name: &str, span: Span) -> Option<Ty> {
        match name {
            "text" => Some(Ty::Text),
            "true_or_false" => Some(Ty::TrueOrFalse),
            _ => match number_type(name) {
                Some(ty) => Some(ty),
                None => {
                    self.error(messages::unknown_type(span, name));
                    None
                }
            },
        }
    }

    fn type_expr(&mut self, ty: &TypeExpr) -> Option<Ty> {
        match ty {
            TypeExpr::Named(name, span) => self.named_type(name, *span),
            TypeExpr::Table { key, item, .. } => {
                let item = self.item_type(item)?;
                let key = match key {
                    Some(key) => {
                        let key = self.item_type(key)?;
                        Some(Box::new(key))
                    }
                    None => None,
                };
                Some(Ty::Table { key, item: Box::new(item) })
            }
        }
    }

    /// trace: decision.tables.item_modifiers
    fn item_type(&mut self, item: &ItemType) -> Option<Ty> {
        Some(self.rules(&item.ty, &item.modifiers, Place::Item)?.ty)
    }

    /// A type with the modifiers written after it.
    fn rules(&mut self, ty: &TypeExpr, modifiers: &[Modifier], place: Place) -> Option<Rules> {
        let ty = self.type_expr(ty)?;
        let mut rules = Rules::of(ty);
        let mut seen: Vec<&ModifierKind> = Vec::new();
        let mut ok = true;
        for modifier in modifiers {
            let same = |seen: &ModifierKind| std::mem::discriminant(seen) == std::mem::discriminant(&modifier.kind);
            if seen.iter().any(|s| same(s)) {
                self.error(messages::modifier_twice(modifier.span));
                ok = false;
                continue;
            }
            seen.push(&modifier.kind);
            match (&modifier.kind, &rules.ty) {
                // trace: decision.syntax.let (`can't_change`)
                (ModifierKind::CantChange, _) if matches!(place, Place::Let | Place::Param) => rules.can_change = false,
                (ModifierKind::NeverNegative, Ty::Whole { size, never_negative: false }) => {
                    rules.ty = Ty::Whole { size: *size, never_negative: true };
                }
                (ModifierKind::FastButInexact, Ty::Exact(size)) => rules.ty = Ty::Fast(*size),
                (ModifierKind::Share, Ty::Table { .. }) if place == Place::Param => rules.share = true,
                (ModifierKind::Share, _) if place == Place::Param => {
                    self.error(messages::share_on_non_table_not_designed(modifier.span));
                    ok = false;
                }
                (ModifierKind::Share, _) if place == Place::Let => {
                    self.error(messages::share_goes_on_value(modifier.span));
                    ok = false;
                }
                (ModifierKind::AtLeast(n) | ModifierKind::AtMost(n), Ty::Whole { size, never_negative })
                    if matches!(place, Place::Let | Place::Param) =>
                {
                    // trace: decision.types.bounds
                    if !numbers::whole_fits(n, *size, *never_negative) {
                        self.error(messages::bound_outside_type(modifier.span, &rules.ty.name()));
                        ok = false;
                    } else if matches!(modifier.kind, ModifierKind::AtLeast(_)) {
                        rules.at_least = Some(n.clone());
                    } else {
                        rules.at_most = Some(n.clone());
                    }
                }
                (ModifierKind::AtLeast(_) | ModifierKind::AtMost(_), Ty::Exact(_) | Ty::Fast(_) | Ty::Fraction(_))
                    if matches!(place, Place::Let | Place::Param) =>
                {
                    self.error(messages::bounds_on_decimals_not_designed(modifier.span));
                    ok = false;
                }
                (ModifierKind::AtLeast(_) | ModifierKind::AtMost(_), _)
                    if matches!(place, Place::Item | Place::Returns) =>
                {
                    self.error(messages::bounds_here_not_designed(modifier.span));
                    ok = false;
                }
                (kind, ty) => {
                    let word = self.text(modifier.span);
                    let _ = kind;
                    self.error(messages::modifier_not_here(modifier.span, &word, &ty.name()));
                    ok = false;
                }
            }
        }
        if let (Some(least), Some(most)) = (&rules.at_least, &rules.at_most)
            && least > most
        {
            let span = modifiers[0].span.to(modifiers[modifiers.len() - 1].span);
            self.error(messages::bounds_crossed(span));
            ok = false;
        }
        ok.then_some(rules)
    }

    // ---- names ------------------------------------------------------------------------------------------------

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().flat_map(|scope| scope.iter().rev()).find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn declare(&mut self, name: &str, span: Span, rules: Rules, loop_item: bool) {
        if let Some(existing) = self.lookup(name)
            && !existing.broken
        {
            let first = existing.declared;
            self.error(messages::name_twice_not_designed(span, name, first));
        }
        let variable = Variable { rules, declared: span, broken: false, loop_item };
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), variable));
        }
    }

    fn declare_broken(&mut self, name: &str, span: Span) {
        let variable = Variable { rules: Rules::of(Ty::Unknown), declared: span, broken: true, loop_item: false };
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), variable));
        }
    }

    fn all_variable_names(&self) -> Vec<String> {
        self.scopes.iter().flat_map(|scope| scope.iter().map(|(n, _)| n.clone())).collect()
    }

    fn variable(&mut self, name: &str, span: Span) -> Option<(Rules, Span, bool, bool)> {
        if let Some(variable) = self.lookup(name) {
            return Some((variable.rules.clone(), variable.declared, variable.broken, variable.loop_item));
        }
        if let Some(ended) = self.ended.get(name).copied() {
            self.error(messages::used_after_block_not_designed(span, name, ended));
        } else {
            let names = self.all_variable_names();
            self.error(messages::unknown_variable(span, name, &names));
        }
        None
    }

    // ---- statements -------------------------------------------------------------------------------------------

    fn block(&mut self, body: &[Stmt], in_loop: bool) {
        self.scopes.push(Vec::new());
        let mut left_by: Option<&'static str> = None;
        for stmt in body {
            if let Some(word) = left_by {
                // trace: decision.errors.return_nothing (a line after it never runs)
                self.error(messages::never_runs(stmt.span, word));
                left_by = None;
                let _ = left_by;
                break;
            }
            self.stmt(stmt);
            if stmt.only_if.is_none() {
                left_by = match stmt.kind {
                    StmtKind::Finish => Some("finish"),
                    StmtKind::Return(_) => Some("return"),
                    StmtKind::Stop => Some("stop"),
                    _ => None,
                };
            }
        }
        // trace: decision.errors.return_nothing (`finish` ending a loop's body was most likely `stop`)
        if in_loop
            && let Some(Stmt { kind: StmtKind::Finish, span, only_if: None }) = body.last()
            && self.returns.is_none()
        {
            self.error(messages::finish_ends_loop(*span));
        }
        if let Some(scope) = self.scopes.pop() {
            for (name, variable) in scope {
                self.ended.insert(name, variable.declared);
            }
        }
    }

    fn loop_block(&mut self, body: &[Stmt]) {
        self.loops += 1;
        self.block(body, true);
        self.loops -= 1;
    }

    fn test(&mut self, expr: &Expr) {
        let ty = self.expr(expr, Some(&Ty::TrueOrFalse));
        if ty != Ty::TrueOrFalse && !ty.is_unknown() {
            self.error(messages::needs_test(expr.span, &ty.name()));
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        if let Some(test) = &stmt.only_if {
            // trace: decision.conditions.trailing_if
            self.test(test);
        }
        match &stmt.kind {
            StmtKind::Let { name, name_span, ty, modifiers, value } => {
                let rules = self.rules(ty, modifiers, Place::Let);
                let Some(rules) = rules else {
                    self.declare_broken(name, *name_span);
                    return;
                };
                self.value(value, &rules, *name_span);
                self.facts.lets.insert(name_span.start, rules.clone());
                self.declare(name, *name_span, rules, false);
            }
            StmtKind::BrokenLet { name } => self.declare_broken(name, stmt.span),
            StmtKind::Assign { name, name_span, value } => {
                let Some((rules, declared, broken, loop_item)) = self.variable(name, *name_span) else { return };
                if broken {
                    return;
                }
                if loop_item {
                    self.error(messages::changing_loop_item_not_designed(*name_span, name));
                    return;
                }
                if !rules.can_change {
                    // trace: decision.lets.change
                    self.error(messages::cant_change(*name_span, name, declared));
                    return;
                }
                self.value(value, &rules, *name_span);
            }
            StmtKind::Show(pieces) => {
                for piece in pieces {
                    let ty = self.expr(piece, None);
                    // trace: decision.show.pieces
                    // trace: decision.show.numbers
                    match ty {
                        Ty::TrueOrFalse => self.error(messages::show_true_or_false_not_designed(piece.span)),
                        Ty::Table { .. } => self.error(messages::show_table_not_designed(piece.span)),
                        _ => {}
                    }
                }
            }
            StmtKind::If { branches, otherwise } => {
                for (test, body) in branches {
                    self.test(test);
                    self.block(body, false);
                }
                if let Some(otherwise) = otherwise {
                    self.block(otherwise, false);
                }
            }
            StmtKind::RepeatTimes { count, body } => {
                self.whole_number(count);
                self.loop_block(body);
            }
            StmtKind::RepeatUntil { test, body } => {
                self.test(test);
                self.loop_block(body);
            }
            StmtKind::RepeatForever { body } => self.loop_block(body),
            StmtKind::ForEach { key, item, table, body } => {
                let ty = self.expr(table, None);
                self.scopes.push(Vec::new());
                match (&ty, key) {
                    (Ty::Table { key: None, item: item_ty }, None) => {
                        self.declare(&item.0, item.1, Rules::of((**item_ty).clone()), true);
                    }
                    (Ty::Table { key: Some(key_ty), item: item_ty }, Some(key)) => {
                        self.declare(&key.0, key.1, Rules::of((**key_ty).clone()), true);
                        self.declare(&item.0, item.1, Rules::of((**item_ty).clone()), true);
                    }
                    (Ty::Table { key: Some(_), .. }, None) | (Ty::Table { key: None, .. }, Some(_)) => {
                        self.error(messages::for_each_shape_not_designed(table.span));
                        self.declare_broken(&item.0, item.1);
                        if let Some(key) = key {
                            self.declare_broken(&key.0, key.1);
                        }
                    }
                    (ty, _) => {
                        if !ty.is_unknown() {
                            self.error(messages::for_each_needs_table(table.span, &ty.name()));
                        }
                        self.declare_broken(&item.0, item.1);
                        if let Some(key) = key {
                            self.declare_broken(&key.0, key.1);
                        }
                    }
                }
                self.loop_block(body);
                self.scopes.pop();
            }
            StmtKind::Stop => {
                if self.loops == 0 {
                    // trace: decision.loops.forever
                    self.error(messages::stop_outside_loop(stmt.span));
                }
            }
            StmtKind::Finish => {
                // trace: decision.procedures.finish
                if let Some(ty) = self.returns.clone() {
                    self.error(messages::finish_in_returning_procedure(stmt.span, &ty.name()));
                }
            }
            StmtKind::Return(value) => match self.returns.clone() {
                None => self.error(messages::return_in_finishing_procedure(stmt.span, &self.procedure_name.clone())),
                Some(ty) => {
                    let got = self.expr(value, Some(&ty));
                    self.expect_type(value, &got, &ty);
                    if self.procedure_name == "start_running" {
                        self.check_known_bounds(value, &Rules::of(ty));
                    }
                }
            },
            StmtKind::Add { value, key, table, at } => self.add(value, key.as_ref(), table, at.as_ref()),
            StmtKind::Call { name, name_span, args } => {
                self.call(name, *name_span, args, stmt.span);
            }
        }
    }

    fn value(&mut self, value: &LetValue, rules: &Rules, name_span: Span) {
        match value {
            LetValue::Expr(expr) => {
                let ty = self.expr(expr, Some(&rules.ty));
                self.expect_type(expr, &ty, &rules.ty);
                self.check_known_bounds(expr, rules);
            }
            LetValue::Share(source, span) => {
                // trace: decision.memory.share
                let Some((source_rules, _, broken, _)) = self.variable(source, *span) else { return };
                if broken {
                    return;
                }
                if !matches!(source_rules.ty, Ty::Table { .. }) {
                    self.error(messages::share_on_non_table_not_designed(*span));
                } else if source_rules.ty != rules.ty && !rules.ty.is_unknown() {
                    self.error(messages::wrong_type(*span, &source_rules.ty.name(), &rules.ty.name()));
                }
            }
            LetValue::Block(items, _) => {
                let Ty::Table { key, item } = &rules.ty else {
                    self.error(messages::block_needs_table(name_span, &rules.ty.name()));
                    return;
                };
                for block_item in items {
                    match (block_item, key) {
                        (BlockItem::Item(expr), None) => {
                            let ty = self.expr(expr, Some(item));
                            self.expect_type(expr, &ty, item);
                        }
                        (BlockItem::Keyed(key_expr, expr), Some(key)) => {
                            let ty = self.expr(key_expr, Some(key));
                            self.expect_type(key_expr, &ty, key);
                            let ty = self.expr(expr, Some(item));
                            self.expect_type(expr, &ty, item);
                        }
                        (BlockItem::Item(expr), Some(_)) => self.error(messages::needs_key(expr.span)),
                        (BlockItem::Keyed(key_expr, _), None) => self.error(messages::no_keys_here(key_expr.span)),
                    }
                }
            }
        }
    }

    fn add(&mut self, value: &Expr, key: Option<&Expr>, table: &(String, Span), at: Option<&(IndexKind, Expr)>) {
        let Some((rules, declared, broken, loop_item)) = self.variable(&table.0, table.1) else { return };
        if broken {
            return;
        }
        if loop_item {
            self.error(messages::changing_loop_item_not_designed(table.1, &table.0));
            return;
        }
        if !rules.can_change {
            self.error(messages::cant_change(table.1, &table.0, declared));
            return;
        }
        let Ty::Table { key: key_ty, item } = &rules.ty else {
            self.error(messages::add_needs_table(table.1, &table.0, &rules.ty.name()));
            return;
        };
        match (key, key_ty) {
            (Some(key), Some(key_ty)) => {
                let ty = self.expr(key, Some(key_ty));
                self.expect_type(key, &ty, key_ty);
            }
            (None, Some(_)) => self.error(messages::needs_key(value.span)),
            (Some(key), None) => self.error(messages::no_keys_here(key.span)),
            (None, None) => {}
        }
        let ty = self.expr(value, Some(item));
        self.expect_type(value, &ty, item);
        if let Some((_, index)) = at {
            if key_ty.is_some() {
                self.error(messages::at_on_keyed_not_designed(index.span));
            }
            self.whole_number(index);
        }
    }

    fn call(&mut self, name: &str, name_span: Span, args: &[Arg], span: Span) -> Option<Ty> {
        let Some(facts) = self.facts.procedures.get(name).cloned() else {
            let names: Vec<String> = self.facts.procedures.keys().cloned().collect();
            if let Some(word) =
                name.strip_suffix("_of").filter(|w| ["sum", "length", "biggest", "smallest", "average"].contains(w))
            {
                // trace: decision.procedures.own_of (`sum_of` is the built-in `sum of`)
                self.error(messages::builtin_of_is_spaced(name_span, word));
            } else {
                self.error(messages::unknown_procedure(name_span, name, &names));
            }
            for arg in args {
                if let Arg::Value(expr) = arg {
                    self.expr(expr, None);
                }
            }
            return None;
        };
        if args.len() > facts.params.len() {
            self.error(messages::too_many_args(args[facts.params.len()].span(), name, facts.params.len()));
        }
        for (index, rules) in facts.params.iter().enumerate() {
            match args.get(index) {
                None => {
                    // trace: decision.procedures.defaults
                    if !self.has_default(name, index) {
                        self.error(messages::missing_arg(span, name, index + 1));
                    }
                }
                Some(Arg::Value(expr)) => {
                    if rules.share {
                        // trace: decision.procedures.share
                        let written = self.text(expr.span);
                        self.error(messages::call_needs_share(expr.span, &written));
                        continue;
                    }
                    let ty = self.expr(expr, Some(&rules.ty));
                    self.expect_type(expr, &ty, &rules.ty);
                    self.check_known_bounds(expr, rules);
                }
                Some(Arg::Share(source, share_span)) => {
                    if !rules.share {
                        self.error(messages::param_not_shared(*share_span, name));
                        continue;
                    }
                    if let Some((source_rules, declared, broken, _)) = self.variable(source, *share_span) {
                        if broken {
                            continue;
                        }
                        if !source_rules.can_change {
                            self.error(messages::cant_change(*share_span, source, declared));
                        } else if source_rules.ty != rules.ty && !rules.ty.is_unknown() {
                            self.error(messages::wrong_type(*share_span, &source_rules.ty.name(), &rules.ty.name()));
                        }
                    }
                }
            }
        }
        Some(facts.returns.clone().unwrap_or(Ty::Unknown)).filter(|_| facts.returns.is_some())
    }

    fn has_default(&self, name: &str, index: usize) -> bool {
        self.defaults.get(name).is_some_and(|defaults| defaults.get(index).copied().unwrap_or(false))
    }

    /// An index or a count: any whole number.
    fn whole_number(&mut self, expr: &Expr) {
        let ty = self.expr(expr, None);
        if !matches!(ty, Ty::Whole { .. } | Ty::WrittenWhole | Ty::Unknown) {
            self.error(messages::needs_whole_number(expr.span, &ty.name()));
        }
    }

    // ---- expressions ------------------------------------------------------------------------------------------

    /// Says so when a value's type isn't the one wanted.
    fn expect_type(&mut self, expr: &Expr, got: &Ty, want: &Ty) {
        if got.is_unknown() || want.is_unknown() || got == want {
            return;
        }
        if got.is_written() {
            // A written number that could not become the wanted type was already reported, or is the wrong kind.
            if !self.targets_has(expr) {
                self.error(messages::wrong_type(expr.span, &got.name(), &want.name()));
            }
            return;
        }
        self.error(messages::wrong_type(expr.span, &got.name(), &want.name()));
    }

    fn targets_has(&self, expr: &Expr) -> bool {
        self.facts.targets.contains_key(&expr.id)
    }

    /// trace: decision.types.bounds (a bound broken by a value known before running is a compile error)
    fn check_known_bounds(&mut self, expr: &Expr, rules: &Rules) {
        let Some(Ok(Known::Whole(n))) = self.known(expr) else { return };
        let below = rules.at_least.as_ref().is_some_and(|least| n < *least);
        let above = rules.at_most.as_ref().is_some_and(|most| n > *most);
        if below || above {
            self.error(messages::outside_bounds(expr.span, &n.to_string(), &rules.at_least, &rules.at_most, true));
        }
    }

    fn expr(&mut self, expr: &Expr, want: Option<&Ty>) -> Ty {
        let ty = self.expr_inner(expr, want);
        match want {
            Some(want) if ty.is_written() && want.is_number() && !want.is_written() => self.settle(expr, &ty, want),
            _ => ty,
        }
    }

    /// Makes a written number the type it is used as, checking before running that it fits when that can be known.
    /// trace: decision.numbers.range
    fn settle(&mut self, expr: &Expr, written: &Ty, want: &Ty) -> Ty {
        let fits_kind = matches!(
            (written, want),
            (Ty::WrittenWhole, Ty::Whole { .. } | Ty::Exact(_) | Ty::Fast(_))
                | (Ty::WrittenDecimal, Ty::Exact(_) | Ty::Fast(_))
                | (Ty::WrittenFraction, Ty::Fraction(_))
        );
        if !fits_kind {
            return written.clone();
        }
        match self.known(expr) {
            Some(Err(error)) => {
                self.error(error);
                return Ty::Unknown;
            }
            Some(Ok(known)) => {
                if let Err(error) = self.known_fits(expr, &known, want) {
                    self.error(error);
                    return Ty::Unknown;
                }
            }
            None => {}
        }
        self.facts.targets.insert(expr.id, want.clone());
        want.clone()
    }

    fn known_fits(&self, expr: &Expr, known: &Known, want: &Ty) -> Result<(), Diagnostic> {
        let written = self.text(expr.span);
        match (known, want) {
            (Known::Whole(n), Ty::Whole { size, never_negative }) => {
                if numbers::whole_fits(n, *size, *never_negative) {
                    Ok(())
                } else {
                    let (least, most) = numbers::whole_range(*size, *never_negative);
                    Err(messages::too_big_for(expr.span, &n.to_string(), &want.name(), &least, &most, *size, true))
                }
            }
            (Known::Whole(n), Ty::Exact(size)) => self.decimal_fits(expr, Exact::whole(n.clone()), *size),
            (Known::Decimal(value), Ty::Exact(size)) => self.decimal_fits(expr, value.clone(), *size),
            (Known::Whole(_) | Known::Decimal(_), Ty::Fast(size)) => {
                let value = match known {
                    Known::Whole(n) => n.to_string(),
                    Known::Decimal(value) => value.to_text(),
                    Known::Fraction(..) => unreachable!("a fraction is not a decimal"),
                };
                match Fast::parse(&value, *size) {
                    Ok((_, true)) => Ok(()),
                    Ok((_, false)) => {
                        // trace: decision.types.approx
                        Err(messages::fast_cant_hold(
                            expr.span,
                            &written,
                            &Fast::nearest_shown(&value, *size),
                            &want.name(),
                        ))
                    }
                    Err(_) => Err(messages::fast_too_big(expr.span, &written, &want.name())),
                }
            }
            (Known::Fraction(top, bottom), Ty::Fraction(size)) => {
                if numbers::fraction_fits(top, bottom, *size) {
                    Ok(())
                } else {
                    Err(messages::fraction_too_big(expr.span, &written, &want.name(), *size))
                }
            }
            _ => Ok(()),
        }
    }

    fn decimal_fits(&self, expr: &Expr, value: Exact, size: Size) -> Result<(), Diagnostic> {
        match value.fit(Some(size)) {
            Ok(_) => Ok(()),
            Err(trouble) => Err(messages::decimal_trouble(
                expr.span,
                trouble,
                &format!("{}_decimal", size.word()),
                Stage::BeforeRunning,
            )),
        }
    }

    /// The value of an expression made only of written numbers, worked out before running.
    fn known(&self, expr: &Expr) -> Option<Result<Known, Diagnostic>> {
        match &expr.kind {
            ExprKind::Whole(n) => Some(Ok(Known::Whole(n.clone()))),
            ExprKind::Decimal { digits, places, .. } => {
                Some(Ok(Known::Decimal(Exact { coefficient: digits.clone(), exponent: -(*places as i64) })))
            }
            ExprKind::Fraction(top, bottom) => Some(Ok(Known::Fraction(top.clone(), bottom.clone()))),
            ExprKind::Brackets(inner) => self.known(inner),
            ExprKind::Maths(op, left, right) => {
                let left = match self.known(left)? {
                    Ok(value) => value,
                    Err(error) => return Some(Err(error)),
                };
                let right = match self.known(right)? {
                    Ok(value) => value,
                    Err(error) => return Some(Err(error)),
                };
                Some(known_maths(*op, left, right, false).map_err(|trouble| self.known_trouble(expr, trouble)))
            }
            ExprKind::Round(inner) => {
                let inner = match &inner.kind {
                    ExprKind::Brackets(inner) => inner,
                    _ => inner,
                };
                match &inner.kind {
                    ExprKind::Maths(MathsOp::DividedBy, left, right) => {
                        let left = match self.known(left)? {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                        let right = match self.known(right)? {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                        // Rounding a written division with no type to round to happens only when it runs.
                        match (&left, &right) {
                            (Known::Whole(_), Known::Whole(_)) => Some(
                                known_maths(MathsOp::DividedBy, left, right, true)
                                    .map_err(|trouble| self.known_trouble(expr, trouble)),
                            ),
                            _ => None,
                        }
                    }
                    _ => self.known(inner),
                }
            }
            _ => None,
        }
    }

    fn known_trouble(&self, expr: &Expr, trouble: Trouble) -> Diagnostic {
        let written = self.text(expr.span);
        match trouble {
            Trouble::Halfway => messages::round_halfway_not_designed(expr.span, Stage::BeforeRunning),
            Trouble::DividedByZero => messages::divided_by_zero(expr.span, Stage::BeforeRunning),
            Trouble::TooManyDigits if matches!(expr.kind, ExprKind::Maths(MathsOp::DividedBy, ..)) => {
                messages::uneven_whole_division(expr.span, &written, Stage::BeforeRunning)
            }
            trouble => messages::decimal_trouble(expr.span, trouble, "decimal", Stage::BeforeRunning),
        }
    }

    fn expr_inner(&mut self, expr: &Expr, want: Option<&Ty>) -> Ty {
        match &expr.kind {
            ExprKind::Whole(_) => Ty::WrittenWhole,
            ExprKind::Decimal { .. } => Ty::WrittenDecimal,
            ExprKind::About { .. } => match want {
                Some(Ty::Fast(_)) => {
                    // trace: decision.types.approx
                    let want = want.cloned().unwrap_or(Ty::Unknown);
                    self.facts.targets.insert(expr.id, want.clone());
                    want
                }
                _ => {
                    self.error(messages::about_needs_fast(expr.span));
                    Ty::Unknown
                }
            },
            ExprKind::Fraction(_, bottom) => {
                if bottom.is_zero() {
                    self.error(messages::divided_by_zero(expr.span, Stage::BeforeRunning));
                    return Ty::Unknown;
                }
                Ty::WrittenFraction
            }
            ExprKind::Text(_) => Ty::Text,
            ExprKind::True | ExprKind::False => Ty::TrueOrFalse,
            ExprKind::Empty => match want {
                // trace: decision.values.empty
                Some(ty @ (Ty::Text | Ty::Table { .. })) => {
                    self.facts.targets.insert(expr.id, ty.clone());
                    ty.clone()
                }
                _ => {
                    self.error(messages::empty_needs_type(expr.span));
                    Ty::Unknown
                }
            },
            ExprKind::Variable(name) => match self.variable(name, expr.span) {
                Some((rules, ..)) => rules.ty,
                None => Ty::Unknown,
            },
            ExprKind::Brackets(inner) => self.expr(inner, want),
            ExprKind::Maths(op, left, right) => self.maths(expr, *op, left, right, want),
            ExprKind::Compare(op, left, right) => {
                // trace: decision.conditions.compare
                let ty = self.type_of_pair_left(left, right);
                let ordering = !matches!(op, CompareOp::Is | CompareOp::Isnt);
                if ordering && !ty.is_number() && !ty.is_unknown() {
                    self.error(messages::ordering_not_designed(expr.span, &ty.name()));
                }
                if !ordering && matches!(ty, Ty::Table { .. }) {
                    self.error(messages::comparing_tables_not_designed(expr.span));
                }
                Ty::TrueOrFalse
            }
            ExprKind::Logic(_, left, right) => {
                self.test(left);
                self.test(right);
                Ty::TrueOrFalse
            }
            ExprKind::Not(inner) => {
                self.test(inner);
                Ty::TrueOrFalse
            }
            ExprKind::Table(items) => self.table_literal(expr, items, want),
            ExprKind::KeyedTable(pairs) => match want {
                Some(Ty::Table { key: Some(key), item }) => {
                    for (k, v) in pairs {
                        let ty = self.expr(k, Some(key));
                        self.expect_type(k, &ty, key);
                        let ty = self.expr(v, Some(item));
                        self.expect_type(v, &ty, item);
                    }
                    want.cloned().unwrap_or(Ty::Unknown)
                }
                _ => {
                    self.error(messages::keyed_table_needs_type_not_designed(expr.span));
                    Ty::Unknown
                }
            },
            ExprKind::Of(word, target) => self.of(expr, *word, target),
            ExprKind::Index(kind, index, target) => {
                let _ = kind;
                self.whole_number(index);
                let ty = self.expr(target, None);
                match ty {
                    Ty::Table { key: None, item } => *item,
                    // trace: decision.text.letters
                    Ty::Text => Ty::Text,
                    Ty::Table { key: Some(_), .. } => {
                        self.error(messages::reading_named_key_not_designed(expr.span));
                        Ty::Unknown
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.error(messages::index_needs_table(target.span, &other.name()));
                        Ty::Unknown
                    }
                }
            }
            ExprKind::Round(inner) => {
                // What is rounded is worked out with no type of its own, then rounded to the type it goes into.
                let ty = self.expr(inner, None);
                let ty = match want {
                    Some(want) if ty.is_written() && want.is_number() && !want.is_written() => {
                        self.facts.targets.insert(expr.id, want.clone());
                        want.clone()
                    }
                    _ => ty,
                };
                if matches!(ty, Ty::Fast(_) | Ty::Fraction(_)) {
                    self.error(messages::round_this_not_designed(expr.span, &ty.name()));
                    return Ty::Unknown;
                }
                if !ty.is_number() && !ty.is_unknown() {
                    self.error(messages::needs_number(inner.span, &ty.name()));
                    return Ty::Unknown;
                }
                ty
            }
            ExprKind::Remainder(left, right) => {
                let ty = self.maths(expr, MathsOp::DividedBy, left, right, want);
                if !matches!(ty, Ty::Whole { .. } | Ty::WrittenWhole | Ty::Unknown) {
                    self.error(messages::remainder_needs_wholes_not_designed(expr.span, &ty.name()));
                    return Ty::Unknown;
                }
                ty
            }
            ExprKind::Call(name, args) => {
                // trace: decision.procedures.nested_calls
                match self.call(name, expr.span, args, expr.span) {
                    Some(ty) => ty,
                    None => {
                        if self.facts.procedures.get(name).is_some_and(|facts| facts.returns.is_none()) {
                            self.error(messages::gives_nothing_back(expr.span, name));
                        }
                        Ty::Unknown
                    }
                }
            }
            ExprKind::BareName(name) => {
                let variable = format!("@{}", name);
                if self.lookup(&variable).is_some() {
                    // trace: decision.names.at_mark
                    self.error(messages::variable_needs_at(expr.span, name));
                } else if let Some(facts) = self.facts.procedures.get(name) {
                    if facts.params.is_empty() {
                        self.error(messages::nested_call_needs_brackets(expr.span, &format!("{}()", name)));
                    } else {
                        self.error(messages::nested_call_needs_brackets(expr.span, &format!("{}(...)", name)));
                    }
                } else if parser_type_word(name) {
                    self.error(messages::type_is_not_a_value(expr.span, name));
                } else {
                    let names: Vec<String> = self.facts.procedures.keys().cloned().collect();
                    self.error(messages::unknown_name(expr.span, name, &names, &self.all_variable_names()));
                }
                Ty::Unknown
            }
        }
    }

    fn table_literal(&mut self, expr: &Expr, items: &[Expr], want: Option<&Ty>) -> Ty {
        match want {
            Some(Ty::Table { key: None, item }) => {
                for value in items {
                    let ty = self.expr(value, Some(item));
                    self.expect_type(value, &ty, item);
                }
                want.cloned().unwrap_or(Ty::Unknown)
            }
            Some(Ty::Table { key: Some(_), .. }) => {
                self.error(messages::needs_key(expr.span));
                Ty::Unknown
            }
            _ => {
                let types: Vec<Ty> = items.iter().map(|value| self.expr(value, None)).collect();
                let first = types[0].clone();
                if types.iter().all(|ty| *ty == first) {
                    Ty::Table { key: None, item: Box::new(first) }
                } else {
                    self.error(messages::table_mixes_types(expr.span));
                    Ty::Unknown
                }
            }
        }
    }

    fn of(&mut self, expr: &Expr, word: OfWord, target: &Expr) -> Ty {
        let ty = self.expr(target, None);
        // trace: decision.tables.of_family
        match (word, &ty) {
            (_, Ty::Unknown) => Ty::Unknown,
            // trace: decision.text.letters (a text's length counts its letters as seen)
            (OfWord::Length, Ty::Table { .. } | Ty::Text) => Ty::WrittenWhole,
            (_, Ty::Table { key: None, item }) if item.is_number() => {
                if matches!(**item, Ty::Fraction(_) | Ty::WrittenFraction) {
                    self.error(messages::fraction_maths_not_designed(expr.span));
                    return Ty::Unknown;
                }
                self.facts.of_types.insert(expr.id, (**item).clone());
                (**item).clone()
            }
            (_, Ty::Table { key: Some(_), .. }) => {
                self.error(messages::of_keyed_not_designed(expr.span));
                Ty::Unknown
            }
            (_, other) => {
                self.error(messages::of_needs_numbers(target.span, &other.name()));
                Ty::Unknown
            }
        }
    }

    fn maths(&mut self, expr: &Expr, op: MathsOp, left: &Expr, right: &Expr, want: Option<&Ty>) -> Ty {
        let _ = (op, want);
        let left_ty = self.expr(left, None);
        let right_ty = self.expr(right, None);
        if left_ty.is_unknown() || right_ty.is_unknown() {
            return Ty::Unknown;
        }
        if !left_ty.is_number() || !right_ty.is_number() {
            let bad = if !left_ty.is_number() { (&left_ty, left) } else { (&right_ty, right) };
            if *bad.0 == Ty::Text && op == MathsOp::Plus {
                self.error(messages::joining_text_not_designed(expr.span));
            } else {
                self.error(messages::needs_number(bad.1.span, &bad.0.name()));
            }
            return Ty::Unknown;
        }
        let ty = match (left_ty.is_written(), right_ty.is_written()) {
            (true, true) => match (&left_ty, &right_ty) {
                (Ty::WrittenFraction, _) | (_, Ty::WrittenFraction) => Ty::WrittenFraction,
                (Ty::WrittenDecimal, _) | (_, Ty::WrittenDecimal) => Ty::WrittenDecimal,
                _ => Ty::WrittenWhole,
            },
            (true, false) => self.settle(left, &left_ty, &right_ty),
            (false, true) => self.settle(right, &right_ty, &left_ty),
            (false, false) => {
                if left_ty != right_ty {
                    self.error(messages::mixing_types_not_designed(expr.span, &left_ty.name(), &right_ty.name()));
                    return Ty::Unknown;
                }
                left_ty.clone()
            }
        };
        if matches!(ty, Ty::Fraction(_) | Ty::WrittenFraction) {
            self.error(messages::fraction_maths_not_designed(expr.span));
            return Ty::Unknown;
        }
        if ty.is_unknown() {
            return ty;
        }
        // A written side that couldn't become the other side's type.
        if left_ty.is_written() != right_ty.is_written() {
            let (written, typed) = if left_ty.is_written() { (left, &right_ty) } else { (right, &left_ty) };
            if !self.targets_has(written) {
                self.error(messages::wrong_type(
                    written.span,
                    &if left_ty.is_written() { left_ty.name() } else { right_ty.name() },
                    &typed.name(),
                ));
                return Ty::Unknown;
            }
        }
        ty
    }

    /// The type both sides of a comparison share, a written side taking the other side's type.
    fn type_of_pair_left(&mut self, left: &Expr, right: &Expr) -> Ty {
        let left_ty = self.expr(left, None);
        let right_ty = self.expr(right, None);
        if left_ty.is_unknown() || right_ty.is_unknown() {
            return Ty::Unknown;
        }
        match (left_ty.is_written(), right_ty.is_written()) {
            (true, false) => {
                let settled = self.settle(left, &left_ty, &right_ty);
                if settled.is_written() {
                    self.error(messages::wrong_type(left.span, &left_ty.name(), &right_ty.name()));
                    return Ty::Unknown;
                }
                right_ty
            }
            (false, true) => {
                let settled = self.settle(right, &right_ty, &left_ty);
                if settled.is_written() {
                    self.error(messages::wrong_type(right.span, &right_ty.name(), &left_ty.name()));
                    return Ty::Unknown;
                }
                left_ty
            }
            (true, true) => left_ty,
            (false, false) => {
                if left_ty != right_ty {
                    self.error(messages::comparing_types_not_designed(
                        left.span.to(right.span),
                        &left_ty.name(),
                        &right_ty.name(),
                    ));
                    return Ty::Unknown;
                }
                left_ty
            }
        }
    }
}

fn parser_type_word(name: &str) -> bool {
    name == "text" || name == "true_or_false" || number_type(name).is_some()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Let,
    Param,
    Returns,
    Item,
}

/// Maths on written numbers, exactly. A whole division that doesn't come out even is refused unless rounded.
/// trace: decision.maths.whole_divide
fn known_maths(op: MathsOp, left: Known, right: Known, round: bool) -> Result<Known, Trouble> {
    match (left, right) {
        (Known::Whole(a), Known::Whole(b)) => match op {
            MathsOp::Plus => Ok(Known::Whole(a + b)),
            MathsOp::Minus => Ok(Known::Whole(a - b)),
            MathsOp::Times => Ok(Known::Whole(a * b)),
            MathsOp::DividedBy => {
                if b.is_zero() {
                    return Err(Trouble::DividedByZero);
                }
                if round {
                    return numbers::round_nearest(&a, &b).map(Known::Whole);
                }
                if (&a % &b).is_zero() { Ok(Known::Whole(a / b)) } else { Err(Trouble::TooManyDigits) }
            }
        },
        (left, right) => {
            let as_exact = |known: Known| match known {
                Known::Whole(n) => Exact::whole(n),
                Known::Decimal(value) => value,
                Known::Fraction(..) => Exact::whole(BigInt::one()),
            };
            let (a, b) = (as_exact(left), as_exact(right));
            let value = match op {
                MathsOp::Plus => a.plus(&b, None),
                MathsOp::Minus => a.minus(&b, None),
                MathsOp::Times => a.times(&b, None),
                MathsOp::DividedBy => a.divided_by(&b, None),
            }?;
            Ok(Known::Decimal(value))
        }
    }
}
