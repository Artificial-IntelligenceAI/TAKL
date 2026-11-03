//! Checks a program's names and types before it runs, and works out maths made only of written numbers.
//!
//! trace: decision.errors.all
//! trace: decision.numbers.range
//!
//! Every separate mistake is reported; one caused by an earlier one is not, since its types come out `Broken`.
//! "Known before running" means made only of written numbers: such maths is worked out exactly here, and a
//! problem in it is a mistake before running where a type is waiting for it, or a stop while running otherwise.

use crate::checked::*;
use crate::failure::Failure;
use crate::messages;
use crate::numbers::{self, Decimal, Problem, Size, WholeType, fast};
use crate::parser::is_grammar_word;
use crate::source::{Source, Span};
use crate::syntax::*;
use crate::types::{MODIFIER_NAMES, TYPE_NAMES, Type, near_match};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::collections::HashMap;

/// How a variable came to be, for the errors that point back at it.
#[derive(Clone)]
struct Made {
    id: VariableId,
    name_span: Span,
    can_change: bool,
    /// Set for a `for each` variable, which isn't designed to change.
    loop_item: bool,
}

struct Signature {
    name: String,
    params: Vec<(String, Type, bool, bool)>,
    returns: Option<Type>,
}

pub struct Checker<'s> {
    _source: std::marker::PhantomData<&'s Source>,
    failures: Vec<Failure>,
    signatures: Vec<Signature>,
    by_name: HashMap<String, ProcedureId>,
    // The procedure being checked.
    variables: Vec<Variable>,
    scopes: Vec<Vec<(String, Made)>>,
    ended: Vec<(String, Made)>,
    loops: usize,
    returns: Option<Type>,
    procedure_name: String,
}

fn whole_type(size: Size, never_negative: bool) -> Type {
    Type::Whole(WholeType { size, never_negative })
}

impl<'s> Checker<'s> {
    fn fail(&mut self, failure: Failure) {
        self.failures.push(failure);
    }

    // ---- types as written ----

    /// The type a written type names, and its bounds; mistakes in it are reported.
    fn resolve_type(&mut self, written: &WrittenType, place: TypePlace) -> (Type, Bounds) {
        let mut ty = match &written.base {
            Base::Named(word, span) => self.named_type(word, *span),
            Base::Table { item, key } => {
                if let Some(key) = key {
                    let (key_type, _) = self.resolve_type(key, TypePlace::Item);
                    if key_type != Type::Text && !key_type.is_broken() {
                        self.fail(messages::not_designed_yet(key.span, "a table with keys that aren't text"));
                    }
                }
                let (item_type, _) = self.resolve_type(item, TypePlace::Item);
                Type::Table { keyed: key.is_some(), item: Box::new(item_type) }
            }
        };
        let mut bounds = Bounds::default();
        let mut first_bound: Option<Span> = None;
        let mut crossed = false;
        for modifier in &written.modifiers {
            match &modifier.kind {
                ModifierKind::NeverNegative => match &mut ty {
                    Type::Whole(whole) => whole.never_negative = true,
                    Type::Broken => {}
                    _ => self.fail(messages::not_designed_yet(
                        modifier.span,
                        format!("`never_negative` on a {}", ty.name()),
                    )),
                },
                ModifierKind::FastButInexact => match &mut ty {
                    Type::Decimal { fast, .. } => *fast = true,
                    Type::Broken => {}
                    _ => self.fail(messages::not_designed_yet(
                        modifier.span,
                        format!("`fast_but_inexact` on a {}", ty.name()),
                    )),
                },
                ModifierKind::CantChange => {
                    if place != TypePlace::Let {
                        self.fail(messages::not_designed_yet(modifier.span, "`can't_change` here"));
                    }
                }
                ModifierKind::Share => {
                    if place != TypePlace::Param {
                        self.fail(messages::not_designed_yet(modifier.span, "`share` here"));
                    } else if !matches!(ty, Type::Table { .. } | Type::Broken) {
                        self.fail(messages::not_designed_yet(modifier.span, "sharing something that isn't a table"));
                    }
                }
                ModifierKind::AtLeast(value) | ModifierKind::AtMost(value) => {
                    let least = matches!(modifier.kind, ModifierKind::AtLeast(_));
                    match &ty {
                        Type::Whole(whole) if place != TypePlace::Item && place != TypePlace::Returns => {
                            let in_range = value.to_i128().filter(|v| *v >= whole.smallest() && *v <= whole.biggest());
                            match in_range {
                                None => self.fail(messages::bound_outside_type(modifier.span, &whole.name())),
                                Some(v) => {
                                    if least {
                                        bounds.least = Some(v)
                                    } else {
                                        bounds.most = Some(v)
                                    }
                                    if let (Some(low), Some(high)) = (bounds.least, bounds.most)
                                        && low > high
                                    {
                                        let start = first_bound.unwrap_or(modifier.span);
                                        self.fail(messages::bounds_crossed(start.to(modifier.span)));
                                        crossed = true;
                                    }
                                }
                            }
                            first_bound.get_or_insert(modifier.span);
                        }
                        Type::Whole(_) => self.fail(messages::not_designed_yet(
                            modifier.span,
                            "bounds here, on a table's items or a procedure's return",
                        )),
                        Type::Decimal { .. } | Type::Fraction(_) => {
                            self.fail(messages::not_designed_yet(modifier.span, "bounds on decimals and fractions"))
                        }
                        Type::Broken => {}
                        _ => self.fail(messages::not_designed_yet(modifier.span, format!("bounds on a {}", ty.name()))),
                    }
                }
                ModifierKind::BrokenBound => {}
                ModifierKind::Unknown(word) => match word.as_str() {
                    // trace: decision.types.fast_but_inexact
                    "fast" => {
                        self.fail(messages::fast_is_now(modifier.span));
                        if let Type::Decimal { fast, .. } = &mut ty {
                            *fast = true;
                        }
                    }
                    "normal" => self.fail(messages::normal_modifier(modifier.span)),
                    _ => match near_match(
                        &word.replace('\'', ""),
                        MODIFIER_NAMES.iter().map(|name| name.trim_matches('\0')),
                    ) {
                        Some(near) => self.fail(messages::unknown_modifier_near(modifier.span, word, near)),
                        None => match near_match(word, MODIFIER_NAMES.iter().copied()) {
                            Some(near) => self.fail(messages::unknown_modifier_near(modifier.span, word, near)),
                            None => self.fail(messages::not_designed_yet(
                                modifier.span,
                                format!("a modifier called `{}`", word),
                            )),
                        },
                    },
                },
            }
        }
        if crossed {
            // Already reported; no value is checked against bounds that no number fits.
            bounds = Bounds::default();
        }
        (ty, bounds)
    }

    // trace: decision.types.whole
    fn named_type(&mut self, word: &str, span: Span) -> Type {
        if let Some(ty) = Type::named(word) {
            return ty;
        }
        if word.is_empty() {
            return Type::Broken;
        }
        match word {
            "int" | "long" => self.fail(messages::no_int(span, word)),
            "float" => self.fail(messages::no_float(span, word)),
            "bool" => self.fail(messages::bool_is_true_or_false(span)),
            "u8" | "u16" | "u32" | "u64" | "uint8" | "uint16" | "uint32" | "uint64" => {
                let bits: u32 = word.trim_start_matches("uint").trim_start_matches('u').parse().unwrap_or(32);
                let size = match bits {
                    8 => Size::Tiny,
                    16 => Size::Small,
                    32 => Size::Medium,
                    _ => Size::Big,
                };
                let fix = format!("{}_whole, never_negative", size.word());
                self.fail(messages::bits_name(span, &fix, &format!("a uint{}", bits)));
            }
            "string" => self.fail(messages::string_is_text(span)),
            "i8" | "i16" | "i32" | "i64" | "int8" | "int16" | "int32" | "int64" => {
                let bits: u32 = word.trim_start_matches("int").trim_start_matches('i').parse().unwrap_or(32);
                let size = match bits {
                    8 => Size::Tiny,
                    16 => Size::Small,
                    32 => Size::Medium,
                    _ => Size::Big,
                };
                self.fail(messages::bits_name(span, &format!("{}_whole", size.word()), &format!("an int{}", bits)));
            }
            _ => match near_match(word, TYPE_NAMES.iter().copied()) {
                Some(near) => self.fail(messages::unknown_type_near(span, word, near)),
                None => self.fail(messages::not_designed_yet(span, format!("a type called `{}`", word))),
            },
        }
        Type::Broken
    }

    // ---- variables ----

    fn lookup(&self, name: &str) -> Option<&Made> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|(found, _)| found == name)
            .map(|(_, made)| made)
    }

    fn visible_names(&self) -> Vec<String> {
        self.scopes.iter().flat_map(|scope| scope.iter().map(|(name, _)| name.clone())).collect()
    }

    fn declare(&mut self, name: &str, name_span: Span, ty: Type, bounds: Bounds, can_change: bool) -> VariableId {
        if let Some(first) = self.lookup(name).cloned() {
            // trace: decision.lets.change
            self.fail(
                messages::not_designed_yet(
                    name_span,
                    format!("making `@{}` again while the first one is still there", name),
                )
                .because(format!("`@{}` is first made here", name), first.name_span),
            );
            return first.id;
        }
        let id = self.variables.len();
        self.variables.push(Variable { name: name.to_string(), ty, bounds, param_site: None });
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), Made { id, name_span, can_change, loop_item: false }));
        }
        id
    }

    fn enter(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn leave(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            self.ended.extend(scope);
        }
    }

    /// A variable used in an expression or as the target of a change.
    fn variable_use(&mut self, name: &str, span: Span) -> Option<Made> {
        if let Some(made) = self.lookup(name) {
            return Some(made.clone());
        }
        if let Some((_, made)) = self.ended.iter().rev().find(|(found, _)| found == name) {
            let failure = messages::not_designed_yet(
                span,
                format!("whether `@{}` can be used after the block it was made in ends", name),
            )
            .because(format!("`@{}` is made inside a block here", name), made.name_span);
            self.fail(failure);
            return None;
        }
        let names = self.visible_names();
        match near_match(name, names.iter().map(String::as_str)) {
            Some(near) => {
                let failure = messages::unknown_variable_near(span, name, near);
                self.fail(failure)
            }
            None => self.fail(messages::unknown_variable(span, name)),
        }
        None
    }

    // ---- expressions ----

    fn broken(span: Span) -> CheckedExpression {
        CheckedExpression { kind: CheckedKind::Constant(Constant::TrueOrFalse(false)), ty: Type::Broken, span }
    }

    /// Every variable read inside an expression, each once.
    fn variables_in(expression: &CheckedExpression, found: &mut Vec<VariableId>) {
        let add = |id: VariableId, found: &mut Vec<VariableId>| {
            if !found.contains(&id) {
                found.push(id);
            }
        };
        match &expression.kind {
            CheckedKind::Variable(id) | CheckedKind::Share(id) => add(*id, found),
            CheckedKind::Maths { left, right, .. }
            | CheckedKind::Compare { left, right, .. }
            | CheckedKind::Logic { left, right, .. } => {
                Self::variables_in(left, found);
                Self::variables_in(right, found);
            }
            CheckedKind::Remainder { top, bottom, .. } | CheckedKind::RoundedDivide { top, bottom, .. } => {
                Self::variables_in(top, found);
                Self::variables_in(bottom, found);
            }
            CheckedKind::Not(inner) | CheckedKind::Resize(inner) => Self::variables_in(inner, found),
            CheckedKind::Family { operand, .. } => Self::variables_in(operand, found),
            CheckedKind::Index { index, table, .. } => {
                Self::variables_in(index, found);
                Self::variables_in(table, found);
            }
            CheckedKind::Call(call) => {
                for argument in &call.arguments {
                    match argument {
                        CheckedArgument::Value(value) => Self::variables_in(value, found),
                        CheckedArgument::Share(id) => add(*id, found),
                        CheckedArgument::Default => {}
                    }
                }
            }
            CheckedKind::Table(items) => items.iter().for_each(|item| Self::variables_in(item, found)),
            CheckedKind::KeyedTable(pairs) => pairs.iter().for_each(|(key, value)| {
                Self::variables_in(key, found);
                Self::variables_in(value, found);
            }),
            _ => {}
        }
    }

    // trace: decision.errors.layout
    /// The variables inside a maths step, for `because:`: every one, nested in brackets, `round`, `of`, a position
    /// or a call's arguments, each once. While running they are told oldest set first. A true_or_false has no way
    /// to be shown yet, so it isn't told.
    fn because_of(&self, operands: &[&CheckedExpression]) -> Vec<VariableId> {
        let mut found = Vec::new();
        for operand in operands {
            Self::variables_in(operand, &mut found);
        }
        found.retain(|id| !matches!(self.variables[*id].ty, Type::TrueOrFalse | Type::Broken));
        found
    }

    /// Checks an expression; with `expected`, it must be that type, and written numbers are made into it.
    fn expression(&mut self, expression: &Expression, expected: Option<&Type>) -> CheckedExpression {
        let span = expression.span;
        if let (Some(expected), ExpressionKind::About { whole, after_point }) = (expected, &expression.kind) {
            return self.about(whole, after_point.as_deref(), expected, span);
        }
        if let (Some(expected @ Type::Table { keyed: true, item }), ExpressionKind::KeyedTable(pairs)) =
            (expected, &expression.kind)
        {
            let mut checked = Vec::new();
            for (key, value) in pairs {
                let key = self.expression(key, Some(&Type::Text));
                let value = self.expression(value, Some(item));
                checked.push((key, value));
            }
            return CheckedExpression { kind: CheckedKind::KeyedTable(checked), ty: expected.clone(), span };
        }
        if let (Some(expected), ExpressionKind::Empty) = (expected, &expression.kind) {
            return match expected {
                Type::Text | Type::Table { .. } => {
                    CheckedExpression { kind: CheckedKind::Empty, ty: expected.clone(), span }
                }
                Type::Broken => Self::broken(span),
                _ => {
                    self.fail(messages::empty_no_type(span));
                    Self::broken(span)
                }
            };
        }
        let checked = self.infer(expression);
        match expected {
            Some(expected) => self.made_into(checked, expected),
            None => checked,
        }
    }

    // trace: decision.types.approx
    fn about(&mut self, whole: &str, after_point: Option<&str>, expected: &Type, span: Span) -> CheckedExpression {
        let written = match after_point {
            Some(digits) => format!("{}.{}", whole, digits),
            None => whole.to_string(),
        };
        match expected {
            Type::Decimal { size, fast: true } => match fast::written(*size, &written) {
                fast::Written::Exact(bits) | fast::Written::Nearest(bits) => {
                    CheckedExpression { kind: CheckedKind::Constant(Constant::Fast(bits)), ty: expected.clone(), span }
                }
                fast::Written::TooBig => {
                    self.fail(messages::fast_too_big_written(span, &format!("~{}", written), &expected.name()));
                    Self::broken(span)
                }
            },
            Type::Broken => Self::broken(span),
            _ => {
                self.fail(messages::about_on_exact(span));
                Self::broken(span)
            }
        }
    }

    /// Makes a checked value into the type a place needs.
    fn made_into(&mut self, checked: CheckedExpression, expected: &Type) -> CheckedExpression {
        if checked.ty.is_broken() || expected.is_broken() {
            return CheckedExpression { ty: Type::Broken, ..checked };
        }
        let span = checked.span;
        if &checked.ty == expected {
            return checked;
        }
        match &checked.kind {
            CheckedKind::Constant(constant) if checked.ty.is_written() => {
                match self.constant_into(constant, expected, span) {
                    Some(made) => CheckedExpression { kind: CheckedKind::Constant(made), ty: expected.clone(), span },
                    None => Self::broken(span),
                }
            }
            CheckedKind::WrittenRound { top, bottom, .. } if matches!(expected, Type::Decimal { fast: false, .. }) => {
                let Type::Decimal { size, .. } = expected else { return Self::broken(span) };
                match written_decimal(top).divided_by_rounded(&written_decimal(bottom), Some(size.digits())) {
                    Ok(value) => CheckedExpression {
                        kind: CheckedKind::Constant(Constant::Decimal(value)),
                        ty: expected.clone(),
                        span,
                    },
                    Err(problem) => {
                        self.fail(messages::problem(problem, span, expected));
                        Self::broken(span)
                    }
                }
            }
            CheckedKind::WrittenRound { problem, .. } | CheckedKind::Fail(problem) if checked.ty.is_written() => {
                let failure = messages::problem(problem.clone(), span, expected);
                self.fail(failure);
                Self::broken(span)
            }
            CheckedKind::Table(items) if matches!(checked.ty, Type::Table { keyed: false, .. }) => {
                if let Type::Table { keyed: false, item } = expected {
                    let items = items.clone().into_iter().map(|value| self.made_into(value, item)).collect();
                    return CheckedExpression { kind: CheckedKind::Table(items), ty: expected.clone(), span };
                }
                self.fail(messages::wrong_type(span, &checked.ty.with_article(), &expected.name()));
                Self::broken(span)
            }
            CheckedKind::Resize(_) | CheckedKind::Family { .. } if checked.ty == Type::WrittenWhole => {
                if matches!(expected, Type::Whole(_)) {
                    return CheckedExpression {
                        kind: CheckedKind::Resize(Box::new(checked)),
                        ty: expected.clone(),
                        span,
                    };
                }
                self.fail(messages::wrong_type(span, &checked.ty.with_article(), &expected.name()));
                Self::broken(span)
            }
            _ => {
                self.fail(messages::wrong_type(span, &checked.ty.with_article(), &expected.name()));
                Self::broken(span)
            }
        }
    }

    /// A written value made into a type, or a mistake before running if it doesn't fit.
    fn constant_into(&mut self, constant: &Constant, expected: &Type, span: Span) -> Option<Constant> {
        let wrong = |checker: &mut Self, is: &str| {
            checker.fail(messages::wrong_type(span, is, &expected.name()));
            None
        };
        match (constant, expected) {
            (Constant::WrittenWhole(value), Type::Whole(whole)) => match whole.check_big(value) {
                Ok(v) => Some(Constant::Whole(v)),
                Err(problem) => {
                    self.fail(messages::problem(problem, span, expected));
                    None
                }
            },
            (Constant::WrittenWhole(value), Type::Decimal { .. }) => {
                self.decimal_into(&Decimal::from_whole(value.clone()), expected, span)
            }
            (Constant::WrittenDecimal(value), Type::Decimal { .. }) => self.decimal_into(value, expected, span),
            (Constant::WrittenFraction(top, bottom), Type::Fraction(size)) => {
                let whole = WholeType { size: *size, never_negative: false };
                if whole.check_big(top).is_err() || whole.check_big(bottom).is_err() {
                    let shown = format!("{} over {}", top, bottom);
                    self.fail(messages::fraction_too_big(span, &shown, &expected.name(), &whole.name()));
                    return None;
                }
                Some(Constant::Fraction(top.clone(), bottom.clone()))
            }
            (Constant::WrittenWhole(_), _) => wrong(self, "a whole number"),
            (Constant::WrittenDecimal(_), _) => wrong(self, "a decimal"),
            (Constant::WrittenFraction(..), _) => wrong(self, "a fraction"),
            _ => wrong(self, "a value"),
        }
    }

    // trace: decision.types.decimals
    fn decimal_into(&mut self, value: &Decimal, expected: &Type, span: Span) -> Option<Constant> {
        let Type::Decimal { size, fast } = expected else { return None };
        if !fast {
            return match value.clone().fit(Some(size.digits())) {
                Ok(fitted) => Some(Constant::Decimal(fitted)),
                Err(problem) => {
                    self.fail(messages::problem(problem, span, expected));
                    None
                }
            };
        }
        let written = value.show();
        match fast::written(*size, &written) {
            fast::Written::Exact(bits) => Some(Constant::Fast(bits)),
            fast::Written::Nearest(bits) => {
                let nearest = fast::nearest_text(*size, bits);
                self.fail(messages::cannot_hold_exactly(span, &expected.name(), &written, &nearest));
                None
            }
            fast::Written::TooBig => {
                self.fail(messages::fast_too_big_written(span, &written, &expected.name()));
                None
            }
        }
    }

    fn infer(&mut self, expression: &Expression) -> CheckedExpression {
        let span = expression.span;
        let constant =
            |constant: Constant, ty: Type| CheckedExpression { kind: CheckedKind::Constant(constant), ty, span };
        match &expression.kind {
            ExpressionKind::Number { whole, after_point: None } => {
                constant(Constant::WrittenWhole(whole.parse().unwrap_or_default()), Type::WrittenWhole)
            }
            ExpressionKind::Number { whole, after_point: Some(digits) } => {
                constant(Constant::WrittenDecimal(Decimal::written(whole, Some(digits))), Type::WrittenDecimal)
            }
            ExpressionKind::About { .. } => {
                self.fail(messages::about_on_exact(span));
                Self::broken(span)
            }
            ExpressionKind::Text(text) => constant(Constant::Text(text.clone()), Type::Text),
            ExpressionKind::True => constant(Constant::TrueOrFalse(true), Type::TrueOrFalse),
            ExpressionKind::False => constant(Constant::TrueOrFalse(false), Type::TrueOrFalse),
            ExpressionKind::Empty => CheckedExpression { kind: CheckedKind::Empty, ty: Type::WrittenEmpty, span },
            ExpressionKind::Table(items) => {
                let checked: Vec<CheckedExpression> = items.iter().map(|item| self.infer(item)).collect();
                let Some(first) = checked.first() else {
                    return CheckedExpression { kind: CheckedKind::Empty, ty: Type::WrittenEmpty, span };
                };
                // A table takes its items' type from the first typed one; written numbers follow it.
                let item_type = checked
                    .iter()
                    .find(|item| !item.ty.is_written())
                    .map(|item| item.ty.clone())
                    .unwrap_or(first.ty.clone());
                let items: Vec<CheckedExpression> = if item_type.is_written() {
                    let kinds: Vec<Type> = checked.iter().map(|item| item.ty.clone()).collect();
                    if kinds.iter().any(|kind| *kind != item_type) {
                        let wrong =
                            checked.iter().find(|item| item.ty != item_type).map(|item| item.span).unwrap_or(span);
                        self.fail(messages::wrong_type(
                            wrong,
                            &checked
                                .iter()
                                .find(|item| item.ty != item_type)
                                .map(|item| item.ty.with_article())
                                .unwrap_or_default(),
                            &item_type.name(),
                        ));
                        return Self::broken(span);
                    }
                    checked
                } else {
                    checked.into_iter().map(|item| self.made_into(item, &item_type)).collect()
                };
                CheckedExpression {
                    kind: CheckedKind::Table(items),
                    ty: Type::Table { keyed: false, item: Box::new(item_type) },
                    span,
                }
            }
            ExpressionKind::KeyedTable(_) => {
                self.fail(messages::not_designed_yet(span, "a table with named keys where its type isn't written"));
                Self::broken(span)
            }
            ExpressionKind::Variable(name) => match self.variable_use(name, span) {
                Some(made) => {
                    let ty = self.variables[made.id].ty.clone();
                    CheckedExpression { kind: CheckedKind::Variable(made.id), ty, span }
                }
                None => Self::broken(span),
            },
            ExpressionKind::Bare(name) => {
                if self.lookup(name).is_some() {
                    self.fail(messages::needs_at(span, name));
                } else if self.by_name.contains_key(name) {
                    self.fail(messages::call_needs_brackets(span, format!("{}()", name)));
                } else {
                    self.fail(messages::bare_unknown(span, name));
                }
                Self::broken(span)
            }
            ExpressionKind::Call { name, arguments, .. } => {
                // A call inside a line is pointed at as a whole, `g(1)`; one on its own line, at its name.
                let Some(call) = self.call(name, span, arguments, span) else { return Self::broken(span) };
                match self.signatures[call.procedure].returns.clone() {
                    Some(returns) => CheckedExpression { kind: CheckedKind::Call(call), ty: returns, span },
                    None => {
                        let failure = messages::gives_nothing_back(span, name);
                        self.fail(failure);
                        Self::broken(span)
                    }
                }
            }
            ExpressionKind::Maths { operator, left, right } => self.maths(*operator, left, right, span),
            ExpressionKind::Over { top, bottom } => {
                let read = |part: &Expression| match &part.kind {
                    ExpressionKind::Number { whole, .. } => whole.parse::<BigInt>().unwrap_or_default(),
                    _ => BigInt::zero(),
                };
                constant(Constant::WrittenFraction(read(top), read(bottom)), Type::WrittenFraction)
            }
            ExpressionKind::Compare { operator, left, right } => self.compare(*operator, left, right, span),
            ExpressionKind::Logic { operator, left, right } => {
                let left = self.test(left);
                let right = self.test(right);
                if left.ty.is_broken() || right.ty.is_broken() {
                    return Self::broken(span);
                }
                if let (
                    CheckedKind::Constant(Constant::TrueOrFalse(a)),
                    CheckedKind::Constant(Constant::TrueOrFalse(b)),
                ) = (&left.kind, &right.kind)
                {
                    let value = match operator {
                        Logic::And | Logic::AndAlso => *a && *b,
                        Logic::Or | Logic::OrAlso => *a || *b,
                    };
                    return constant(Constant::TrueOrFalse(value), Type::TrueOrFalse);
                }
                CheckedExpression {
                    kind: CheckedKind::Logic { operator: *operator, left: Box::new(left), right: Box::new(right) },
                    ty: Type::TrueOrFalse,
                    span,
                }
            }
            ExpressionKind::Not(inner) => {
                let inner = self.test(inner);
                if inner.ty.is_broken() {
                    return Self::broken(span);
                }
                if let CheckedKind::Constant(Constant::TrueOrFalse(value)) = inner.kind {
                    return constant(Constant::TrueOrFalse(!value), Type::TrueOrFalse);
                }
                CheckedExpression { kind: CheckedKind::Not(Box::new(inner)), ty: Type::TrueOrFalse, span }
            }
            ExpressionKind::Family { family, operand } => self.family(*family, operand, span),
            ExpressionKind::Remainder(division) => self.division_word(division, span, true),
            ExpressionKind::Round(operand) => {
                let inner = operand.unbracketed();
                if matches!(inner.kind, ExpressionKind::Maths { operator: Maths::DividedBy, .. }) {
                    if let ExpressionKind::Maths { left, .. } = &inner.kind {
                        let peek = self.quiet_type(left);
                        if let Type::Decimal { fast: true, .. } = peek {
                            self.fail(messages::not_designed_yet(span, format!("rounding a {}", peek.name())));
                            return Self::broken(span);
                        }
                    }
                    // `round`'s brackets are its own, so an error inside them points at what they hold.
                    let mut checked = self.division_word(inner, inner.span, false);
                    if !matches!(checked.kind, CheckedKind::Constant(_)) {
                        checked.span = inner.span;
                    }
                    return checked;
                }
                let checked = self.infer(inner);
                if let Type::Decimal { fast: true, .. } = checked.ty {
                    self.fail(messages::not_designed_yet(span, format!("rounding a {}", checked.ty.name())));
                    return Self::broken(span);
                }
                checked
            }
            ExpressionKind::Index { counting, index, table } => {
                let table = self.infer(table);
                let index_checked = self.infer(index);
                if table.ty.is_broken() || index_checked.ty.is_broken() {
                    return Self::broken(span);
                }
                let index_checked = self.whole_index(index_checked);
                let Some(index_checked) = index_checked else { return Self::broken(span) };
                let item = match &table.ty {
                    Type::Table { keyed: false, item } => (**item).clone(),
                    Type::Text => Type::Text,
                    Type::Table { keyed: true, .. } => {
                        self.fail(messages::not_designed_yet(span, "how to read one item of a table with named keys"));
                        return Self::broken(span);
                    }
                    other => {
                        self.fail(messages::not_designed_yet(
                            table.span,
                            format!("`{}` of a {}", counting.word(), other.name()),
                        ));
                        return Self::broken(span);
                    }
                };
                if item.is_written() {
                    self.fail(messages::not_designed_yet(span, "an item of a table whose items have no type"));
                    return Self::broken(span);
                }
                CheckedExpression {
                    kind: CheckedKind::Index {
                        counting: *counting,
                        index: Box::new(index_checked),
                        table: Box::new(table),
                    },
                    ty: item,
                    span,
                }
            }
            ExpressionKind::Share(inner) => {
                let checked = self.infer(inner);
                if checked.ty.is_broken() {
                    return Self::broken(span);
                }
                match (&checked.kind, &checked.ty) {
                    (CheckedKind::Variable(id), Type::Table { .. }) => {
                        CheckedExpression { kind: CheckedKind::Share(*id), ty: checked.ty.clone(), span }
                    }
                    (_, Type::Table { .. }) => {
                        self.fail(messages::not_designed_yet(span, "sharing something that isn't a variable"));
                        Self::broken(span)
                    }
                    _ => {
                        self.fail(messages::not_designed_yet(span, "sharing something that isn't a table"));
                        Self::broken(span)
                    }
                }
            }
            ExpressionKind::Bracketed(inner) => {
                let mut checked = self.infer(inner);
                // A step that stops while running points at the maths inside the brackets; a written value made into
                // a type points at the brackets too.
                let runs = matches!(
                    checked.kind,
                    CheckedKind::Maths { .. } | CheckedKind::Remainder { .. } | CheckedKind::RoundedDivide { .. }
                );
                if !runs {
                    checked.span = span;
                }
                checked
            }
            ExpressionKind::Broken => Self::broken(span),
        }
    }

    /// An index or a count: any whole number.
    fn whole_index(&mut self, checked: CheckedExpression) -> Option<CheckedExpression> {
        match &checked.ty {
            Type::Whole(_) => Some(checked),
            Type::WrittenWhole => {
                Some(self.made_into(checked, &whole_type(Size::Big, false))).filter(|made| !made.ty.is_broken())
            }
            other => {
                self.fail(messages::wrong_type(checked.span, &other.with_article(), "whole number"));
                None
            }
        }
    }

    /// A test, which is true or false.
    fn test(&mut self, expression: &Expression) -> CheckedExpression {
        let checked = self.infer(expression);
        if checked.ty.is_broken() || checked.ty == Type::TrueOrFalse {
            return checked;
        }
        let failure = messages::needs_test(checked.span, &checked.ty.with_article());
        self.fail(failure);
        Self::broken(checked.span)
    }

    /// Two sides of maths or a comparison, made the same type; `None` if that can't be done (already reported).
    fn two_sides(
        &mut self,
        left: &Expression,
        right: &Expression,
        span: Span,
        comparing: bool,
    ) -> Option<(CheckedExpression, CheckedExpression)> {
        let left = self.infer(left);
        let right = self.infer(right);
        if left.ty.is_broken() || right.ty.is_broken() {
            return None;
        }
        let both_written = left.ty.is_written() && right.ty.is_written();
        if both_written {
            return Some((left, right));
        }
        if left.ty == Type::Text || right.ty == Type::Text {
            if comparing && left.ty == right.ty {
                return Some((left, right));
            }
            if !comparing {
                self.fail(messages::not_designed_yet(span, "joining text with `plus`"));
                return None;
            }
        }
        if left.ty.is_written() {
            let made = self.made_into(left, &right.ty.clone());
            return if made.ty.is_broken() { None } else { Some((made, right)) };
        }
        if right.ty.is_written() {
            let made = self.made_into(right, &left.ty.clone());
            return if made.ty.is_broken() { None } else { Some((left, made)) };
        }
        if left.ty != right.ty {
            let what = if comparing {
                format!("comparing a {} with a {}", left.ty.name(), right.ty.name())
            } else {
                format!("maths that mixes a {} with a {}", left.ty.name(), right.ty.name())
            };
            self.fail(messages::not_designed_yet(span, what));
            return None;
        }
        Some((left, right))
    }

    // trace: decision.maths.words
    // trace: decision.maths.whole_divide
    fn maths(&mut self, operator: Maths, left: &Expression, right: &Expression, span: Span) -> CheckedExpression {
        let Some((left, right)) = self.two_sides(left, right, span, false) else { return Self::broken(span) };
        let ty = left.ty.clone();
        if matches!(ty, Type::Fraction(_) | Type::WrittenFraction) || matches!(right.ty, Type::WrittenFraction) {
            self.fail(messages::not_designed_yet(span, "maths on fractions"));
            return Self::broken(span);
        }
        if left.ty.is_written() && right.ty.is_written() {
            return self.written_maths(operator, &left, &right, span);
        }
        if !matches!(ty, Type::Whole(_) | Type::Decimal { .. }) {
            self.fail(messages::not_designed_yet(span, format!("maths on a {}", ty.name())));
            return Self::broken(span);
        }
        let because = self.because_of(&[&left, &right]);
        CheckedExpression {
            kind: CheckedKind::Maths { operator, left: Box::new(left), right: Box::new(right), because },
            ty,
            span,
        }
    }

    /// Maths on written numbers, worked out now, exactly.
    fn written_maths(
        &mut self,
        operator: Maths,
        left: &CheckedExpression,
        right: &CheckedExpression,
        span: Span,
    ) -> CheckedExpression {
        if let CheckedKind::Fail(problem) | CheckedKind::WrittenRound { problem, .. } = &left.kind {
            return CheckedExpression { kind: CheckedKind::Fail(problem.clone()), ty: left.ty.clone(), span };
        }
        if let CheckedKind::Fail(problem) | CheckedKind::WrittenRound { problem, .. } = &right.kind {
            return CheckedExpression { kind: CheckedKind::Fail(problem.clone()), ty: right.ty.clone(), span };
        }
        let (CheckedKind::Constant(a), CheckedKind::Constant(b)) = (&left.kind, &right.kind) else {
            // Something with no size of its own that isn't known before running, like `length of`.
            let ty = whole_type(Size::Big, false);
            let left = self.made_into(left.clone(), &ty);
            let right = self.made_into(right.clone(), &ty);
            let because = self.because_of(&[&left, &right]);
            return CheckedExpression {
                kind: CheckedKind::Maths { operator, left: Box::new(left), right: Box::new(right), because },
                ty,
                span,
            };
        };
        let outcome = match (a, b) {
            (Constant::WrittenWhole(x), Constant::WrittenWhole(y)) => match operator {
                Maths::Plus => Ok(Constant::WrittenWhole(x + y)),
                Maths::Minus => Ok(Constant::WrittenWhole(x - y)),
                Maths::Times => Ok(Constant::WrittenWhole(x * y)),
                Maths::DividedBy => numbers::whole_divided_by(x, y).map(Constant::WrittenWhole),
            },
            _ => {
                let x = written_decimal(a);
                let y = written_decimal(b);
                match operator {
                    Maths::Plus => x.plus(&y, None),
                    Maths::Minus => x.minus(&y, None),
                    Maths::Times => x.times(&y, None),
                    Maths::DividedBy => x.divided_by(&y, None),
                }
                .map(Constant::WrittenDecimal)
            }
        };
        let ty = if matches!(outcome, Ok(Constant::WrittenWhole(_)))
            || (left.ty == Type::WrittenWhole && right.ty == Type::WrittenWhole)
        {
            Type::WrittenWhole
        } else {
            Type::WrittenDecimal
        };
        match outcome {
            Ok(value) => CheckedExpression { kind: CheckedKind::Constant(value), ty, span },
            Err(problem) => CheckedExpression { kind: CheckedKind::Fail(problem), ty, span },
        }
    }

    /// `remainder of (a divided by b)` when `remainder`, else `round (a divided by b)`.
    fn division_word(&mut self, division: &Expression, span: Span, remainder: bool) -> CheckedExpression {
        let ExpressionKind::Maths { left, right, .. } = &division.unbracketed().kind else {
            return Self::broken(span);
        };
        let Some((top, bottom)) = self.two_sides(left, right, division.span, false) else { return Self::broken(span) };
        let ty = top.ty.clone();
        if matches!(ty, Type::Fraction(_) | Type::WrittenFraction) {
            self.fail(messages::not_designed_yet(span, "maths on fractions"));
            return Self::broken(span);
        }
        if remainder && !matches!(ty, Type::Whole(_) | Type::WrittenWhole) {
            self.fail(messages::not_designed_yet(span, format!("the remainder of a {}", ty.name())));
            return Self::broken(span);
        }
        if !remainder && matches!(ty, Type::Decimal { fast: true, .. }) {
            self.fail(messages::not_designed_yet(span, format!("rounding a {}", ty.name())));
            return Self::broken(span);
        }
        if top.ty.is_written() && bottom.ty.is_written() {
            if let CheckedKind::Fail(problem) = &top.kind {
                return CheckedExpression { kind: CheckedKind::Fail(problem.clone()), ty, span };
            }
            if let CheckedKind::Fail(problem) = &bottom.kind {
                return CheckedExpression { kind: CheckedKind::Fail(problem.clone()), ty, span };
            }
            if let (CheckedKind::Constant(a), CheckedKind::Constant(b)) = (&top.kind, &bottom.kind) {
                let outcome = match (a, b) {
                    (Constant::WrittenWhole(x), Constant::WrittenWhole(y)) => {
                        if remainder { numbers::whole_remainder(x, y) } else { numbers::whole_rounded(x, y) }
                            .map(Constant::WrittenWhole)
                    }
                    _ => written_decimal(a).divided_by_rounded(&written_decimal(b), None).map(Constant::WrittenDecimal),
                };
                let ty = if matches!(outcome, Ok(Constant::WrittenWhole(_))) || ty == Type::WrittenWhole {
                    Type::WrittenWhole
                } else {
                    Type::WrittenDecimal
                };
                return match outcome {
                    Ok(value) => CheckedExpression { kind: CheckedKind::Constant(value), ty, span },
                    // A decimal is rounded to the digits of the type it goes into, so this waits until that is known.
                    Err(problem) if !remainder && ty == Type::WrittenDecimal => CheckedExpression {
                        kind: CheckedKind::WrittenRound { top: a.clone(), bottom: b.clone(), problem },
                        ty,
                        span,
                    },
                    Err(problem) => CheckedExpression { kind: CheckedKind::Fail(problem), ty, span },
                };
            }
        }
        let (top, bottom, ty) = if top.ty.is_written() {
            let big = whole_type(Size::Big, false);
            let top = self.made_into(top, &big);
            let bottom = self.made_into(bottom, &big);
            (top, bottom, big)
        } else {
            (top, bottom, ty)
        };
        let because = self.because_of(&[&top, &bottom]);
        let kind = if remainder {
            CheckedKind::Remainder { top: Box::new(top), bottom: Box::new(bottom), because }
        } else {
            CheckedKind::RoundedDivide { top: Box::new(top), bottom: Box::new(bottom), because }
        };
        CheckedExpression { kind, ty, span }
    }

    // trace: decision.conditions.compare
    fn compare(&mut self, operator: Compare, left: &Expression, right: &Expression, span: Span) -> CheckedExpression {
        let Some((left, right)) = self.two_sides(left, right, span, true) else { return Self::broken(span) };
        let ordering = !matches!(operator, Compare::Is | Compare::IsNot);
        match &left.ty {
            Type::Table { .. } => {
                self.fail(messages::not_designed_yet(span, "whether two tables are the same"));
                return Self::broken(span);
            }
            Type::Text if ordering => {
                self.fail(messages::not_designed_yet(span, "whether one text is more or less than another"));
                return Self::broken(span);
            }
            Type::TrueOrFalse if ordering => {
                self.fail(messages::not_designed_yet(span, "whether one true_or_false is more or less than another"));
                return Self::broken(span);
            }
            Type::Fraction(_) | Type::WrittenFraction => {
                self.fail(messages::not_designed_yet(span, "comparing fractions"));
                return Self::broken(span);
            }
            _ => {}
        }
        if let (CheckedKind::Constant(a), CheckedKind::Constant(b)) = (&left.kind, &right.kind)
            && left.ty.is_written()
        {
            let order = match (a, b) {
                (Constant::WrittenWhole(x), Constant::WrittenWhole(y)) => x.cmp(y),
                _ => written_decimal(a).compare(&written_decimal(b)),
            };
            let value = compare_holds(operator, order);
            return CheckedExpression {
                kind: CheckedKind::Constant(Constant::TrueOrFalse(value)),
                ty: Type::TrueOrFalse,
                span,
            };
        }
        let (left, right) = if left.ty.is_written() {
            let big = whole_type(Size::Big, false);
            let decimal = left.ty == Type::WrittenDecimal || right.ty == Type::WrittenDecimal;
            if decimal || left.ty != Type::WrittenWhole {
                (left, right)
            } else {
                let left = self.made_into(left, &big);
                let right = self.made_into(right, &big);
                (left, right)
            }
        } else {
            (left, right)
        };
        CheckedExpression {
            kind: CheckedKind::Compare { operator, left: Box::new(left), right: Box::new(right) },
            ty: Type::TrueOrFalse,
            span,
        }
    }

    // trace: decision.tables.of_family
    fn family(&mut self, family: Family, operand: &Expression, span: Span) -> CheckedExpression {
        let operand = self.infer(operand);
        if operand.ty.is_broken() {
            return Self::broken(span);
        }
        if family == Family::Length {
            return match &operand.ty {
                Type::Text | Type::Table { .. } | Type::WrittenEmpty => {
                    if let CheckedKind::Table(items) = &operand.kind
                        && operand.ty.is_written()
                    {
                        return CheckedExpression {
                            kind: CheckedKind::Constant(Constant::WrittenWhole(BigInt::from(items.len()))),
                            ty: Type::WrittenWhole,
                            span,
                        };
                    }
                    CheckedExpression {
                        kind: CheckedKind::Family { family, operand: Box::new(operand), because: Vec::new() },
                        ty: Type::WrittenWhole,
                        span,
                    }
                }
                other => {
                    self.fail(messages::not_designed_yet(span, format!("`length of` a {}", other.name())));
                    Self::broken(span)
                }
            };
        }
        let item = match &operand.ty {
            Type::Table { keyed: false, item } => (**item).clone(),
            Type::Table { keyed: true, item } => (**item).clone(),
            other => {
                self.fail(messages::not_designed_yet(span, format!("`{} of` a {}", family.word(), other.name())));
                return Self::broken(span);
            }
        };
        if !matches!(item, Type::Whole(_) | Type::Decimal { .. } | Type::WrittenWhole | Type::WrittenDecimal) {
            self.fail(messages::not_designed_yet(span, format!("`{} of` a table of {}", family.word(), item.name())));
            return Self::broken(span);
        }
        if let CheckedKind::Table(items) = &operand.kind
            && item.is_written()
        {
            return self.written_family(family, items, &item, span);
        }
        if item.is_written() {
            self.fail(messages::not_designed_yet(span, "a table whose items have no type"));
            return Self::broken(span);
        }
        let because = self.because_of(&[&operand]);
        CheckedExpression { kind: CheckedKind::Family { family, operand: Box::new(operand), because }, ty: item, span }
    }

    fn written_family(
        &mut self,
        family: Family,
        items: &[CheckedExpression],
        item: &Type,
        span: Span,
    ) -> CheckedExpression {
        let values: Vec<Decimal> = items
            .iter()
            .filter_map(|value| match &value.kind {
                CheckedKind::Constant(constant) => Some(written_decimal(constant)),
                _ => None,
            })
            .collect();
        if values.is_empty() {
            return CheckedExpression { kind: CheckedKind::Fail(Problem::DividesByZero), ty: item.clone(), span };
        }
        let mut sum = Decimal::from_whole(BigInt::zero());
        for value in &values {
            sum = sum.plus(value, None).unwrap_or(sum);
        }
        let outcome = match family {
            Family::Sum => Ok(sum),
            Family::Biggest => {
                Ok(values.iter().cloned().fold(values[0].clone(), |a, b| if b.compare(&a).is_gt() { b } else { a }))
            }
            Family::Smallest => {
                Ok(values.iter().cloned().fold(values[0].clone(), |a, b| if b.compare(&a).is_lt() { b } else { a }))
            }
            Family::Average => {
                if *item == Type::WrittenWhole {
                    let total = sum.rational().0;
                    numbers::whole_divided_by(&total, &BigInt::from(values.len())).map(Decimal::from_whole)
                } else {
                    sum.divided_by(&Decimal::from_whole(BigInt::from(values.len())), None)
                }
            }
            Family::Length => Ok(Decimal::from_whole(BigInt::from(values.len()))),
        };
        match outcome {
            Ok(value) if *item == Type::WrittenWhole => CheckedExpression {
                kind: CheckedKind::Constant(Constant::WrittenWhole(value.rational().0)),
                ty: Type::WrittenWhole,
                span,
            },
            Ok(value) => CheckedExpression {
                kind: CheckedKind::Constant(Constant::WrittenDecimal(value)),
                ty: Type::WrittenDecimal,
                span,
            },
            Err(problem) => CheckedExpression { kind: CheckedKind::Fail(problem), ty: item.clone(), span },
        }
    }

    // trace: decision.procedures.shape
    // trace: decision.procedures.defaults
    // trace: decision.procedures.share
    fn call(&mut self, name: &str, name_span: Span, arguments: &[Argument], span: Span) -> Option<CheckedCall> {
        let Some(&procedure) = self.by_name.get(name) else {
            if let Some(family) = name.strip_suffix("_of").and_then(Family::from_word) {
                self.fail(messages::built_in_two_words(span, family.word()));
            } else {
                let names: Vec<String> = self.signatures.iter().map(|signature| signature.name.clone()).collect();
                match near_match(name, names.iter().map(String::as_str)) {
                    Some(near) => self.fail(messages::unknown_procedure_near(name_span, name, near)),
                    None => self.fail(messages::unknown_procedure(name_span, name)),
                }
            }
            for argument in arguments {
                self.infer(&argument.value);
            }
            return None;
        };
        let params: Vec<(String, Type, bool, bool)> = self.signatures[procedure].params.clone();
        let mut checked = Vec::new();
        let mut ok = true;
        for (index, argument) in arguments.iter().enumerate() {
            let Some((_, param_type, share, _)) = params.get(index) else {
                let takes = params.len();
                let things = if takes == 1 { "thing" } else { "things" };
                let failure = messages::one_too_many(argument.span, name, takes);
                let failure = if takes == 1 {
                    crate::failure::Failure {
                        head: crate::failure::Head::What(format!(
                            "`{}` takes 1 {}, and this is one too many",
                            name, things
                        )),
                        ..failure
                    }
                } else {
                    failure
                };
                self.fail(failure);
                for rest in &arguments[index..] {
                    self.infer(&rest.value);
                }
                return None;
            };
            if *share {
                let value = self.infer(&argument.value);
                if value.ty.is_broken() {
                    ok = false;
                    continue;
                }
                let CheckedKind::Variable(id) = value.kind else {
                    self.fail(messages::not_designed_yet(argument.span, "sharing something that isn't a variable"));
                    ok = false;
                    continue;
                };
                if !argument.share {
                    let failure = messages::share_at_call(argument.span, &self.variables[id].name);
                    self.fail(failure);
                    ok = false;
                    continue;
                }
                if &value.ty != param_type {
                    self.fail(messages::wrong_type(argument.value.span, &value.ty.with_article(), &param_type.name()));
                    ok = false;
                    continue;
                }
                checked.push(CheckedArgument::Share(id));
            } else {
                if argument.share {
                    self.fail(messages::not_designed_yet(
                        argument.span,
                        "`share` for a param that doesn't say `share`",
                    ));
                }
                let value = self.expression(&argument.value, Some(param_type));
                if value.ty.is_broken() {
                    ok = false;
                }
                checked.push(CheckedArgument::Value(value));
            }
        }
        for (index, (_, _, _, has_default)) in params.iter().enumerate().skip(arguments.len()) {
            if !has_default {
                self.fail(messages::missing_argument(span, name, index + 1));
                return None;
            }
            checked.push(CheckedArgument::Default);
        }
        if !ok {
            return None;
        }
        Some(CheckedCall { procedure, arguments: checked, span })
    }

    // ---- statements ----

    fn block(&mut self, statements: &[Statement], in_loop: bool) -> Vec<CheckedStatement> {
        self.enter();
        let mut checked = Vec::new();
        let mut left_by: Option<&'static str> = None;
        let mut reported_never = false;
        for (index, statement) in statements.iter().enumerate() {
            if let Some(word) = left_by
                && !reported_never
            {
                // trace: decision.errors.return_nothing
                self.fail(messages::never_runs(statement.span, word));
                reported_never = true;
            }
            if statement.only_if.is_none() {
                match statement.kind {
                    StatementKind::Stop => left_by = left_by.or(Some("stop")),
                    StatementKind::Finish => left_by = left_by.or(Some("finish")),
                    StatementKind::Return(_) => left_by = left_by.or(Some("return")),
                    _ => {}
                }
            }
            let last_in_loop = in_loop && index + 1 == statements.len();
            if let Some(statement) = self.statement(statement, last_in_loop) {
                checked.push(statement);
            }
        }
        self.leave();
        checked
    }

    fn statement(&mut self, statement: &Statement, last_in_loop: bool) -> Option<CheckedStatement> {
        let checked = self.statement_kind(statement, last_in_loop)?;
        match &statement.only_if {
            Some(test) => {
                let test = self.test(test);
                Some(CheckedStatement::OnlyIf { test, statement: Box::new(checked) })
            }
            None => Some(checked),
        }
    }

    fn statement_kind(&mut self, statement: &Statement, last_in_loop: bool) -> Option<CheckedStatement> {
        let span = statement.span;
        match &statement.kind {
            StatementKind::Let { name, name_span, written_type, value } => {
                let (ty, bounds, can_change) = match written_type {
                    Some(written) => {
                        let (ty, bounds) = self.resolve_type(written, TypePlace::Let);
                        let can_change = !written.modifiers.iter().any(|m| m.kind == ModifierKind::CantChange);
                        (ty, bounds, can_change)
                    }
                    None => (Type::Broken, Bounds::default(), true),
                };
                if is_grammar_word(name) && name == "add" {
                    // A variable's `@` keeps it apart from TAKL's own words.
                }
                let value = match value {
                    LetValue::Expression(expression) => Some(self.value_for(expression, &ty, &bounds)),
                    LetValue::Block(items) => Some(self.block_table(items, &ty, span)),
                    LetValue::Missing => {
                        let failure = messages::not_designed_yet(
                            span,
                            format!("a `let` with no value, so `@{}` would start as nothing", name),
                        );
                        self.fail(failure);
                        None
                    }
                };
                let id = self.declare(name, *name_span, ty, bounds, can_change);
                let value = value?;
                Some(CheckedStatement::Set { variable: id, value, site: span })
            }
            StatementKind::Assign { name, name_span, value } => {
                let made = self.variable_use(name, *name_span);
                let Some(made) = made else {
                    self.infer(value);
                    return None;
                };
                if made.loop_item {
                    self.fail(messages::not_designed_yet(
                        *name_span,
                        format!("changing `@{}` while `for each` goes through it", name),
                    ));
                    return None;
                }
                if !made.can_change {
                    self.fail(messages::cant_change(*name_span, name, made.name_span));
                    return None;
                }
                let ty = self.variables[made.id].ty.clone();
                let bounds = self.variables[made.id].bounds.clone();
                let value = self.value_for(value, &ty, &bounds);
                Some(CheckedStatement::Set { variable: made.id, value, site: span })
            }
            StatementKind::Show(pieces) => {
                let mut checked = Vec::new();
                for piece in pieces {
                    let value = self.infer(piece);
                    match &value.ty {
                        Type::TrueOrFalse => {
                            self.fail(messages::not_designed_yet(value.span, "how `show` prints true or false"))
                        }
                        Type::Table { .. } => {
                            self.fail(messages::not_designed_yet(value.span, "how `show` prints a table"))
                        }
                        Type::WrittenEmpty => self.fail(messages::empty_no_type(value.span)),
                        Type::Nothing => {}
                        _ => {}
                    }
                    let value = if value.ty == Type::WrittenWhole
                        && !matches!(value.kind, CheckedKind::Constant(_) | CheckedKind::Fail(_))
                    {
                        self.made_into(value, &whole_type(Size::Big, false))
                    } else {
                        value
                    };
                    checked.push(value);
                }
                Some(CheckedStatement::Show(checked))
            }
            StatementKind::If { branches, otherwise } => {
                let mut checked = Vec::new();
                for (test, body) in branches {
                    let test = self.test(test);
                    let body = self.block(body, false);
                    checked.push((test, body));
                }
                let otherwise = otherwise.as_ref().map(|body| self.block(body, false)).unwrap_or_default();
                Some(CheckedStatement::If { branches: checked, otherwise })
            }
            StatementKind::RepeatTimes { count, body } => {
                let count = self.infer(count);
                let count = if count.ty.is_broken() {
                    count
                } else {
                    match self.whole_index(count) {
                        Some(count) => count,
                        None => Self::broken(span),
                    }
                };
                self.loops += 1;
                let body = self.block(body, true);
                self.loops -= 1;
                Some(CheckedStatement::RepeatTimes { count, body, span })
            }
            StatementKind::RepeatUntil { test, body } => {
                let test = self.test(test);
                self.loops += 1;
                let body = self.block(body, true);
                self.loops -= 1;
                Some(CheckedStatement::RepeatUntil { test, body })
            }
            StatementKind::RepeatForever { body } => {
                self.loops += 1;
                let body = self.block(body, true);
                self.loops -= 1;
                Some(CheckedStatement::RepeatForever { body })
            }
            StatementKind::ForEach { names, table, body } => {
                let table = self.infer(table);
                let (keyed, item) = match &table.ty {
                    Type::Table { keyed, item } => (*keyed, (**item).clone()),
                    Type::Broken => (false, Type::Broken),
                    other => {
                        self.fail(messages::not_designed_yet(
                            table.span,
                            format!("`for each` over a {}", other.name()),
                        ));
                        (false, Type::Broken)
                    }
                };
                if item.is_written() {
                    self.fail(messages::not_designed_yet(table.span, "a loop over a table whose items have no type"));
                }
                let names_wanted = if keyed { 2 } else { 1 };
                if !table.ty.is_broken() && names.len() != names_wanted {
                    self.fail(messages::not_designed_yet(
                        table.span,
                        "this shape of `for each`: one name for a table with named keys, or two for a numbered one",
                    ));
                }
                self.enter();
                let mut ids = Vec::new();
                for (index, (name, name_span)) in names.iter().enumerate() {
                    let ty = if keyed && index == 0 && names.len() == 2 { Type::Text } else { item.clone() };
                    let id = self.declare(name, *name_span, ty, Bounds::default(), true);
                    if let Some(scope) = self.scopes.last_mut()
                        && let Some((_, made)) = scope.last_mut()
                        && made.id == id
                    {
                        made.loop_item = true;
                    }
                    ids.push((id, *name_span));
                }
                self.loops += 1;
                let body = self.block(body, true);
                self.loops -= 1;
                self.leave();
                if table.ty.is_broken() || names.len() != names_wanted || ids.is_empty() {
                    return None;
                }
                let (key, item) = if keyed { (Some(ids[0]), ids[1]) } else { (None, ids[0]) };
                Some(CheckedStatement::ForEach { key, item, table, body })
            }
            StatementKind::Stop => {
                if self.loops == 0 {
                    self.fail(messages::stop_outside_loops(span));
                }
                Some(CheckedStatement::Stop)
            }
            StatementKind::Finish => {
                if let Some(returns) = &self.returns {
                    let failure = messages::finish_in_returns(span, &returns.name());
                    self.fail(failure);
                } else if last_in_loop && statement.only_if.is_none() {
                    self.fail(messages::finish_in_loop(span));
                }
                Some(CheckedStatement::Finish)
            }
            StatementKind::Return(value) => {
                let Some(returns) = self.returns.clone() else {
                    let failure = messages::return_in_finishing(span, &self.procedure_name);
                    self.fail(failure);
                    self.infer(value);
                    return None;
                };
                let value = self.expression(value, Some(&returns));
                Some(CheckedStatement::Return(value))
            }
            StatementKind::Add { key, item, table, table_span, at } => {
                let made = self.variable_use(table, *table_span)?;
                let ty = self.variables[made.id].ty.clone();
                let (keyed, item_type) = match &ty {
                    Type::Table { keyed, item } => (*keyed, (**item).clone()),
                    Type::Broken => return None,
                    other => {
                        self.fail(messages::not_designed_yet(*table_span, format!("`add` to a {}", other.name())));
                        return None;
                    }
                };
                if !made.can_change {
                    self.fail(messages::cant_change(*table_span, table, made.name_span));
                    return None;
                }
                let key = match (key, keyed) {
                    (Some(key), true) => Some(self.expression(key, Some(&Type::Text))),
                    (None, false) => None,
                    (Some(key), false) => {
                        self.fail(messages::not_designed_yet(key.span, "a named key on a numbered table"));
                        return None;
                    }
                    (None, true) => {
                        self.fail(messages::not_designed_yet(
                            item.span,
                            "an item with no name on a table with named keys",
                        ));
                        return None;
                    }
                };
                let item = self.expression(item, Some(&item_type));
                let at = match at {
                    Some((_, index)) if keyed => {
                        self.fail(messages::not_designed_yet(
                            index.span,
                            "putting a named item at a place in the table",
                        ));
                        return None;
                    }
                    Some((counting, index)) => {
                        let index = self.infer(index);
                        if index.ty.is_broken() {
                            return None;
                        }
                        Some((*counting, self.whole_index(index)?))
                    }
                    None => None,
                };
                if item.ty.is_broken() || key.as_ref().is_some_and(|key| key.ty.is_broken()) {
                    return None;
                }
                Some(CheckedStatement::Add { table: made.id, key, item, at, span })
            }
            StatementKind::Call { name, name_span, arguments } => {
                let call_span = arguments.last().map(|argument| name_span.to(argument.span)).unwrap_or(*name_span);
                let call = self.call(name, *name_span, arguments, call_span)?;
                Some(CheckedStatement::Call(call))
            }
            StatementKind::Broken => None,
        }
    }

    /// The value for a `let` or an `=`: made into the variable's type, a table copied, its bounds checked.
    fn value_for(&mut self, expression: &Expression, ty: &Type, bounds: &Bounds) -> CheckedExpression {
        let value = self.expression(expression, Some(ty));
        if let CheckedKind::Constant(Constant::Whole(v)) = &value.kind
            && !bounds.allows(*v)
        {
            // trace: decision.types.bounds
            self.fail(messages::outside_bounds(value.span, &v.to_string(), &bounds.rule()));
            return Self::broken(value.span);
        }
        value
    }

    // trace: decision.tables.shape
    fn block_table(&mut self, items: &[BlockItem], ty: &Type, span: Span) -> CheckedExpression {
        let Type::Table { keyed, item } = ty else {
            if !ty.is_broken() {
                self.fail(messages::not_designed_yet(span, format!("a block of items for a {}", ty.name())));
            }
            return Self::broken(span);
        };
        if *keyed {
            let mut pairs = Vec::new();
            for block_item in items {
                let Some(key) = &block_item.key else {
                    self.fail(messages::not_designed_yet(
                        block_item.value.span,
                        "an item with no name on a table with named keys",
                    ));
                    return Self::broken(span);
                };
                let key = self.expression(key, Some(&Type::Text));
                let value = self.expression(&block_item.value, Some(item));
                pairs.push((key, value));
            }
            return CheckedExpression { kind: CheckedKind::KeyedTable(pairs), ty: ty.clone(), span };
        }
        let mut values = Vec::new();
        for block_item in items {
            if let Some(key) = &block_item.key {
                self.fail(messages::not_designed_yet(key.span, "a named key on a numbered table"));
                return Self::broken(span);
            }
            values.push(self.expression(&block_item.value, Some(item)));
        }
        CheckedExpression { kind: CheckedKind::Table(values), ty: ty.clone(), span }
    }

    // ---- procedures ----

    fn procedure(&mut self, procedure: &Procedure, id: ProcedureId) -> CheckedProcedure {
        self.variables.clear();
        self.scopes = vec![Vec::new()];
        self.ended.clear();
        self.loops = 0;
        self.procedure_name = procedure.name.clone();
        self.returns = self.signatures[id].returns.clone();
        let mut params = Vec::new();
        for param in &procedure.params {
            let (ty, bounds) = self.resolve_type(&param.written_type, TypePlace::Param);
            let share = param.written_type.modifiers.iter().any(|m| m.kind == ModifierKind::Share);
            let default = param.default.as_ref().map(|default| self.value_for(default, &ty, &bounds));
            let variable = self.declare(&param.name, param.name_span, ty, bounds.clone(), true);
            self.variables[variable].param_site = Some(param.name_span);
            params.push(CheckedParam { variable, default, share, bounds });
        }
        let body = self.block(&procedure.body, false);
        // trace: decision.procedures.finish
        let end = procedure.end_span.unwrap_or(procedure.header_span);
        // The line that ends the procedure is its first that leaves it; any after it is reported as never running.
        let leaves = |statement: &&Statement| {
            statement.only_if.is_none() && matches!(statement.kind, StatementKind::Finish | StatementKind::Return(_))
        };
        let last = procedure.body.iter().find(leaves).or(procedure.body.last());
        let last = last.filter(|statement| statement.only_if.is_none()).map(|statement| &statement.kind);
        // A last line already reported says nothing about how the procedure ends.
        let last_broken = matches!(last, Some(StatementKind::Broken));
        match &self.returns {
            None => {
                if !matches!(procedure.ending, Ending::Missing(_))
                    && !last_broken
                    && !matches!(last, Some(StatementKind::Finish | StatementKind::Return(_)))
                {
                    let failure = messages::must_finish(end, &procedure.name);
                    self.fail(failure);
                }
            }
            Some(returns) => {
                if !last_broken && !matches!(last, Some(StatementKind::Return(_) | StatementKind::Finish)) {
                    let failure = messages::needs_return(end, &procedure.name, &returns.name());
                    self.fail(failure);
                }
            }
        }
        CheckedProcedure {
            name: procedure.name.clone(),
            params,
            returns: self.returns.clone(),
            variables: std::mem::take(&mut self.variables),
            body,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TypePlace {
    Let,
    Param,
    Item,
    Returns,
}

fn written_decimal(constant: &Constant) -> Decimal {
    match constant {
        Constant::WrittenWhole(value) => Decimal::from_whole(value.clone()),
        Constant::WrittenDecimal(value) => value.clone(),
        _ => Decimal::from_whole(BigInt::zero()),
    }
}

pub fn compare_holds(operator: Compare, order: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match operator {
        Compare::Is => order == Equal,
        Compare::IsNot => order != Equal,
        Compare::MoreThan => order == Greater,
        Compare::LessThan => order == Less,
        Compare::AtLeast => order != Less,
        Compare::AtMost => order != Greater,
    }
}

/// Checks a parsed program. The mistakes come back in the order they are in the program.
pub fn check(source: &Source, program: &Program, mut failures: Vec<Failure>) -> Result<CheckedProgram, Vec<Failure>> {
    let mut checker = Checker {
        _source: std::marker::PhantomData,
        failures: Vec::new(),
        signatures: Vec::new(),
        by_name: HashMap::new(),
        variables: Vec::new(),
        scopes: Vec::new(),
        ended: Vec::new(),
        loops: 0,
        returns: None,
        procedure_name: String::new(),
    };
    // Every procedure's first line first, so a call can come before the procedure it calls.
    for procedure in &program.procedures {
        let mut params = Vec::new();
        for param in &procedure.params {
            let (ty, _) = checker.resolve_type_quietly(&param.written_type);
            let share = param.written_type.modifiers.iter().any(|m| m.kind == ModifierKind::Share);
            params.push((param.name.clone(), ty, share, param.default.is_some()));
        }
        let returns = match &procedure.ending {
            Ending::Returns(written, span) => {
                let (ty, _) = checker.resolve_type(written, TypePlace::Returns);
                if procedure.name == "start_running" && !ty.is_broken() && ty != whole_type(Size::Tiny, true) {
                    // trace: decision.procedures.exit_code
                    checker.fail(messages::exit_code_type(*span, &ty.name()));
                }
                Some(ty)
            }
            Ending::JustFinishes => None,
            Ending::Missing(span) => {
                // trace: decision.procedures.just_finishes
                checker.fail(messages::needs_ending(*span, &procedure.name));
                None
            }
        };
        if checker.by_name.contains_key(&procedure.name) {
            checker.fail(messages::not_designed_yet(
                procedure.name_span,
                format!("two procedures called `{}`", procedure.name),
            ));
        } else {
            checker.by_name.insert(procedure.name.clone(), checker.signatures.len());
        }
        checker.signatures.push(Signature { name: procedure.name.clone(), params, returns });
        if procedure.name == "start_running"
            && let Some(param) = procedure.params.first()
        {
            checker.fail(messages::not_designed_yet(param.name_span, "what `start_running`'s params would be given"));
        }
    }
    let mut procedures = Vec::new();
    for (id, procedure) in program.procedures.iter().enumerate() {
        procedures.push(checker.procedure(procedure, id));
    }
    // trace: decision.procedures.start_running_required
    let start = checker.by_name.get("start_running").copied();
    if start.is_none() {
        let span = program
            .procedures
            .first()
            .map(|procedure| procedure.header_span)
            .unwrap_or_else(|| Span::new(1, 0, source.line(1).len()));
        checker.fail(messages::no_start_running(span));
    }
    failures.append(&mut checker.failures);
    if !failures.is_empty() {
        failures.sort_by_key(|failure| (failure.span.line, failure.span.start));
        failures.dedup();
        return Err(failures);
    }
    Ok(CheckedProgram { procedures, start: start.unwrap_or(0) })
}

impl<'s> Checker<'s> {
    /// An expression's type, with nothing reported.
    fn quiet_type(&mut self, expression: &Expression) -> Type {
        let before = self.failures.len();
        let ty = self.infer(expression).ty;
        self.failures.truncate(before);
        ty
    }

    /// A param's type for the procedure's signature; its mistakes are reported when the procedure itself is checked.
    fn resolve_type_quietly(&mut self, written: &WrittenType) -> (Type, Bounds) {
        let before = self.failures.len();
        let resolved = self.resolve_type(written, TypePlace::Param);
        self.failures.truncate(before);
        resolved
    }
}
