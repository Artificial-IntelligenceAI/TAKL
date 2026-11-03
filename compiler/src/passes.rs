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
pub fn speed(_program: &crate::bytecode::Program, function: &mut Function) {
    quick(function);
}
