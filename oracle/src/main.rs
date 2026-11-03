//! `takl-oracle program.takl`: runs a program and prints what it shows, then any error.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: oracle <program.takl>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("can't read {}: {}", path, error);
            return ExitCode::from(2);
        }
    };
    let outcome = oracle::run_with_room(&path, &text);
    print!("{}", outcome.output);
    ExitCode::from(outcome.exit_code)
}
