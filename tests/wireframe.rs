use drawtool::{is_wireframe, parse_wireframe, render_wireframe, render_wireframe_fit};
use drawtool::{El, WIREFRAME_MIN_WIDTH as MIN_WIDTH, WIREFRAME_WIDTH as DEFAULT_WIDTH};
use unicode_width::UnicodeWidthStr;

fn draw(src: &str) -> String {
    render_wireframe(&parse_wireframe(src).unwrap())
}

/// Every line of a single screen is exactly as wide as the screen.
fn assert_rectangular(out: &str, width: usize) {
    for l in out.lines() {
        assert_eq!(l.width(), width, "line {l:?} in\n{out}");
    }
}

#[test]
fn detects_wireframes() {
    assert!(is_wireframe("%% hi\n\nwireframe Home\n  text: hi"));
    assert!(is_wireframe("Wireframe"));
    assert!(!is_wireframe("flowchart LR\na --> b"));
}

#[test]
fn parses_containers_and_leaves() {
    let wf = parse_wireframe(
        "wireframe Pay width=50\n  card Total:\n    center: $20\n    row: [ A ] | [ B ]\n  row:\n    text: left\n    text: right\n  - item | 1\n  button: Go",
    )
    .unwrap();
    let s = &wf.screens[0];
    assert_eq!((s.title.as_str(), s.width), ("Pay", 50));
    match &s.children[0] {
        El::Card { title, children } => {
            assert_eq!(title, "Total");
            assert_eq!(children.len(), 2);
            assert!(matches!(&children[1], El::Row(cells) if cells.len() == 2));
        }
        other => panic!("expected card, got {other:?}"),
    }
    assert!(matches!(&s.children[1], El::Row(cells) if cells.len() == 2));
    assert_eq!(s.children[2], El::Bullet("item".into(), Some("1".into())));
    assert_eq!(s.children[3], El::Button("Go".into()));
}

#[test]
fn unknown_keywords_are_text() {
    let wf = parse_wireframe("wireframe\n  Total: $20").unwrap();
    assert!(matches!(&wf.screens[0].children[0], El::Text(t, _) if t == "Total: $20"));
}

#[test]
fn every_element_renders_rectangular() {
    let out = draw(
        "wireframe Everything
  header: ← Back | Title | ?
  text: Some text long enough that it has to wrap onto another line
  center: centred
  right: right
  ---
  input: Email = you@example.com
  select: Country = Germany
  button: Continue
  image: photo
  card Details:
    row: [ $20 ] | [ $50 ] | [ $100 ]
    checkbox: Remember me = on
    toggle: Notifications = off
  radio: *Card | Bank
  tabs: Home | *Activity
  - Coffee | -$4.50
  spacer
  nav: *Home | Profile",
    );
    assert_rectangular(&out, DEFAULT_WIDTH);
    for needle in [
        "┌─ Everything",
        "├────",
        "┏━━",
        "Continue",
        "░ photo ░",
        "┌─ Details",
        "[x] Remember me",
        "(○  )",
        "(•) Card",
        "━━━━━━━━",
        "• Coffee",
        "-$4.50",
        "▾",
        "you@example.com",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in\n{out}");
    }
}

#[test]
fn wide_characters_stay_aligned() {
    let out = draw("wireframe 数据\n  header: ☰ | 你好 | 🔔\n  - 咖啡 | ¥30\n  button: 继续");
    assert_rectangular(&out, DEFAULT_WIDTH);
}

#[test]
fn screens_sit_side_by_side_and_wrap_to_the_budget() {
    let wf = parse_wireframe("wireframe A\n  text: a\nwireframe B\n  text: b").unwrap();
    let wide = render_wireframe_fit(&wf, 120);
    assert!(wide.fits && wide.text.lines().next().unwrap().contains("┌─ B"), "\n{}", wide.text);
    let narrow = render_wireframe_fit(&wf, 50);
    assert!(narrow.fits && narrow.text.lines().filter(|l| l.contains("┌─")).count() == 2, "\n{}", narrow.text);
    let tiny = render_wireframe_fit(&wf, 10);
    assert!(!tiny.fits && tiny.width == MIN_WIDTH, "\n{}", tiny.text);
}

#[test]
fn errors_name_the_line() {
    assert_eq!(parse_wireframe("wireframe\n  text: a\n    text: b").unwrap_err().line, Some(3));
    assert_eq!(parse_wireframe("text: a").unwrap_err().line, Some(1));
    assert!(parse_wireframe("wireframe x width=abc").unwrap_err().message.contains("invalid width"));
}

/// The single line of `out` containing `needle`.
fn line_with<'a>(out: &'a str, needle: &str) -> &'a str {
    out.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} not in\n{out}"))
}

#[test]
fn an_empty_active_tab_draws_no_underline() {
    let out = draw("wireframe T width=24\n  tabs: * | b | c");
    assert!(!out.contains('━'), "\n{out}");
    assert_rectangular(&out, 24);
}

#[test]
fn dividers_reach_the_frame_through_a_col() {
    let out = draw("wireframe T width=24\n  card C:\n    col:\n      text: a\n      ---\n      text: b");
    assert!(out.contains("│ ├──────────────────┤ │"), "\n{out}");
}

#[test]
fn nested_card_dividers_meet_the_inner_frame() {
    let out = draw("wireframe T width=24\n  card A:\n    card B:\n      ---");
    assert!(out.contains("│ │ ├──────────────┤ │ │"), "\n{out}");
    assert_rectangular(&out, 24);
}

#[test]
fn an_empty_card_is_just_its_border() {
    let out = draw("wireframe T width=20\n  card Empty:");
    assert!(out.contains("│ ┌─ Empty ──────┐ │\n│ └──────────────┘ │"), "\n{out}");
}

#[test]
fn a_row_too_narrow_for_its_children_stacks_them() {
    let out = draw("wireframe T width=20\n  row: hello | world | foo | bar | baz | qux | quux");
    for word in ["hello", "world", "quux"] {
        assert!(out.contains(word), "{word} spelled out whole:\n{out}");
    }
    assert_rectangular(&out, 20);
}

#[test]
fn a_bullet_never_loses_its_item_to_the_right_part() {
    let out = draw("wireframe T width=24\n  - item | a-very-long-right-hand-side-value");
    let line = line_with(&out, "item");
    assert!(line.contains('…'), "right part truncated: {line:?}");
    assert_rectangular(&out, 24);
}

#[test]
fn header_slots_never_overlap() {
    let out = draw("wireframe T width=24\n  header: Back | A very long centred title | Help");
    let line = line_with(&out, "Back");
    assert!(line.contains("Back") && line.contains("Help") && line.contains('…'), "{line:?}");
    assert_rectangular(&out, 24);
    // A title that fits stays exactly centred.
    let fits = draw("wireframe T width=24\n  header: < | Title | >");
    assert_eq!(line_with(&fits, "Title"), "│ <      Title       > │");
}

#[test]
fn long_words_split_across_lines() {
    let out = draw("wireframe T width=20\n  text: supercalifragilisticexpialidocious");
    let text: String = out.lines().skip(1).map(|l| l.trim_matches(|c| c == '│' || c == ' ')).collect();
    assert!(text.starts_with("supercalifragilisticexpialidocious"), "split, not truncated:\n{out}");
    assert_rectangular(&out, 20);
}

#[test]
fn radio_options_wrap_when_they_dont_fit_on_one_line() {
    let out = draw("wireframe T width=20\n  radio: *Credit card | Bank transfer | Cash");
    assert!(line_with(&out, "Credit card").contains("(•)"), "\n{out}");
    // A wrapped option continues under its own text, not under the markers.
    assert!(line_with(&out, "Bank").starts_with("│ ( ) Bank"), "\n{out}");
    assert!(line_with(&out, "transfer").starts_with("│     transfer"), "\n{out}");
}

#[test]
fn errors_point_at_the_line_and_column() {
    let at = |src: &str| {
        let e = parse_wireframe(src).unwrap_err();
        (e.line, e.column, e.message)
    };
    let (line, column, message) = at("wireframe\n    card A:\n        text: x\n      text: y");
    assert_eq!((line, column), (Some(4), Some(7)));
    assert!(message.contains("doesn't line up"), "{message}");
    let (line, column, message) = at("wireframe\n  text: a\n    text: b");
    assert_eq!((line, column), (Some(3), Some(5)));
    assert!(message.contains("only card, row and col"), "{message}");
    let (line, column, _) = at("wireframe x width=abc");
    assert_eq!((line, column), (Some(1), Some(13)), "points at `width=`");
}
