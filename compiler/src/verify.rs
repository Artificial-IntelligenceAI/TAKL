//! Checks bytecode before it runs, so the interpreter and compiled code can read registers without checking each one.
//!
//! trace: decision.house.impl_rules
//!
//! The bytecode is made by `lower`, so this never fails on a program TAKL made; if it does, TAKL broke.

use crate::bytecode::*;

pub fn verify(program: &Program) -> Result<(), String> {
    for (index, procedure) in program.procedures.iter().enumerate() {
        let registers = procedure.registers;
        let length = procedure.instructions.len();
        let fail = |what: &str| Err(format!("procedure {} ({}): {}", index, procedure.name, what));
        if procedure.kinds.len() != registers || procedure.params > registers {
            return fail("its register kinds don't match its frame");
        }
        if !matches!(procedure.instructions.last(), Some(Instruction::Finish | Instruction::Return { .. } | Instruction::Jump { .. })) {
            return fail("its code doesn't end");
        }
        for failure in &procedure.failures {
            for entry in &failure.because {
                if entry.register as usize >= registers {
                    return fail("a `because:` register is outside the frame");
                }
                if let Site::Register(site) = entry.site
                    && site as usize >= registers
                {
                    return fail("a site register is outside the frame");
                }
            }
        }
        for arguments in &procedure.call_arguments {
            if arguments.iter().any(|register| *register as usize >= registers) {
                return fail("an argument is outside the frame");
            }
        }
        for instruction in &procedure.instructions {
            let mut ok = true;
            let mut check = |register: Register| ok &= (register as usize) < registers;
            let mut target_ok = true;
            let mut target = |place: Place| target_ok &= (place as usize) < length;
            let failure_ok = |failure: FailureId| (failure as usize) < procedure.failures.len();
            let type_ok = |type_id: u16| (type_id as usize) < procedure.types.len();
            use Instruction::*;
            let extra = match *instruction {
                Load { dest, .. } | SetSite { dest, .. } | NewTable { dest, .. } => {
                    check(dest);
                    true
                }
                LoadHeap { dest, constant } => {
                    check(dest);
                    (constant as usize) < procedure.heap_constants.len()
                }
                Move { dest, source } | MoveHeap { dest, source } | CopyTable { dest, source } | Not { dest, source } => {
                    check(dest);
                    check(source);
                    true
                }
                WholeMaths { dest, a, b, failure, kind, .. } | WholeRemainder { dest, a, b, failure, kind } | WholeRounded { dest, a, b, failure, kind } => {
                    check(dest);
                    check(a);
                    check(b);
                    failure_ok(failure) && (kind as usize) < 8
                }
                WholeMathsBy { dest, a, failure, kind, .. } => {
                    check(dest);
                    check(a);
                    failure_ok(failure) && (kind as usize) < 8
                }
                WholeCompare { dest, a, b, .. } | FastCompare { dest, a, b, .. } | TextCompare { dest, a, b, .. } => {
                    check(dest);
                    check(a);
                    check(b);
                    true
                }
                HeapCompare { dest, a, b, type_id, .. } => {
                    check(dest);
                    check(a);
                    check(b);
                    type_ok(type_id)
                }
                JumpIfWhole { a, b, target: place, .. } => {
                    check(a);
                    check(b);
                    target(place);
                    true
                }
                JumpIfWholeBy { a, target: place, .. } => {
                    check(a);
                    target(place);
                    true
                }
                CheckWhole { source, failure, kind } => {
                    check(source);
                    failure_ok(failure) && (kind as usize) < 8
                }
                CheckBounds { source, failure } | CheckCount { source, failure } => {
                    check(source);
                    failure_ok(failure)
                }
                FastMaths { dest, a, b, failure, .. } => {
                    check(dest);
                    check(a);
                    check(b);
                    failure_ok(failure)
                }
                HeapMaths { dest, a, b, failure, type_id, .. } | HeapRounded { dest, a, b, failure, type_id } => {
                    check(dest);
                    check(a);
                    check(b);
                    failure_ok(failure) && type_ok(type_id)
                }
                Jump { target: place } | LoopBack { target: place, .. } => {
                    target(place);
                    true
                }
                JumpIfTrue { source, target: place } | JumpIfFalse { source, target: place } => {
                    check(source);
                    target(place);
                    true
                }
                RepeatNext { counter, limit, target: place, loop_id } => {
                    check(counter);
                    check(limit);
                    target(place);
                    (loop_id as usize) < procedure.loops.len()
                }
                ShowPiece { source, type_id, failure } => {
                    check(source);
                    type_ok(type_id) && failure_ok(failure)
                }
                Call { procedure: callee, dest, arguments, failure } => {
                    check(dest);
                    let callee_ok = (callee as usize) < program.procedures.len();
                    callee_ok
                        && failure_ok(failure)
                        && procedure.call_arguments.get(arguments as usize).map(Vec::len)
                            == Some(program.procedures[callee as usize].params)
                }
                Return { source } => {
                    check(source);
                    true
                }
                ShowEnd | Finish => true,
                Fail { failure } => failure_ok(failure),
                Push { table, source } | Add { table, source } => {
                    check(table);
                    check(source);
                    true
                }
                PushKeyed { table, key, source, failure } | AddKeyed { table, key, source, failure } => {
                    check(table);
                    check(key);
                    check(source);
                    failure_ok(failure)
                }
                AddAt { table, source, at, failure, .. } => {
                    check(table);
                    check(source);
                    check(at);
                    failure_ok(failure)
                }
                Length { dest, source, .. } => {
                    check(dest);
                    check(source);
                    true
                }
                Item { dest, table, at, failure, .. } | Letter { dest, text: table, at, failure, .. } => {
                    check(dest);
                    check(table);
                    check(at);
                    failure_ok(failure)
                }
                KeyAt { dest, table, at } => {
                    check(dest);
                    check(table);
                    check(at);
                    true
                }
                OfFamily { dest, table, failure, type_id, .. } => {
                    check(dest);
                    check(table);
                    failure_ok(failure) && type_ok(type_id)
                }
            };
            if !ok || !target_ok || !extra {
                return fail(&format!("an instruction names something outside its procedure: {:?}", instruction));
            }
        }
    }
    if program.start as usize >= program.procedures.len() {
        return Err("no start".to_string());
    }
    Ok(())
}
