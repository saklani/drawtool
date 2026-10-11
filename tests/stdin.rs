//! The CLI reads its diagram from stdin.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn drawtool(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_drawtool"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run drawtool");
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

#[test]
fn renders_stdin() {
    let out = drawtool(&["-w", "0"], "flowchart LR\na[build] --> b[ship]\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("│ build ├─►│ ship │"), "{}", stdout(&out));
}

#[test]
fn empty_stdin_is_an_error() {
    let out = drawtool(&[], "  \n");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no diagram"), "{}", stderr(&out));
}

#[test]
fn a_file_argument_is_rejected() {
    assert_eq!(drawtool(&["diagram.mmd"], "a --> b").status.code(), Some(2));
}
