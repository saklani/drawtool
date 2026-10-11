//! Snapshot tests: every examples/*.mmd, run through `drawtool -w 120`, must print
//! exactly as tests/golden/*.txt.
//! Regenerate after an intentional change with: UPDATE_GOLDEN=1 cargo test

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// What `drawtool -w 120` prints for `source`.
fn drawtool(source: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_drawtool"))
        .args(["-w", "120"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run drawtool");
    child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "drawtool failed on\n{source}");
    String::from_utf8(out.stdout).unwrap()
}

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
        let got = drawtool(&source);
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
