//! The passes over TAKL's IR. The quick strength runs the cheap ones; the speed strength runs every pass.
//!
//! trace: decision.backend.two_strengths
//! trace: decision.house.optimize_early
//!
//! A pass may only change speed. Every check that can stop the program stays unless the pass proves it can't fail,
//! and a check that stays keeps its failure and its `because:` values, so the output is the interpreter's to the byte.

use crate::bytecode::{WHOLE_LIMITS, BIG_NEVER_NEGATIVE};
use crate::interpreter;
use crate::ir::*;
use std::collections::{HashMap, HashSet};

/// Drops blocks nothing reaches, and numbers the rest in order.
pub fn prune(function: &mut Function) {
    let mut seen = vec![false; function.blocks.len()];
    let mut order = Vec::new();
    let mut work = vec![0 as BlockId];
    while let Some(block) = work.pop() {
        if std::mem::replace(&mut seen[block as usize], true) {
            continue;
        }
        order.push(block);
        for next in function.blocks[block as usize].end.successors().into_iter().rev() {
            work.push(next);
        }
    }
    order.sort();
    let renumber: HashMap<BlockId, BlockId> = order.iter().enumerate().map(|(new, old)| (*old, new as BlockId)).collect();
    let mut blocks = Vec::new();
    for old in &order {
        let mut block = function.blocks[*old as usize].clone();
        match &mut block.end {
            End::Jump(target, _) => *target = renumber[target],
            End::Branch(_, (a, _), (b, _)) => {
                *a = renumber[a];
                *b = renumber[b];
            }
            _ => {}
        }
        blocks.push(block);
    }
    function.blocks = blocks;
}

/// Replaces values through a map, following chains.
fn substitute(function: &mut Function, replace: &HashMap<Value, Value>) {
    if replace.is_empty() {
        return;
    }
    let resolve = |mut value: Value| {
        let mut steps = 0;
        while let Some(next) = replace.get(&value) {
            value = *next;
            steps += 1;
            if steps > 1000 {
                break;
            }
        }
        value
    };
    for block in &mut function.blocks {
        for instr in &mut block.instrs {
            instr.op.map_operands(&mut |value| resolve(value));
        }
        block.end.map_operands(&mut |value| resolve(value));
    }
}

/// A block param that every jump into the block gives the same value (or itself) is that value.
pub fn trivial_params(function: &mut Function) {
    loop {
        let mut incoming: Vec<Vec<Vec<Value>>> = vec![Vec::new(); function.blocks.len()];
        for block in &function.blocks {
            match &block.end {
                End::Jump(target, arguments) => incoming[*target as usize].push(arguments.clone()),
                End::Branch(_, (a, x), (b, y)) => {
                    incoming[*a as usize].push(x.clone());
                    incoming[*b as usize].push(y.clone());
                }
                _ => {}
            }
        }
        let mut replace = HashMap::new();
        let mut drop: HashMap<BlockId, Vec<usize>> = HashMap::new();
        for (index, block) in function.blocks.iter().enumerate() {
            if index == 0 {
                continue;
            }
            for (position, param) in block.params.iter().enumerate() {
                let mut only: Option<Value> = None;
                let mut trivial = true;
                for arguments in &incoming[index] {
                    let value = arguments[position];
                    if value == *param {
                        continue;
                    }
                    match only {
                        None => only = Some(value),
                        Some(seen) if seen == value => {}
                        Some(_) => {
                            trivial = false;
                            break;
                        }
                    }
                }
                if trivial && let Some(value) = only {
                    replace.insert(*param, value);
                    drop.entry(index as BlockId).or_default().push(position);
                }
            }
        }
        if replace.is_empty() {
            return;
        }
        for (block, positions) in &drop {
            let params = &mut function.blocks[*block as usize].params;
            for position in positions.iter().rev() {
                params.remove(*position);
            }
        }
        let fix = |target: BlockId, arguments: &mut Vec<Value>| {
            if let Some(positions) = drop.get(&target) {
                for position in positions.iter().rev() {
                    arguments.remove(*position);
                }
            }
        };
        for block in &mut function.blocks {
            match &mut block.end {
                End::Jump(target, arguments) => fix(*target, arguments),
                End::Branch(_, (a, x), (b, y)) => {
                    fix(*a, x);
                    fix(*b, y);
                }
                _ => {}
            }
        }
        substitute(function, &replace);
    }
}

fn constants(function: &Function) -> HashMap<Value, i64> {
    let mut found = HashMap::new();
    for block in &function.blocks {
        for instr in &block.instrs {
            if let (Some(result), Op::Constant(value)) = (instr.result, &instr.op) {
                found.insert(result, *value);
            }
        }
    }
    found
}

/// Works out maths on known numbers that can't fail, and branches on known tests.
pub fn fold(function: &mut Function) {
    let known = constants(function);
    for block in &mut function.blocks {
        for instr in &mut block.instrs {
            let folded = match &instr.op {
                Op::WholeMaths { operator, kind, a, b, .. } => match (known.get(a), known.get(b)) {
                    (Some(x), Some(y)) => interpreter::whole_maths(*operator, *kind, *x as u64, *y as u64).map(|v| v as i64),
                    _ => None,
                },
                Op::WholeCompare { operator, unsigned, a, b } => match (known.get(a), known.get(b)) {
                    (Some(x), Some(y)) => Some(interpreter::whole_compare(*operator, *unsigned, *x as u64, *y as u64) as i64),
                    _ => None,
                },
                Op::Not(a) => known.get(a).map(|x| (*x == 0) as i64),
                Op::Step1(a) => known.get(a).and_then(|x| x.checked_add(1)),
                _ => None,
            };
            if let Some(value) = folded {
                instr.op = Op::Constant(value);
            }
        }
        let known = &known;
        if let End::Branch(test, yes, no) = &block.end
            && let Some(value) = known.get(test).copied().or_else(|| {
                block.instrs.iter().find(|instr| instr.result == Some(*test)).and_then(|instr| match instr.op {
                    Op::Constant(value) => Some(value),
                    _ => None,
                })
            })
        {
            let (target, arguments) = if value != 0 { yes.clone() } else { no.clone() };
            block.end = End::Jump(target, arguments);
        }
    }
}

/// Removes pure instructions whose answers nothing uses.
pub fn dead_code(function: &mut Function) {
    loop {
        let mut used = HashSet::new();
        for block in &function.blocks {
            for instr in &block.instrs {
                used.extend(instr.op.operands());
            }
            used.extend(block.end.operands());
        }
        let mut removed = false;
        for block in &mut function.blocks {
            let before = block.instrs.len();
            block.instrs.retain(|instr| !(instr.op.is_pure() && instr.result.is_some_and(|result| !used.contains(&result))));
            removed |= block.instrs.len() != before;
        }
        if !removed {
            return;
        }
    }
}

pub fn quick(function: &mut Function) {
    prune(function);
    trivial_params(function);
    fold(function);
    prune(function);
    trivial_params(function);
    dead_code(function);
}

/// The range a whole kind holds, as `i64`.
pub fn limits(kind: u8) -> (i64, i64) {
    if kind == BIG_NEVER_NEGATIVE { (0, i64::MAX) } else { WHOLE_LIMITS[kind as usize] }
}

/// Every pass, for very hot code.
pub fn speed(program: &crate::bytecode::Program, function: &mut Function) {
    let debug = std::env::var("TAKL_DEBUG").is_ok();
    let check = |function: &Function, after: &str| {
        if debug && let Err(problem) = validate(function) {
            eprintln!("after {}: {}", after, problem);
        }
    };
    quick(function);
    check(function, "quick");
    inline_calls(program, function);
    check(function, "inline_calls");
    recursive_inline(program, function);
    check(function, "recursive_inline");
    quick(function);
    check(function, "quick again");
    checks_that_cannot_fail(function);
    check(function, "checks_that_cannot_fail");
    common_subexpressions(function);
    check(function, "common_subexpressions");
    fold(function);
    prune(function);
    trivial_params(function);
    dead_code(function);
    check(function, "the last passes");
}

// ---- the speed strength's own passes ----

use crate::bytecode::{BIG_WHOLE, Program};
use crate::syntax::Maths;

/// How big a callee may be, in IR instructions, to be inlined.
const INLINE_SIZE: usize = 80;
/// How many levels a recursive procedure is inlined into itself.
pub const RECURSIVE_LEVELS: usize = 2;
/// The most a function may grow to by inlining, in IR instructions.
const GROWTH_LIMIT: usize = 4_000;

fn size(function: &Function) -> usize {
    function.blocks.iter().map(|block| block.instrs.len() + 1).sum()
}

/// A callee's IR, built and tidied once for each time it is inlined.
fn callee_ir(program: &Program, callee: u32) -> Function {
    let mut callee_function = build(program, callee, Entry::Start);
    quick(&mut callee_function);
    callee_function
}

// trace: decision.compiler.recursive_inline
/// Inlines the call at `block`, `index`: the callee's blocks are copied in, its params made its arguments, its
/// frame placed after the caller's, and its returns made jumps to the rest of the caller's block. The call still
/// counts how deep calls go, so a program that goes too deep stops at the same call.
fn inline_at(program: &Program, function: &mut Function, block: usize, index: usize, callee_function: &Function) {
    let call = function.blocks[block].instrs[index].clone();
    let Op::Call { arguments, fail, .. } = &call.op else { return };
    let Some(result) = call.result else { return };
    let offset = fail.frame_offset + program.procedures[fail.procedure as usize].registers as u32;
    // The rest of the caller's block, waiting for the answer.
    let rest_block = function.blocks.len() as BlockId;
    let rest_instrs = function.blocks[block].instrs.split_off(index + 1);
    function.blocks[block].instrs.pop();
    let rest_end = std::mem::replace(&mut function.blocks[block].end, End::Return { value: None, heap: None, release: false });
    function.blocks.push(Block { params: vec![result], instrs: rest_instrs, end: rest_end });
    // The callee's blocks, with new values and block numbers.
    let first = function.blocks.len() as BlockId;
    let mut values: HashMap<Value, Value> = HashMap::new();
    for callee_block in &callee_function.blocks {
        for param in &callee_block.params {
            let made = function.new_value();
            values.insert(*param, made);
        }
        for instr in &callee_block.instrs {
            if let Some(found) = instr.result {
                let made = function.new_value();
                values.insert(found, made);
            }
        }
    }
    for callee_block in &callee_function.blocks {
        let mut instrs = Vec::new();
        for instr in &callee_block.instrs {
            let mut op = instr.op.clone();
            if let Op::Load(register) = op {
                // A param's value is its argument; anything else isn't read before it is made.
                match (arguments.get(register as usize), instr.result) {
                    (Some(argument), Some(result)) => {
                        values.insert(result, *argument);
                        continue;
                    }
                    _ => op = Op::Constant(0),
                }
            }
            match &mut op {
                Op::WholeMaths { fail, .. }
                | Op::WholeRemainder { fail, .. }
                | Op::CheckWhole { fail, .. }
                | Op::CheckBounds { fail, .. }
                | Op::CheckCount { fail, .. }
                | Op::FastMaths { fail, .. }
                | Op::Call { fail, .. }
                | Op::DepthEnter { fail } => {
                    fail.frame_offset += offset;
                }
                Op::Step { frame_offset, .. } => *frame_offset += offset,
                _ => {}
            }
            op.map_operands(&mut |value| values.get(&value).copied().unwrap_or(value));
            instrs.push(Instr { result: instr.result.map(|found| values[&found]), op });
        }
        let mut end = callee_block.end.clone();
        end.map_operands(&mut |value| values.get(&value).copied().unwrap_or(value));
        let end = match end {
            End::Jump(target, arguments) => End::Jump(target + first, arguments),
            End::Branch(test, (a, x), (b, y)) => End::Branch(test, (a + first, x), (b + first, y)),
            End::Return { value, .. } => {
                instrs.push(Instr { result: None, op: Op::DepthLeave });
                let answer = match value {
                    Some(value) => value,
                    None => {
                        let zero = function.new_value();
                        instrs.push(Instr { result: Some(zero), op: Op::Constant(0) });
                        zero
                    }
                };
                End::Jump(rest_block, vec![answer])
            }
            End::Fail(mut fail) => {
                fail.frame_offset += offset;
                End::Fail(fail)
            }
        };
        let params = callee_block.params.iter().map(|param| values[param]).collect();
        function.blocks.push(Block { params, instrs, end });
    }
    function.blocks[block].instrs.push(Instr { result: None, op: Op::DepthEnter { fail: fail.clone() } });
    function.blocks[block].end = End::Jump(first, Vec::new());
}

/// Finds calls in the function, as (block, index, callee), the way `pick` wants them.
fn calls(function: &Function, pick: impl Fn(u32) -> bool) -> Vec<(usize, usize, u32)> {
    let mut found = Vec::new();
    for (block, made) in function.blocks.iter().enumerate() {
        for (index, instr) in made.instrs.iter().enumerate() {
            if let Op::Call { callee, .. } = instr.op
                && pick(callee)
            {
                found.push((block, index, callee));
            }
        }
    }
    found
}

/// Inlines small calls to other procedures.
pub fn inline_calls(program: &Program, function: &mut Function) {
    let mut cache: HashMap<u32, Function> = HashMap::new();
    for _round in 0..2 {
        let own = function.procedure;
        let found = calls(function, |callee| callee != own);
        if found.is_empty() {
            return;
        }
        // From the last, so earlier places stay where they are.
        for (block, index, callee) in found.into_iter().rev() {
            let callee_function = cache.entry(callee).or_insert_with(|| callee_ir(program, callee)).clone();
            if size(&callee_function) > INLINE_SIZE || size(function) > GROWTH_LIMIT {
                continue;
            }
            inline_at(program, function, block, index, &callee_function);
        }
    }
}

// trace: decision.compiler.recursive_inline
/// Inlines a recursive procedure into itself, a few levels: each level's calls to itself become the next level.
pub fn recursive_inline(program: &Program, function: &mut Function) {
    if function.entry != Entry::Start {
        return;
    }
    let own = function.procedure;
    let body = callee_ir(program, own);
    if size(&body) > INLINE_SIZE {
        return;
    }
    // The calls the original body makes to itself, then the copies' calls, level by level.
    let mut level_calls: Vec<(usize, usize)> = calls(function, |callee| callee == own).into_iter().map(|(b, i, _)| (b, i)).collect();
    for _level in 0..RECURSIVE_LEVELS {
        if level_calls.is_empty() || size(function) > GROWTH_LIMIT {
            return;
        }
        let blocks_before = function.blocks.len();
        for (block, index) in level_calls.into_iter().rev() {
            inline_at(program, function, block, index, &body);
        }
        // The next level: self-calls inside the blocks just copied in.
        level_calls = calls(function, |callee| callee == own)
            .into_iter()
            .filter(|(block, _, _)| *block >= blocks_before)
            .map(|(block, index, _)| (block, index))
            .collect();
    }
}

type Range = (i128, i128);

fn arith_range(operator: Maths, a: Range, b: Range) -> Option<Range> {
    let corners = match operator {
        Maths::Plus => [a.0 + b.0, a.0 + b.1, a.1 + b.0, a.1 + b.1],
        Maths::Minus => [a.0 - b.1, a.0 - b.0, a.1 - b.1, a.1 - b.0],
        Maths::Times => [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1],
        Maths::DividedBy => return None,
    };
    Some((*corners.iter().min()?, *corners.iter().max()?))
}

/// Removes checks that can't fail, from what is known of each value's range: written numbers, answers of a kind,
/// remainders, and loop steps. A value from a block param is taken to be anything.
pub fn checks_that_cannot_fail(function: &mut Function) {
    let mut ranges: HashMap<Value, Range> = HashMap::new();
    for block in &mut function.blocks {
        for instr in &mut block.instrs {
            let range = |value: &Value| ranges.get(value).copied();
            let mut found: Option<Range> = None;
            let mut replace: Option<Op> = None;
            let mut remove = false;
            match &instr.op {
                Op::Constant(value) => found = Some((*value as i128, *value as i128)),
                Op::WholeCompare { .. } | Op::FastCompare { .. } | Op::Not(_) => found = Some((0, 1)),
                Op::Step1(a) => found = range(a).map(|(low, high)| (low + 1, high + 1)),
                Op::Arith { operator, a, b, .. } => found = range(a).zip(range(b)).and_then(|(x, y)| arith_range(*operator, x, y)),
                Op::WholeMaths { operator, kind, a, b, .. } => {
                    let (smallest, biggest) = limits(*kind);
                    let (smallest, biggest) = if *kind == crate::bytecode::BIG_NEVER_NEGATIVE { (0, u64::MAX as i128) } else { (smallest as i128, biggest as i128) };
                    found = Some((smallest, biggest));
                    if let (Some(x), Some(y)) = (range(a), range(b))
                        && let Some((low, high)) = arith_range(*operator, x, y)
                        && low >= smallest
                        && high <= biggest
                        && low >= i64::MIN as i128
                        && high <= i64::MAX as i128
                    {
                        replace = Some(Op::Arith { operator: *operator, unsigned: false, a: *a, b: *b });
                        found = Some((low, high));
                    }
                }
                Op::WholeRemainder { a, b, .. } => {
                    if let (Some(x), Some(y)) = (range(a), range(b))
                        && x.0 >= 0
                        && y.0 > 0
                    {
                        found = Some((0, y.1 - 1));
                    }
                }
                Op::CheckWhole { kind, value, .. } => {
                    let (smallest, biggest) = limits(*kind);
                    if let Some((low, high)) = range(value) {
                        remove = low >= smallest as i128 && high <= biggest as i128;
                    }
                }
                Op::CheckBounds { value, least, most, .. } => {
                    if let Some((low, high)) = range(value) {
                        remove = least.is_none_or(|least| low >= least as i128) && most.is_none_or(|most| high <= most as i128);
                    }
                }
                Op::CheckCount { value, .. } => remove = range(value).is_some_and(|(low, _)| low >= 0),
                _ => {}
            }
            if let Some(op) = replace {
                instr.op = op;
            }
            if remove {
                instr.op = Op::Constant(0);
                instr.result = None;
            }
            if let (Some(result), Some(found)) = (instr.result, found) {
                ranges.insert(result, found);
            }
        }
        block.instrs.retain(|instr| !(instr.result.is_none() && matches!(instr.op, Op::Constant(_))));
    }
    let _ = BIG_WHOLE;
}

/// One instruction for each pure computation repeated in a block.
pub fn common_subexpressions(function: &mut Function) {
    let mut replace = HashMap::new();
    for block in &mut function.blocks {
        let mut seen: HashMap<String, Value> = HashMap::new();
        for instr in &mut block.instrs {
            if !instr.op.is_pure() || matches!(instr.op, Op::Load(_)) {
                continue;
            }
            let Some(result) = instr.result else { continue };
            let mut op = instr.op.clone();
            op.map_operands(&mut |value| *replace.get(&value).unwrap_or(&value));
            let key = format!("{:?}", op);
            match seen.get(&key) {
                Some(earlier) => {
                    replace.insert(result, *earlier);
                }
                None => {
                    seen.insert(key, result);
                }
            }
        }
    }
    substitute(function, &replace);
}

/// Every value used is made somewhere: for finding a pass that broke the IR while developing.
pub fn validate(function: &Function) -> Result<(), String> {
    let mut made = HashSet::new();
    for block in &function.blocks {
        made.extend(block.params.iter().copied());
        for instr in &block.instrs {
            made.extend(instr.result);
        }
    }
    for (index, block) in function.blocks.iter().enumerate() {
        for instr in &block.instrs {
            for value in instr.op.operands() {
                if !made.contains(&value) {
                    return Err(format!("block {} uses v{} in {:?}", index, value, instr.op));
                }
            }
        }
        for value in block.end.operands() {
            if !made.contains(&value) {
                return Err(format!("block {} end uses v{}: {:?}", index, value, block.end));
            }
        }
        for target in block.end.successors() {
            let wanted = function.blocks[target as usize].params.len();
            let given = match &block.end {
                End::Jump(_, arguments) => arguments.len(),
                End::Branch(_, (a, x), (_, y)) => if *a == target { x.len() } else { y.len() },
                _ => 0,
            };
            if wanted != given {
                return Err(format!("block {} gives {} arguments to block {} which wants {}", index, given, target, wanted));
            }
        }
    }
    Ok(())
}
