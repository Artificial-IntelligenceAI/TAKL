//! Holds every golden test to its `.out` and `.exit`, byte for byte, on the interpreter and on each of Cranelift's
//! strengths forced from the start.
//!
//! trace: decision.tests.plan
//!
//! A program is named by its path from the repository's root, as the oracle names it.

use std::path::{Path, PathBuf};
use takl::Mode;

fn programs(folder: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            programs(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "takl") {
            found.push(path);
        }
    }
}

fn run_all(mode: Mode) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the compiler lives in the repository").to_path_buf();
    let mut found = Vec::new();
    programs(&root.join("tests"), &mut found);
    found.sort();
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = found
            .chunks(found.len().div_ceil(8).max(1))
            .map(|chunk| {
                let root = root.clone();
                std::thread::Builder::new()
                    .stack_size(1 << 30)
                    .spawn_scoped(scope, move || {
                        let mut failures = Vec::new();
                        for path in chunk {
                            let name = path.strip_prefix(&root).expect("under the root").to_string_lossy().to_string();
                            let text = std::fs::read_to_string(path).expect("readable");
                            let outcome = takl::run(&name, &text, mode);
                            let expected = std::fs::read_to_string(path.with_extension("out")).unwrap_or_default();
                            let expected_exit: u8 = std::fs::read_to_string(path.with_extension("exit"))
                                .ok()
                                .and_then(|text| text.trim().parse().ok())
                                .unwrap_or(0);
                            if outcome.output != expected || outcome.exit_code != expected_exit {
                                failures.push(format!(
                                    "--- {} ({:?})\nexpected (exit {}):\n{}\ngot (exit {}):\n{}",
                                    name, mode, expected_exit, expected, outcome.exit_code, outcome.output
                                ));
                            }
                        }
                        failures
                    })
                    .expect("a thread")
            })
            .collect();
        handles.into_iter().flat_map(|handle| handle.join().expect("finished")).collect()
    });
    println!("{} golden programs on {:?}", found.len(), mode);
    assert!(failures.is_empty(), "{} of {} failed on {:?}:\n{}", failures.len(), found.len(), mode, failures.join("\n"));
}

#[test]
fn golden_on_the_interpreter() {
    run_all(Mode::Interpret);
}

#[test]
fn golden_on_quick() {
    run_all(Mode::Quick);
}

#[test]
fn golden_on_speed() {
    run_all(Mode::Speed);
}

#[test]
fn golden_tiered() {
    run_all(Mode::Tiered);
}

#[test]
fn golden_tiered_eagerly() {
    run_all(Mode::TieredEager);
}
