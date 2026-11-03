//! Runs bytecode at once, and counts what gets hot.
//!
//! trace: decision.backend.tiers
//! trace: decision.backend.no_llvm
//!
//! On iPads and in the browser no JIT is allowed, so there this interpreter is all of TAKL: it stands on its own,
//! with every check and every message. Where Cranelift is allowed, it hands hot procedures and loops to `tiering`.
//!
//! Registers are read through a raw pointer to the frame. That is sound because `verify` checked, when the program
//! was loaded, that every register an instruction names is inside its procedure's frame, every jump lands inside the
//! procedure, and every index into a procedure's tables is in range; and the stack is reserved up front for the
//! deepest calls allowed, so it never moves.

use crate::bytecode::*;
use crate::failure::Failure;
use crate::messages;
use crate::numbers::{self, Problem, Size, fast};
use crate::runtime::{self, HeapValue};
use crate::syntax::{Compare, Counting, Family, Maths};
use crate::types::Type;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Zero};
use std::sync::Arc;

/// A site register holds the clock when the variable was set, above this many bits of where.
pub const SITE_BITS: u32 = 20;

/// How many procedures may be running at once, `start_running` among them; one more isn't designed.
pub const MOST_CALLS: u64 = 10_000;

/// The program stopped; what it said is in `Machine::failure`.
#[derive(Debug, Clone, Copy)]
pub struct Stopped;

/// Extra facts for an error's message.
pub enum Detail {
    Problem(Problem),
    Value(i128),
    Place { asked: i128, count: usize },
    Key(String),
    Fraction(BigInt, BigInt),
    Nothing,
}

/// A running program's state. Compiled code reaches `depth` at offset 0, so the layout is fixed.
#[repr(C)]
pub struct Machine {
    pub depth: u64,
    /// Counts every time a variable is set, so `because:` can tell them oldest set first; at offset 8.
    pub clock: u64,
    pub program: Arc<Program>,
    stack: Vec<u64>,
    top: usize,
    pub output: Vec<u8>,
    line: String,
    pub failure: Option<Failure>,
    /// Writes output to stdout as it goes, rather than keeping it all.
    pub live: bool,
    pub tiers: Option<Arc<crate::tiering::Tiers>>,
    /// While a table of answers is being built, the procedure whose calls look in it.
    pub building: Option<std::rc::Rc<crate::precompute::Building>>,
    /// The deepest calls have gone, for a table's entries.
    pub deepest: u64,
    /// Loop rounds and calls left before a run is given up, while building a table.
    pub budget: u64,
}

pub fn compare(operator: Compare, order: std::cmp::Ordering) -> bool {
    crate::check::compare_holds(operator, order)
}

impl Machine {
    pub fn new(program: Arc<Program>, live: bool) -> Machine {
        let widest = program.procedures.iter().map(|procedure| procedure.registers).max().unwrap_or(0) + 1;
        let stack = vec![0u64; widest * (MOST_CALLS as usize + 2)];
        Machine {
            depth: 0,
            clock: 0,
            program,
            stack,
            top: 0,
            output: Vec::new(),
            line: String::new(),
            failure: None,
            live,
            tiers: None,
            building: None,
            deepest: 0,
            budget: u64::MAX,
        }
    }

    // trace: decision.procedures.start_running
    /// Runs `start_running`, giving back its exit code.
    pub fn run_start(&mut self) -> Result<u8, Stopped> {
        let start = self.program.start;
        self.depth = 1;
        let value = self.call_with(start, &[])?;
        self.flush();
        let returns = self.program.procedures[start as usize].returns.is_some();
        Ok(if returns { value as u8 } else { 0 })
    }

    pub fn flush(&mut self) {
        if self.live && !self.output.is_empty() {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(&self.output);
            let _ = stdout.flush();
            self.output.clear();
        }
    }

    /// The program, which lives as long as the machine holding it.
    #[inline(always)]
    fn code<'a>(&self) -> &'a Program {
        // SAFETY: `self.program` is never replaced, and the machine keeps it alive while anything runs.
        unsafe { &*Arc::as_ptr(&self.program) }
    }

    /// Calls a procedure with these arguments, already counted for the callee.
    pub fn call_with(&mut self, procedure: u32, arguments: &[u64]) -> Result<u64, Stopped> {
        let base = self.top;
        let callee = &self.code().procedures[procedure as usize];
        // SAFETY: the stack is reserved for the deepest calls allowed (see the module's comment).
        unsafe {
            let frame = self.stack.as_mut_ptr().add(base);
            for register in &callee.heap_registers {
                *frame.add(*register as usize) = 0;
            }
            for (index, argument) in arguments.iter().enumerate() {
                *frame.add(index) = *argument;
            }
        }
        self.enter(procedure, base)
    }

    /// Runs a procedure whose arguments are in its frame at `base`, then releases its heap registers.
    #[inline(always)]
    pub fn enter(&mut self, procedure: u32, base: usize) -> Result<u64, Stopped> {
        if let Some(tiers) = &self.tiers {
            // SAFETY: the tiers live as long as the machine holding them.
            let tiers: &crate::tiering::Tiers = unsafe { &*Arc::as_ptr(tiers) };
            if let Some(answer) = tiers.call(self, procedure, base) {
                return answer;
            }
        }
        self.interpret(procedure, base, 0)
    }

    /// Interprets a procedure from `start`, its frame at `base`.
    pub fn interpret(&mut self, procedure: u32, base: usize, start: usize) -> Result<u64, Stopped> {
        let this = &self.code().procedures[procedure as usize];
        let saved_top = self.top;
        self.top = base + this.registers;
        let answer = self.run(this, procedure, base, start);
        self.top = saved_top;
        // SAFETY: as in `call_with`.
        let frame = unsafe { self.stack.as_mut_ptr().add(base) };
        for register in &this.heap_registers {
            // SAFETY: inside the frame; a heap register holds a counted pointer or 0.
            unsafe {
                let slot = frame.add(*register as usize);
                runtime::release(*slot);
                *slot = 0;
            }
        }
        answer
    }

    /// The frame of a procedure running at `base`, for compiled code jumping in mid-loop.
    pub fn frame_at(&mut self, base: usize) -> *mut u64 {
        // SAFETY: as in `call_with`.
        unsafe { self.stack.as_mut_ptr().add(base) }
    }

    fn run(&mut self, procedure: &Procedure, id: u32, base: usize, start: usize) -> Result<u64, Stopped> {
        let code = &procedure.instructions[..];
        // SAFETY: see the module's comment; `verify` checked every register and jump below.
        let frame = unsafe { self.stack.as_mut_ptr().add(base) };
        let mut pc = start;
        loop {
            // SAFETY: `verify` checked that every jump lands inside the code, and the code ends in `Finish`.
            let instruction = unsafe { *code.get_unchecked(pc) };
            pc += 1;
            // SAFETY: `frame` is this procedure's frame, as `execute` needs.
            if let Some(answer) = unsafe { self.execute(procedure, id, instruction, &mut pc, base, frame) } {
                return answer;
            }
        }
    }

    /// Runs one instruction of compiled code that has no inline form of its own, its frame at `frame`.
    ///
    /// # Safety
    /// `frame` is the frame of `procedure`, at `base` in the stack, with every number register the instruction
    /// reads stored in it.
    pub unsafe fn step(&mut self, id: u32, at: usize, frame: *mut u64) -> Result<(), Stopped> {
        let procedure = &self.code().procedures[id as usize];
        let instruction = procedure.instructions[at];
        let base = (frame as usize - self.stack.as_ptr() as usize) / 8;
        let saved_top = self.top;
        self.top = base + procedure.registers;
        let mut pc = at + 1;
        let answer = unsafe { self.execute(procedure, id, instruction, &mut pc, base, frame) };
        self.top = saved_top;
        match answer {
            Some(Err(stopped)) => Err(stopped),
            _ => Ok(()),
        }
    }

    /// Runs one instruction; `Some` when the procedure is done or stopped.
    ///
    /// # Safety
    /// `frame` is `procedure`'s frame, checked by `verify`.
    #[inline(always)]
    unsafe fn execute(
        &mut self,
        procedure: &Procedure,
        id: u32,
        instruction: Instruction,
        pc: &mut usize,
        base: usize,
        frame: *mut u64,
    ) -> Option<Result<u64, Stopped>> {
        macro_rules! reg {
            ($register:expr) => {
                *frame.add($register as usize)
            };
        }
        macro_rules! set_heap {
            ($register:expr, $value:expr) => {{
                let value = $value;
                let slot = frame.add($register as usize);
                runtime::release(*slot);
                *slot = value;
            }};
        }
        {
            unsafe {
                #[allow(clippy::unit_arg)]
                let _: () = match instruction {
                    Instruction::Load { dest, value } => reg!(dest) = value as u64,
                    Instruction::LoadHeap { dest, constant } => {
                        let value = procedure.heap_constants[constant as usize];
                        runtime::retain(value);
                        set_heap!(dest, value);
                    }
                    Instruction::Move { dest, source } => reg!(dest) = reg!(source),
                    Instruction::MoveHeap { dest, source } => {
                        let value = reg!(source);
                        runtime::retain(value);
                        set_heap!(dest, value);
                    }
                    Instruction::CopyTable { dest, source } => {
                        let value = runtime::copy_table(reg!(source));
                        set_heap!(dest, value);
                    }
                    Instruction::SetSite { dest, site } => {
                        self.clock += 1;
                        reg!(dest) = (self.clock << SITE_BITS) | site as u64;
                    }
                    Instruction::WholeMaths { operator, kind, dest, a, b, failure } => {
                        let (x, y) = (reg!(a), reg!(b));
                        match whole_maths(operator, kind, x, y) {
                            Some(answer) => reg!(dest) = answer,
                            None => {
                                return Some(Err(self.whole_failure(procedure, failure, operator, kind, x, y, frame)));
                            }
                        }
                    }
                    Instruction::WholeMathsBy { operator, kind, dest, a, by, failure } => {
                        let x = reg!(a);
                        let y = by as i64 as u64;
                        match whole_maths(operator, kind, x, y) {
                            Some(answer) => reg!(dest) = answer,
                            None => {
                                return Some(Err(self.whole_failure(procedure, failure, operator, kind, x, y, frame)));
                            }
                        }
                    }
                    Instruction::WholeRemainder { kind, dest, a, b, failure } => {
                        let (x, y) = (runtime::whole_value(kind, reg!(a)), runtime::whole_value(kind, reg!(b)));
                        if x >= 0 && y > 0 {
                            reg!(dest) = (x % y) as u64;
                        } else {
                            let problem = numbers::whole_remainder(&BigInt::from(x), &BigInt::from(y))
                                .err()
                                .unwrap_or(Problem::DividesByZero);
                            return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                        }
                    }
                    Instruction::WholeRounded { kind, dest, a, b, failure } => {
                        let (x, y) = (runtime::whole_value(kind, reg!(a)), runtime::whole_value(kind, reg!(b)));
                        let answer = numbers::whole_rounded(&BigInt::from(x), &BigInt::from(y))
                            .and_then(|answer| WHOLE_KINDS[kind as usize].check_big(&answer));
                        match answer {
                            Ok(answer) => reg!(dest) = answer as u64,
                            Err(problem) => {
                                return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                            }
                        }
                    }
                    Instruction::WholeCompare { operator, unsigned, dest, a, b } => {
                        reg!(dest) = whole_compare(operator, unsigned, reg!(a), reg!(b)) as u64;
                    }
                    Instruction::JumpIfWhole { operator, unsigned, a, b, target } => {
                        if whole_compare(operator, unsigned, reg!(a), reg!(b)) {
                            *pc = target as usize;
                        }
                    }
                    Instruction::JumpIfWholeBy { operator, unsigned, a, by, target } => {
                        if whole_compare(operator, unsigned, reg!(a), by as i64 as u64) {
                            *pc = target as usize;
                        }
                    }
                    Instruction::CheckWhole { kind, source, failure } => {
                        let value = reg!(source) as i64;
                        let (smallest, biggest) = WHOLE_LIMITS[kind as usize];
                        if value < smallest || value > biggest {
                            let problem = WHOLE_KINDS[kind as usize]
                                .check(value as i128)
                                .err()
                                .unwrap_or(Problem::TooBig(BigInt::from(value)));
                            return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                        }
                    }
                    Instruction::CheckBounds { source, failure } => {
                        let value = reg!(source) as i64 as i128;
                        if let FailureKind::Bounds { bounds } = &procedure.failures[failure as usize].kind
                            && !bounds.allows(value)
                        {
                            return Some(Err(self.stop(procedure, failure, Detail::Value(value), frame)));
                        }
                    }
                    Instruction::FastMaths { operator, size, dest, a, b, failure } => {
                        let (x, y) = (reg!(a), reg!(b));
                        match fast_maths(operator, size, x, y) {
                            Ok(answer) => reg!(dest) = answer,
                            Err(problem) => {
                                return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                            }
                        }
                    }
                    Instruction::FastCompare { operator, size, dest, a, b } => {
                        let order = fast::compare(size, reg!(a) as u128, reg!(b) as u128);
                        reg!(dest) = compare(operator, order) as u64;
                    }
                    Instruction::HeapMaths { operator, type_id, dest, a, b, failure } => {
                        let ty = &procedure.types[type_id as usize];
                        match heap_maths(operator, ty, reg!(a), reg!(b)) {
                            Ok(answer) => set_heap!(dest, answer),
                            Err(problem) => {
                                return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                            }
                        }
                    }
                    Instruction::HeapRounded { type_id, dest, a, b, failure } => {
                        let ty = &procedure.types[type_id as usize];
                        let digits = match ty {
                            Type::Decimal { size, .. } => Some(size.digits()),
                            _ => None,
                        };
                        let x = runtime::decimal_of(reg!(a));
                        let y = runtime::decimal_of(reg!(b));
                        match x.divided_by_rounded(y, digits) {
                            Ok(answer) => set_heap!(dest, runtime::new_heap(HeapValue::Decimal(answer))),
                            Err(problem) => {
                                return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                            }
                        }
                    }
                    Instruction::HeapCompare { operator, type_id, dest, a, b } => {
                        let ty = &procedure.types[type_id as usize];
                        let order = match ty {
                            Type::Decimal { size: Size::Big, fast: true } => {
                                fast::compare(Size::Big, runtime::quad_of(reg!(a)), runtime::quad_of(reg!(b)))
                            }
                            _ => runtime::decimal_of(reg!(a)).compare(runtime::decimal_of(reg!(b))),
                        };
                        reg!(dest) = compare(operator, order) as u64;
                    }
                    Instruction::Not { dest, source } => reg!(dest) = (reg!(source) == 0) as u64,
                    Instruction::Jump { target } => *pc = target as usize,
                    Instruction::JumpIfTrue { source, target } => {
                        if reg!(source) != 0 {
                            *pc = target as usize;
                        }
                    }
                    Instruction::JumpIfFalse { source, target } => {
                        if reg!(source) == 0 {
                            *pc = target as usize;
                        }
                    }
                    Instruction::LoopBack { target, loop_id } => {
                        *pc = target as usize;
                        self.budget -= 1;
                        if self.budget == 0 {
                            return Some(Err(Stopped));
                        }
                        if let Some(tiers) = &self.tiers
                            && tiers.loop_is_hot(id, loop_id)
                        {
                            let tiers = Arc::clone(tiers);
                            if let Some(answer) = tiers.jump_in(self, id, loop_id, base) {
                                return Some(answer);
                            }
                        }
                    }
                    Instruction::RepeatNext { counter, limit, target, loop_id } => {
                        let next = reg!(counter) as i64 + 1;
                        reg!(counter) = next as u64;
                        if next < reg!(limit) as i64 {
                            *pc = target as usize;
                            self.budget -= 1;
                            if self.budget == 0 {
                                return Some(Err(Stopped));
                            }
                            if let Some(tiers) = &self.tiers
                                && tiers.loop_is_hot(id, loop_id)
                            {
                                let tiers = Arc::clone(tiers);
                                if let Some(answer) = tiers.jump_in(self, id, loop_id, base) {
                                    return Some(answer);
                                }
                            }
                        }
                    }
                    Instruction::CheckCount { source, failure } => {
                        let count = reg!(source) as i64;
                        if count < 0 {
                            return Some(Err(self.stop(procedure, failure, Detail::Value(count as i128), frame)));
                        }
                    }
                    Instruction::ShowPiece { source, type_id, failure } => {
                        let ty = &procedure.types[type_id as usize];
                        let bits = reg!(source);
                        if let Type::Fraction(_) = ty
                            && let HeapValue::Fraction(top, bottom) = runtime::heap(bits)
                            && !top.gcd(bottom).is_one()
                        {
                            // trace: decision.types.over
                            let detail = Detail::Fraction(top.clone(), bottom.clone());
                            self.line.clear();
                            return Some(Err(self.stop(procedure, failure, detail, frame)));
                        }
                        self.line.push_str(&runtime::show(ty, bits));
                    }
                    Instruction::ShowEnd => self.show_end(),
                    Instruction::Call { procedure: callee, dest, arguments, failure } => {
                        let answer = match self.call_from(
                            procedure,
                            callee,
                            &procedure.call_arguments[arguments as usize],
                            failure,
                            frame,
                        ) {
                            Ok(answer) => answer,
                            Err(stopped) => return Some(Err(stopped)),
                        };
                        if procedure.kinds[dest as usize] == RegisterKind::Heap {
                            set_heap!(dest, answer);
                        } else {
                            reg!(dest) = answer;
                        }
                    }
                    Instruction::Return { source } => {
                        let value = reg!(source);
                        if procedure.kinds[source as usize] == RegisterKind::Heap {
                            runtime::retain(value);
                        }
                        return Some(Ok(value));
                    }
                    Instruction::Finish => return Some(Ok(0)),
                    Instruction::Fail { failure } => {
                        return Some(Err(self.stop(procedure, failure, Detail::Nothing, frame)));
                    }
                    Instruction::NewTable { dest, keyed, item_heap } => {
                        set_heap!(dest, runtime::new_table(keyed, item_heap))
                    }
                    Instruction::Push { table, source } | Instruction::Add { table, source } => {
                        let value = reg!(source);
                        runtime::change_table(reg!(table), |storage| {
                            if storage.item_heap {
                                runtime::retain(value);
                            }
                            storage.items.push(value);
                        });
                    }
                    Instruction::PushKeyed { table, key, source, failure }
                    | Instruction::AddKeyed { table, key, source, failure } => {
                        let key = runtime::text_of(reg!(key)).text.clone();
                        let value = reg!(source);
                        let added = runtime::change_table(reg!(table), |storage| {
                            let Some((keys, places)) = &mut storage.keys else { return false };
                            if places.contains_key(&key) {
                                return false;
                            }
                            places.insert(key.clone(), keys.len());
                            keys.push(key.clone());
                            if storage.item_heap {
                                runtime::retain(value);
                            }
                            storage.items.push(value);
                            true
                        });
                        if added != Some(true) {
                            return Some(Err(self.stop(procedure, failure, Detail::Key(key), frame)));
                        }
                    }
                    Instruction::AddAt { table, source, counting, at, failure } => {
                        let asked = reg!(at) as i64 as i128;
                        let offset = if counting == Counting::Position { asked - 1 } else { asked };
                        let length = runtime::table_length(reg!(table));
                        if offset < 0 || offset > length as i128 {
                            return Some(Err(self.stop(
                                procedure,
                                failure,
                                Detail::Place { asked, count: length },
                                frame,
                            )));
                        }
                        let value = reg!(source);
                        runtime::change_table(reg!(table), |storage| {
                            if storage.item_heap {
                                runtime::retain(value);
                            }
                            storage.items.insert(offset as usize, value);
                        });
                    }
                    Instruction::Length { dest, source, text } => {
                        reg!(dest) = if text {
                            runtime::text_of(reg!(source)).letters
                        } else {
                            runtime::table_length(reg!(source))
                        } as u64;
                    }
                    Instruction::Item { dest, table, counting, at, failure, heap } => {
                        let asked = reg!(at) as i64 as i128;
                        let offset = if counting == Counting::Position { asked - 1 } else { asked };
                        let storage = runtime::storage(reg!(table));
                        if offset < 0 || offset >= storage.items.len() as i128 {
                            let count = storage.items.len();
                            return Some(Err(self.stop(procedure, failure, Detail::Place { asked, count }, frame)));
                        }
                        let value = storage.items[offset as usize];
                        if heap {
                            // An item that is a table is read as a copy, so changing it never changes the table it is in.
                            let value = if matches!(runtime::heap(value), HeapValue::Table(_)) {
                                runtime::copy_table(value)
                            } else {
                                runtime::retain(value);
                                value
                            };
                            set_heap!(dest, value);
                        } else {
                            reg!(dest) = value;
                        }
                    }
                    Instruction::KeyAt { dest, table, at } => {
                        let storage = runtime::storage(reg!(table));
                        let key = storage
                            .keys
                            .as_ref()
                            .and_then(|(keys, _)| keys.get(reg!(at) as usize))
                            .cloned()
                            .unwrap_or_default();
                        set_heap!(dest, runtime::new_text(key));
                    }
                    Instruction::Letter { dest, text, counting, at, failure } => {
                        let asked = reg!(at) as i64 as i128;
                        let offset = if counting == Counting::Position { asked - 1 } else { asked };
                        let whole = runtime::text_of(reg!(text));
                        let letter = if offset < 0 { None } else { runtime::letter_at(&whole.text, offset as usize) };
                        match letter {
                            Some(letter) => set_heap!(dest, runtime::new_text(letter)),
                            None => {
                                let count = whole.letters;
                                return Some(Err(self.stop(procedure, failure, Detail::Place { asked, count }, frame)));
                            }
                        }
                    }
                    Instruction::OfFamily { family, type_id, dest, table, failure } => {
                        let ty = &procedure.types[type_id as usize];
                        match of_family(family, ty, reg!(table)) {
                            Ok(answer) => {
                                if ty.is_heap() {
                                    set_heap!(dest, answer);
                                } else {
                                    reg!(dest) = answer;
                                }
                            }
                            Err(problem) => {
                                return Some(Err(self.stop(procedure, failure, Detail::Problem(problem), frame)));
                            }
                        }
                    }
                    Instruction::TextCompare { operator, dest, a, b } => {
                        let order = runtime::text_of(reg!(a)).text.cmp(&runtime::text_of(reg!(b)).text);
                        reg!(dest) = compare(operator, order) as u64;
                    }
                };
            }
        }
        None
    }

    fn show_end(&mut self) {
        // trace: decision.show.pieces
        self.output.extend_from_slice(self.line.as_bytes());
        self.output.push(b'\n');
        self.line.clear();
        if self.live && self.output.len() > 1 << 16 {
            self.flush();
        }
    }

    /// A call from a procedure's `Call`: checks how deep calls go, counts the arguments for the callee, runs it.
    ///
    /// # Safety
    /// `frame` is the caller's frame, and `arguments` name registers inside it.
    pub unsafe fn call_from(
        &mut self,
        caller: &Procedure,
        callee: u32,
        arguments: &[Register],
        failure: FailureId,
        frame: *mut u64,
    ) -> Result<u64, Stopped> {
        unsafe {
            if self.depth >= MOST_CALLS {
                return Err(self.stop(caller, failure, Detail::Nothing, frame));
            }
            self.budget -= 1;
            if self.budget == 0 {
                return Err(Stopped);
            }
            let called = &self.code().procedures[callee as usize];
            if self.building.is_some()
                && let Some(building) = self.building.clone()
                && let Some(answer) = building.look_up(self, callee, arguments, frame)
            {
                return Ok(answer);
            }
            let base = self.top;
            let callee_frame = self.frame_at(base);
            for register in &called.heap_registers {
                *callee_frame.add(*register as usize) = 0;
            }
            for (index, register) in arguments.iter().enumerate() {
                let value = *frame.add(*register as usize);
                if called.kinds[index] == RegisterKind::Heap {
                    runtime::retain(value);
                }
                *callee_frame.add(index) = value;
            }
            self.depth += 1;
            self.deepest = self.deepest.max(self.depth);
            let answer = self.enter(callee, base);
            self.depth -= 1;
            answer
        }
    }

    #[cold]
    #[allow(clippy::too_many_arguments)]
    fn whole_failure(
        &mut self,
        procedure: &Procedure,
        failure: FailureId,
        operator: Maths,
        kind: WholeKind,
        x: u64,
        y: u64,
        frame: *mut u64,
    ) -> Stopped {
        let problem = whole_problem(operator, kind, x, y);
        // SAFETY: the caller passes its own frame.
        unsafe { self.stop(procedure, failure, Detail::Problem(problem), frame) }
    }

    /// Stops the program with an error made from `failure` in `procedure`, reading `because:` values from `frame`.
    ///
    /// # Safety
    /// `frame` is `procedure`'s frame, which holds the registers its `because:` lines name.
    #[cold]
    pub unsafe fn stop(
        &mut self,
        procedure: &Procedure,
        failure: FailureId,
        detail: Detail,
        frame: *const u64,
    ) -> Stopped {
        let info = &procedure.failures[failure as usize];
        let span = info.span;
        let mut made = match (&info.kind, detail) {
            (FailureKind::Maths { ty }, Detail::Problem(problem)) => messages::problem(problem, span, ty),
            (FailureKind::Written { problem, ty }, _) => messages::problem(problem.clone(), span, ty),
            (FailureKind::Bounds { bounds }, Detail::Value(value)) => {
                messages::outside_bounds(span, &value.to_string(), &bounds.rule())
            }
            (FailureKind::Place { counting, letters }, Detail::Place { asked, count }) => messages::no_such_place(
                span,
                counting.word(),
                &asked.to_string(),
                count,
                if *letters { "letters" } else { "items" },
            ),
            (FailureKind::Hole { counting, table }, Detail::Place { asked, count }) => {
                messages::would_leave_hole(span, counting.word(), &asked.to_string(), table, count)
            }
            (FailureKind::KeyTwice, Detail::Key(key)) => {
                messages::not_designed_yet(span, format!("adding \"{}\" again, when it's already in the table", key))
            }
            (FailureKind::Family { family, .. }, Detail::Problem(Problem::DividesByZero)) => {
                messages::no_items(span, family.word())
            }
            (FailureKind::Family { ty, .. }, Detail::Problem(problem)) => messages::problem(problem, span, ty),
            (FailureKind::Count, Detail::Value(value)) => {
                messages::not_designed_yet(span, format!("repeating {} times", value))
            }
            (FailureKind::FractionShow, Detail::Fraction(top, bottom)) => messages::not_designed_yet(
                span,
                format!("whether `show` prints {} over {} as it is or in its lowest terms", top, bottom),
            ),
            (FailureKind::TooDeep, _) => messages::not_designed_yet(
                span,
                format!("what happens when procedures call each other this deep ({} calls)", MOST_CALLS),
            ),
            _ => crate::broke(span, "a stop with nothing to say"),
        };
        // trace: decision.errors.layout
        // A maths step that stops names where each of its variables got its value, oldest set first. An empty
        // table has no values to tell of.
        let tells = matches!(info.kind, FailureKind::Maths { .. })
            || (matches!(info.kind, FailureKind::Family { .. }) && !made.head_says("the table is empty"));
        if tells {
            let mut told = Vec::new();
            for entry in &info.because {
                // SAFETY: `verify` checked these registers are inside the frame.
                let value = unsafe { *frame.add(entry.register as usize) };
                let (stamp, site) = match entry.site {
                    Site::Fixed(span) => (0, span),
                    Site::Register(register) => {
                        let held = unsafe { *frame.add(register as usize) };
                        let index = (held & ((1 << SITE_BITS) - 1)) as usize;
                        (held >> SITE_BITS, procedure.sites.get(index).copied().unwrap_or_default())
                    }
                };
                let text = match &entry.ty {
                    Type::Table { .. } => format!("@{} was last set here", entry.name),
                    ty => format!("@{} was {}", entry.name, runtime::show(ty, value)),
                };
                told.push((stamp, text, site));
            }
            told.sort_by_key(|(stamp, _, _)| *stamp);
            for (_, text, site) in told {
                made = made.because(text, site);
            }
        }
        self.line.clear();
        self.failure = Some(made);
        Stopped
    }
}

/// Whole-number maths in a kind's range, or `None` when the answer doesn't fit or can't be given.
#[inline(always)]
pub fn whole_maths(operator: Maths, kind: WholeKind, x: u64, y: u64) -> Option<u64> {
    if kind == BIG_NEVER_NEGATIVE {
        return match operator {
            Maths::Plus => x.checked_add(y),
            Maths::Minus => x.checked_sub(y),
            Maths::Times => x.checked_mul(y),
            Maths::DividedBy => {
                if y == 0 || !x.is_multiple_of(y) {
                    None
                } else {
                    Some(x / y)
                }
            }
        };
    }
    let (a, b) = (x as i64, y as i64);
    let answer = match operator {
        Maths::Plus => a.checked_add(b)?,
        Maths::Minus => a.checked_sub(b)?,
        Maths::Times => a.checked_mul(b)?,
        Maths::DividedBy => {
            if b == 0 || a.checked_rem(b)? != 0 {
                return None;
            }
            a.checked_div(b)?
        }
    };
    let (smallest, biggest) = WHOLE_LIMITS[kind as usize];
    if answer < smallest || answer > biggest { None } else { Some(answer as u64) }
}

/// Why whole-number maths gave no answer, worked out exactly.
pub fn whole_problem(operator: Maths, kind: WholeKind, x: u64, y: u64) -> Problem {
    let a = BigInt::from(runtime::whole_value(kind, x));
    let b = BigInt::from(runtime::whole_value(kind, y));
    let exact = match operator {
        Maths::Plus => Ok(a + b),
        Maths::Minus => Ok(a - b),
        Maths::Times => Ok(a * b),
        Maths::DividedBy => numbers::whole_divided_by(&a, &b),
    };
    match exact {
        Ok(answer) => WHOLE_KINDS[kind as usize].check_big(&answer).err().unwrap_or(Problem::TooBig(answer)),
        Err(problem) => problem,
    }
}

#[inline(always)]
pub fn whole_compare(operator: Compare, unsigned: bool, x: u64, y: u64) -> bool {
    let order = if unsigned { x.cmp(&y) } else { (x as i64).cmp(&(y as i64)) };
    compare(operator, order)
}

// trace: decision.types.fast_but_inexact
#[inline(always)]
pub fn fast_maths(operator: Maths, size: Size, x: u64, y: u64) -> Result<u64, Problem> {
    match size {
        Size::Medium => {
            let (a, b) = (f64::from_bits(x), f64::from_bits(y));
            let answer = match operator {
                Maths::Plus => a + b,
                Maths::Minus => a - b,
                Maths::Times => a * b,
                Maths::DividedBy => {
                    if b == 0.0 {
                        return Err(Problem::DividesByZero);
                    }
                    a / b
                }
            };
            if answer.is_finite() { Ok(answer.to_bits()) } else { Err(Problem::FastTooBig) }
        }
        Size::Small => {
            let (a, b) = (f32::from_bits(x as u32), f32::from_bits(y as u32));
            let answer = match operator {
                Maths::Plus => a + b,
                Maths::Minus => a - b,
                Maths::Times => a * b,
                Maths::DividedBy => {
                    if b == 0.0 {
                        return Err(Problem::DividesByZero);
                    }
                    a / b
                }
            };
            if answer.is_finite() { Ok(answer.to_bits() as u64) } else { Err(Problem::FastTooBig) }
        }
        _ => fast::maths(size, operator, x as u128, y as u128).map(|bits| bits as u64),
    }
}

/// Exact decimals and float128s.
pub fn heap_maths(operator: Maths, ty: &Type, x: u64, y: u64) -> Result<u64, Problem> {
    match ty {
        Type::Decimal { size: Size::Big, fast: true } => {
            let answer = fast::maths(Size::Big, operator, runtime::quad_of(x), runtime::quad_of(y))?;
            Ok(runtime::new_heap(HeapValue::Quad(answer)))
        }
        Type::Decimal { size, .. } => {
            let (a, b) = (runtime::decimal_of(x), runtime::decimal_of(y));
            let digits = Some(size.digits());
            let answer = match operator {
                Maths::Plus => a.plus(b, digits),
                Maths::Minus => a.minus(b, digits),
                Maths::Times => a.times(b, digits),
                Maths::DividedBy => a.divided_by(b, digits),
            }?;
            Ok(runtime::new_heap(HeapValue::Decimal(answer)))
        }
        _ => Err(Problem::DividesByZero),
    }
}

// trace: decision.tables.of_family
/// `sum of`, `biggest of`, `smallest of` and `average of`, in the items' type; an empty table has no answer but its
/// sum, which is said as "divides by zero" for the caller to turn into "the table is empty".
pub fn of_family(family: Family, ty: &Type, table: u64) -> Result<u64, Problem> {
    let storage = runtime::storage(table);
    let items = &storage.items;
    if items.is_empty() && family != Family::Sum {
        return Err(Problem::DividesByZero);
    }
    match held(ty) {
        Held::Whole(kind) => {
            let values = items.iter().map(|item| runtime::whole_value(kind, *item));
            let whole = WHOLE_KINDS[kind as usize];
            match family {
                Family::Sum | Family::Average => {
                    let mut sum: i128 = 0;
                    for value in values {
                        sum = whole.check(sum + value)?;
                    }
                    if family == Family::Sum {
                        return Ok(sum as u64);
                    }
                    let count = items.len() as i128;
                    if sum % count != 0 {
                        return Err(Problem::Uneven(BigInt::from(sum), BigInt::from(count)));
                    }
                    Ok((sum / count) as u64)
                }
                Family::Biggest => Ok(values.max().unwrap_or(0) as u64),
                Family::Smallest => Ok(values.min().unwrap_or(0) as u64),
                Family::Length => Ok(items.len() as u64),
            }
        }
        Held::Fast(size) => {
            let pick = |want: std::cmp::Ordering| {
                items
                    .iter()
                    .copied()
                    .reduce(|a, b| if fast::compare(size, b as u128, a as u128) == want { b } else { a })
                    .unwrap_or(0)
            };
            match family {
                Family::Biggest => Ok(pick(std::cmp::Ordering::Greater)),
                Family::Smallest => Ok(pick(std::cmp::Ordering::Less)),
                Family::Sum | Family::Average => {
                    let mut sum = fast::from_whole(size, 0) as u64;
                    for item in items {
                        sum = fast_maths(Maths::Plus, size, sum, *item)?;
                    }
                    if family == Family::Sum {
                        return Ok(sum);
                    }
                    let count = fast::from_whole(size, items.len() as i128) as u64;
                    fast_maths(Maths::DividedBy, size, sum, count)
                }
                Family::Length => Ok(items.len() as u64),
            }
        }
        _ => {
            if let Type::Decimal { size: Size::Big, fast: true } = ty {
                let values: Vec<u128> = items.iter().map(|item| runtime::quad_of(*item)).collect();
                let answer = match family {
                    Family::Biggest => values
                        .iter()
                        .copied()
                        .reduce(|a, b| if fast::compare(Size::Big, b, a).is_gt() { b } else { a })
                        .unwrap_or(0),
                    Family::Smallest => values
                        .iter()
                        .copied()
                        .reduce(|a, b| if fast::compare(Size::Big, b, a).is_lt() { b } else { a })
                        .unwrap_or(0),
                    _ => {
                        let mut sum = fast::from_whole(Size::Big, 0);
                        for value in &values {
                            sum = fast::maths(Size::Big, Maths::Plus, sum, *value)?;
                        }
                        if family == Family::Average {
                            sum = fast::maths(
                                Size::Big,
                                Maths::DividedBy,
                                sum,
                                fast::from_whole(Size::Big, values.len() as i128),
                            )?;
                        }
                        sum
                    }
                };
                return Ok(runtime::new_heap(HeapValue::Quad(answer)));
            }
            let digits = match ty {
                Type::Decimal { size, .. } => Some(size.digits()),
                _ => None,
            };
            let values: Vec<&numbers::Decimal> = items.iter().map(|item| runtime::decimal_of(*item)).collect();
            let answer = match family {
                Family::Biggest => {
                    values.iter().copied().reduce(|a, b| if b.compare(a).is_gt() { b } else { a }).cloned()
                }
                Family::Smallest => {
                    values.iter().copied().reduce(|a, b| if b.compare(a).is_lt() { b } else { a }).cloned()
                }
                _ => {
                    let mut sum = numbers::Decimal::from_whole(BigInt::zero());
                    for value in &values {
                        sum = sum.plus(value, digits)?;
                    }
                    if family == Family::Average {
                        sum = sum.divided_by(&numbers::Decimal::from_whole(BigInt::from(values.len())), digits)?;
                    }
                    Some(sum)
                }
            };
            Ok(runtime::new_heap(HeapValue::Decimal(
                answer.unwrap_or_else(|| numbers::Decimal::from_whole(BigInt::zero())),
            )))
        }
    }
}
