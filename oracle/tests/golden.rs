//! Runs every program in `tests/` and holds its output and exit code to the files beside it, byte for byte.
//!
//! trace: decision.tests.plan
//!
//! A program's `.out` is everything it shows, then its error if it stops; `.exit` holds its exit code when that isn't
//! 0. Programs are named by their path from the repository's root, as compiler claude's runner must name them too.

use std::path::{Path, PathBuf};

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

#[test]
fn golden() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the oracle lives in the repository").to_path_buf();
    let mut found = Vec::new();
    programs(&root.join("tests"), &mut found);
    found.sort();
    let bless = std::env::var("TAKL_BLESS").is_ok();
    let mut failures = Vec::new();
    for path in &found {
        let name = path.strip_prefix(&root).expect("under the root").to_string_lossy().to_string();
        let text = std::fs::read_to_string(path).expect("readable");
        let outcome = oracle::run_with_room(&name, &text);
        let out_path = path.with_extension("out");
        let exit_path = path.with_extension("exit");
        if bless {
            std::fs::write(&out_path, &outcome.output).expect("writable");
            if outcome.exit_code == 0 {
                let _ = std::fs::remove_file(&exit_path);
            } else {
                std::fs::write(&exit_path, format!("{}\n", outcome.exit_code)).expect("writable");
            }
            continue;
        }
        let expected = std::fs::read_to_string(&out_path).unwrap_or_default();
        let expected_exit: u8 =
            std::fs::read_to_string(&exit_path).ok().and_then(|text| text.trim().parse().ok()).unwrap_or(0);
        if outcome.output != expected || outcome.exit_code != expected_exit {
            failures.push(format!(
                "--- {}\nexpected (exit {}):\n{}\ngot (exit {}):\n{}",
                name, expected_exit, expected, outcome.exit_code, outcome.output
            ));
        }
    }
    println!("{} golden programs", found.len());
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), found.len(), failures.join("\n"));
}
