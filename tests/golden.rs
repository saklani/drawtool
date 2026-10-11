//! Snapshot tests: every examples/*.mmd, fitted to 120 columns, must render
//! exactly as tests/golden/*.txt.
//! Regenerate after an intentional change with: UPDATE_GOLDEN=1 cargo test

use std::fs;
use std::path::Path;

#[test]
fn examples_match_golden_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(root.join("examples")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "mmd") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let opts = drawtool::Options { max_width: 120, ..Default::default() };
        let got = drawtool::render_source(&source, &opts).unwrap().text + "\n";
        let stem = path.file_stem().unwrap().to_str().unwrap();
        count += 1;
        let golden = root.join("tests/golden").join(format!("{stem}.txt"));
        if update {
            fs::write(&golden, &got).unwrap();
        } else if fs::read_to_string(&golden).ok().as_deref() != Some(got.as_str()) {
            failures.push(format!("{}:\n{got}", golden.display()));
        }
    }
    assert!(count > 0, "no examples found");
    assert!(failures.is_empty(), "output differs from golden:\n{}", failures.join("\n"));
}
