//! The flowchart parser, input by input.

use drawtool::{parse, Arrow, Direction, Shape, Style};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;

/// (style, arrowheads, label) of the single edge in `src`.
fn edge(src: &str) -> (Style, Arrow, Option<String>) {
    let g = parse(&format!("flowchart LR\n{src}")).unwrap_or_else(|e| panic!("{src:?}: {e}"));
    assert_eq!(g.g.edge_count(), 1, "{src:?}");
    let e = &g.g[EdgeIndex::new(0)];
    (e.style, e.arrow, e.label.as_ref().map(|l| l.join("\n")))
}

#[test]
fn every_connector_form() {
    use Arrow::{Both, End};
    use Style::*;
    const OPEN: Arrow = Arrow::None;
    let cases: &[(&str, Style, Arrow, Option<&str>)] = &[
        ("a --> b", Solid, End, None),
        ("a-->b", Solid, End, None),
        ("a ----> b", Solid, End, None),
        ("a --- b", Solid, OPEN, None),
        ("a -.-> b", Dotted, End, None),
        ("a -.- b", Dotted, OPEN, None),
        ("a ==> b", Thick, End, None),
        ("a === b", Thick, OPEN, None),
        ("a -->|yes| b", Solid, End, Some("yes")),
        ("a -- yes --> b", Solid, End, Some("yes")),
        ("A--text-->B", Solid, End, Some("text")),
        ("a -- yes ---> b", Solid, End, Some("yes")),
        ("a -- no arrow --- b", Solid, OPEN, Some("no arrow")),
        ("a -. maybe .-> b", Dotted, End, Some("maybe")),
        ("a -. maybe .- b", Dotted, OPEN, Some("maybe")),
        ("a == main ==> b", Thick, End, Some("main")),
        ("a == main === b", Thick, OPEN, Some("main")),
        ("a -->|\"two<br>lines\"| b", Solid, End, Some("two\nlines")),
        ("a -->| | b", Solid, End, None),
        // Two-headed: one edge, a head at each end, laid out like `-->`.
        ("a <--> b", Solid, Both, None),
        ("a<---->b", Solid, Both, None),
        ("a <-.-> b", Dotted, Both, None),
        ("a <==> b", Thick, Both, None),
        ("a <-- sync --> b", Solid, Both, Some("sync")),
        ("a <-->|sync| b", Solid, Both, Some("sync")),
        ("a <-. sync ..-> b", Dotted, Both, Some("sync")),
        ("a <== sync ===> b", Thick, Both, Some("sync")),
        ("a[A] <--> b", Solid, Both, None),
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
        let g = parse(&format!("flowchart LR\n{src}")).unwrap();
        assert_eq!(g.g[NodeIndex::new(0)].shape, shape, "{src:?}");
        let label = src.trim_start_matches('a').trim_matches(|c: char| !c.is_alphanumeric());
        assert_eq!(g.g[NodeIndex::new(0)].lines, vec![if label.is_empty() { "a" } else { label }], "{src:?}");
    }
}

#[test]
fn labels_quotes_and_line_breaks() {
    // As in Mermaid: only double quotes are stripped and only `<br>` breaks a line.
    let g = parse("flowchart LR\na[\"quoted label\"]\nb[one<br/>two\\nthree]\nc['single']").unwrap();
    let lines: Vec<Vec<&str>> = g.g.node_weights().map(|n| n.lines.iter().map(String::as_str).collect()).collect();
    assert_eq!(lines, vec![vec!["quoted label"], vec!["one", "two\\nthree"], vec!["'single'"]]);
}

#[test]
fn bare_ids_are_their_own_label_and_the_last_label_wins() {
    let g = parse("flowchart LR\na --> b\na[First] --> c\na(Second)\na --> d").unwrap();
    assert_eq!(g.g[NodeIndex::new(0)].lines, vec!["Second"]);
    assert_eq!(g.g[NodeIndex::new(0)].shape, Shape::Round);
    assert_eq!(g.g[NodeIndex::new(1)].lines, vec!["b"]);
}

#[test]
fn fan_out_fan_in_and_chains() {
    let pairs = |src: &str| -> Vec<(String, String)> {
        let g = parse(&format!("flowchart LR\n{src}")).unwrap();
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
        ("flowchart\na", Direction::TD), // Mermaid's default once there is a header
        ("%% a comment first\nflowchart LR\na", Direction::LR),
    ] {
        assert_eq!(parse(src).unwrap().direction, dir, "{src:?}");
    }
    let g =
        parse("%% a comment\nflowchart LR\na --> b %% trailing\nclassDef x fill:#f00\nclass a x\nb --> c;").unwrap();
    assert_eq!((g.g.node_count(), g.g.edge_count()), (3, 2));
    // As in Mermaid, `style` declares the node it styles.
    assert_eq!(parse("flowchart LR\na --> b\nstyle c fill:#0f0").unwrap().g.node_count(), 3);
    // A subgraph's own `direction` doesn't change the diagram's.
    assert_eq!(parse("flowchart LR\nsubgraph s\ndirection TD\na\nend").unwrap().direction, Direction::LR);
}

#[test]
fn the_header_is_required_and_comes_once() {
    let message = |src: &str| parse(src).unwrap_err().to_string();
    assert!(message("a --> b").contains("no diagram"), "{}", message("a --> b"));
    assert!(message("%% only a comment").contains("no diagram"));
    assert!(message("flowchart LR\na\nflowchart TD").starts_with("line 3, column 1: unexpected flowchart"));
}

#[test]
fn errors_point_at_line_and_column() {
    let at = |src: &str| {
        let e = parse(&format!("flowchart LR\n{src}")).unwrap_err();
        (e.line, e.column)
    };
    // Each source gets a header line first, so its lines count from 2.
    assert_eq!(at("a --> b\n  b ~~> c"), (Some(3), Some(5)));
    assert_eq!(at("a --> "), (Some(2), Some(6)), "an expected node goes where the text ends");
    assert_eq!(at("a --> b\nb -->\n\n"), (Some(3), Some(6)), "not on a trailing blank line");
    assert_eq!(at("a[unclosed --> b"), (Some(2), Some(2)));
    assert_eq!(at("subgraph s\na"), (Some(3), Some(2)));
    let e = parse("flowchart LR\nx --> 数据 ~~> c").unwrap_err();
    assert_eq!(e.column, Some(10), "columns count characters, not bytes: {e}");
}

#[test]
fn standard_mermaid_parses_and_what_drawtool_cannot_draw_yet_is_an_error() {
    let message = |src: &str| parse(src).unwrap_err().to_string();
    assert!(message("flowchart RL\na --> b").contains("`RL` isn't supported yet"));
    assert!(message("flowchart BT\na --> b").contains("`BT` isn't supported yet"));
    assert!(message("flowchart LR\nsubgraph S\nx\nend\nS --> y").contains("edges to subgraphs"));
    for src in ["a --o b", "a --x b", "a ~~~ b", "a o--o b", "a@{ shape: cyl, label: \"db\" } --> b"] {
        assert!(parse(&format!("flowchart LR\n{src}")).is_ok(), "{src:?}");
    }
    for src in ["a->b", "a <-> b"] {
        assert!(parse(&format!("flowchart LR\n{src}")).is_err(), "Mermaid rejects {src:?}");
    }
    let g = parse("flowchart LR\na ~~~ b\nc@{ shape: cyl, label: \"db\" }").unwrap();
    assert_eq!(g.g.edge_count(), 0, "an invisible link draws nothing");
    assert_eq!(g.g.node_weights().last().map(|n| (n.lines.clone(), n.shape)), Some((vec!["db".into()], drawtool::Shape::Database)));
}

#[test]
fn the_first_subgraph_to_close_claims_a_node() {
    let group = |src: &str, id: &str| {
        let g = parse(&format!("flowchart LR\n{src}")).unwrap();
        g.g.node_weights().find(|n| n.id == id).unwrap().group.map(|i| g.groups[i].title.clone())
    };
    let nested = "subgraph outer\na --> b\nsubgraph inner\nb\nend\nend";
    assert_eq!(group(nested, "a").as_deref(), Some("outer"));
    assert_eq!(group(nested, "b").as_deref(), Some("inner"));
    assert_eq!(group("subgraph one\na\nend\nsubgraph two\na\nend", "a").as_deref(), Some("one"));
    assert_eq!(group("a\nsubgraph s\na\nend", "a").as_deref(), Some("s"));
    assert_eq!(group("subgraph s\nstyle a fill:#f00\nend", "a"), None);
}

#[test]
fn stack_lines_list_boxes_in_order() {
    let g = parse("flowchart LR\na --> b --> c\n%% stack: c, a").unwrap();
    let id = |i: &NodeIndex| g.g[*i].id.clone();
    assert_eq!(g.stacks.iter().map(|st| st.iter().map(id).collect::<Vec<_>>()).collect::<Vec<_>>(), [["c", "a"]]);
    // To Mermaid it's a comment.
    assert_eq!(parse("flowchart LR\na --> b\n%% stack: a b").unwrap().g.edge_count(), 1);
}

#[test]
fn bad_stack_lines_name_their_line() {
    let fail = |src: &str| {
        let e = parse(&format!("flowchart LR\n{src}")).unwrap_err();
        (e.line, e.message)
    };
    let (line, message) = fail("a --> b\n%% stack: a zz");
    assert!(line == Some(3) && message.contains("no node \"zz\""), "{message}");
    assert!(fail("a --> b\n%% stack: a").1.contains("two or more"));
    assert!(fail("a --> b\n%% stack: a b a").1.contains("already in a stack"));
    let (line, message) = fail("a --> b --> c\n%% stack: a b\n%% stack: c b");
    assert!(line == Some(4) && message.contains("already in a stack"), "{message}");
    assert!(fail("subgraph s\na\nend\na --> b\n%% stack: a b").1.contains("same subgraph"));
}
