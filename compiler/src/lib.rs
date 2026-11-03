//! TAKL's compiler: the front end, the bytecode, the interpreter that runs it, TAKL's IR, and Cranelift at two
//! strengths.
//!
//! trace: decision.backend.bytecode

pub mod check;
pub mod checked;
pub mod failure;
pub mod lexer;
pub mod messages;
pub mod numbers;
pub mod parser;
pub mod source;
pub mod syntax;
pub mod types;

/// What a program showed, and the exit code it ended with.
pub struct Outcome {
    pub output: String,
    pub exit_code: u8,
}

/// Checks a program; on mistakes, what a kid sees.
pub fn check_program(name: &str, text: &str) -> Result<checked::CheckedProgram, String> {
    let source = source::Source::new(name, text);
    let (program, failures) = parser::parse(&source);
    check::check(&source, &program, failures).map_err(|failures| failure::render_before_running(&failures, &source))
}
