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

#[test]
fn wide_lr_wraps_labels_before_giving_up_on_lr() {
    let src = "flowchart LR\na[a fairly long label here] --> b[another quite long label] --> c[and a third long one]";
    let g = parse(src).unwrap();
    let unwrapped = drawtool::fit::text_width(&render(&g, None, false));
    let fitted = drawtool::render_fit(&g, drawtool::Direction::LR, false, unwrapped - 10);
    assert_eq!(fitted.direction, drawtool::Direction::LR);
    assert!(fitted.fits && fitted.width < unwrapped, "\n{}", fitted.text);
}

#[test]
fn long_lr_chain_breaks_into_rows_and_stays_lr() {
    let src = "flowchart LR\na --> b --> c --> d --> e --> f --> g --> h";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 30);
    assert_eq!(fitted.direction, drawtool::Direction::LR);
    assert!(fitted.bands > 1 && fitted.width <= 30, "\n{}", fitted.text);
}

#[test]
fn lr_never_switches_to_td_on_its_own() {
    // Each box is 12 wide: no LR layout fits in 12 columns, but TD would.
    let src = "flowchart LR\na[abcdefgh] --> b[ijklmnop] --> c[qrstuvwx]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 12);
    assert_eq!(fitted.direction, drawtool::Direction::LR, "\n{}", fitted.text);
    assert!(!fitted.fits);
}

#[test]
fn td_is_kept_when_asked_for() {
    let src = "flowchart TD\na --> b --> c";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::TD, false, 120);
    assert_eq!(fitted.direction, drawtool::Direction::TD);
}

#[test]
fn max_width_zero_means_unlimited() {
    let g = parse("flowchart LR\na --> b --> c --> d --> e --> f --> g --> h").unwrap();
    let fitted = drawtool::render_fit(&g, drawtool::Direction::LR, false, 0);
    assert_eq!(fitted.text, render(&g, None, false));
}

#[test]
fn wrapping_keeps_long_words_whole() {
    let g = drawtool::wrap_labels(&parse("a[supercalifragilistic word]").unwrap(), 8);
    assert_eq!(g.nodes[0].lines, vec!["supercalifragilistic", "word"]);
}

#[test]
fn single_crossing_edge_wraps_around_like_text() {
    let src = "flowchart LR\na[first step] --> b[second step] --> c[third step] --> d[fourth step]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 50);
    assert!(fitted.bands > 1 && fitted.width <= 50, "\n{}", fitted.text);
    assert!(!fitted.text.contains('①'), "a lone crossing edge is drawn, not numbered:\n{}", fitted.text);
    assert!(fitted.text.contains("└─►"), "\n{}", fitted.text);
}

#[test]
fn several_crossing_edges_keep_connectors() {
    let src = "flowchart LR\na --> b --> c --> d --> e\nb --> e";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 20);
    assert!(fitted.text.contains('①'), "\n{}", fitted.text);
}
