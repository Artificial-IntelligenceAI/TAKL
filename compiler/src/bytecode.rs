//! TAKL's bytecode, the one shared form: the interpreter runs it, and Cranelift compiles from it.
//!
//! trace: decision.backend.bytecode
//!
//! A procedure's frame is an array of 64-bit registers. Its variables come first (params first of all), then the
//! registers that remember where a variable last got its value, then scratch registers for numbers, then scratch
//! registers for heap values. A register is a heap register or a number register for its whole life, so a heap
//! register always holds a counted pointer or nothing, and releasing what it held is always safe.
//!
//! Instructions are typed: which kind of number, and which size, is in the instruction, so no tags are checked
//! while running. Every place in the bytecode is an index, and compiled code maps back to those indices, which is
//! how a loop is jumped into mid-way.

use crate::checked::Bounds;
use crate::numbers::WholeType;
use crate::runtime::HeapRef;
use crate::source::Span;
use crate::syntax::{Compare, Counting, Family, Maths};
use crate::types::Type;

pub type Register = u16;
pub type FailureId = u16;
pub type Place = u32;

/// The eight whole-number types, as a small index into `WHOLE_KINDS`.
pub type WholeKind = u8;

pub const WHOLE_KINDS: [WholeType; 8] = {
    use crate::numbers::Size::*;
    [
        WholeType { size: Tiny, never_negative: false },
        WholeType { size: Small, never_negative: false },
        WholeType { size: Medium, never_negative: false },
        WholeType { size: Big, never_negative: false },
        WholeType { size: Tiny, never_negative: true },
        WholeType { size: Small, never_negative: true },
        WholeType { size: Medium, never_negative: true },
        WholeType { size: Big, never_negative: true },
    ]
};

pub fn whole_kind(whole: WholeType) -> WholeKind {
    WHOLE_KINDS.iter().position(|kind| *kind == whole).unwrap_or(3) as WholeKind
}

/// The smallest and biggest each whole kind holds, as `i64`; a big never_negative's biggest doesn't fit, and is
/// handled by its own checks.
pub const WHOLE_LIMITS: [(i64, i64); 8] = [
    (-128, 127),
    (-32768, 32767),
    (-2147483648, 2147483647),
    (i64::MIN, i64::MAX),
    (0, 255),
    (0, 65535),
    (0, 4294967295),
    (0, i64::MAX),
];

pub const BIG_WHOLE: WholeKind = 3;
pub const BIG_NEVER_NEGATIVE: WholeKind = 7;

/// How a value is held, for `show` and for the slow paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Held {
    Whole(WholeKind),
    /// A float16, float32 or float64, in a register's bits.
    Fast(crate::numbers::Size),
    /// In the heap: an exact decimal, a fraction, a float128, text or a table.
    Heap,
    TrueOrFalse,
}

pub fn held(ty: &Type) -> Held {
    match ty {
        Type::Whole(whole) => Held::Whole(whole_kind(*whole)),
        Type::Decimal { size, fast: true } if *size != crate::numbers::Size::Big => Held::Fast(*size),
        Type::TrueOrFalse => Held::TrueOrFalse,
        _ => Held::Heap,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    // ---- values ----
    /// Any number held in a register's bits: a whole number, a float's bits, true as 1.
    Load { dest: Register, value: i64 },
    /// A heap constant from the procedure's pool.
    LoadHeap { dest: Register, constant: u32 },
    Move { dest: Register, source: Register },
    /// Counts one more holder of a heap value.
    MoveHeap { dest: Register, source: Register },
    /// A table as a value: a new table with the same items, shared until either changes.
    CopyTable { dest: Register, source: Register },
    /// Remembers where a variable last got its value, for `because:`.
    SetSite { dest: Register, site: u32 },

    // ---- whole numbers; each checks its answer against its kind's range ----
    WholeMaths { operator: Maths, kind: WholeKind, dest: Register, a: Register, b: Register, failure: FailureId },
    WholeMathsBy { operator: Maths, kind: WholeKind, dest: Register, a: Register, by: i32, failure: FailureId },
    WholeRemainder { kind: WholeKind, dest: Register, a: Register, b: Register, failure: FailureId },
    WholeRounded { kind: WholeKind, dest: Register, a: Register, b: Register, failure: FailureId },
    WholeCompare { operator: Compare, unsigned: bool, dest: Register, a: Register, b: Register },
    /// Jumps when `a operator b` holds.
    JumpIfWhole { operator: Compare, unsigned: bool, a: Register, b: Register, target: Place },
    JumpIfWholeBy { operator: Compare, unsigned: bool, a: Register, by: i32, target: Place },
    /// A whole number of any kind, made into `kind`.
    CheckWhole { kind: WholeKind, source: Register, failure: FailureId },
    CheckBounds { source: Register, failure: FailureId },

    // ---- floats in registers ----
    FastMaths { operator: Maths, size: crate::numbers::Size, dest: Register, a: Register, b: Register, failure: FailureId },
    FastCompare { operator: Compare, size: crate::numbers::Size, dest: Register, a: Register, b: Register },

    // ---- everything else, worked out by `numbers` ----
    /// Exact decimals and float128s, in heap registers.
    HeapMaths { operator: Maths, type_id: u16, dest: Register, a: Register, b: Register, failure: FailureId },
    HeapRounded { type_id: u16, dest: Register, a: Register, b: Register, failure: FailureId },
    HeapCompare { operator: Compare, type_id: u16, dest: Register, a: Register, b: Register },

    // ---- true or false ----
    Not { dest: Register, source: Register },
    Jump { target: Place },
    JumpIfTrue { source: Register, target: Place },
    JumpIfFalse { source: Register, target: Place },
    /// The jump back to the top of a loop, which counts how hot the loop is.
    LoopBack { target: Place, loop_id: u16 },
    /// `counter` up by one, and back to `target` while it is below `limit`; a loop's jump back too.
    RepeatNext { counter: Register, limit: Register, target: Place, loop_id: u16 },
    /// A `repeat`'s count, which must not be below zero.
    CheckCount { source: Register, failure: FailureId },

    // ---- show ----
    /// One piece of a `show`, after every piece is worked out.
    ShowPiece { source: Register, type_id: u16, failure: FailureId },
    ShowEnd,

    // ---- procedures ----
    /// Calls with the registers in `call_arguments[arguments]` as the callee's first registers; the answer goes
    /// in `dest`. A heap argument is handed on counted, so the caller copies a table first unless it is shared.
    Call { procedure: u32, dest: Register, arguments: u32, failure: FailureId },
    Return { source: Register },
    Finish,
    /// Stops the program with a written-number problem worked out before running.
    Fail { failure: FailureId },

    // ---- tables and text ----
    NewTable { dest: Register, keyed: bool, item_heap: bool },
    /// Adds onto the end of a table this procedure just made, for a written table.
    Push { table: Register, source: Register },
    PushKeyed { table: Register, key: Register, source: Register, failure: FailureId },
    /// `add x to @t`, changing only this table.
    Add { table: Register, source: Register },
    AddAt { table: Register, source: Register, counting: Counting, at: Register, failure: FailureId },
    AddKeyed { table: Register, key: Register, source: Register, failure: FailureId },
    Length { dest: Register, source: Register, text: bool },
    Item { dest: Register, table: Register, counting: Counting, at: Register, failure: FailureId, heap: bool },
    /// The key at an offset, for `for each @key and @value`.
    KeyAt { dest: Register, table: Register, at: Register },
    Letter { dest: Register, text: Register, counting: Counting, at: Register, failure: FailureId },
    OfFamily { family: Family, type_id: u16, dest: Register, table: Register, failure: FailureId },
    TextCompare { operator: Compare, dest: Register, a: Register, b: Register },
}

/// What a stop says, beyond the problem itself.
#[derive(Clone, Debug)]
pub struct FailureInfo {
    pub span: Span,
    pub kind: FailureKind,
    /// The variables in the step, for `because:`.
    pub because: Vec<BecauseEntry>,
}

#[derive(Clone, Debug)]
pub enum FailureKind {
    /// Maths in this type: the problem is worked out from the operands when it stops.
    Maths { ty: Type },
    /// Written-number maths that stops, worked out before running.
    Written { problem: crate::numbers::Problem, ty: Type },
    Bounds { bounds: Bounds },
    /// No such position or offset in a table (`letters` for text).
    Place { counting: Counting, letters: bool },
    Hole { counting: Counting, table: String },
    KeyTwice,
    Family { family: Family, ty: Type },
    Count,
    FractionShow,
    TooDeep,
}

#[derive(Clone, Debug)]
pub struct BecauseEntry {
    pub name: String,
    pub register: Register,
    pub ty: Type,
    pub site: Site,
}

#[derive(Clone, Copy, Debug)]
pub enum Site {
    /// The variable gets its value in one place only.
    Fixed(Span),
    /// A register holding an index into the procedure's `sites`.
    Register(Register),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterKind {
    Number(Held),
    Heap,
}

#[derive(Debug)]
pub struct Procedure {
    pub name: String,
    pub instructions: Vec<Instruction>,
    pub registers: usize,
    pub kinds: Vec<RegisterKind>,
    pub params: usize,
    pub returns: Option<Type>,
    pub heap_constants: Vec<HeapRef>,
    pub failures: Vec<FailureInfo>,
    pub sites: Vec<Span>,
    pub call_arguments: Vec<Vec<Register>>,
    /// Types the slow paths and `show` need, indexed by the `type_id`s in instructions.
    pub types: Vec<Type>,
    /// Each loop's top, by loop id.
    pub loops: Vec<Place>,
    /// Pure: no `share` param, no `show`, calling only procedures like it; so the same input gives the same answer.
    pub pure: bool,
    /// Each param's whole-number bounds, for precomputed tables.
    pub param_ranges: Vec<Option<(i128, i128)>>,
    pub param_types: Vec<Type>,
}

impl Procedure {
    pub fn is_heap(&self, register: Register) -> bool {
        self.kinds.get(register as usize) == Some(&RegisterKind::Heap)
    }
}

#[derive(Debug)]
pub struct Program {
    pub procedures: Vec<Procedure>,
    pub start: u32,
}

// The interpreter's speed depends on instructions staying small.
const _: () = assert!(std::mem::size_of::<Instruction>() <= 16);
