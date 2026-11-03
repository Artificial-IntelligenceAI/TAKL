//! The TAKL oracle: a plain reference interpreter that walks the parsed source.
//!
//! It is the executable spec. The compiler must match its output byte for byte on every golden test, so every rule
//! here is written to be checked against its decision by eye, never to be fast.
//!
//! trace: decision.house.optimize_early (the compiler is optimized from the start; the oracle stays plain)
//! trace: decision.house.naming

// An error is a whole message with its code lines, and a program stops at the first one while running, so carrying it
// by value is simpler than boxing it and costs nothing that matters.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod check;
pub mod diagnostic;
pub mod interpret;
pub mod lexer;
pub mod messages;
pub mod numbers;
pub mod parser;
pub mod source;
pub mod types;
pub mod value;

use diagnostic::Diagnostic;
use source::Source;

/// What running a program gave: everything it showed, then any error, and its exit code.
pub struct Outcome {
    pub output: String,
    pub exit_code: u8,
}

/// Runs one TAKL program. `path` is the name shown in error messages.
pub fn run(path: &str, text: &str) -> Outcome {
    let source = Source::new(path, text);
    let (program, mut errors) = parser::parse(&source);
    let checked = check::check(&program, &source);
    errors.extend(checked.errors);
    if !errors.is_empty() {
        return Outcome { output: diagnostic::render_compile_errors(&source, errors), exit_code: 1 };
    }
    let mut output = String::new();
    let result = interpret::run(&program, &checked.facts, &mut output);
    match result {
        Ok(code) => Outcome { output, exit_code: code },
        Err(error) => {
            output.push_str(&diagnostic::render(&source, &error));
            Outcome { output, exit_code: 1 }
        }
    }
}

/// Runs a program on a thread with a large stack, so deep recursion in a kid's program reaches TAKL's own depth limit
/// rather than overflowing the oracle's stack.
pub fn run_with_room(path: &str, text: &str) -> Outcome {
    let path = path.to_string();
    let text = text.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run(&path, &text))
        .map(|handle| handle.join().unwrap_or_else(|_| broke()))
        .unwrap_or_else(|_| broke())
}

/// rule 6: a kid's program never crashes TAKL; if the oracle itself fails, it says so.
fn broke() -> Outcome {
    Outcome { output: messages::takl_broke(), exit_code: 1 }
}

pub type Errors = Vec<Diagnostic>;
