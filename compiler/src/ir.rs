//! TAKL's IR: the one form between bytecode and Cranelift, worked at two strengths.
//!
//! trace: decision.backend.two_strengths
//!
//! Built from a procedure's bytecode in SSA form: each number register becomes values, a block's live registers its
//! params. Heap registers stay in the frame, as they are in the interpreter, and the instructions that use them run
//! as one interpreter `Step`. Every check that can stop the program carries its bytecode failure and the values its
//! `because:` lines need, so compiled code says exactly what the interpreter says, and every place maps back to a
//! place in the bytecode.

use crate::bytecode::{self, FailureId, FailureKind, Held, Instruction, Place, Procedure, Program, Register, RegisterKind};
use crate::numbers::Size;
use crate::syntax::{Compare, Maths};
use std::collections::{BTreeSet, HashMap};

pub type Value = u32;
pub type BlockId = u32;

/// Where a check that stops the program points back to.
#[derive(Clone, Debug)]
pub struct FailPoint {
    pub procedure: u32,
    pub failure: FailureId,
    /// How far, in registers, this procedure's frame is past the compiled function's own (above 0 when inlined).
    pub frame_offset: u32,
    /// The registers its `because:` lines read, and their values here.
    pub because: Vec<(Register, Value)>,
}

#[derive(Clone, Debug)]
pub enum Op {
    Constant(i64),
    /// A number register's value, from the frame where compiled code is entered.
    Load(Register),
    /// `a + by`, which can't overflow (a loop's step below its limit).
    Step1(Value),
    WholeMaths { operator: Maths, kind: bytecode::WholeKind, a: Value, b: Value, fail: FailPoint },
    WholeRemainder { kind: bytecode::WholeKind, a: Value, b: Value, fail: FailPoint },
    WholeCompare { operator: Compare, unsigned: bool, a: Value, b: Value },
    CheckWhole { kind: bytecode::WholeKind, value: Value, fail: FailPoint },
    CheckBounds { value: Value, least: Option<i64>, most: Option<i64>, fail: FailPoint },
    CheckCount { value: Value, fail: FailPoint },
    FastMaths { operator: Maths, size: Size, a: Value, b: Value, fail: FailPoint },
    FastCompare { operator: Compare, size: Size, a: Value, b: Value },
    Not(Value),
    /// A call whose arguments and answer are all numbers, to a procedure with no heap registers.
    Call { callee: u32, arguments: Vec<Value>, fail: FailPoint },
    /// One bytecode instruction run by the interpreter, with the number registers it reads stored first.
    Step { procedure: u32, at: Place, reads: Vec<(Register, Value)>, write: Option<Register>, frame_offset: u32 },
    /// An inlined call's entry: checks how deep calls go, and counts one more.
    DepthEnter { fail: FailPoint },
    DepthLeave,
}

impl Op {
    /// Uses no memory and can't stop the program, so it can go if nothing uses it.
    pub fn is_pure(&self) -> bool {
        matches!(self, Op::Constant(_) | Op::Load(_) | Op::Step1(_) | Op::WholeCompare { .. } | Op::FastCompare { .. } | Op::Not(_))
    }

    pub fn operands(&self) -> Vec<Value> {
        let mut found = Vec::new();
        let fail_values = |fail: &FailPoint, found: &mut Vec<Value>| found.extend(fail.because.iter().map(|(_, value)| *value));
        match self {
            Op::Constant(_) | Op::Load(_) | Op::DepthLeave => {}
            Op::Step1(a) | Op::Not(a) => found.push(*a),
            Op::WholeMaths { a, b, fail, .. } | Op::WholeRemainder { a, b, fail, .. } | Op::FastMaths { a, b, fail, .. } => {
                found.push(*a);
                found.push(*b);
                fail_values(fail, &mut found);
            }
            Op::WholeCompare { a, b, .. } | Op::FastCompare { a, b, .. } => {
                found.push(*a);
                found.push(*b);
            }
            Op::CheckWhole { value, fail, .. } | Op::CheckBounds { value, fail, .. } | Op::CheckCount { value, fail } => {
                found.push(*value);
                fail_values(fail, &mut found);
            }
            Op::Call { arguments, fail, .. } => {
                found.extend(arguments);
                fail_values(fail, &mut found);
            }
            Op::Step { reads, .. } => found.extend(reads.iter().map(|(_, value)| *value)),
            Op::DepthEnter { fail } => fail_values(fail, &mut found),
        }
        found
    }

    pub fn map_operands(&mut self, map: &mut impl FnMut(Value) -> Value) {
        let map_fail = |fail: &mut FailPoint, map: &mut dyn FnMut(Value) -> Value| {
            for (_, value) in &mut fail.because {
                *value = map(*value);
            }
        };
        match self {
            Op::Constant(_) | Op::Load(_) | Op::DepthLeave => {}
            Op::Step1(a) | Op::Not(a) => *a = map(*a),
            Op::WholeMaths { a, b, fail, .. } | Op::WholeRemainder { a, b, fail, .. } | Op::FastMaths { a, b, fail, .. } => {
                *a = map(*a);
                *b = map(*b);
                map_fail(fail, map);
            }
            Op::WholeCompare { a, b, .. } | Op::FastCompare { a, b, .. } => {
                *a = map(*a);
                *b = map(*b);
            }
            Op::CheckWhole { value, fail, .. } | Op::CheckBounds { value, fail, .. } | Op::CheckCount { value, fail } => {
                *value = map(*value);
                map_fail(fail, map);
            }
            Op::Call { arguments, fail, .. } => {
                for argument in arguments.iter_mut() {
                    *argument = map(*argument);
                }
                map_fail(fail, map);
            }
            Op::Step { reads, .. } => {
                for (_, value) in reads.iter_mut() {
                    *value = map(*value);
                }
            }
            Op::DepthEnter { fail } => map_fail(fail, map),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Instr {
    pub result: Option<Value>,
    pub op: Op,
}

#[derive(Clone, Debug)]
pub enum End {
    Jump(BlockId, Vec<Value>),
    /// To the first when the value isn't 0.
    Branch(Value, (BlockId, Vec<Value>), (BlockId, Vec<Value>)),
    /// The procedure is done: a number answer, or a heap register's, or nothing.
    Return { value: Option<Value>, heap: Option<Register>, release: bool },
    Fail(FailPoint),
}

impl End {
    pub fn targets_mut(&mut self) -> Vec<&mut (BlockId, Vec<Value>)> {
        match self {
            End::Jump(..) => Vec::new(),
            End::Branch(_, a, b) => vec![a, b],
            _ => Vec::new(),
        }
    }

    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            End::Jump(block, _) => vec![*block],
            End::Branch(_, (a, _), (b, _)) => vec![*a, *b],
            _ => Vec::new(),
        }
    }

    pub fn map_operands(&mut self, map: &mut impl FnMut(Value) -> Value) {
        match self {
            End::Jump(_, arguments) => arguments.iter_mut().for_each(|value| *value = map(*value)),
            End::Branch(condition, (_, a), (_, b)) => {
                *condition = map(*condition);
                a.iter_mut().for_each(|value| *value = map(*value));
                b.iter_mut().for_each(|value| *value = map(*value));
            }
            End::Return { value, .. } => {
                if let Some(value) = value {
                    *value = map(*value);
                }
            }
            End::Fail(fail) => fail.because.iter_mut().for_each(|(_, value)| *value = map(*value)),
        }
    }

    pub fn operands(&self) -> Vec<Value> {
        let mut found = Vec::new();
        let mut copy = self.clone();
        copy.map_operands(&mut |value| {
            found.push(value);
            value
        });
        found
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub params: Vec<Value>,
    pub instrs: Vec<Instr>,
    pub end: End,
}

/// Where compiled code is entered: a procedure's start, or the top of one of its loops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    Start,
    Loop(u16),
}

#[derive(Clone, Debug)]
pub struct Function {
    pub procedure: u32,
    pub entry: Entry,
    pub blocks: Vec<Block>,
    pub values: u32,
}

impl Function {
    pub fn new_value(&mut self) -> Value {
        self.values += 1;
        self.values - 1
    }
}

/// The number registers an instruction reads, and the one it writes if any.
pub fn number_uses(procedure: &Procedure, instruction: &Instruction) -> (Vec<Register>, Option<Register>) {
    use Instruction::*;
    let (reads, write): (Vec<Register>, Option<Register>) = match *instruction {
        Load { dest, .. } | LoadHeap { dest, .. } | SetSite { dest, .. } | NewTable { dest, .. } => (vec![], Some(dest)),
        Move { dest, source } | MoveHeap { dest, source } | CopyTable { dest, source } | Not { dest, source } => (vec![source], Some(dest)),
        WholeMaths { dest, a, b, .. }
        | WholeRemainder { dest, a, b, .. }
        | WholeRounded { dest, a, b, .. }
        | WholeCompare { dest, a, b, .. }
        | FastMaths { dest, a, b, .. }
        | FastCompare { dest, a, b, .. }
        | HeapMaths { dest, a, b, .. }
        | HeapRounded { dest, a, b, .. }
        | HeapCompare { dest, a, b, .. }
        | TextCompare { dest, a, b, .. } => (vec![a, b], Some(dest)),
        WholeMathsBy { dest, a, .. } => (vec![a], Some(dest)),
        JumpIfWhole { a, b, .. } => (vec![a, b], None),
        JumpIfWholeBy { a, .. } => (vec![a], None),
        CheckWhole { source, .. } | CheckBounds { source, .. } | CheckCount { source, .. } | ShowPiece { source, .. } => (vec![source], None),
        JumpIfTrue { source, .. } | JumpIfFalse { source, .. } | Return { source } => (vec![source], None),
        RepeatNext { counter, limit, .. } => (vec![counter, limit], Some(counter)),
        Call { dest, arguments, .. } => (procedure.call_arguments[arguments as usize].clone(), Some(dest)),
        Push { table, source } | Add { table, source } => (vec![table, source], None),
        PushKeyed { table, key, source, .. } | AddKeyed { table, key, source, .. } => (vec![table, key, source], None),
        AddAt { table, source, at, .. } => (vec![table, source, at], None),
        Length { dest, source, .. } => (vec![source], Some(dest)),
        Item { dest, table, at, .. } | KeyAt { dest, table, at } => (vec![table, at], Some(dest)),
        Letter { dest, text, at, .. } => (vec![text, at], Some(dest)),
        OfFamily { dest, table, .. } => (vec![table], Some(dest)),
        Jump { .. } | LoopBack { .. } | ShowEnd | Finish | Fail { .. } => (vec![], None),
    };
    let number = |register: &Register| procedure.kinds[*register as usize] != RegisterKind::Heap;
    let mut reads: Vec<Register> = reads.into_iter().filter(number).collect();
    // A failure's `because:` lines read their registers too.
    let failure = match *instruction {
        WholeMaths { failure, .. }
        | WholeMathsBy { failure, .. }
        | WholeRemainder { failure, .. }
        | WholeRounded { failure, .. }
        | FastMaths { failure, .. }
        | HeapMaths { failure, .. }
        | HeapRounded { failure, .. } => Some(failure),
        _ => None,
    };
    if let Some(failure) = failure {
        for entry in &procedure.failures[failure as usize].because {
            if number(&entry.register) {
                reads.push(entry.register);
            }
            if let bytecode::Site::Register(site) = entry.site {
                reads.push(site);
            }
        }
    }
    (reads, write.filter(number))
}

/// Where each bytecode block starts, and which blocks follow it.
fn blocks_of(procedure: &Procedure) -> (Vec<usize>, Vec<Vec<usize>>) {
    let code = &procedure.instructions;
    let mut leaders = BTreeSet::new();
    leaders.insert(0usize);
    for (at, instruction) in code.iter().enumerate() {
        use Instruction::*;
        match *instruction {
            Jump { target } | LoopBack { target, .. } => {
                leaders.insert(target as usize);
                leaders.insert(at + 1);
            }
            JumpIfTrue { target, .. } | JumpIfFalse { target, .. } | JumpIfWhole { target, .. } | JumpIfWholeBy { target, .. } | RepeatNext { target, .. } => {
                leaders.insert(target as usize);
                leaders.insert(at + 1);
            }
            Return { .. } | Finish | Fail { .. } => {
                leaders.insert(at + 1);
            }
            _ => {}
        }
    }
    for top in &procedure.loops {
        leaders.insert(*top as usize);
    }
    let starts: Vec<usize> = leaders.into_iter().filter(|start| *start < code.len()).collect();
    let index_of: HashMap<usize, usize> = starts.iter().enumerate().map(|(index, start)| (*start, index)).collect();
    let mut successors = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(code.len());
        let last = &code[end - 1];
        let mut next = Vec::new();
        use Instruction::*;
        let fall = index_of.get(&end).copied();
        match *last {
            Jump { target } | LoopBack { target, .. } => next.push(index_of[&(target as usize)]),
            JumpIfTrue { target, .. } | JumpIfFalse { target, .. } | JumpIfWhole { target, .. } | JumpIfWholeBy { target, .. } | RepeatNext { target, .. } => {
                next.push(index_of[&(target as usize)]);
                next.extend(fall);
            }
            Return { .. } | Finish | Fail { .. } => {}
            _ => next.extend(fall),
        }
        let _ = start;
        successors.push(next);
    }
    (starts, successors)
}

/// Builds a procedure's IR, entered at its start or at the top of one of its loops.
pub fn build(program: &Program, procedure_id: u32, entry: Entry) -> Function {
    let procedure = &program.procedures[procedure_id as usize];
    let code = &procedure.instructions;
    let (starts, successors) = blocks_of(procedure);
    let count = starts.len();
    let end_of = |index: usize| starts.get(index + 1).copied().unwrap_or(code.len());
    // Liveness of number registers.
    let mut uses = vec![BTreeSet::new(); count];
    let mut defs = vec![BTreeSet::new(); count];
    for index in 0..count {
        for instruction in &code[starts[index]..end_of(index)] {
            let (reads, write) = number_uses(procedure, instruction);
            for read in reads {
                if !defs[index].contains(&read) {
                    uses[index].insert(read);
                }
            }
            if let Some(write) = write {
                defs[index].insert(write);
            }
        }
    }
    let mut live_in: Vec<BTreeSet<Register>> = uses.clone();
    loop {
        let mut changed = false;
        for index in (0..count).rev() {
            let mut live: BTreeSet<Register> = BTreeSet::new();
            for next in &successors[index] {
                live.extend(live_in[*next].iter().copied());
            }
            for def in &defs[index] {
                live.remove(def);
            }
            live.extend(uses[index].iter().copied());
            if live != live_in[index] {
                live_in[index] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let block_of: HashMap<usize, usize> = starts.iter().enumerate().map(|(index, start)| (*start, index)).collect();
    let mut function = Function { procedure: procedure_id, entry, blocks: Vec::new(), values: 0 };
    // IR block 0 enters; IR block i + 1 is bytecode block i.
    let mut params: Vec<Vec<(Register, Value)>> = Vec::new();
    function.blocks.push(Block { params: Vec::new(), instrs: Vec::new(), end: End::Return { value: None, heap: None, release: false } });
    for live in &live_in {
        let mut list = Vec::new();
        for register in live {
            list.push((*register, function.new_value()));
        }
        params.push(list.clone());
        function.blocks.push(Block { params: list.iter().map(|(_, value)| *value).collect(), instrs: Vec::new(), end: End::Return { value: None, heap: None, release: false } });
    }
    let release = !procedure.heap_registers.is_empty();
    // The entry: loads what is live where it starts.
    let (start_block, loaded): (usize, Vec<Register>) = match entry {
        Entry::Start => (0, live_in[0].iter().copied().collect()),
        Entry::Loop(loop_id) => {
            let top = block_of[&(procedure.loops[loop_id as usize] as usize)];
            (top, live_in[top].iter().copied().collect())
        }
    };
    let mut entry_values = HashMap::new();
    let mut entry_instrs = Vec::new();
    for register in &loaded {
        let value = function.new_value();
        entry_instrs.push(Instr { result: Some(value), op: Op::Load(*register) });
        entry_values.insert(*register, value);
    }
    let arguments: Vec<Value> = params[start_block].iter().map(|(register, _)| entry_values[register]).collect();
    function.blocks[0].instrs = entry_instrs;
    function.blocks[0].end = End::Jump(start_block as BlockId + 1, arguments);
    for index in 0..count {
        let mut current: HashMap<Register, Value> = params[index].iter().copied().collect();
        let mut instrs = Vec::new();
        let mut end = None;
        let fall_through = index + 1;
        let mut get = |register: Register, current: &mut HashMap<Register, Value>, instrs: &mut Vec<Instr>, function: &mut Function| -> Value {
            if let Some(value) = current.get(&register) {
                return *value;
            }
            // Read before any write on this path: a register made only later; its value is never used.
            let value = function.new_value();
            instrs.push(Instr { result: Some(value), op: Op::Constant(0) });
            current.insert(register, value);
            value
        };
        let at_targets = |target: usize, current: &mut HashMap<Register, Value>, instrs: &mut Vec<Instr>, function: &mut Function, get: &mut dyn FnMut(Register, &mut HashMap<Register, Value>, &mut Vec<Instr>, &mut Function) -> Value| -> (BlockId, Vec<Value>) {
            let arguments = params[target].iter().map(|(register, _)| get(*register, current, instrs, function)).collect();
            (target as BlockId + 1, arguments)
        };
        for at in starts[index]..end_of(index) {
            let instruction = code[at];
            let fail_point = |failure: FailureId, current: &mut HashMap<Register, Value>, instrs: &mut Vec<Instr>, function: &mut Function, get: &mut dyn FnMut(Register, &mut HashMap<Register, Value>, &mut Vec<Instr>, &mut Function) -> Value| -> FailPoint {
                let mut because = Vec::new();
                for entry in &procedure.failures[failure as usize].because {
                    // A heap value's `because:` is read from the frame, where it already is.
                    if procedure.kinds[entry.register as usize] != RegisterKind::Heap {
                        because.push((entry.register, get(entry.register, current, instrs, function)));
                    }
                    if let bytecode::Site::Register(site) = entry.site {
                        because.push((site, get(site, current, instrs, function)));
                    }
                }
                FailPoint { procedure: procedure_id, failure, frame_offset: 0, because }
            };
            let mut emit = |op: Op, function: &mut Function, instrs: &mut Vec<Instr>| -> Value {
                let value = function.new_value();
                instrs.push(Instr { result: Some(value), op });
                value
            };
            use Instruction::*;
            match instruction {
                Load { dest, value } => {
                    let made = emit(Op::Constant(value), &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                SetSite { dest, site } => {
                    let made = emit(Op::Constant(site as i64), &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                Move { dest, source } if procedure.kinds[source as usize] != RegisterKind::Heap => {
                    let value = get(source, &mut current, &mut instrs, &mut function);
                    current.insert(dest, value);
                }
                WholeMaths { operator, kind, dest, a, b, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::WholeMaths { operator, kind, a, b, fail }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                WholeMathsBy { operator, kind, dest, a, by, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = emit(Op::Constant(by as i64), &mut function, &mut instrs);
                    let made = emit(Op::WholeMaths { operator, kind, a, b, fail }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                WholeRemainder { kind, dest, a, b, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::WholeRemainder { kind, a, b, fail }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                WholeCompare { operator, unsigned, dest, a, b } => {
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::WholeCompare { operator, unsigned, a, b }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                JumpIfWhole { operator, unsigned, a, b, target } => {
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let test = emit(Op::WholeCompare { operator, unsigned, a, b }, &mut function, &mut instrs);
                    let yes = at_targets(block_of[&(target as usize)], &mut current, &mut instrs, &mut function, &mut get);
                    let no = at_targets(fall_through, &mut current, &mut instrs, &mut function, &mut get);
                    end = Some(End::Branch(test, yes, no));
                }
                JumpIfWholeBy { operator, unsigned, a, by, target } => {
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = emit(Op::Constant(by as i64), &mut function, &mut instrs);
                    let test = emit(Op::WholeCompare { operator, unsigned, a, b }, &mut function, &mut instrs);
                    let yes = at_targets(block_of[&(target as usize)], &mut current, &mut instrs, &mut function, &mut get);
                    let no = at_targets(fall_through, &mut current, &mut instrs, &mut function, &mut get);
                    end = Some(End::Branch(test, yes, no));
                }
                CheckWhole { kind, source, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let value = get(source, &mut current, &mut instrs, &mut function);
                    instrs.push(Instr { result: None, op: Op::CheckWhole { kind, value, fail } });
                }
                CheckBounds { source, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let value = get(source, &mut current, &mut instrs, &mut function);
                    let (least, most) = match &procedure.failures[failure as usize].kind {
                        FailureKind::Bounds { bounds } => (bounds.least.map(|v| v as i64), bounds.most.map(|v| v as i64)),
                        _ => (None, None),
                    };
                    instrs.push(Instr { result: None, op: Op::CheckBounds { value, least, most, fail } });
                }
                CheckCount { source, failure } => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let value = get(source, &mut current, &mut instrs, &mut function);
                    instrs.push(Instr { result: None, op: Op::CheckCount { value, fail } });
                }
                FastMaths { operator, size, dest, a, b, failure } if matches!(size, Size::Small | Size::Medium) => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::FastMaths { operator, size, a, b, fail }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                FastCompare { operator, size, dest, a, b } if matches!(size, Size::Small | Size::Medium) => {
                    let a = get(a, &mut current, &mut instrs, &mut function);
                    let b = get(b, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::FastCompare { operator, size, a, b }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                Not { dest, source } => {
                    let source = get(source, &mut current, &mut instrs, &mut function);
                    let made = emit(Op::Not(source), &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                Jump { target } | LoopBack { target, .. } => {
                    let to = at_targets(block_of[&(target as usize)], &mut current, &mut instrs, &mut function, &mut get);
                    end = Some(End::Jump(to.0, to.1));
                }
                JumpIfTrue { source, target } | JumpIfFalse { source, target } => {
                    let test = get(source, &mut current, &mut instrs, &mut function);
                    let jump = at_targets(block_of[&(target as usize)], &mut current, &mut instrs, &mut function, &mut get);
                    let fall = at_targets(fall_through, &mut current, &mut instrs, &mut function, &mut get);
                    end = Some(if matches!(instruction, JumpIfTrue { .. }) { End::Branch(test, jump, fall) } else { End::Branch(test, fall, jump) });
                }
                RepeatNext { counter, limit, target, .. } => {
                    let counter_value = get(counter, &mut current, &mut instrs, &mut function);
                    let limit_value = get(limit, &mut current, &mut instrs, &mut function);
                    let next = emit(Op::Step1(counter_value), &mut function, &mut instrs);
                    current.insert(counter, next);
                    let test = emit(Op::WholeCompare { operator: Compare::LessThan, unsigned: false, a: next, b: limit_value }, &mut function, &mut instrs);
                    let back = at_targets(block_of[&(target as usize)], &mut current, &mut instrs, &mut function, &mut get);
                    let out = at_targets(fall_through, &mut current, &mut instrs, &mut function, &mut get);
                    end = Some(End::Branch(test, back, out));
                }
                Call { procedure: callee, dest, arguments, failure } if fast_call(program, procedure, callee, dest, arguments) => {
                    let fail = fail_point(failure, &mut current, &mut instrs, &mut function, &mut get);
                    let values = procedure.call_arguments[arguments as usize]
                        .iter()
                        .map(|register| get(*register, &mut current, &mut instrs, &mut function))
                        .collect();
                    let made = emit(Op::Call { callee, arguments: values, fail }, &mut function, &mut instrs);
                    current.insert(dest, made);
                }
                Return { source } => {
                    end = Some(if procedure.kinds[source as usize] == RegisterKind::Heap {
                        End::Return { value: None, heap: Some(source), release }
                    } else {
                        End::Return { value: Some(get(source, &mut current, &mut instrs, &mut function)), heap: None, release }
                    });
                }
                Finish => end = Some(End::Return { value: None, heap: None, release }),
                Fail { failure } => {
                    end = Some(End::Fail(fail_point(failure, &mut current, &mut instrs, &mut function, &mut get)));
                }
                other => {
                    let (reads, write) = number_uses(procedure, &other);
                    let reads = reads.into_iter().map(|register| (register, get(register, &mut current, &mut instrs, &mut function))).collect();
                    let result = write.map(|_| function.new_value());
                    instrs.push(Instr { result, op: Op::Step { procedure: procedure_id, at: at as Place, reads, write, frame_offset: 0 } });
                    if let (Some(register), Some(value)) = (write, result) {
                        current.insert(register, value);
                    }
                }
            }
            if end.is_some() {
                break;
            }
        }
        let end = match end {
            Some(end) => end,
            None => {
                let to = at_targets(fall_through, &mut current, &mut instrs, &mut function, &mut get);
                End::Jump(to.0, to.1)
            }
        };
        let block = &mut function.blocks[index + 1];
        block.instrs = instrs;
        block.end = end;
    }
    function
}

/// Whether a call can be made straight from compiled code: numbers in and out, and a callee with no heap registers.
pub fn fast_call(program: &Program, procedure: &Procedure, callee: u32, dest: Register, arguments: u32) -> bool {
    let called = &program.procedures[callee as usize];
    called.heap_registers.is_empty()
        && procedure.kinds[dest as usize] != RegisterKind::Heap
        && procedure.call_arguments[arguments as usize].iter().all(|register| procedure.kinds[*register as usize] != RegisterKind::Heap)
        && called.returns.as_ref().is_none_or(|ty| !ty.is_heap())
}

pub fn held_of(procedure: &Procedure, register: Register) -> Option<Held> {
    match procedure.kinds[register as usize] {
        RegisterKind::Number(held) => Some(held),
        RegisterKind::Heap => None,
    }
}
