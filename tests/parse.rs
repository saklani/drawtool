//! The flowchart parser, input by input.

use drawtool::{parse, Direction, Shape, Style};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;

/// (style, arrow, label) of the single edge in `src`.
fn edge(src: &str) -> (Style, bool, Option<String>) {
    let g = parse(src).unwrap_or_else(|e| panic!("{src:?}: {e}"));
    assert_eq!(g.g.edge_count(), 1, "{src:?}");
    let e = &g.g[EdgeIndex::new(0)];
    (e.style, e.arrow, e.label.as_ref().map(|l| l.join("\n")))
}

#[test]
fn every_connector_form() {
    use Style::*;
    let cases: &[(&str, Style, bool, Option<&str>)] = &[
        ("a --> b", Solid, true, None),
        ("a-->b", Solid, true, None),
        ("a ----> b", Solid, true, None),
        ("a --- b", Solid, false, None),
        ("a -.-> b", Dotted, true, None),
        ("a -.- b", Dotted, false, None),
        ("a ==> b", Thick, true, None),
        ("a === b", Thick, false, None),
        ("a -->|yes| b", Solid, true, Some("yes")),
        ("a -- yes --> b", Solid, true, Some("yes")),
        ("A--text-->B", Solid, true, Some("text")),
        ("a -- yes ---> b", Solid, true, Some("yes")),
        ("a -- no arrow --- b", Solid, false, Some("no arrow")),
        ("a -. maybe .-> b", Dotted, true, Some("maybe")),
        ("a -. maybe .- b", Dotted, false, Some("maybe")),
        ("a == main ==> b", Thick, true, Some("main")),
        ("a == main === b", Thick, false, Some("main")),
        ("a -->|\"two<br>lines\"| b", Solid, true, Some("two\nlines")),
        ("a -->| | b", Solid, true, None),
    ];
    for &(src, style, arrow, label) in cases {
        assert_eq!(edge(src), (style, arrow, label.map(String::from)), "{src:?}");
    }
}

#[test]
fn every_shape() {
    let cases = [
        ("a[box]", Shape::Box),
        ("a(round)", Shape::Round),
        ("a{decision}", Shape::Decision),
        ("a([stadium])", Shape::Round),
        ("a[(database)]", Shape::Database),
        ("a[[subroutine]]", Shape::Box),
        ("a((circle))", Shape::Round),
        ("a{{hexagon}}", Shape::Decision),
        // Shapes without glyphs of their own are drawn as the nearest one.
        ("a(((double)))", Shape::Round),
        ("a[/lean/]", Shape::Box),
        ("a[\\lean\\]", Shape::Box),
        ("a[/trapezoid\\]", Shape::Box),
        ("a>flag]", Shape::Box),
        ("a", Shape::Box),
    ];
    for (src, shape) in cases {
        let g = parse(src).unwrap();
        assert_eq!(g.g[NodeIndex::new(0)].shape, shape, "{src:?}");
        let label = src.trim_start_matches('a').trim_matches(|c: char| !c.is_alphanumeric());
        assert_eq!(g.g[NodeIndex::new(0)].lines, vec![if label.is_empty() { "a" } else { label }], "{src:?}");
    }
}

#[test]
fn labels_quotes_and_line_breaks() {
    // As in Mermaid: only double quotes are stripped and only `<br>` breaks a line.
    let g = parse("a[\"quoted label\"]\nb[one<br/>two\\nthree]\nc['single']").unwrap();
    let lines: Vec<Vec<&str>> = g.g.node_weights().map(|n| n.lines.iter().map(String::as_str).collect()).collect();
    assert_eq!(lines, vec![vec!["quoted label"], vec!["one", "two\\nthree"], vec!["'single'"]]);
}

#[test]
fn bare_ids_are_their_own_label_and_the_last_label_wins() {
    let g = parse("a --> b\na[First] --> c\na(Second)\na --> d").unwrap();
    assert_eq!(g.g[NodeIndex::new(0)].lines, vec!["Second"]);
    assert_eq!(g.g[NodeIndex::new(0)].shape, Shape::Round);
    assert_eq!(g.g[NodeIndex::new(1)].lines, vec!["b"]);
}

#[test]
fn fan_out_fan_in_and_chains() {
    let pairs = |src: &str| -> Vec<(String, String)> {
        let g = parse(src).unwrap();
        g.g.edge_references().map(|e| (g.g[e.source()].id.clone(), g.g[e.target()].id.clone())).collect()
    };
    let p = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(pairs("a --> b & c"), vec![p("a", "b"), p("a", "c")]);
    assert_eq!(pairs("a & b --> c"), vec![p("a", "c"), p("b", "c")]);
    assert_eq!(pairs("a --> b --> c"), vec![p("a", "b"), p("b", "c")]);
    // A label ends at the first closing arrow.
    assert_eq!(pairs("a -- x --- b -- y --> c"), vec![p("a", "b"), p("b", "c")]);
    // `;` separates statements; `:::class` is ignored.
    assert_eq!(pairs("a[x] --> b[y];c-->d"), vec![p("a", "b"), p("c", "d")]);
    assert_eq!(pairs("a:::hot --> b[B]:::cold"), vec![p("a", "b")]);
}

#[test]
fn headers_comments_and_ignored_lines() {
    for (src, dir) in [
        ("flowchart TD\na", Direction::TD),
        ("graph TB\na", Direction::TD),
        ("direction TD\na", Direction::TD),
        ("flowchart\na", Direction::TD), // Mermaid's default once there is a header
        ("a", Direction::LR),            // no header: drawtool's own default
    ] {
        assert_eq!(parse(src).unwrap().direction, dir, "{src:?}");
    }
    let g = parse("%% a comment\na --> b %% trailing\nclassDef x fill:#f00\nclass a x\nb --> c;").unwrap();
    assert_eq!((g.g.node_count(), g.g.edge_count()), (3, 2));
    // As in Mermaid, `style` declares the node it styles.
    assert_eq!(parse("a --> b\nstyle c fill:#0f0").unwrap().g.node_count(), 3);
    // A subgraph's own `direction` doesn't change the diagram's.
    assert_eq!(parse("flowchart LR\nsubgraph s\ndirection TD\na\nend").unwrap().direction, Direction::LR);
}

#[test]
fn errors_point_at_line_and_column() {
    let at = |src: &str| {
        let e = parse(src).unwrap_err();
        (e.line, e.column)
    };
    assert_eq!(at("a --> b\n  b ~~> c"), (Some(2), Some(5)));
    assert_eq!(at("a --> "), (Some(1), Some(6)));
    assert_eq!(at("a[unclosed --> b"), (Some(1), Some(2)));
    assert_eq!(at("subgraph s\na"), (Some(1), None));
    let e = parse("x --> 数据 ~~> c").unwrap_err();
    assert_eq!(e.column, Some(10), "columns count characters, not bytes: {e}");
}

#[test]
fn mermaid_syntax_outside_the_subset_is_an_error() {
    let message = |src: &str| parse(src).unwrap_err().to_string();
    assert!(message("flowchart RL\na --> b").contains("`RL` isn't supported"));
    assert!(message("flowchart BT\na --> b").contains("`BT` isn't supported"));
    assert!(message("subgraph S\nx\nend\nS --> y").contains("both a subgraph and a node"));
    for src in ["a->b", "a <--> b", "a --o b", "a --x b", "a ~~~ b", "a-b --> c"] {
        assert!(parse(src).is_err(), "{src:?}");
    }
}

#[test]
fn the_first_subgraph_to_close_claims_a_node() {
    let group = |src: &str, id: &str| {
        let g = parse(src).unwrap();
        g.g.node_weights().find(|n| n.id == id).unwrap().group.map(|i| g.groups[i].title.clone())
    };
    let nested = "subgraph outer\na --> b\nsubgraph inner\nb\nend\nend";
    assert_eq!(group(nested, "a").as_deref(), Some("outer"));
    assert_eq!(group(nested, "b").as_deref(), Some("inner"));
    assert_eq!(group("subgraph one\na\nend\nsubgraph two\na\nend", "a").as_deref(), Some("one"));
    assert_eq!(group("a\nsubgraph s\na\nend", "a").as_deref(), Some("s"));
    assert_eq!(group("subgraph s\nstyle a fill:#f00\nend", "a"), None);
}
