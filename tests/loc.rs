//! Size budget: the code in src/ (non-blank, non-comment lines, as formatted
//! by `cargo fmt` with rustfmt.toml) must not grow past what the
//! rearchitecture and layout improvements left it at (down from 3,049).
//! Tests are not counted.

use std::fs;

const BUDGET: usize = 2475;

#[test]
fn src_stays_within_line_budget() {
    let mut total = 0;
    let mut per_file = Vec::new();
    for entry in fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        let n = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).count();
        per_file.push(format!("{n:5} {}", path.file_name().unwrap().to_string_lossy()));
        total += n;
    }
    per_file.sort();
    assert!(total <= BUDGET, "src/ has {total} code lines, budget is {BUDGET}:\n{}", per_file.join("\n"));
}
