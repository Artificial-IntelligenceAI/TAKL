//! Tables of answers for pure procedures over small bounded inputs.
//!
//! trace: decision.compiler.precompute

use crate::bytecode::Register;
use crate::interpreter::Machine;

pub struct Building {}

impl Building {
    pub fn look_up(&self, _machine: &mut Machine, _procedure: u32, _arguments: &[Register], _frame: *mut u64) -> Option<u64> {
        None
    }
}
