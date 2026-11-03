//! The parsed program, as written.

use crate::source::Span;
use num_bigint::BigInt;

pub type ExprId = usize;

#[derive(Clone, Debug)]
pub struct Program {
    pub procedures: Vec<Procedure>,
}

#[derive(Clone, Debug)]
pub enum Ending {
    Returns(TypeExpr, Vec<Modifier>),
    JustFinishes,
}

#[derive(Clone, Debug)]
pub struct Procedure {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub ending: Ending,
    pub ending_span: Span,
    pub body: Vec<Stmt>,
    pub end_span: Span,
    /// Its first line could not be read; it is known by name only, so nothing more is said about it.
    pub broken_header: bool,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub ty: TypeExpr,
    pub modifiers: Vec<Modifier>,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum TypeExpr {
    Named(String, Span),
    Table { key: Option<Box<ItemType>>, item: Box<ItemType>, span: Span },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(_, span) => *span,
            TypeExpr::Table { span, .. } => *span,
        }
    }
}

/// A table's item or key type, with the modifiers that were bracketed with it.
#[derive(Clone, Debug)]
pub struct ItemType {
    pub ty: TypeExpr,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModifierKind {
    CantChange,
    NeverNegative,
    FastButInexact,
    Share,
    AtLeast(BigInt),
    AtMost(BigInt),
}

#[derive(Clone, Debug)]
pub struct Modifier {
    pub kind: ModifierKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    /// A line ending in `if`, run only when the test holds.
    pub only_if: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum LetValue {
    Expr(Expr),
    Share(String, Span),
    Block(Vec<BlockItem>, Span),
}

#[derive(Clone, Debug)]
pub enum BlockItem {
    Item(Expr),
    Keyed(Expr, Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Position,
    Offset,
}

#[derive(Clone, Debug)]
pub enum Arg {
    Value(Expr),
    Share(String, Span),
}

impl Arg {
    pub fn span(&self) -> Span {
        match self {
            Arg::Value(expr) => expr.span,
            Arg::Share(_, span) => *span,
        }
    }
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Let {
        name: String,
        name_span: Span,
        ty: TypeExpr,
        modifiers: Vec<Modifier>,
        value: LetValue,
    },
    /// A `let` whose line could not be read; its name is known so later lines don't repeat the error.
    BrokenLet {
        name: String,
    },
    Assign {
        name: String,
        name_span: Span,
        value: LetValue,
    },
    Show(Vec<Expr>),
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Option<Vec<Stmt>>,
    },
    RepeatTimes {
        count: Expr,
        body: Vec<Stmt>,
    },
    RepeatUntil {
        test: Expr,
        body: Vec<Stmt>,
    },
    RepeatForever {
        body: Vec<Stmt>,
    },
    ForEach {
        key: Option<(String, Span)>,
        item: (String, Span),
        table: Expr,
        body: Vec<Stmt>,
    },
    Stop,
    Finish,
    Return(Expr),
    Add {
        value: Expr,
        key: Option<Expr>,
        table: (String, Span),
        at: Option<(IndexKind, Expr)>,
    },
    Call {
        name: String,
        name_span: Span,
        args: Vec<Arg>,
    },
    /// A line that could not be read; its error is already given.
    Broken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathsOp {
    Plus,
    Minus,
    Times,
    DividedBy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Is,
    Isnt,
    MoreThan,
    LessThan,
    AtLeast,
    AtMost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    AndAlso,
    OrAlso,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfWord {
    Length,
    Sum,
    Biggest,
    Smallest,
    Average,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Whole(BigInt),
    /// A written decimal: its digits without the point, and how many came after it.
    Decimal {
        digits: BigInt,
        places: u32,
        written: String,
    },
    /// `~0.1`: the nearest a fast decimal holds.
    About {
        written: String,
    },
    Fraction(BigInt, BigInt),
    Text(String),
    True,
    False,
    Empty,
    Variable(String),
    Brackets(Box<Expr>),
    Maths(MathsOp, Box<Expr>, Box<Expr>),
    Compare(CompareOp, Box<Expr>, Box<Expr>),
    Logic(LogicOp, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Table(Vec<Expr>),
    KeyedTable(Vec<(Expr, Expr)>),
    Of(OfWord, Box<Expr>),
    Index(IndexKind, Box<Expr>, Box<Expr>),
    Round(Box<Expr>),
    Remainder(Box<Expr>, Box<Expr>),
    Call(String, Vec<Arg>),
    /// A lone name with no `@` and no brackets: a variable missing its `@`, or a call missing `()`.
    BareName(String),
}
