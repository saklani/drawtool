mod common;
use common::graph;
use drawtool::{render_fit, Shape};

fn draw(src: &str) -> String {
    let g = graph(src).unwrap();
    render_fit(&g, g.direction, 0).text
}

#[test]
fn states_start_and_end_become_boxes() {
    let g = graph("stateDiagram-v2\ndirection LR\n[*] --> Idle\nIdle --> Busy : go\nBusy --> [*]").unwrap();
    let lines: Vec<String> = g.g.node_weights().map(|n| n.lines.join(" ")).collect();
    assert_eq!(lines, ["●", "Idle", "Busy", "◉"]);
    let out = draw("stateDiagram-v2\ndirection LR\n[*] --> Idle\nIdle --> Busy : go\nBusy --> [*]");
    assert!(out.contains("│ ● ├─►│ Idle ├──go─►│ Busy ├─►│ ◉ │"), "\n{out}");
}

#[test]
fn choices_notes_and_composites() {
    let src = "stateDiagram-v2\nstate c <<choice>>\nA --> c\nc --> B : yes\nnote right of A : hi\nstate B {\n  [*] --> X\n}";
    let g = graph(src).unwrap();
    let node = |text: &str| g.g.node_weights().find(|n| n.lines.join(" ") == text).unwrap();
    assert_eq!(g.g.node_weights().find(|n| n.id == "c").unwrap().shape, Shape::Decision);
    assert_eq!(g.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["B"]);
    assert_eq!((node("B").group, node("X").group, node("A").group), (Some(0), Some(0), None));
    let note = g.g.edge_indices().map(|e| &g.g[e]).find(|e| e.style == drawtool::Style::Dotted);
    assert!(note.is_some_and(|e| e.arrow == drawtool::Arrow::None), "a note hangs on a dotted line");
}

#[test]
fn mermaid_errors_point_at_the_line() {
    let e = graph("stateDiagram-v2\nA --> B\nA -->").unwrap_err();
    assert_eq!(e.line, Some(3), "{e}");
    let e = graph("stateDiagram-v2\nstate Running {\n[*] --> A").unwrap_err();
    assert_eq!((e.line, e.message.as_str()), (Some(2), "`state` is never closed with `}`"));
}
