//! The syntax examples in `--help` must be valid diagrams.

mod common;

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

    let flowchart = examples(&help, "Two comment lines drawtool reads");
    let graph = common::graph(&flowchart).unwrap_or_else(|e| panic!("{e}\n{flowchart}"));
    let marked = graph.g.node_weights().filter(|n| n.mark == drawtool::Mark::Highlight).count();
    assert_eq!((marked, graph.stacks.len()), (3, 1), "{flowchart}");
}
