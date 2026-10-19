//! Highlighting a path.

mod common;
use common::graph;
use drawtool::{highlight, EdgeDef, Graph, Mark, Style};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

fn node<'g>(g: &'g Graph, id: &str) -> &'g drawtool::NodeDef {
    g.g.node_weights().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

/// (source id, target id, style, label) of every edge.
fn edges(g: &Graph) -> Vec<(String, String, Style, Option<String>)> {
    let id = |v: NodeIndex| g.g[v].id.clone();
    let label = |e: &EdgeDef| e.label.as_ref().map(|l| l.join("\n"));
    g.g.edge_references().map(|e| (id(e.source()), id(e.target()), e.weight().style, label(e.weight()))).collect()
}

#[test]
fn highlight_marks_the_path_and_reports_bad_paths() {
    let mut g = graph("flowchart LR\na --> b\nc --> b\nb -.-> d").unwrap();
    highlight(&mut g, &["a".into(), "b".into(), "c".into()]).unwrap();
    let marks: Vec<_> = ["a", "b", "c", "d"].iter().map(|id| node(&g, id).mark).collect();
    assert_eq!(marks, vec![Mark::Highlight, Mark::Highlight, Mark::Highlight, Mark::None]);
    let styles: Vec<_> = edges(&g).into_iter().map(|e| e.2).collect();
    assert_eq!(styles, vec![Style::Thick, Style::Thick, Style::Dotted], "edges are thickened in either direction");

    let mut g = graph("flowchart LR\na --> b\nc").unwrap();
    assert!(highlight(&mut g, &["a".into(), "nope".into()]).unwrap_err().message.contains("no node \"nope\""));
    assert!(highlight(&mut g, &["a".into(), "c".into()]).unwrap_err().message.contains("no edge"));
}
