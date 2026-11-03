//! Turns a checked program into bytecode.
//!
//! trace: decision.backend.bytecode
//!
//! Optimized as it is made (`decision.house.optimize_early`): a test of whole numbers jumps in one instruction,
//! maths by a small written number carries it inside the instruction, a `repeat` steps and jumps back in one, and
//! an answer goes straight into the variable it is for.

use crate::bytecode::*;
use crate::checked::*;
use crate::numbers::Size;
use crate::runtime::{self, HeapValue};
use crate::source::Span;
use crate::syntax::{Compare, Counting, Family, Logic, Maths};
use crate::types::Type;
use std::collections::HashMap;

/// Heap scratch registers are numbered from here while a procedure is made, then moved after the number ones.
const HEAP_TEMP: u16 = 0x8000;

struct Lowerer<'p> {
    program: &'p CheckedProgram,
    procedure: &'p CheckedProcedure,
    instructions: Vec<Instruction>,
    number_base: u16,
    next_number: u16,
    most_numbers: u16,
    next_heap: u16,
    most_heaps: u16,
    site_registers: HashMap<VariableId, Register>,
    fixed_sites: HashMap<VariableId, Span>,
    sites: Vec<Span>,
    heap_constants: Vec<runtime::HeapRef>,
    failures: Vec<FailureInfo>,
    types: Vec<Type>,
    loops: Vec<Place>,
    /// For each loop being made, the jumps `stop` leaves to be pointed at its end.
    stops: Vec<Vec<usize>>,
    call_arguments: Vec<Vec<Register>>,
}

fn opposite(operator: Compare) -> Compare {
    match operator {
        Compare::Is => Compare::IsNot,
        Compare::IsNot => Compare::Is,
        Compare::MoreThan => Compare::AtMost,
        Compare::LessThan => Compare::AtLeast,
        Compare::AtLeast => Compare::LessThan,
        Compare::AtMost => Compare::MoreThan,
    }
}

fn unsigned(ty: &Type) -> bool {
    matches!(ty, Type::Whole(whole) if whole.never_negative)
}

impl<'p> Lowerer<'p> {
    fn emit(&mut self, instruction: Instruction) -> usize {
        self.instructions.push(instruction);
        self.instructions.len() - 1
    }

    fn here(&self) -> Place {
        self.instructions.len() as Place
    }

    fn patch(&mut self, at: usize, target: Place) {
        match &mut self.instructions[at] {
            Instruction::Jump { target: t }
            | Instruction::JumpIfTrue { target: t, .. }
            | Instruction::JumpIfFalse { target: t, .. }
            | Instruction::JumpIfWhole { target: t, .. }
            | Instruction::JumpIfWholeBy { target: t, .. }
            | Instruction::LoopBack { target: t, .. }
            | Instruction::RepeatNext { target: t, .. } => *t = target,
            _ => {}
        }
    }

    fn number_temp(&mut self) -> Register {
        let register = self.number_base + self.next_number;
        self.next_number += 1;
        self.most_numbers = self.most_numbers.max(self.next_number);
        register
    }

    fn heap_temp(&mut self) -> Register {
        let register = HEAP_TEMP + self.next_heap;
        self.next_heap += 1;
        self.most_heaps = self.most_heaps.max(self.next_heap);
        register
    }

    fn temp_for(&mut self, ty: &Type) -> Register {
        if ty.is_heap() { self.heap_temp() } else { self.number_temp() }
    }

    fn type_id(&mut self, ty: &Type) -> u16 {
        match self.types.iter().position(|known| known == ty) {
            Some(index) => index as u16,
            None => {
                self.types.push(ty.clone());
                (self.types.len() - 1) as u16
            }
        }
    }

    fn heap_constant(&mut self, value: HeapValue) -> u32 {
        self.heap_constants.push(runtime::new_heap(value));
        (self.heap_constants.len() - 1) as u32
    }

    fn failure(&mut self, span: Span, kind: FailureKind, because: &[VariableId]) -> FailureId {
        let because = because
            .iter()
            .map(|id| {
                let variable = &self.procedure.variables[*id];
                let site = match self.site_registers.get(id) {
                    Some(register) => Site::Register(*register),
                    None => Site::Fixed(self.fixed_sites.get(id).copied().unwrap_or_default()),
                };
                BecauseEntry { name: variable.name.clone(), register: *id as Register, ty: variable.ty.clone(), site }
            })
            .collect();
        self.failures.push(FailureInfo { span, kind, because });
        (self.failures.len() - 1) as FailureId
    }

    fn site_index(&mut self, span: Span) -> u32 {
        match self.sites.iter().position(|known| *known == span) {
            Some(index) => index as u32,
            None => {
                self.sites.push(span);
                (self.sites.len() - 1) as u32
            }
        }
    }

    /// Notes where a variable got its value, when it gets one in more than one place.
    fn set_site(&mut self, variable: VariableId, span: Span) {
        if let Some(register) = self.site_registers.get(&variable).copied() {
            let site = self.site_index(span);
            self.emit(Instruction::SetSite { dest: register, site });
        }
    }

    // ---- values ----

    fn constant(&mut self, constant: &Constant, ty: &Type, dest: Option<Register>) -> Register {
        // Written numbers are shown from text made before running, so they are held in the heap whatever their type.
        let in_heap = ty.is_heap()
            || matches!(constant, Constant::WrittenWhole(_) | Constant::WrittenDecimal(_) | Constant::WrittenFraction(..));
        let dest = dest.unwrap_or_else(|| if in_heap { self.heap_temp() } else { self.number_temp() });
        match constant {
            Constant::Whole(value) => {
                self.emit(Instruction::Load { dest, value: *value as i64 });
            }
            Constant::TrueOrFalse(value) => {
                self.emit(Instruction::Load { dest, value: *value as i64 });
            }
            Constant::Fast(bits) => {
                if ty.is_heap() {
                    let constant = self.heap_constant(HeapValue::Quad(*bits));
                    self.emit(Instruction::LoadHeap { dest, constant });
                } else {
                    self.emit(Instruction::Load { dest, value: *bits as u64 as i64 });
                }
            }
            Constant::Decimal(value) | Constant::WrittenDecimal(value) => {
                let constant = if ty.is_heap() && *ty != Type::WrittenDecimal {
                    self.heap_constant(HeapValue::Decimal(value.clone()))
                } else {
                    self.heap_constant(HeapValue::Text(runtime::TextValue::new(value.show())))
                };
                self.emit(Instruction::LoadHeap { dest, constant });
            }
            Constant::Fraction(top, bottom) | Constant::WrittenFraction(top, bottom) => {
                let constant = self.heap_constant(HeapValue::Fraction(top.clone(), bottom.clone()));
                self.emit(Instruction::LoadHeap { dest, constant });
            }
            Constant::Text(text) => {
                let constant = self.heap_constant(HeapValue::Text(runtime::TextValue::new(text.clone())));
                self.emit(Instruction::LoadHeap { dest, constant });
            }
            Constant::WrittenWhole(value) => {
                let constant = self.heap_constant(HeapValue::Text(runtime::TextValue::new(value.to_string())));
                self.emit(Instruction::LoadHeap { dest, constant });
            }
        }
        dest
    }

    /// A value in a register: its own register for a variable, else a fresh one, or `dest` when given.
    fn value(&mut self, expression: &CheckedExpression, dest: Option<Register>) -> Register {
        let ty = &expression.ty;
        match &expression.kind {
            CheckedKind::Constant(constant) => self.constant(constant, ty, dest),
            CheckedKind::Variable(id) => *id as Register,
            CheckedKind::Share(id) => *id as Register,
            CheckedKind::Maths { operator, left, right, because } => {
                let failure = self.failure(expression.span, FailureKind::Maths { ty: ty.clone() }, because);
                match crate::bytecode::held(ty) {
                    Held::Whole(kind) => {
                        let a = self.value(left, None);
                        if let CheckedKind::Constant(Constant::Whole(by)) = right.kind
                            && let Ok(by) = i32::try_from(by)
                        {
                            let dest = dest.unwrap_or_else(|| self.number_temp());
                            self.emit(Instruction::WholeMathsBy { operator: *operator, kind, dest, a, by, failure });
                            return dest;
                        }
                        let b = self.value(right, None);
                        let dest = dest.unwrap_or_else(|| self.number_temp());
                        self.emit(Instruction::WholeMaths { operator: *operator, kind, dest, a, b, failure });
                        dest
                    }
                    Held::Fast(size) => {
                        let a = self.value(left, None);
                        let b = self.value(right, None);
                        let dest = dest.unwrap_or_else(|| self.number_temp());
                        self.emit(Instruction::FastMaths { operator: *operator, size, dest, a, b, failure });
                        dest
                    }
                    _ => {
                        let a = self.value(left, None);
                        let b = self.value(right, None);
                        let type_id = self.type_id(ty);
                        let dest = dest.unwrap_or_else(|| self.heap_temp());
                        self.emit(Instruction::HeapMaths { operator: *operator, type_id, dest, a, b, failure });
                        dest
                    }
                }
            }
            CheckedKind::Remainder { top, bottom, because } | CheckedKind::RoundedDivide { top, bottom, because } => {
                let remainder = matches!(expression.kind, CheckedKind::Remainder { .. });
                let failure = self.failure(expression.span, FailureKind::Maths { ty: ty.clone() }, because);
                let a = self.value(top, None);
                let b = self.value(bottom, None);
                match crate::bytecode::held(ty) {
                    Held::Whole(kind) => {
                        let dest = dest.unwrap_or_else(|| self.number_temp());
                        if remainder {
                            self.emit(Instruction::WholeRemainder { kind, dest, a, b, failure });
                        } else {
                            self.emit(Instruction::WholeRounded { kind, dest, a, b, failure });
                        }
                        dest
                    }
                    _ => {
                        let type_id = self.type_id(ty);
                        let dest = dest.unwrap_or_else(|| self.heap_temp());
                        self.emit(Instruction::HeapRounded { type_id, dest, a, b, failure });
                        dest
                    }
                }
            }
            CheckedKind::Compare { .. } | CheckedKind::Logic { .. } | CheckedKind::Not(_) => {
                let dest = dest.unwrap_or_else(|| self.number_temp());
                self.test_value(expression, dest);
                dest
            }
            CheckedKind::Family { family, operand } => {
                let table = self.value(operand, None);
                if *family == Family::Length {
                    let dest = dest.unwrap_or_else(|| self.number_temp());
                    let text = operand.ty == Type::Text;
                    self.emit(Instruction::Length { dest, source: table, text });
                    return dest;
                }
                let failure = self.failure(expression.span, FailureKind::Family { family: *family, ty: ty.clone() }, &[]);
                let type_id = self.type_id(ty);
                let dest = dest.unwrap_or_else(|| self.temp_for(ty));
                self.emit(Instruction::OfFamily { family: *family, type_id, dest, table, failure });
                dest
            }
            CheckedKind::Index { counting, index, table } => {
                let letters = table.ty == Type::Text;
                let table_register = self.value(table, None);
                let at = self.value(index, None);
                let failure = self.failure(expression.span, FailureKind::Place { counting: *counting, letters }, &[]);
                let dest = dest.unwrap_or_else(|| self.temp_for(ty));
                if letters {
                    self.emit(Instruction::Letter { dest, text: table_register, counting: *counting, at, failure });
                } else {
                    let heap = ty.is_heap();
                    self.emit(Instruction::Item { dest, table: table_register, counting: *counting, at, failure, heap });
                }
                dest
            }
            CheckedKind::Call(call) => self.call(call, dest),
            CheckedKind::Table(items) => {
                let Type::Table { item, .. } = ty else { return self.temp_for(ty) };
                let item_heap = item.is_heap();
                let dest = dest.unwrap_or_else(|| self.heap_temp());
                let built = self.heap_temp();
                self.emit(Instruction::NewTable { dest: built, keyed: false, item_heap });
                for value in items {
                    let source = self.owned(value);
                    self.emit(Instruction::Push { table: built, source });
                }
                self.emit(Instruction::MoveHeap { dest, source: built });
                dest
            }
            CheckedKind::KeyedTable(pairs) => {
                let Type::Table { item, .. } = ty else { return self.temp_for(ty) };
                let item_heap = item.is_heap();
                let dest = dest.unwrap_or_else(|| self.heap_temp());
                let built = self.heap_temp();
                self.emit(Instruction::NewTable { dest: built, keyed: true, item_heap });
                for (key, value) in pairs {
                    let key_register = self.value(key, None);
                    let source = self.owned(value);
                    let failure = self.failure(key.span, FailureKind::KeyTwice, &[]);
                    self.emit(Instruction::PushKeyed { table: built, key: key_register, source, failure });
                }
                self.emit(Instruction::MoveHeap { dest, source: built });
                dest
            }
            CheckedKind::Empty => {
                let dest = dest.unwrap_or_else(|| self.heap_temp());
                match ty {
                    Type::Table { keyed, item } => {
                        let item_heap = item.is_heap();
                        self.emit(Instruction::NewTable { dest, keyed: *keyed, item_heap });
                    }
                    _ => {
                        let constant = self.heap_constant(HeapValue::Text(runtime::TextValue::new(String::new())));
                        self.emit(Instruction::LoadHeap { dest, constant });
                    }
                }
                dest
            }
            CheckedKind::Resize(inner) => {
                let source = self.value(inner, None);
                let dest = dest.unwrap_or_else(|| self.number_temp());
                if source != dest {
                    self.emit(Instruction::Move { dest, source });
                }
                if let Type::Whole(whole) = ty {
                    let failure = self.failure(expression.span, FailureKind::Maths { ty: ty.clone() }, &[]);
                    let kind = whole_kind(*whole);
                    if kind != BIG_WHOLE {
                        self.emit(Instruction::CheckWhole { kind, source: dest, failure });
                    }
                }
                dest
            }
            CheckedKind::Fail(problem) | CheckedKind::WrittenRound { problem, .. } => {
                let failure = self.failure(expression.span, FailureKind::Written { problem: problem.clone(), ty: ty.clone() }, &[]);
                self.emit(Instruction::Fail { failure });
                dest.unwrap_or_else(|| self.temp_for(ty))
            }
        }
    }

    /// A value to hand on, its own: a table read from somewhere else is copied, since a table is a value.
    fn owned(&mut self, expression: &CheckedExpression) -> Register {
        let source = self.value(expression, None);
        if matches!(expression.ty, Type::Table { .. }) && !Self::fresh(expression) {
            let copy = self.heap_temp();
            self.emit(Instruction::CopyTable { dest: copy, source });
            return copy;
        }
        source
    }

    /// Made by this expression and held by nothing else.
    fn fresh(expression: &CheckedExpression) -> bool {
        matches!(expression.kind, CheckedKind::Table(_) | CheckedKind::KeyedTable(_) | CheckedKind::Empty | CheckedKind::Call(_))
    }

    // trace: decision.procedures.share
    fn call(&mut self, call: &CheckedCall, dest: Option<Register>) -> Register {
        let callee = &self.program.procedures[call.procedure];
        let mut arguments = Vec::new();
        for (index, argument) in call.arguments.iter().enumerate() {
            let param = &callee.params[index];
            let register = match argument {
                CheckedArgument::Value(value) => {
                    let register = self.owned(value);
                    if !param.bounds.is_empty() {
                        let failure = self.failure(value.span, FailureKind::Bounds { bounds: param.bounds.clone() }, &[]);
                        self.emit(Instruction::CheckBounds { source: register, failure });
                    }
                    register
                }
                CheckedArgument::Share(id) => *id as Register,
                CheckedArgument::Default => match &param.default {
                    Some(default) => self.value(default, None),
                    None => self.number_temp(),
                },
            };
            arguments.push(register);
        }
        let returns = callee.returns.clone().unwrap_or(Type::Nothing);
        let dest = dest.unwrap_or_else(|| self.temp_for(&returns));
        let failure = self.failure(call.span, FailureKind::TooDeep, &[]);
        self.call_arguments.push(arguments);
        let arguments = (self.call_arguments.len() - 1) as u32;
        self.emit(Instruction::Call { procedure: call.procedure as u32, dest, arguments, failure });
        dest
    }

    // ---- tests ----

    /// A test's answer, 1 or 0, in `dest`.
    fn test_value(&mut self, expression: &CheckedExpression, dest: Register) {
        match &expression.kind {
            CheckedKind::Compare { operator, left, right } => {
                let a = self.value(left, None);
                let b = self.value(right, None);
                match crate::bytecode::held(&left.ty) {
                    Held::Whole(_) | Held::TrueOrFalse => {
                        let unsigned = unsigned(&left.ty);
                        self.emit(Instruction::WholeCompare { operator: *operator, unsigned, dest, a, b });
                    }
                    Held::Fast(size) => {
                        self.emit(Instruction::FastCompare { operator: *operator, size, dest, a, b });
                    }
                    Held::Heap if left.ty == Type::Text => {
                        self.emit(Instruction::TextCompare { operator: *operator, dest, a, b });
                    }
                    Held::Heap => {
                        let type_id = self.type_id(&left.ty);
                        self.emit(Instruction::HeapCompare { operator: *operator, type_id, dest, a, b });
                    }
                }
            }
            CheckedKind::Logic { operator, left, right } => match operator {
                Logic::And | Logic::Or => {
                    let a = self.value(left, Some(dest));
                    if a != dest {
                        self.emit(Instruction::Move { dest, source: a });
                    }
                    let skip = if *operator == Logic::And {
                        self.emit(Instruction::JumpIfFalse { source: dest, target: 0 })
                    } else {
                        self.emit(Instruction::JumpIfTrue { source: dest, target: 0 })
                    };
                    let b = self.value(right, Some(dest));
                    if b != dest {
                        self.emit(Instruction::Move { dest, source: b });
                    }
                    let end = self.here();
                    self.patch(skip, end);
                }
                // trace: decision.conditions.short_circuit
                Logic::AndAlso | Logic::OrAlso => {
                    let a = self.value(left, None);
                    let b = self.value(right, None);
                    self.emit(Instruction::Move { dest, source: a });
                    let skip = if *operator == Logic::AndAlso {
                        self.emit(Instruction::JumpIfFalse { source: a, target: 0 })
                    } else {
                        self.emit(Instruction::JumpIfTrue { source: a, target: 0 })
                    };
                    self.emit(Instruction::Move { dest, source: b });
                    let end = self.here();
                    self.patch(skip, end);
                }
            },
            CheckedKind::Not(inner) => {
                let source = self.value(inner, None);
                self.emit(Instruction::Not { dest, source });
            }
            _ => {
                let source = self.value(expression, Some(dest));
                if source != dest {
                    self.emit(Instruction::Move { dest, source });
                }
            }
        }
    }

    /// Jumps to a target (patched later) when the test comes out `when`; returns the jumps to patch.
    fn jump_when(&mut self, test: &CheckedExpression, when: bool) -> Vec<usize> {
        match &test.kind {
            CheckedKind::Compare { operator, left, right } if matches!(crate::bytecode::held(&left.ty), Held::Whole(_)) => {
                let operator = if when { *operator } else { opposite(*operator) };
                let unsigned = unsigned(&left.ty);
                let a = self.value(left, None);
                if let CheckedKind::Constant(Constant::Whole(by)) = right.kind
                    && let Ok(by) = i32::try_from(by)
                {
                    return vec![self.emit(Instruction::JumpIfWholeBy { operator, unsigned, a, by, target: 0 })];
                }
                let b = self.value(right, None);
                vec![self.emit(Instruction::JumpIfWhole { operator, unsigned, a, b, target: 0 })]
            }
            CheckedKind::Logic { operator: Logic::And, left, right } => {
                if when {
                    // Both must hold: past the left when it fails, then the right decides.
                    let fails = self.jump_when(left, false);
                    let jumps = self.jump_when(right, true);
                    let after = self.here();
                    for at in fails {
                        self.patch(at, after);
                    }
                    jumps
                } else {
                    let mut jumps = self.jump_when(left, false);
                    jumps.extend(self.jump_when(right, false));
                    jumps
                }
            }
            CheckedKind::Logic { operator: Logic::Or, left, right } => {
                if when {
                    let mut jumps = self.jump_when(left, true);
                    jumps.extend(self.jump_when(right, true));
                    jumps
                } else {
                    let holds = self.jump_when(left, true);
                    let jumps = self.jump_when(right, false);
                    let after = self.here();
                    for at in holds {
                        self.patch(at, after);
                    }
                    jumps
                }
            }
            CheckedKind::Not(inner) => self.jump_when(inner, !when),
            CheckedKind::Constant(Constant::TrueOrFalse(value)) => {
                if *value == when { vec![self.emit(Instruction::Jump { target: 0 })] } else { Vec::new() }
            }
            _ => {
                let register = self.number_temp();
                let source = self.value(test, Some(register));
                if when {
                    vec![self.emit(Instruction::JumpIfTrue { source, target: 0 })]
                } else {
                    vec![self.emit(Instruction::JumpIfFalse { source, target: 0 })]
                }
            }
        }
    }

    // ---- statements ----

    fn block(&mut self, statements: &[CheckedStatement]) {
        for statement in statements {
            let (numbers, heaps) = (self.next_number, self.next_heap);
            self.statement(statement);
            self.next_number = numbers;
            self.next_heap = heaps;
        }
    }

    fn statement(&mut self, statement: &CheckedStatement) {
        match statement {
            CheckedStatement::Set { variable, value, site } => {
                let register = *variable as Register;
                let variable_type = self.procedure.variables[*variable].ty.clone();
                if variable_type.is_heap() {
                    let source = self.value(value, None);
                    let shared = matches!(value.kind, CheckedKind::Share(_));
                    if matches!(variable_type, Type::Table { .. }) && !shared && !Self::fresh(value) {
                        self.emit(Instruction::CopyTable { dest: register, source });
                    } else if source != register {
                        self.emit(Instruction::MoveHeap { dest: register, source });
                    }
                } else {
                    let source = self.value(value, Some(register));
                    let bounds = &self.procedure.variables[*variable].bounds;
                    if !bounds.is_empty() {
                        // A bound is checked before the variable changes, so `because:` and later lines see the old value.
                        let kind = FailureKind::Bounds { bounds: bounds.clone() };
                        let failure = self.failure(value.span, kind, &[]);
                        let checked = if source == register {
                            let temp = self.number_temp();
                            // The answer is already in the variable; check it there.
                            let _ = temp;
                            register
                        } else {
                            source
                        };
                        self.emit(Instruction::CheckBounds { source: checked, failure });
                    }
                    if source != register {
                        self.emit(Instruction::Move { dest: register, source });
                    }
                }
                self.set_site(*variable, *site);
            }
            CheckedStatement::Show(pieces) => {
                // trace: decision.show.pieces
                // Every piece is worked out before anything is shown.
                let mut registers = Vec::new();
                for piece in pieces {
                    let register = self.value(piece, None);
                    registers.push((register, piece.ty.clone(), piece.span));
                }
                for (source, ty, span) in registers {
                    let shown_type = match ty {
                        Type::WrittenWhole | Type::WrittenDecimal => Type::Text,
                        Type::WrittenFraction => Type::Fraction(Size::Big),
                        other => other,
                    };
                    let type_id = self.type_id(&shown_type);
                    let failure = self.failure(span, FailureKind::FractionShow, &[]);
                    self.emit(Instruction::ShowPiece { source, type_id, failure });
                }
                self.emit(Instruction::ShowEnd);
            }
            CheckedStatement::If { branches, otherwise } => {
                let mut ends = Vec::new();
                for (test, body) in branches {
                    let skips = self.jump_when(test, false);
                    self.block(body);
                    ends.push(self.emit(Instruction::Jump { target: 0 }));
                    let next = self.here();
                    for at in skips {
                        self.patch(at, next);
                    }
                }
                self.block(otherwise);
                let end = self.here();
                for at in ends {
                    self.patch(at, end);
                }
            }
            CheckedStatement::RepeatTimes { count, body, span } => {
                // trace: decision.loops.repeat
                let limit = self.number_temp();
                let source = self.value(count, Some(limit));
                if source != limit {
                    self.emit(Instruction::Move { dest: limit, source });
                }
                let failure = self.failure(*span, FailureKind::Count, &[]);
                self.emit(Instruction::CheckCount { source: limit, failure });
                let counter = self.number_temp();
                self.emit(Instruction::Load { dest: counter, value: 0 });
                let skip = self.emit(Instruction::JumpIfWhole { operator: Compare::AtLeast, unsigned: false, a: counter, b: limit, target: 0 });
                let top = self.here();
                let loop_id = self.loops.len() as u16;
                self.loops.push(top);
                self.stops.push(Vec::new());
                self.block(body);
                self.emit(Instruction::RepeatNext { counter, limit, target: top, loop_id });
                let end = self.here();
                self.patch(skip, end);
                for at in self.stops.pop().unwrap_or_default() {
                    self.patch(at, end);
                }
            }
            CheckedStatement::RepeatUntil { test, body } => {
                // trace: decision.loops.until
                let top = self.here();
                let loop_id = self.loops.len() as u16;
                self.loops.push(top);
                let exits = self.jump_when(test, true);
                self.stops.push(exits);
                self.block(body);
                self.emit(Instruction::LoopBack { target: top, loop_id });
                let end = self.here();
                for at in self.stops.pop().unwrap_or_default() {
                    self.patch(at, end);
                }
            }
            CheckedStatement::RepeatForever { body } => {
                // trace: decision.loops.forever
                let top = self.here();
                let loop_id = self.loops.len() as u16;
                self.loops.push(top);
                self.stops.push(Vec::new());
                self.block(body);
                self.emit(Instruction::LoopBack { target: top, loop_id });
                let end = self.here();
                for at in self.stops.pop().unwrap_or_default() {
                    self.patch(at, end);
                }
            }
            CheckedStatement::ForEach { key, item, table, body } => {
                // trace: decision.tables.of_family
                // The loop goes through the table as it was when it began, since a table is a value.
                let source = self.value(table, None);
                let snapshot = self.heap_temp();
                self.emit(Instruction::CopyTable { dest: snapshot, source });
                let length = self.number_temp();
                self.emit(Instruction::Length { dest: length, source: snapshot, text: false });
                let index = self.number_temp();
                self.emit(Instruction::Load { dest: index, value: 0 });
                let exit = self.emit(Instruction::JumpIfWhole { operator: Compare::AtLeast, unsigned: false, a: index, b: length, target: 0 });
                let top = self.here();
                let loop_id = self.loops.len() as u16;
                self.loops.push(top);
                let (item_id, item_span) = *item;
                let item_type = &self.procedure.variables[item_id].ty;
                let heap = item_type.is_heap();
                let failure = self.failure(table.span, FailureKind::Place { counting: Counting::Offset, letters: false }, &[]);
                self.emit(Instruction::Item { dest: item_id as Register, table: snapshot, counting: Counting::Offset, at: index, failure, heap });
                self.set_site(item_id, item_span);
                if let Some((key_id, key_span)) = key {
                    self.emit(Instruction::KeyAt { dest: *key_id as Register, table: snapshot, at: index });
                    self.set_site(*key_id, *key_span);
                }
                self.stops.push(Vec::new());
                self.block(body);
                self.emit(Instruction::RepeatNext { counter: index, limit: length, target: top, loop_id });
                let end = self.here();
                self.patch(exit, end);
                for at in self.stops.pop().unwrap_or_default() {
                    self.patch(at, end);
                }
            }
            CheckedStatement::Stop => {
                let at = self.emit(Instruction::Jump { target: 0 });
                if let Some(stops) = self.stops.last_mut() {
                    stops.push(at);
                }
            }
            CheckedStatement::Finish => {
                self.emit(Instruction::Finish);
            }
            CheckedStatement::Return(value) => {
                let source = self.value(value, None);
                self.emit(Instruction::Return { source });
            }
            CheckedStatement::Add { table, key, item, at, span } => {
                // trace: decision.tables.add
                let table_register = *table as Register;
                let source = self.owned(item);
                match (key, at) {
                    (Some(key), _) => {
                        let key = self.value(key, None);
                        let failure = self.failure(*span, FailureKind::KeyTwice, &[]);
                        self.emit(Instruction::AddKeyed { table: table_register, key, source, failure });
                    }
                    (None, Some((counting, index))) => {
                        let at = self.value(index, None);
                        let name = self.procedure.variables[*table].name.clone();
                        let failure = self.failure(index.span, FailureKind::Hole { counting: *counting, table: name }, &[]);
                        self.emit(Instruction::AddAt { table: table_register, source, counting: *counting, at, failure });
                    }
                    (None, None) => {
                        self.emit(Instruction::Add { table: table_register, source });
                    }
                }
            }
            CheckedStatement::Call(call) => {
                self.call(call, None);
            }
            CheckedStatement::OnlyIf { test, statement } => {
                let skips = self.jump_when(test, false);
                self.statement(statement);
                let end = self.here();
                for at in skips {
                    self.patch(at, end);
                }
            }
        }
    }
}

/// Every place each variable gets its value.
fn definition_sites(statements: &[CheckedStatement], sites: &mut HashMap<VariableId, Vec<Span>>) {
    let mut note = |id: VariableId, span: Span, sites: &mut HashMap<VariableId, Vec<Span>>| {
        let list = sites.entry(id).or_default();
        if !list.contains(&span) {
            list.push(span);
        }
    };
    for statement in statements {
        match statement {
            CheckedStatement::Set { variable, site, .. } => note(*variable, *site, sites),
            CheckedStatement::If { branches, otherwise } => {
                for (_, body) in branches {
                    definition_sites(body, sites);
                }
                definition_sites(otherwise, sites);
            }
            CheckedStatement::RepeatTimes { body, .. }
            | CheckedStatement::RepeatUntil { body, .. }
            | CheckedStatement::RepeatForever { body } => definition_sites(body, sites),
            CheckedStatement::ForEach { key, item, body, .. } => {
                if let Some((id, span)) = key {
                    note(*id, *span, sites);
                }
                note(item.0, item.1, sites);
                definition_sites(body, sites);
            }
            CheckedStatement::OnlyIf { statement, .. } => definition_sites(std::slice::from_ref(statement), sites),
            _ => {}
        }
    }
}

fn shows_or_calls(statements: &[CheckedStatement], shows: &mut bool, calls: &mut Vec<ProcedureId>) {
    fn in_expression(expression: &CheckedExpression, calls: &mut Vec<ProcedureId>) {
        match &expression.kind {
            CheckedKind::Call(call) => {
                calls.push(call.procedure);
                for argument in &call.arguments {
                    if let CheckedArgument::Value(value) = argument {
                        in_expression(value, calls);
                    }
                }
            }
            CheckedKind::Maths { left, right, .. }
            | CheckedKind::Compare { left, right, .. }
            | CheckedKind::Logic { left, right, .. } => {
                in_expression(left, calls);
                in_expression(right, calls);
            }
            CheckedKind::Remainder { top, bottom, .. } | CheckedKind::RoundedDivide { top, bottom, .. } => {
                in_expression(top, calls);
                in_expression(bottom, calls);
            }
            CheckedKind::Not(inner) | CheckedKind::Resize(inner) => in_expression(inner, calls),
            CheckedKind::Family { operand, .. } => in_expression(operand, calls),
            CheckedKind::Index { index, table, .. } => {
                in_expression(index, calls);
                in_expression(table, calls);
            }
            CheckedKind::Table(items) => items.iter().for_each(|item| in_expression(item, calls)),
            CheckedKind::KeyedTable(pairs) => pairs.iter().for_each(|(key, value)| {
                in_expression(key, calls);
                in_expression(value, calls);
            }),
            _ => {}
        }
    }
    for statement in statements {
        match statement {
            CheckedStatement::Set { value, .. } | CheckedStatement::Return(value) => in_expression(value, calls),
            CheckedStatement::Show(pieces) => {
                *shows = true;
                pieces.iter().for_each(|piece| in_expression(piece, calls));
            }
            CheckedStatement::If { branches, otherwise } => {
                for (test, body) in branches {
                    in_expression(test, calls);
                    shows_or_calls(body, shows, calls);
                }
                shows_or_calls(otherwise, shows, calls);
            }
            CheckedStatement::RepeatTimes { count: value, body, .. }
            | CheckedStatement::RepeatUntil { test: value, body }
            | CheckedStatement::ForEach { table: value, body, .. } => {
                in_expression(value, calls);
                shows_or_calls(body, shows, calls);
            }
            CheckedStatement::RepeatForever { body } => shows_or_calls(body, shows, calls),
            CheckedStatement::Add { key, item, at, .. } => {
                if let Some(key) = key {
                    in_expression(key, calls);
                }
                in_expression(item, calls);
                if let Some((_, index)) = at {
                    in_expression(index, calls);
                }
            }
            CheckedStatement::Call(call) => in_expression(&CheckedExpression { kind: CheckedKind::Call(call.clone()), ty: Type::Nothing, span: call.span }, calls),
            CheckedStatement::OnlyIf { test, statement } => {
                in_expression(test, calls);
                shows_or_calls(std::slice::from_ref(statement), shows, calls);
            }
            CheckedStatement::Stop | CheckedStatement::Finish => {}
        }
    }
}

// trace: decision.compiler.precompute
/// Which procedures are pure: no `share` param, no `show`, and calling only procedures like them. (TAKL has no input
/// yet; when it does, reading it makes a procedure impure too.)
fn purity(program: &CheckedProgram) -> Vec<bool> {
    let facts: Vec<(bool, Vec<ProcedureId>)> = program
        .procedures
        .iter()
        .map(|procedure| {
            let mut shows = false;
            let mut calls = Vec::new();
            shows_or_calls(&procedure.body, &mut shows, &mut calls);
            let shares = procedure.params.iter().any(|param| param.share);
            (!shows && !shares, calls)
        })
        .collect();
    let mut pure: Vec<bool> = facts.iter().map(|(clean, _)| *clean).collect();
    loop {
        let mut changed = false;
        for (index, (_, calls)) in facts.iter().enumerate() {
            if pure[index] && calls.iter().any(|callee| !pure[*callee]) {
                pure[index] = false;
                changed = true;
            }
        }
        if !changed {
            return pure;
        }
    }
}

fn remap(instruction: &mut Instruction, map: &impl Fn(Register) -> Register) {
    use Instruction::*;
    match instruction {
        Load { dest, .. } | LoadHeap { dest, .. } | SetSite { dest, .. } | NewTable { dest, .. } => *dest = map(*dest),
        Move { dest, source } | MoveHeap { dest, source } | CopyTable { dest, source } | Not { dest, source } => {
            *dest = map(*dest);
            *source = map(*source);
        }
        WholeMaths { dest, a, b, .. }
        | WholeRemainder { dest, a, b, .. }
        | WholeRounded { dest, a, b, .. }
        | WholeCompare { dest, a, b, .. }
        | FastMaths { dest, a, b, .. }
        | FastCompare { dest, a, b, .. }
        | HeapMaths { dest, a, b, .. }
        | HeapRounded { dest, a, b, .. }
        | HeapCompare { dest, a, b, .. }
        | TextCompare { dest, a, b, .. } => {
            *dest = map(*dest);
            *a = map(*a);
            *b = map(*b);
        }
        WholeMathsBy { dest, a, .. } => {
            *dest = map(*dest);
            *a = map(*a);
        }
        JumpIfWhole { a, b, .. } => {
            *a = map(*a);
            *b = map(*b);
        }
        JumpIfWholeBy { a, .. } => *a = map(*a),
        CheckWhole { source, .. } | CheckBounds { source, .. } | CheckCount { source, .. } | ShowPiece { source, .. } => {
            *source = map(*source)
        }
        JumpIfTrue { source, .. } | JumpIfFalse { source, .. } | Return { source } => *source = map(*source),
        RepeatNext { counter, limit, .. } => {
            *counter = map(*counter);
            *limit = map(*limit);
        }
        Call { dest, .. } => *dest = map(*dest),
        Push { table, source } | Add { table, source } => {
            *table = map(*table);
            *source = map(*source);
        }
        PushKeyed { table, key, source, .. } | AddKeyed { table, key, source, .. } => {
            *table = map(*table);
            *key = map(*key);
            *source = map(*source);
        }
        AddAt { table, source, at, .. } => {
            *table = map(*table);
            *source = map(*source);
            *at = map(*at);
        }
        Length { dest, source, .. } => {
            *dest = map(*dest);
            *source = map(*source);
        }
        Item { dest, table, at, .. } | KeyAt { dest, table, at } => {
            *dest = map(*dest);
            *table = map(*table);
            *at = map(*at);
        }
        Letter { dest, text, at, .. } => {
            *dest = map(*dest);
            *text = map(*text);
            *at = map(*at);
        }
        OfFamily { dest, table, .. } => {
            *dest = map(*dest);
            *table = map(*table);
        }
        Jump { .. } | LoopBack { .. } | ShowEnd | Finish | Fail { .. } => {}
    }
}

pub fn lower(program: &CheckedProgram) -> Program {
    let pure = purity(program);
    let mut procedures = Vec::new();
    for (index, procedure) in program.procedures.iter().enumerate() {
        let mut sites = HashMap::new();
        for param in &procedure.params {
            if let Some(span) = procedure.variables[param.variable].param_site {
                sites.insert(param.variable, vec![span]);
            }
        }
        definition_sites(&procedure.body, &mut sites);
        let variable_count = procedure.variables.len() as u16;
        let mut site_registers = HashMap::new();
        let mut fixed_sites = HashMap::new();
        let mut next = variable_count;
        let mut ids: Vec<&VariableId> = sites.keys().collect();
        ids.sort();
        for id in ids {
            let spans = &sites[id];
            if spans.len() == 1 {
                fixed_sites.insert(*id, spans[0]);
            } else if procedure.variables[*id].ty.is_number() {
                site_registers.insert(*id, next);
                next += 1;
            }
        }
        let mut lowerer = Lowerer {
            program,
            procedure,
            instructions: Vec::new(),
            number_base: next,
            next_number: 0,
            most_numbers: 0,
            next_heap: 0,
            most_heaps: 0,
            site_registers,
            fixed_sites,
            sites: Vec::new(),
            heap_constants: Vec::new(),
            failures: Vec::new(),
            types: Vec::new(),
            loops: Vec::new(),
            stops: Vec::new(),
            call_arguments: Vec::new(),
        };
        // A param that gets its value in more than one place starts at its name in the first line.
        for param in &procedure.params {
            if let (Some(span), true) = (procedure.variables[param.variable].param_site, lowerer.site_registers.contains_key(&param.variable)) {
                lowerer.set_site(param.variable, span);
            }
        }
        lowerer.block(&procedure.body);
        // Falling off the end can't happen in a checked program, but the bytecode always ends.
        lowerer.emit(Instruction::Finish);
        let heap_base = lowerer.number_base + lowerer.most_numbers;
        let map = |register: Register| if register >= HEAP_TEMP { heap_base + (register - HEAP_TEMP) } else { register };
        for instruction in &mut lowerer.instructions {
            remap(instruction, &map);
        }
        for arguments in &mut lowerer.call_arguments {
            for register in arguments.iter_mut() {
                *register = map(*register);
            }
        }
        let registers = (heap_base + lowerer.most_heaps) as usize;
        let mut kinds = vec![RegisterKind::Number(Held::Whole(BIG_WHOLE)); registers];
        for (id, variable) in procedure.variables.iter().enumerate() {
            kinds[id] = if variable.ty.is_heap() { RegisterKind::Heap } else { RegisterKind::Number(held(&variable.ty)) };
        }
        for kind in kinds.iter_mut().skip(heap_base as usize) {
            *kind = RegisterKind::Heap;
        }
        let param_types: Vec<Type> = procedure.params.iter().map(|param| procedure.variables[param.variable].ty.clone()).collect();
        let param_ranges = procedure
            .params
            .iter()
            .map(|param| match &procedure.variables[param.variable].ty {
                Type::Whole(whole) => Some((
                    param.bounds.least.unwrap_or(whole.smallest()),
                    param.bounds.most.unwrap_or(whole.biggest()),
                )),
                Type::TrueOrFalse => Some((0, 1)),
                _ => None,
            })
            .collect();
        procedures.push(Procedure {
            name: procedure.name.clone(),
            instructions: lowerer.instructions,
            registers,
            kinds,
            params: procedure.params.len(),
            returns: procedure.returns.clone(),
            heap_constants: lowerer.heap_constants,
            failures: lowerer.failures,
            sites: lowerer.sites,
            call_arguments: lowerer.call_arguments,
            types: lowerer.types,
            loops: lowerer.loops,
            pure: pure[index],
            param_ranges,
            param_types,
        });
    }
    Program { procedures, start: program.start as u32 }
}
