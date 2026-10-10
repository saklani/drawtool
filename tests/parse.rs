//! The flowchart parser, input by input.

use drawtool::{parse, Direction, Shape, Style};

/// (style, arrow, label) of the single edge in `src`.
fn edge(src: &str) -> (Style, bool, Option<String>) {
    let g = parse(src).unwrap_or_else(|e| panic!("{src:?}: {e}"));
    assert_eq!(g.edges.len(), 1, "{src:?}");
    let e = &g.edges[0];
    (e.style, e.arrow, e.label.as_ref().map(|l| l.join("\n")))
}

#[test]
fn every_connector_form() {
    use Style::*;
    let cases: &[(&str, Style, bool, Option<&str>)] = &[
        ("a --> b", Solid, true, None),
        ("a->b", Solid, true, None),
        ("a --- b", Solid, false, None),
        ("a -.-> b", Dotted, true, None),
        ("a -.- b", Dotted, false, None),
        ("a ==> b", Thick, true, None),
        ("a === b", Thick, false, None),
        ("a -->|yes| b", Solid, true, Some("yes")),
        ("a -- yes --> b", Solid, true, Some("yes")),
        ("a -- no arrow --- b", Solid, false, Some("no arrow")),
        ("a -. maybe .-> b", Dotted, true, Some("maybe")),
        ("a == main ==> b", Thick, true, Some("main")),
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
        ("a", Shape::Box),
    ];
    for (src, shape) in cases {
        let g = parse(src).unwrap();
        assert_eq!(g.nodes[0].shape, shape, "{src:?}");
    }
}

#[test]
fn labels_quotes_and_line_breaks() {
    let g = parse("a[\"quoted label\"]\nb[one<br/>two\\nthree]\nc['single']").unwrap();
    let lines: Vec<Vec<&str>> = g.nodes.iter().map(|n| n.lines.iter().map(String::as_str).collect()).collect();
    assert_eq!(lines, vec![vec!["quoted label"], vec!["one", "two", "three"], vec!["single"]]);
}

#[test]
fn bare_ids_are_their_own_label_and_the_first_label_wins() {
    let g = parse("a --> b\na[First] --> c\na[Second]").unwrap();
    assert_eq!(g.nodes[0].lines, vec!["First"]);
    assert_eq!(g.nodes[1].lines, vec!["b"]);
}

#[test]
fn fan_out_fan_in_and_chains() {
    let pairs = |src: &str| -> Vec<(String, String)> {
        let g = parse(src).unwrap();
        g.edges.iter().map(|e| (g.nodes[e.src].id.clone(), g.nodes[e.dst].id.clone())).collect()
    };
    let p = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(pairs("a --> b & c"), vec![p("a", "b"), p("a", "c")]);
    assert_eq!(pairs("a & b --> c"), vec![p("a", "c"), p("b", "c")]);
    assert_eq!(pairs("a --> b --> c"), vec![p("a", "b"), p("b", "c")]);
}

#[test]
fn headers_comments_and_ignored_lines() {
    for (src, dir) in [
        ("flowchart TD\na", Direction::TD),
        ("graph TB\na", Direction::TD),
        ("graph BT\na", Direction::TD),
        ("flowchart RL\na", Direction::LR),
        ("direction TD\na", Direction::TD),
        ("a", Direction::LR),
    ] {
        assert_eq!(parse(src).unwrap().direction, dir, "{src:?}");
    }
    let g = parse("%% a comment\na --> b %% trailing\nclassDef x fill:#f00\nstyle a fill:#0f0\nb --> c;").unwrap();
    assert_eq!((g.nodes.len(), g.edges.len()), (3, 2));
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
