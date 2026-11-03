//! TAKL's compiler: the front end, the bytecode, the interpreter that runs it, TAKL's IR, and Cranelift at two
//! strengths.
//!
//! trace: decision.backend.bytecode

pub mod bytecode;
pub mod check;
pub mod checked;
pub mod failure;
pub mod interpreter;
pub mod lexer;
pub mod lower;
pub mod messages;
pub mod numbers;
pub mod parser;
pub mod precompute;
pub mod runtime;
pub mod source;
pub mod syntax;
pub mod tiering;
pub mod types;
pub mod verify;

use std::sync::Arc;

/// What a program showed, and the exit code it ended with.
pub struct Outcome {
    pub output: String,
    pub exit_code: u8,
}

/// Where code runs: the interpreter alone, or every procedure sent to one of Cranelift's strengths from the start, or
/// the interpreter handing what gets hot to Cranelift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Interpret,
    Quick,
    Speed,
    Tiered,
}

// trace: decision.house.impl_rules
/// What a kid sees when TAKL itself fails.
pub fn broke(span: source::Span, what: &str) -> failure::Failure {
    failure::Failure::what(
        span,
        "TAKL broke, not you",
        format!("This is a mistake in TAKL itself ({}). Please report it, with your program, at\nhttps://github.com/Artificial-IntelligenceAI/TAKL/issues", what),
    )
}

/// Checks a program; on mistakes, what a kid sees.
pub fn check_program(name: &str, text: &str) -> Result<checked::CheckedProgram, String> {
    let source = source::Source::new(name, text);
    let (program, failures) = parser::parse(&source);
    check::check(&source, &program, failures).map_err(|failures| failure::render_before_running(&failures, &source))
}

/// Checks and runs a program, keeping everything it shows.
pub fn run(name: &str, text: &str, mode: Mode) -> Outcome {
    run_with(name, text, mode, false)
}

/// Checks and runs a program; `live` writes what it shows to stdout as it goes.
pub fn run_with(name: &str, text: &str, mode: Mode, live: bool) -> Outcome {
    let source = source::Source::new(name, text);
    let (program, failures) = parser::parse(&source);
    let checked = match check::check(&source, &program, failures) {
        Ok(checked) => checked,
        // trace: decision.errors.all
        Err(failures) => return shown(Outcome { output: failure::render_before_running(&failures, &source), exit_code: 1 }, live),
    };
    let bytecode = lower::lower(&checked);
    if let Err(problem) = verify::verify(&bytecode) {
        let output = broke(source::Span::new(1, 0, 0), &problem).render_running(&source);
        return shown(Outcome { output, exit_code: 1 }, live);
    }
    let program = Arc::new(bytecode);
    let mut machine = interpreter::Machine::new(Arc::clone(&program), live);
    machine.tiers = tiering::start(&program, mode);
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| machine.run_start()));
    let mut output = String::from_utf8_lossy(&machine.output).to_string();
    let exit_code = match ran {
        Ok(Ok(code)) => code,
        Ok(Err(interpreter::Stopped)) => {
            if let Some(failure) = &machine.failure {
                output.push_str(&failure.render_running(&source));
            }
            1
        }
        Err(_) => {
            output.push_str(&broke(source::Span::new(1, 0, 0), "it panicked").render_running(&source));
            1
        }
    };
    shown(Outcome { output, exit_code }, live)
}

/// Writes what is left to show when running live.
fn shown(mut outcome: Outcome, live: bool) -> Outcome {
    if live {
        use std::io::Write;
        let _ = std::io::stdout().write_all(outcome.output.as_bytes());
        let _ = std::io::stdout().flush();
        outcome.output.clear();
    }
    outcome
}
