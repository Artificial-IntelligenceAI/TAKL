//! Lowers TAKL's IR to Cranelift, at its quick or its speed strength, and the helpers compiled code calls.
//!
//! trace: decision.backend.cranelift
//! trace: decision.backend.two_strengths
//!
//! A compiled procedure is `fn(machine, frame) -> Answer`, the same frame the interpreter uses, so the interpreter
//! can call compiled code, compiled code can call the interpreter, and a loop can be jumped into mid-way. Number
//! registers live in Cranelift values; heap registers stay in the frame. A check that fails jumps to a cold block
//! that stores the `because:` values into the frame and calls the helper that makes the message, so a stop in
//! compiled code says what the interpreter says.

use crate::bytecode::{BIG_NEVER_NEGATIVE, BIG_WHOLE, FailureKind, Program};
use crate::interpreter::{Detail, MOST_CALLS, Machine};
use crate::ir::{self, End, FailPoint, Op};
use crate::numbers::{self, Size};
use crate::passes;
use crate::syntax::{Compare, Maths};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{self as clir, AbiParam, BlockArg, InstBuilder, MemFlagsData, Signature, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use num_bigint::BigInt;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

/// What compiled code gives back: 0 and the answer, or 1 when the program stopped.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Answer {
    pub status: u64,
    pub value: u64,
}

pub type Compiled = unsafe extern "C" fn(*mut Machine, *mut u64) -> Answer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Strength {
    /// Few passes, and Cranelift's quick mode: for warm code.
    Quick,
    /// Every pass, and Cranelift's speed mode: for very hot code.
    Speed,
}

// ---- helpers compiled code calls ----

unsafe fn machine<'a>(machine: *mut Machine) -> &'a mut Machine {
    // SAFETY: compiled code only ever passes the machine it was entered with.
    unsafe { &mut *machine }
}

/// Runs one bytecode instruction that has no inline form.
unsafe extern "C" fn takl_step(machine_pointer: *mut Machine, procedure: u64, at: u64, frame: *mut u64) -> u64 {
    let machine = unsafe { machine(machine_pointer) };
    match unsafe { machine.step(procedure as u32, at as usize, frame) } {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Whole-number maths with no answer; `operator` 4 is `remainder of`.
unsafe extern "C" fn takl_fail_whole(machine_pointer: *mut Machine, procedure: u64, failure: u64, operator: u64, kind: u64, x: u64, y: u64, frame: *mut u64) {
    let machine = unsafe { machine(machine_pointer) };
    let program = std::sync::Arc::clone(&machine.program);
    let procedure = &program.procedures[procedure as usize];
    let kind = kind as u8;
    let problem = if operator == 4 {
        let a = BigInt::from(crate::runtime::whole_value(kind, x));
        let b = BigInt::from(crate::runtime::whole_value(kind, y));
        numbers::whole_remainder(&a, &b).err().unwrap_or(numbers::Problem::DividesByZero)
    } else {
        let operator = [Maths::Plus, Maths::Minus, Maths::Times, Maths::DividedBy][operator as usize % 4];
        crate::interpreter::whole_problem(operator, kind, x, y)
    };
    machine.stop(procedure, failure as u16, Detail::Problem(problem), frame);
}

/// A check on one value: a whole number's range, bounds, or a repeat's count.
unsafe extern "C" fn takl_fail_value(machine_pointer: *mut Machine, procedure: u64, failure: u64, value: u64, frame: *mut u64) {
    let machine = unsafe { machine(machine_pointer) };
    let program = std::sync::Arc::clone(&machine.program);
    let procedure = &program.procedures[procedure as usize];
    let detail = match &procedure.failures[failure as usize].kind {
        FailureKind::Maths { ty: crate::types::Type::Whole(whole) } => {
            Detail::Problem(whole.check(value as i64 as i128).err().unwrap_or(numbers::Problem::TooBig(BigInt::from(value as i64))))
        }
        _ => Detail::Value(value as i64 as i128),
    };
    machine.stop(procedure, failure as u16, detail, frame);
}

/// Float maths with no answer: dividing by zero, or too big.
unsafe extern "C" fn takl_fail_fast(machine_pointer: *mut Machine, procedure: u64, failure: u64, divides_by_zero: u64, frame: *mut u64) {
    let machine = unsafe { machine(machine_pointer) };
    let program = std::sync::Arc::clone(&machine.program);
    let procedure = &program.procedures[procedure as usize];
    let problem = if divides_by_zero != 0 { numbers::Problem::DividesByZero } else { numbers::Problem::FastTooBig };
    machine.stop(procedure, failure as u16, Detail::Problem(problem), frame);
}

/// A stop with nothing more to say: calls too deep, or written maths that fails.
unsafe extern "C" fn takl_fail_plain(machine_pointer: *mut Machine, procedure: u64, failure: u64, frame: *mut u64) {
    let machine = unsafe { machine(machine_pointer) };
    let program = std::sync::Arc::clone(&machine.program);
    let procedure = &program.procedures[procedure as usize];
    machine.stop(procedure, failure as u16, Detail::Nothing, frame);
}

/// Calls a procedure that isn't compiled, or is compiled elsewhere, from compiled code.
unsafe extern "C" fn takl_enter(machine_pointer: *mut Machine, callee: u64, callee_frame: *mut u64) -> Answer {
    let machine = unsafe { machine(machine_pointer) };
    let base = (callee_frame as usize - machine.frame_at(0) as usize) / 8;
    match machine.enter(callee as u32, base) {
        Ok(value) => Answer { status: 0, value },
        Err(_) => Answer { status: 1, value: 0 },
    }
}

/// Releases a procedure's heap registers as it ends.
unsafe extern "C" fn takl_release_frame(machine_pointer: *mut Machine, procedure: u64, frame: *mut u64) {
    let machine = unsafe { machine(machine_pointer) };
    let program = std::sync::Arc::clone(&machine.program);
    for register in &program.procedures[procedure as usize].heap_registers {
        // SAFETY: the frame is this procedure's, checked by `verify`.
        unsafe {
            let slot = frame.add(*register as usize);
            crate::runtime::release(*slot);
            *slot = 0;
        }
    }
}

unsafe extern "C" fn takl_retain(value: u64) {
    crate::runtime::retain(value);
}

// ---- lowering ----

/// How a failing check says what happened.
enum Helper {
    Whole { operator: u64, kind: u8, x: clir::Value, y: clir::Value },
    Value(clir::Value),
    Fast { divides: u8 },
    Plain,
}

struct Lowering<'a, 'b> {
    pending: Vec<(clir::Block, FailPoint, Helper)>,
    builder: FunctionBuilder<'b>,
    program: &'a Program,
    function: &'a ir::Function,
    values: HashMap<ir::Value, clir::Value>,
    blocks: Vec<clir::Block>,
    machine: clir::Value,
    frame: clir::Value,
    pointer: clir::Type,
    entries: *const AtomicUsize,
    self_reference: Option<clir::FuncRef>,
    helper_signatures: HashMap<usize, clir::SigRef>,
    compiled_signature: clir::SigRef,
}

fn int_condition(operator: Compare, unsigned: bool) -> IntCC {
    match (operator, unsigned) {
        (Compare::Is, _) => IntCC::Equal,
        (Compare::IsNot, _) => IntCC::NotEqual,
        (Compare::MoreThan, false) => IntCC::SignedGreaterThan,
        (Compare::LessThan, false) => IntCC::SignedLessThan,
        (Compare::AtLeast, false) => IntCC::SignedGreaterThanOrEqual,
        (Compare::AtMost, false) => IntCC::SignedLessThanOrEqual,
        (Compare::MoreThan, true) => IntCC::UnsignedGreaterThan,
        (Compare::LessThan, true) => IntCC::UnsignedLessThan,
        (Compare::AtLeast, true) => IntCC::UnsignedGreaterThanOrEqual,
        (Compare::AtMost, true) => IntCC::UnsignedLessThanOrEqual,
    }
}

fn float_condition(operator: Compare) -> FloatCC {
    match operator {
        Compare::Is => FloatCC::Equal,
        Compare::IsNot => FloatCC::NotEqual,
        Compare::MoreThan => FloatCC::GreaterThan,
        Compare::LessThan => FloatCC::LessThan,
        Compare::AtLeast => FloatCC::GreaterThanOrEqual,
        Compare::AtMost => FloatCC::LessThanOrEqual,
    }
}

impl<'a, 'b> Lowering<'a, 'b> {
    fn value(&self, value: ir::Value) -> clir::Value {
        self.values[&value]
    }

    fn arguments(&self, values: &[ir::Value]) -> Vec<BlockArg> {
        values.iter().map(|value| BlockArg::Value(self.value(*value))).collect()
    }

    fn helper_signature(&mut self, arguments: usize, answers: usize) -> clir::SigRef {
        let key = arguments * 10 + answers;
        if let Some(signature) = self.helper_signatures.get(&key) {
            return *signature;
        }
        let call_conv = self.builder.func.signature.call_conv;
        let mut signature = Signature::new(call_conv);
        for _ in 0..arguments {
            signature.params.push(AbiParam::new(types::I64));
        }
        for _ in 0..answers {
            signature.returns.push(AbiParam::new(types::I64));
        }
        let made = self.builder.import_signature(signature);
        self.helper_signatures.insert(key, made);
        made
    }

    /// Calls a helper at a fixed address.
    fn helper(&mut self, address: usize, arguments: &[clir::Value], answers: usize) -> Vec<clir::Value> {
        let signature = self.helper_signature(arguments.len(), answers);
        let callee = self.builder.ins().iconst(self.pointer, address as i64);
        let call = self.builder.ins().call_indirect(signature, callee, arguments);
        self.builder.inst_results(call).to_vec()
    }

    fn constant(&mut self, value: i64) -> clir::Value {
        self.builder.ins().iconst(types::I64, value)
    }

    /// The frame of the procedure a fail point is in.
    fn frame_of(&mut self, frame_offset: u32) -> clir::Value {
        if frame_offset == 0 { self.frame } else { self.builder.ins().iadd_imm(self.frame, frame_offset as i64 * 8) }
    }

    fn stopped(&mut self) {
        let one = self.constant(1);
        let zero = self.constant(0);
        self.builder.ins().return_(&[one, zero]);
    }

    /// A cold block for a failing check, filled after the procedure's own blocks: it stores the `because:` values
    /// and calls the helper that says what happened.
    fn fail_block(&mut self, fail: &FailPoint, helper: Helper) -> clir::Block {
        let block = self.builder.create_block();
        self.builder.set_cold_block(block);
        self.pending.push((block, fail.clone(), helper));
        block
    }

    fn fill_failures(&mut self) {
        for (block, fail, helper) in std::mem::take(&mut self.pending) {
            self.builder.switch_to_block(block);
            let frame = self.frame_of(fail.frame_offset);
            for (register, value) in &fail.because {
                let value = self.value(*value);
                self.builder.ins().store(MemFlagsData::trusted(), value, frame, *register as i32 * 8);
            }
            let procedure = self.constant(fail.procedure as i64);
            let failure = self.constant(fail.failure as i64);
            match helper {
                Helper::Whole { operator, kind, x, y } => {
                    let operator = self.constant(operator as i64);
                    let kind = self.constant(kind as i64);
                    self.helper(takl_fail_whole as usize, &[self.machine, procedure, failure, operator, kind, x, y, frame], 0);
                }
                Helper::Value(value) => {
                    self.helper(takl_fail_value as usize, &[self.machine, procedure, failure, value, frame], 0);
                }
                Helper::Fast { divides } => {
                    let divides = self.constant(divides as i64);
                    self.helper(takl_fail_fast as usize, &[self.machine, procedure, failure, divides, frame], 0);
                }
                Helper::Plain => {
                    self.helper(takl_fail_plain as usize, &[self.machine, procedure, failure, frame], 0);
                }
            }
            self.stopped();
        }
    }

    /// Goes on when `ok` holds, else to `failed`.
    fn check(&mut self, failing: clir::Value, failed: clir::Block) {
        let next = self.builder.create_block();
        self.builder.ins().brif(failing, failed, &[], next, &[]);
        self.builder.switch_to_block(next);
    }

    fn whole_fail(&mut self, fail: &FailPoint, operator: u64, kind: u8, x: clir::Value, y: clir::Value) -> clir::Block {
        self.fail_block(fail, Helper::Whole { operator, kind, x, y })
    }

    fn value_fail(&mut self, fail: &FailPoint, value: clir::Value) -> clir::Block {
        self.fail_block(fail, Helper::Value(value))
    }

    fn plain_fail(&mut self, fail: &FailPoint) -> clir::Block {
        self.fail_block(fail, Helper::Plain)
    }

    fn whole_maths(&mut self, operator: Maths, kind: u8, x: clir::Value, y: clir::Value, fail: &FailPoint) -> clir::Value {
        let failed = self.whole_fail(fail, operator as u64, kind, x, y);
        let b = &mut self.builder;
        if kind == BIG_WHOLE || kind == BIG_NEVER_NEGATIVE {
            let unsigned = kind == BIG_NEVER_NEGATIVE;
            let (answer, overflow) = match (operator, unsigned) {
                (Maths::Plus, false) => b.ins().sadd_overflow(x, y),
                (Maths::Minus, false) => b.ins().ssub_overflow(x, y),
                (Maths::Times, false) => b.ins().smul_overflow(x, y),
                (Maths::Plus, true) => b.ins().uadd_overflow(x, y),
                (Maths::Minus, true) => b.ins().usub_overflow(x, y),
                (Maths::Times, true) => b.ins().umul_overflow(x, y),
                (Maths::DividedBy, _) => return self.whole_divide(x, y, unsigned, None, failed),
            };
            self.check(overflow, failed);
            return answer;
        }
        let answer = match operator {
            Maths::Plus => b.ins().iadd(x, y),
            Maths::Minus => b.ins().isub(x, y),
            Maths::Times => {
                let (answer, overflow) = b.ins().smul_overflow(x, y);
                self.check(overflow, failed);
                answer
            }
            Maths::DividedBy => return self.whole_divide(x, y, false, Some(kind), failed),
        };
        self.range_check(answer, kind, failed);
        answer
    }

    /// Fails unless `value` is in a narrow kind's range: one subtraction and one unsigned comparison.
    fn range_check(&mut self, value: clir::Value, kind: u8, failed: clir::Block) {
        let (smallest, biggest) = passes::limits(kind);
        let shifted = self.builder.ins().iadd_imm(value, smallest.wrapping_neg());
        let span = biggest.wrapping_sub(smallest);
        let outside = self.builder.ins().icmp_imm(IntCC::UnsignedGreaterThan, shifted, span);
        self.check(outside, failed);
    }

    fn whole_divide(&mut self, x: clir::Value, y: clir::Value, unsigned: bool, narrow: Option<u8>, failed: clir::Block) -> clir::Value {
        let zero = self.builder.ins().icmp_imm(IntCC::Equal, y, 0);
        self.check(zero, failed);
        if unsigned {
            let rest = self.builder.ins().urem(x, y);
            let uneven = self.builder.ins().icmp_imm(IntCC::NotEqual, rest, 0);
            self.check(uneven, failed);
            return self.builder.ins().udiv(x, y);
        }
        if narrow.is_none() {
            // i64::MIN divided by -1 doesn't fit.
            let smallest = self.builder.ins().icmp_imm(IntCC::Equal, x, i64::MIN);
            let minus_one = self.builder.ins().icmp_imm(IntCC::Equal, y, -1);
            let both = self.builder.ins().band(smallest, minus_one);
            self.check(both, failed);
        }
        let rest = self.builder.ins().srem(x, y);
        let uneven = self.builder.ins().icmp_imm(IntCC::NotEqual, rest, 0);
        self.check(uneven, failed);
        let answer = self.builder.ins().sdiv(x, y);
        if let Some(kind) = narrow {
            self.range_check(answer, kind, failed);
        }
        answer
    }

    fn fast_maths(&mut self, operator: Maths, size: Size, x: clir::Value, y: clir::Value, fail: &FailPoint) -> clir::Value {
        let fast_fail = |this: &mut Self, divides: u8| this.fail_block(fail, Helper::Fast { divides });
        let float = if size == Size::Small { types::F32 } else { types::F64 };
        let (x, y) = if size == Size::Small {
            let x = self.builder.ins().ireduce(types::I32, x);
            let y = self.builder.ins().ireduce(types::I32, y);
            (x, y)
        } else {
            (x, y)
        };
        let fx = self.builder.ins().bitcast(float, MemFlagsData::new(), x);
        let fy = self.builder.ins().bitcast(float, MemFlagsData::new(), y);
        let answer = match operator {
            Maths::Plus => self.builder.ins().fadd(fx, fy),
            Maths::Minus => self.builder.ins().fsub(fx, fy),
            Maths::Times => self.builder.ins().fmul(fx, fy),
            Maths::DividedBy => {
                let zero = if size == Size::Small { self.builder.ins().f32const(0.0) } else { self.builder.ins().f64const(0.0) };
                let is_zero = self.builder.ins().fcmp(FloatCC::Equal, fy, zero);
                let failed = fast_fail(self, 1);
                self.check(is_zero, failed);
                self.builder.ins().fdiv(fx, fy)
            }
        };
        let magnitude = self.builder.ins().fabs(answer);
        let infinity = if size == Size::Small { self.builder.ins().f32const(f32::INFINITY) } else { self.builder.ins().f64const(f64::INFINITY) };
        let too_big = self.builder.ins().fcmp(FloatCC::Equal, magnitude, infinity);
        let failed = fast_fail(self, 0);
        self.check(too_big, failed);
        if size == Size::Small {
            let bits = self.builder.ins().bitcast(types::I32, MemFlagsData::new(), answer);
            self.builder.ins().uextend(types::I64, bits)
        } else {
            self.builder.ins().bitcast(types::I64, MemFlagsData::new(), answer)
        }
    }

    fn fast_compare(&mut self, operator: Compare, size: Size, x: clir::Value, y: clir::Value) -> clir::Value {
        let float = if size == Size::Small { types::F32 } else { types::F64 };
        let (x, y) = if size == Size::Small {
            (self.builder.ins().ireduce(types::I32, x), self.builder.ins().ireduce(types::I32, y))
        } else {
            (x, y)
        };
        let fx = self.builder.ins().bitcast(float, MemFlagsData::new(), x);
        let fy = self.builder.ins().bitcast(float, MemFlagsData::new(), y);
        let holds = self.builder.ins().fcmp(float_condition(operator), fx, fy);
        self.builder.ins().uextend(types::I64, holds)
    }

    /// Checks how deep calls go, and counts one more; gives back the depth before.
    fn depth_enter(&mut self, fail: &FailPoint) -> clir::Value {
        let depth = self.builder.ins().load(types::I64, MemFlagsData::trusted(), self.machine, 0);
        let too_deep = self.builder.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, depth, MOST_CALLS as i64);
        let failed = self.plain_fail(fail);
        self.check(too_deep, failed);
        let deeper = self.builder.ins().iadd_imm(depth, 1);
        self.builder.ins().store(MemFlagsData::trusted(), deeper, self.machine, 0);
        depth
    }

    fn call(&mut self, callee: u32, arguments: &[clir::Value], fail: &FailPoint) -> clir::Value {
        let depth = self.depth_enter(fail);
        let caller_registers = self.program.procedures[fail.procedure as usize].registers as i64;
        let callee_frame = self.builder.ins().iadd_imm(self.frame, (fail.frame_offset as i64 + caller_registers) * 8);
        for (index, argument) in arguments.iter().enumerate() {
            self.builder.ins().store(MemFlagsData::trusted(), *argument, callee_frame, index as i32 * 8);
        }
        let answer = if let Some(self_reference) = self.self_reference.filter(|_| callee == self.function.procedure) {
            let call = self.builder.ins().call(self_reference, &[self.machine, callee_frame]);
            self.builder.inst_results(call).to_vec()
        } else {
            // Compiled code for the callee, if it is ready; else the interpreter.
            // SAFETY: `entries` lives as long as the program's tiers, which outlive all compiled code.
            let slot = unsafe { self.entries.add(callee as usize) } as i64;
            let slot = self.builder.ins().iconst(self.pointer, slot);
            let entry = self.builder.ins().load(self.pointer, MemFlagsData::trusted(), slot, 0);
            let compiled_block = self.builder.create_block();
            let interpreted_block = self.builder.create_block();
            let after = self.builder.create_block();
            self.builder.append_block_param(after, types::I64);
            self.builder.append_block_param(after, types::I64);
            self.builder.ins().brif(entry, compiled_block, &[], interpreted_block, &[]);
            self.builder.switch_to_block(compiled_block);
            let call = self.builder.ins().call_indirect(self.compiled_signature, entry, &[self.machine, callee_frame]);
            let results: Vec<BlockArg> = self.builder.inst_results(call).iter().map(|value| BlockArg::Value(*value)).collect();
            self.builder.ins().jump(after, &results);
            self.builder.switch_to_block(interpreted_block);
            let callee_constant = self.constant(callee as i64);
            let results = self.helper(takl_enter as usize, &[self.machine, callee_constant, callee_frame], 2);
            let results: Vec<BlockArg> = results.iter().map(|value| BlockArg::Value(*value)).collect();
            self.builder.ins().jump(after, &results);
            self.builder.switch_to_block(after);
            self.builder.block_params(after).to_vec()
        };
        self.builder.ins().store(MemFlagsData::trusted(), depth, self.machine, 0);
        let (status, value) = (answer[0], answer[1]);
        let stop = self.builder.create_block();
        self.builder.set_cold_block(stop);
        let next = self.builder.create_block();
        self.builder.ins().brif(status, stop, &[], next, &[]);
        self.builder.switch_to_block(stop);
        self.stopped();
        self.builder.switch_to_block(next);
        value
    }

    fn op(&mut self, op: &Op) -> Option<clir::Value> {
        Some(match op {
            Op::Constant(value) => self.constant(*value),
            Op::Load(register) => self.builder.ins().load(types::I64, MemFlagsData::trusted(), self.frame, *register as i32 * 8),
            Op::Step1(value) => {
                let value = self.value(*value);
                self.builder.ins().iadd_imm(value, 1)
            }
            Op::WholeMaths { operator, kind, a, b, fail } => {
                let (x, y) = (self.value(*a), self.value(*b));
                self.whole_maths(*operator, *kind, x, y, fail)
            }
            Op::WholeRemainder { kind, a, b, fail } => {
                let (x, y) = (self.value(*a), self.value(*b));
                let failed = self.whole_fail(fail, 4, *kind, x, y);
                let negative = if *kind == BIG_NEVER_NEGATIVE {
                    self.builder.ins().icmp_imm(IntCC::Equal, y, 0)
                } else {
                    let below = self.builder.ins().icmp_imm(IntCC::SignedLessThan, x, 0);
                    let bottom = self.builder.ins().icmp_imm(IntCC::SignedLessThanOrEqual, y, 0);
                    self.builder.ins().bor(below, bottom)
                };
                self.check(negative, failed);
                if *kind == BIG_NEVER_NEGATIVE { self.builder.ins().urem(x, y) } else { self.builder.ins().srem(x, y) }
            }
            Op::WholeCompare { operator, unsigned, a, b } => {
                let (x, y) = (self.value(*a), self.value(*b));
                let holds = self.builder.ins().icmp(int_condition(*operator, *unsigned), x, y);
                self.builder.ins().uextend(types::I64, holds)
            }
            Op::CheckWhole { kind, value, fail } => {
                let value = self.value(*value);
                let failed = self.value_fail(fail, value);
                if *kind == BIG_NEVER_NEGATIVE {
                    let below = self.builder.ins().icmp_imm(IntCC::SignedLessThan, value, 0);
                    self.check(below, failed);
                } else {
                    self.range_check(value, *kind, failed);
                }
                return None;
            }
            Op::CheckBounds { value, least, most, fail } => {
                let value = self.value(*value);
                let failed = self.value_fail(fail, value);
                if let Some(least) = least {
                    let below = self.builder.ins().icmp_imm(IntCC::SignedLessThan, value, *least);
                    self.check(below, failed);
                }
                if let Some(most) = most {
                    let above = self.builder.ins().icmp_imm(IntCC::SignedGreaterThan, value, *most);
                    self.check(above, failed);
                }
                return None;
            }
            Op::CheckCount { value, fail } => {
                let value = self.value(*value);
                let failed = self.value_fail(fail, value);
                let below = self.builder.ins().icmp_imm(IntCC::SignedLessThan, value, 0);
                self.check(below, failed);
                return None;
            }
            Op::FastMaths { operator, size, a, b, fail } => {
                let (x, y) = (self.value(*a), self.value(*b));
                self.fast_maths(*operator, *size, x, y, fail)
            }
            Op::FastCompare { operator, size, a, b } => {
                let (x, y) = (self.value(*a), self.value(*b));
                self.fast_compare(*operator, *size, x, y)
            }
            Op::Not(value) => {
                let value = self.value(*value);
                let holds = self.builder.ins().icmp_imm(IntCC::Equal, value, 0);
                self.builder.ins().uextend(types::I64, holds)
            }
            Op::Call { callee, arguments, fail } => {
                let arguments: Vec<clir::Value> = arguments.iter().map(|value| self.value(*value)).collect();
                self.call(*callee, &arguments, fail)
            }
            Op::Step { procedure, at, reads, write, frame_offset } => {
                let frame = self.frame_of(*frame_offset);
                for (register, value) in reads {
                    let value = self.value(*value);
                    self.builder.ins().store(MemFlagsData::trusted(), value, frame, *register as i32 * 8);
                }
                let procedure = self.constant(*procedure as i64);
                let at = self.constant(*at as i64);
                let status = self.helper(takl_step as usize, &[self.machine, procedure, at, frame], 1)[0];
                let stop = self.builder.create_block();
                self.builder.set_cold_block(stop);
                let next = self.builder.create_block();
                self.builder.ins().brif(status, stop, &[], next, &[]);
                self.builder.switch_to_block(stop);
                self.stopped();
                self.builder.switch_to_block(next);
                match write {
                    Some(register) => self.builder.ins().load(types::I64, MemFlagsData::trusted(), frame, *register as i32 * 8),
                    None => return None,
                }
            }
            Op::DepthEnter { fail } => {
                let _ = self.depth_enter(fail);
                return None;
            }
            Op::DepthLeave => {
                let depth = self.builder.ins().load(types::I64, MemFlagsData::trusted(), self.machine, 0);
                let shallower = self.builder.ins().iadd_imm(depth, -1);
                self.builder.ins().store(MemFlagsData::trusted(), shallower, self.machine, 0);
                return None;
            }
        })
    }

    fn end(&mut self, end: &End) {
        match end {
            End::Jump(target, arguments) => {
                let arguments = self.arguments(arguments);
                let block = self.blocks[*target as usize];
                self.builder.ins().jump(block, &arguments);
            }
            End::Branch(test, (yes, yes_arguments), (no, no_arguments)) => {
                let test = self.value(*test);
                let yes_arguments = self.arguments(yes_arguments);
                let no_arguments = self.arguments(no_arguments);
                let (yes, no) = (self.blocks[*yes as usize], self.blocks[*no as usize]);
                self.builder.ins().brif(test, yes, &yes_arguments, no, &no_arguments);
            }
            End::Return { value, heap, release } => {
                let answer = match (value, heap) {
                    (Some(value), _) => self.value(*value),
                    (None, Some(register)) => {
                        let held = self.builder.ins().load(types::I64, MemFlagsData::trusted(), self.frame, *register as i32 * 8);
                        self.helper(takl_retain as usize, &[held], 0);
                        held
                    }
                    (None, None) => self.constant(0),
                };
                if *release {
                    let procedure = self.constant(self.function.procedure as i64);
                    self.helper(takl_release_frame as usize, &[self.machine, procedure, self.frame], 0);
                }
                let zero = self.constant(0);
                self.builder.ins().return_(&[zero, answer]);
            }
            End::Fail(fail) => {
                let failed = self.plain_fail(fail);
                self.builder.ins().jump(failed, &[]);
            }
        }
    }
}

/// Compiles a procedure's IR at a strength, giving back the address of its code.
pub fn compile(program: &Program, function: &ir::Function, strength: Strength, entries: *const AtomicUsize) -> Result<usize, String> {
    let mut flags = settings::builder();
    let opt_level = match strength {
        Strength::Quick => "none",
        Strength::Speed => "speed",
    };
    flags.set("opt_level", opt_level).map_err(|error| error.to_string())?;
    flags.set("enable_verifier", if cfg!(debug_assertions) { "true" } else { "false" }).map_err(|error| error.to_string())?;
    flags.set("use_colocated_libcalls", "false").map_err(|error| error.to_string())?;
    flags.set("is_pic", "false").map_err(|error| error.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|error| error.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|error| error.to_string())?;
    let pointer = isa.pointer_type();
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(types::I64));
    signature.params.push(AbiParam::new(types::I64));
    signature.returns.push(AbiParam::new(types::I64));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("procedure", Linkage::Local, &signature).map_err(|error| error.to_string())?;
    let mut context = module.make_context();
    context.func.signature = signature.clone();
    let mut builder_context = FunctionBuilderContext::new();
    let builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let mut lowering = Lowering {
        pending: Vec::new(),
        builder,
        program,
        function,
        values: HashMap::new(),
        blocks: Vec::new(),
        machine: clir::Value::from_u32(0),
        frame: clir::Value::from_u32(0),
        pointer,
        entries,
        self_reference: None,
        helper_signatures: HashMap::new(),
        compiled_signature: clir::SigRef::from_u32(0),
    };
    lowering.compiled_signature = lowering.builder.import_signature(signature);
    if function.entry == ir::Entry::Start {
        lowering.self_reference = Some(module.declare_func_in_func(id, lowering.builder.func));
    }
    for (index, block) in function.blocks.iter().enumerate() {
        let made = lowering.builder.create_block();
        if index == 0 {
            lowering.builder.append_block_params_for_function_params(made);
        }
        for param in &block.params {
            let value = lowering.builder.append_block_param(made, types::I64);
            lowering.values.insert(*param, value);
        }
        lowering.blocks.push(made);
    }
    let entry = lowering.blocks[0];
    lowering.machine = lowering.builder.block_params(entry)[0];
    lowering.frame = lowering.builder.block_params(entry)[1];
    for (index, block) in function.blocks.iter().enumerate() {
        let made = lowering.blocks[index];
        lowering.builder.switch_to_block(made);
        for instr in &block.instrs {
            if let Some(value) = lowering.op(&instr.op)
                && let Some(result) = instr.result
            {
                lowering.values.insert(result, value);
            }
        }
        lowering.end(&block.end);
    }
    lowering.fill_failures();
    lowering.builder.seal_all_blocks();
    lowering.builder.finalize(module.isa().frontend_config());
    module.define_function(id, &mut context).map_err(|error| format!("{:?}", error))?;
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|error| error.to_string())?;
    let code = module.get_finalized_function(id) as usize;
    // The code lives as long as the program runs.
    std::mem::forget(module);
    Ok(code)
}
