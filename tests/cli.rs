use drawtool::{parse, render};

#[test]
fn parse_error_names_the_line() {
    let err = parse("a --> b\nb -->\n").unwrap_err();
    assert!(err.starts_with("line 2:"), "{err}");
}

#[test]
fn empty_input_renders_nothing() {
    assert_eq!(render(&parse("%% nothing here\n").unwrap(), None, false), "");
}

#[test]
fn wide_characters_keep_boxes_aligned() {
    let out = render(&parse("a[数据] --> b[ok 🚀]").unwrap(), None, false);
    let widths: Vec<usize> = out.lines().map(unicode_width::UnicodeWidthStr::width).collect();
    // Top border, text row and bottom border of the first box are equally wide.
    assert_eq!(widths[0], widths[1], "\n{out}");
    assert_eq!(widths[1], widths[2], "\n{out}");
}

#[test]
fn first_explicit_label_wins() {
    let g = parse("a[First] --> b\na[Second] --> c").unwrap();
    assert_eq!(g.nodes[0].lines, vec!["First"]);
}
