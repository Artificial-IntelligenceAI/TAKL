//! Every `trace:` tag must name a decision in `DECISIONS.md`; the decisions nothing traces yet are listed.
//!
//! trace: decision.house.trace

use std::collections::BTreeSet;
use std::path::Path;

fn decision_names(decisions: &str) -> BTreeSet<String> {
    decisions
        .lines()
        .filter_map(|line| line.strip_prefix('`').and_then(|rest| rest.strip_suffix('`')))
        .filter(|name| name.starts_with("decision."))
        .map(str::to_string)
        .collect()
}

fn tags_in(folder: &Path, tags: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            tags_in(&path, tags);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = std::fs::read_to_string(&path).expect("readable");
            for (number, line) in text.lines().enumerate() {
                let Some((before, after)) = line.split_once("trace: ") else { continue };
                if !before.trim_start().starts_with("//") {
                    continue;
                }
                let name: String =
                    after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_').collect();
                tags.push((name, format!("{}:{}", path.display(), number + 1)));
            }
        }
    }
}

#[test]
fn every_tag_names_a_decision() {
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR"));
    let decisions = std::fs::read_to_string(oracle.join("../DECISIONS.md")).expect("DECISIONS.md is beside the oracle");
    let names = decision_names(&decisions);
    let mut tags = Vec::new();
    tags_in(&oracle.join("src"), &mut tags);
    tags_in(&oracle.join("tests"), &mut tags);

    let unknown: Vec<&(String, String)> = tags.iter().filter(|(name, _)| !names.contains(name)).collect();
    let traced: BTreeSet<&String> = tags.iter().map(|(name, _)| name).collect();
    let untraced: Vec<&String> = names.iter().filter(|name| !traced.contains(name)).collect();

    println!("{} decisions, {} traced, {} not traced yet:", names.len(), names.len() - untraced.len(), untraced.len());
    for name in &untraced {
        println!("  {}", name);
    }
    assert!(
        unknown.is_empty(),
        "trace tags naming no decision:\n{}",
        unknown.iter().map(|(name, at)| format!("  {} at {}", name, at)).collect::<Vec<_>>().join("\n")
    );
}
