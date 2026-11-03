//! The program after checking: every name resolved, every value typed, written-number maths worked out.

use crate::numbers::{Decimal, Problem};
use crate::source::Span;
use crate::syntax::{Compare, Counting, Family, Logic, Maths};
use crate::types::Type;
use num_bigint::BigInt;

pub type VariableId = usize;
pub type ProcedureId = usize;

#[derive(Clone, Debug)]
pub struct CheckedProgram {
    pub procedures: Vec<CheckedProcedure>,
    pub start: ProcedureId,
}

#[derive(Clone, Debug)]
pub struct CheckedProcedure {
    pub name: String,
    pub params: Vec<CheckedParam>,
    /// `None` when it just finishes.
    pub returns: Option<Type>,
    pub variables: Vec<Variable>,
    pub body: Vec<CheckedStatement>,
}

#[derive(Clone, Debug)]
pub struct CheckedParam {
    pub variable: VariableId,
    pub default: Option<CheckedExpression>,
    pub share: bool,
    pub bounds: Bounds,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bounds {
    pub least: Option<i128>,
    pub most: Option<i128>,
}

impl Bounds {
    pub fn is_empty(&self) -> bool {
        self.least.is_none() && self.most.is_none()
    }

    pub fn allows(&self, value: i128) -> bool {
        self.least.is_none_or(|least| value >= least) && self.most.is_none_or(|most| value <= most)
    }

    /// "at least 0 and at most 3", as the error says it.
    pub fn rule(&self) -> String {
        match (self.least, self.most) {
            (Some(least), Some(most)) => format!("at least {} and at most {}", least, most),
            (Some(least), None) => format!("at least {}", least),
            (None, Some(most)) => format!("at most {}", most),
            (None, None) => String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Variable {
    pub name: String,
    pub ty: Type,
    pub bounds: Bounds,
    /// For a param, its name in the first line: where it got its value.
    pub param_site: Option<Span>,
}

// The checked tree is made once per program, so a big variant costs nothing worth boxing for.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum CheckedStatement {
    /// A `let` or an `=`. `site` is the whole line, which `because:` points at.
    Set {
        variable: VariableId,
        value: CheckedExpression,
        site: Span,
    },
    Show(Vec<CheckedExpression>),
    If {
        branches: Vec<(CheckedExpression, Vec<CheckedStatement>)>,
        otherwise: Vec<CheckedStatement>,
    },
    RepeatTimes {
        count: CheckedExpression,
        body: Vec<CheckedStatement>,
        span: Span,
    },
    RepeatUntil {
        test: CheckedExpression,
        body: Vec<CheckedStatement>,
    },
    RepeatForever {
        body: Vec<CheckedStatement>,
    },
    ForEach {
        key: Option<(VariableId, Span)>,
        item: (VariableId, Span),
        table: CheckedExpression,
        body: Vec<CheckedStatement>,
    },
    Stop,
    Finish,
    Return(CheckedExpression),
    Add {
        table: VariableId,
        key: Option<CheckedExpression>,
        item: CheckedExpression,
        at: Option<(Counting, CheckedExpression)>,
        span: Span,
    },
    Call(CheckedCall),
    /// A line that runs only when the test holds.
    OnlyIf {
        test: CheckedExpression,
        statement: Box<CheckedStatement>,
    },
}

#[derive(Clone, Debug)]
pub struct CheckedCall {
    pub procedure: ProcedureId,
    pub arguments: Vec<CheckedArgument>,
    pub span: Span,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum CheckedArgument {
    /// A value, copied into the param; `bounds` and `span` check it against the param's bounds.
    Value(CheckedExpression),
    Share(VariableId),
    /// The param's own default.
    Default,
}

/// A value known before running.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Whole(i128),
    /// A fast decimal's bits.
    Fast(u128),
    Decimal(Decimal),
    Fraction(BigInt, BigInt),
    Text(String),
    TrueOrFalse(bool),
    /// Written numbers, exact and with no size.
    WrittenWhole(BigInt),
    WrittenDecimal(Decimal),
    WrittenFraction(BigInt, BigInt),
}

#[derive(Clone, Debug)]
pub struct CheckedExpression {
    pub kind: CheckedKind,
    pub ty: Type,
    /// Where an error in this step points, brackets included.
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum CheckedKind {
    Constant(Constant),
    Variable(VariableId),
    /// `because` names the variables in the step, in the order they are written.
    Maths {
        operator: Maths,
        left: Box<CheckedExpression>,
        right: Box<CheckedExpression>,
        because: Vec<VariableId>,
    },
    Compare {
        operator: Compare,
        left: Box<CheckedExpression>,
        right: Box<CheckedExpression>,
    },
    Logic {
        operator: Logic,
        left: Box<CheckedExpression>,
        right: Box<CheckedExpression>,
    },
    Not(Box<CheckedExpression>),
    Family {
        family: Family,
        operand: Box<CheckedExpression>,
    },
    Remainder {
        top: Box<CheckedExpression>,
        bottom: Box<CheckedExpression>,
        because: Vec<VariableId>,
    },
    /// `round (a divided by b)`.
    RoundedDivide {
        top: Box<CheckedExpression>,
        bottom: Box<CheckedExpression>,
        because: Vec<VariableId>,
    },
    Index {
        counting: Counting,
        index: Box<CheckedExpression>,
        table: Box<CheckedExpression>,
    },
    Call(CheckedCall),
    Table(Vec<CheckedExpression>),
    KeyedTable(Vec<(CheckedExpression, CheckedExpression)>),
    /// `empty`, of the type it has.
    Empty,
    /// The same table, not a copy.
    Share(VariableId),
    /// A whole number with no size of its own (`length of`), made into `ty` while running.
    Resize(Box<CheckedExpression>),
    /// Written-number maths that stops when it runs.
    Fail(Problem),
    /// `round (a divided by b)` on written numbers whose answer doesn't end: rounded once the type is known, and
    /// otherwise the problem, when it runs.
    WrittenRound {
        top: Constant,
        bottom: Constant,
        problem: Problem,
    },
}
