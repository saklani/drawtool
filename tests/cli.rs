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
fn long_lr_chain_breaks_into_bands_before_td() {
    let src = "flowchart LR\na --> b --> c --> d --> e --> f --> g --> h";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 30);
    assert_eq!(fitted.direction, drawtool::Direction::LR);
    assert!(fitted.bands > 1 && fitted.width <= 30, "\n{}", fitted.text);
    assert!(fitted.text.contains('①'), "bands are joined by connectors:\n{}", fitted.text);
}

#[test]
fn lr_falls_back_to_td_when_no_band_fits() {
    // Each box is 12 wide: alone it fits, but not with a connector beside it.
    let src = "flowchart LR\na[abcdefgh] --> b[ijklmnop] --> c[qrstuvwx]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 12);
    assert_eq!(fitted.direction, drawtool::Direction::TD, "\n{}", fitted.text);
    assert!(fitted.fits);
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
