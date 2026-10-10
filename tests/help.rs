//! The syntax examples in `--help` must be valid diagrams.

use std::process::Command;

/// The indented example lines of the `--help` section starting with
/// `header`, skipping summary lines that list several alternatives (" / ").
fn examples(help: &str, header: &str) -> String {
    let lines = help.lines().skip_while(|l| !l.starts_with(header)).skip(1);
    let block = lines.take_while(|l| l.starts_with("  ") || l.is_empty()).filter(|l| !l.contains(" / "));
    block.map(|l| l.trim_start_matches("  ")).collect::<Vec<_>>().join("\n")
}

#[test]
fn help_examples_parse() {
    let out = Command::new(env!("CARGO_BIN_EXE_drawtool")).arg("--help").output().expect("run drawtool --help");
    let help = String::from_utf8(out.stdout).unwrap();

    let flowchart = examples(&help, "Syntax (a subset");
    let graph = drawtool::parse(&flowchart).unwrap_or_else(|e| panic!("{e}\n{flowchart}"));
    assert_eq!(graph.groups[0].title, "API server");

    let sequence = examples(&help, "Sequence diagrams");
    let seq = drawtool::parse_sequence(&sequence).unwrap_or_else(|e| panic!("{e}\n{sequence}"));
    let api = seq.participants.iter().find(|p| p.id == "api").unwrap();
    assert_eq!(api.lines, vec!["API server"], "trailing %% comments are not part of the text");
}
