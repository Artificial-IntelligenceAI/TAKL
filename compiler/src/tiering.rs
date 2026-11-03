//! Decides what gets compiled, and runs compiled code.
//!
//! trace: decision.backend.tiers

use crate::interpreter::{Machine, Stopped};

pub struct Tiers {}

impl Tiers {
    pub fn call(&self, _machine: &mut Machine, _procedure: u32, _base: usize) -> Option<Result<u64, Stopped>> {
        None
    }

    pub fn loop_is_hot(&self, _procedure: u32, _loop_id: u16) -> bool {
        false
    }

    pub fn jump_in(&self, _machine: &mut Machine, _procedure: u32, _loop_id: u16, _base: usize) -> Option<Result<u64, Stopped>> {
        None
    }
}

/// Starts the tiers a mode asks for; the interpreter alone needs none.
pub fn start(_program: &std::sync::Arc<crate::bytecode::Program>, mode: crate::Mode) -> Option<std::sync::Arc<Tiers>> {
    match mode {
        crate::Mode::Interpret => None,
        _ => None,
    }
}
