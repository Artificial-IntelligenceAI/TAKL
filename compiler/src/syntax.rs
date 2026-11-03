//! The program as written, before its names and types are checked.

use crate::source::Span;

#[derive(Clone, Debug)]
pub struct Program {
    pub procedures: Vec<Procedure>,
}

#[derive(Clone, Debug)]
pub struct Procedure {
    pub name: String,
    pub name_span: Span,
    /// The whole first line, `procedure ... :`.
    pub header_span: Span,
    pub params: Vec<Param>,
    pub ending: Ending,
    pub body: Vec<Statement>,
    /// The `end procedure name` line, if it was found.
    pub end_span: Option<Span>,
}

#[derive(Clone, Debug)]
pub enum Ending {
    Returns(WrittenType, Span),
    JustFinishes,
    /// Neither was written; the span is the header's `:`.
    Missing(Span),
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub name_span: Span,
    pub written_type: WrittenType,
    pub default: Option<Expression>,
}

#[derive(Clone, Debug)]
pub struct WrittenType {
    pub base: Base,
    pub modifiers: Vec<Modifier>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Base {
    Named(String, Span),
    Table { item: Box<WrittenType>, key: Option<Box<WrittenType>> },
}

#[derive(Clone, Debug)]
pub struct Modifier {
    pub kind: ModifierKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModifierKind {
    NeverNegative,
    CantChange,
    FastButInexact,
    Share,
    AtLeast(num_bigint::BigInt),
    AtMost(num_bigint::BigInt),
    /// A bound that couldn't be read; already reported.
    BrokenBound,
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct Statement {
    pub kind: StatementKind,
    /// The whole line, from its first token to its last.
    pub span: Span,
    /// `... if test` at the end of the line.
    pub only_if: Option<Expression>,
}

#[derive(Clone, Debug)]
pub enum StatementKind {
    Let { name: String, name_span: Span, written_type: Option<WrittenType>, value: LetValue },
    Assign { name: String, name_span: Span, value: Expression },
    Show(Vec<Expression>),
    If { branches: Vec<(Expression, Vec<Statement>)>, otherwise: Option<Vec<Statement>> },
    RepeatTimes { count: Expression, body: Vec<Statement> },
    RepeatUntil { test: Expression, body: Vec<Statement> },
    RepeatForever { body: Vec<Statement> },
    ForEach { names: Vec<(String, Span)>, table: Expression, body: Vec<Statement> },
    Stop,
    Finish,
    Return(Expression),
    Add { key: Option<Expression>, item: Expression, table: String, table_span: Span, at: Option<(Counting, Expression)> },
    Call { name: String, name_span: Span, arguments: Vec<Argument> },
    /// A line that couldn't be read; already reported.
    Broken,
}

#[derive(Clone, Debug)]
pub enum LetValue {
    Expression(Expression),
    /// A table written as a block, one item a line.
    Block(Vec<BlockItem>),
    Missing,
}

#[derive(Clone, Debug)]
pub struct BlockItem {
    pub key: Option<Expression>,
    pub value: Expression,
}

#[derive(Clone, Debug)]
pub struct Argument {
    pub value: Expression,
    pub share: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Maths {
    Plus,
    Minus,
    Times,
    DividedBy,
}

impl Maths {
    pub fn word(self) -> &'static str {
        match self {
            Maths::Plus => "plus",
            Maths::Minus => "minus",
            Maths::Times => "times",
            Maths::DividedBy => "divided by",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compare {
    Is,
    IsNot,
    MoreThan,
    LessThan,
    AtLeast,
    AtMost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Logic {
    And,
    Or,
    AndAlso,
    OrAlso,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Length,
    Sum,
    Biggest,
    Smallest,
    Average,
}

impl Family {
    pub fn word(self) -> &'static str {
        match self {
            Family::Length => "length",
            Family::Sum => "sum",
            Family::Biggest => "biggest",
            Family::Smallest => "smallest",
            Family::Average => "average",
        }
    }

    pub fn from_word(word: &str) -> Option<Family> {
        Some(match word {
            "length" => Family::Length,
            "sum" => Family::Sum,
            "biggest" => Family::Biggest,
            "smallest" => Family::Smallest,
            "average" => Family::Average,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Counting {
    /// Counts from 1.
    Position,
    /// Counts from 0.
    Offset,
}

impl Counting {
    pub fn word(self) -> &'static str {
        match self {
            Counting::Position => "position",
            Counting::Offset => "offset",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Expression {
    pub kind: ExpressionKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExpressionKind {
    Number { whole: String, after_point: Option<String> },
    /// `~0.1`, the nearest a fast decimal holds.
    About { whole: String, after_point: Option<String> },
    Text(String),
    True,
    False,
    Empty,
    Table(Vec<Expression>),
    KeyedTable(Vec<(Expression, Expression)>),
    Variable(String),
    /// A name with no `@` and no brackets after it.
    Bare(String),
    Call { name: String, name_span: Span, arguments: Vec<Argument>, bracketed: bool },
    Maths { operator: Maths, left: Box<Expression>, right: Box<Expression> },
    Over { top: Box<Expression>, bottom: Box<Expression> },
    Compare { operator: Compare, left: Box<Expression>, right: Box<Expression> },
    Logic { operator: Logic, left: Box<Expression>, right: Box<Expression> },
    Not(Box<Expression>),
    Family { family: Family, operand: Box<Expression> },
    /// `remainder of (a divided by b)`, holding the division.
    Remainder(Box<Expression>),
    Index { counting: Counting, index: Box<Expression>, table: Box<Expression> },
    Round(Box<Expression>),
    Share(Box<Expression>),
    Bracketed(Box<Expression>),
    /// Couldn't be read; already reported.
    Broken,
}

impl Expression {
    pub fn broken(span: Span) -> Expression {
        Expression { kind: ExpressionKind::Broken, span }
    }

    /// Brackets only group; what is inside is the value.
    pub fn unbracketed(&self) -> &Expression {
        match &self.kind {
            ExpressionKind::Bracketed(inner) => inner.unbracketed(),
            _ => self,
        }
    }
}
