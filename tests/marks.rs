//! Highlighting a path, and diffing two versions of a diagram.

use drawtool::{diff, highlight, parse, EdgeDef, Graph, Mark, Style};
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

fn diffed(before: &str, after: &str) -> Graph {
    diff(&parse(before).unwrap(), &parse(after).unwrap())
}

#[test]
fn added_removed_and_relabelled_nodes() {
    let g = diffed("a[A] --> b[B] --> c[C]", "a[A] --> b[B two] --> d[D]");
    assert_eq!((node(&g, "a").mark, node(&g, "a").lines.clone()), (Mark::None, vec!["A".to_string()]));
    assert_eq!((node(&g, "b").mark, node(&g, "b").lines.clone()), (Mark::Changed, vec!["~ B two".to_string()]));
    assert_eq!((node(&g, "d").mark, node(&g, "d").lines.clone()), (Mark::Added, vec!["+ D".to_string()]));
    assert_eq!((node(&g, "c").mark, node(&g, "c").lines.clone()), (Mark::Removed, vec!["- C".to_string()]));
}

#[test]
fn a_shape_change_alone_is_a_change() {
    let g = diffed("a[check]", "a{check}");
    assert_eq!(node(&g, "a").mark, Mark::Changed);
    assert_eq!(node(&g, "a").lines, vec!["~ check"]);
}

#[test]
fn added_and_removed_edges() {
    let g = diffed("a --> b\na --> c", "a --> b\nb --> c");
    let s = |x: &str| x.to_string();
    assert_eq!(
        edges(&g),
        vec![
            (s("a"), s("b"), Style::Solid, None),
            (s("b"), s("c"), Style::Thick, Some(s("+"))),
            (s("a"), s("c"), Style::Dotted, Some(s("-"))),
        ]
    );
}

#[test]
fn edges_whose_label_or_style_changed_are_marked() {
    let g = diffed("a -->|yes| b\nb --> c\nc --> d", "a -->|ok| b\nb -.-> c\nc --> d");
    let labels: Vec<_> = edges(&g).into_iter().map(|e| (e.2, e.3)).collect();
    assert_eq!(
        labels,
        vec![(Style::Solid, Some("~ ok".to_string())), (Style::Dotted, Some("~".to_string())), (Style::Solid, None),]
    );
}

#[test]
fn parallel_edges_are_matched_in_order_not_collapsed() {
    let g = diffed("a -->|one| b", "a -->|one| b\na -->|two| b");
    let labels: Vec<_> = edges(&g).into_iter().map(|e| e.3).collect();
    assert_eq!(labels, vec![Some("one".to_string()), Some("+ two".to_string())]);

    let g = diffed("a -->|one| b\na -->|two| b", "a -->|one| b");
    let labels: Vec<_> = edges(&g).into_iter().map(|e| e.3).collect();
    assert_eq!(labels, vec![Some("one".to_string()), Some("- two".to_string())]);
}

#[test]
fn removed_nodes_stay_in_their_group() {
    let before = "subgraph api [API]\n  a --> b\nend\nsubgraph old [Legacy]\n  x\nend\nc --> a";
    let after = "c --> a\nsubgraph api [API]\n  a\nend";
    let g = diffed(before, after);
    let group = |id: &str| node(&g, id).group.map(|i| g.groups[i].title.clone());
    assert_eq!(group("b"), Some("API".to_string()), "same title, so the after-version's frame");
    assert_eq!(group("x"), Some("Legacy".to_string()), "a vanished group comes back for its removed nodes");
    assert_eq!(g.groups.iter().filter(|gr| gr.title == "API").count(), 1);
}

#[test]
fn highlight_marks_the_path_and_reports_bad_paths() {
    let mut g = parse("a --> b\nc --> b\nb -.-> d").unwrap();
    highlight(&mut g, &["a".into(), "b".into(), "c".into()]).unwrap();
    let marks: Vec<_> = ["a", "b", "c", "d"].iter().map(|id| node(&g, id).mark).collect();
    assert_eq!(marks, vec![Mark::Highlight, Mark::Highlight, Mark::Highlight, Mark::None]);
    let styles: Vec<_> = edges(&g).into_iter().map(|e| e.2).collect();
    assert_eq!(styles, vec![Style::Thick, Style::Thick, Style::Dotted], "edges are thickened in either direction");

    let mut g = parse("a --> b\nc").unwrap();
    assert!(highlight(&mut g, &["a".into(), "nope".into()]).unwrap_err().message.contains("no node \"nope\""));
    assert!(highlight(&mut g, &["a".into(), "c".into()]).unwrap_err().message.contains("no edge"));
}
