//! Tables of answers for pure procedures over small bounded inputs.
//!
//! trace: decision.compiler.precompute
//!
//! A procedure with no `share` param, no `show` and no input, calling only procedures like it, gives the same answer
//! for the same input, since a param is a copy. When its params are whole numbers (or true_or_false) whose bounds
//! leave few inputs, a table of its answers is built on another core, and a call becomes a lookup.
//!
//! The table is built from the smallest input up, and while building, the procedure's calls to itself look in what
//! is already built, so `fib` over 0 to 92 takes 93 short runs rather than exponential time.
//!
//! Output matches the oracle byte for byte, so the table changes nothing a kid can see:
//! - an input whose own run would stop with an error has no answer in the table, and its call runs for real, so it
//!   stops exactly where and how it would have (what a table should hold for it isn't designed);
//! - each answer keeps how many calls deep its run goes, and the lookup is used only when that still fits under
//!   10,000 calls, so a program that goes too deep stops at the same call;
//! - each entry's run has a budget of loop rounds and calls, and one that runs out has no answer either, so building
//!   can't hang on an input the program never asks for.

use crate::bytecode::{Held, Program, Register, RegisterKind};
use crate::interpreter::{MOST_CALLS, Machine};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The most a table may take, in bytes: 16 for each input. CyborgPL's `table_bytes` is 65,536 too.
pub const TABLE_BYTES: usize = 65_536;
const ENTRY_BYTES: usize = 16;
/// Loop rounds and calls each input's run may take while building.
pub const ENTRY_BUDGET: u64 = 1_000_000;
/// Set in an entry's `meta` when it has an answer; the low 32 bits hold how many calls deep its run goes.
pub const HAS_ANSWER: u64 = 1 << 63;

/// One input's answer: compiled code reads these, so the layout is fixed.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Entry {
    pub value: u64,
    pub meta: u64,
}

/// Each param's smallest input and how many inputs it has.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub lows: Vec<i64>,
    pub counts: Vec<u64>,
}

impl Shape {
    pub fn inputs(&self) -> u64 {
        self.counts.iter().product()
    }

    /// The entry for these arguments, or `None` when one is outside its param's range.
    pub fn index(&self, arguments: &[u64]) -> Option<usize> {
        let mut index = 0u64;
        for ((argument, low), count) in arguments.iter().zip(&self.lows).zip(&self.counts) {
            let offset = (*argument as i64).wrapping_sub(*low) as u64;
            if offset >= *count {
                return None;
            }
            index = index * count + offset;
        }
        Some(index as usize)
    }

    /// The arguments of entry `index`.
    fn arguments(&self, mut index: u64) -> Vec<u64> {
        let mut arguments = vec![0u64; self.counts.len()];
        for position in (0..self.counts.len()).rev() {
            let count = self.counts[position];
            arguments[position] = (self.lows[position] + (index % count) as i64) as u64;
            index /= count;
        }
        arguments
    }
}

pub struct Table {
    pub shape: Shape,
    pub entries: Vec<Entry>,
}

/// Whether a procedure gets a table, and its shape.
pub fn eligible(program: &Program, procedure: u32) -> Option<Shape> {
    let made = &program.procedures[procedure as usize];
    if !made.pure || procedure == program.start || made.params == 0 {
        return None;
    }
    let answer_held = made.returns.as_ref().map(crate::bytecode::held)?;
    if matches!(answer_held, Held::Heap) {
        return None;
    }
    let mut lows = Vec::new();
    let mut counts = Vec::new();
    for range in &made.param_ranges {
        let (low, high) = (*range)?;
        let count = (high - low + 1) as u128;
        lows.push(low as i64);
        counts.push(u64::try_from(count).ok()?);
    }
    let shape = Shape { lows, counts };
    let bytes = (shape.inputs() as u128) * ENTRY_BYTES as u128;
    if bytes > TABLE_BYTES as u128 {
        return None;
    }
    // Every param is a number register, so the arguments can be read from a frame.
    if (0..made.params).any(|param| made.kinds[param] == RegisterKind::Heap) {
        return None;
    }
    Some(shape)
}

/// While a table is built: the procedure, and what is built so far.
pub struct Building {
    procedure: u32,
    shape: Shape,
    entries: RefCell<Vec<Option<Entry>>>,
}

impl Building {
    /// During building, the procedure's calls to itself use the answers already built.
    /// # Safety
    /// `frame` is the caller's frame, and `arguments` name registers inside it.
    pub unsafe fn look_up(
        &self,
        machine: &mut Machine,
        callee: u32,
        arguments: &[Register],
        frame: *mut u64,
    ) -> Option<u64> {
        if callee != self.procedure {
            return None;
        }
        // SAFETY: `arguments` are registers of the caller's frame, checked by `verify`.
        let values: Vec<u64> = arguments.iter().map(|register| unsafe { *frame.add(*register as usize) }).collect();
        let index = self.shape.index(&values)?;
        let entry = (*self.entries.borrow().get(index)?)?;
        if entry.meta & HAS_ANSWER == 0 {
            return None;
        }
        let deepest = machine.depth + (entry.meta & 0xffff_ffff);
        if deepest > MOST_CALLS {
            return None;
        }
        machine.deepest = machine.deepest.max(deepest);
        Some(entry.value)
    }
}

/// Builds a procedure's table, on whatever thread calls it.
pub fn build(program: &Arc<Program>, procedure: u32, shape: Shape) -> Table {
    let inputs = shape.inputs() as usize;
    let building = Rc::new(Building { procedure, shape: shape.clone(), entries: RefCell::new(vec![None; inputs]) });
    let mut machine = Machine::new(Arc::clone(program), false);
    machine.building = Some(Rc::clone(&building));
    for index in 0..inputs {
        let arguments = shape.arguments(index as u64);
        machine.depth = 1;
        machine.deepest = 1;
        machine.budget = ENTRY_BUDGET;
        machine.failure = None;
        let entry = match machine.call_with(procedure, &arguments) {
            Ok(value) => Entry { value, meta: HAS_ANSWER | machine.deepest.min(u32::MAX as u64) },
            Err(_) => Entry::default(),
        };
        building.entries.borrow_mut()[index] = Some(entry);
    }
    let entries = building.entries.borrow().iter().map(|entry| entry.unwrap_or_default()).collect();
    Table { shape, entries }
}
