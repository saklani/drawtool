use drawtool::wireframe::{El, DEFAULT_WIDTH, MIN_WIDTH};
use drawtool::{is_wireframe, parse_wireframe, render_wireframe, render_wireframe_fit};
use unicode_width::UnicodeWidthStr;

fn draw(src: &str) -> String {
    render_wireframe(&parse_wireframe(src).unwrap(), false)
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
fn ascii_mode_uses_only_ascii() {
    let wf = parse_wireframe("wireframe A\n  header: a | b\n  button: Go\n  image\n  select: x = y\n  toggle: t = on\n  - item\n  radio: *a | b\n  nav: *x | y").unwrap();
    let out = render_wireframe(&wf, true);
    assert!(out.is_ascii(), "\n{out}");
}

#[test]
fn wide_characters_stay_aligned() {
    let out = draw("wireframe 数据\n  header: ☰ | 你好 | 🔔\n  - 咖啡 | ¥30\n  button: 继续");
    assert_rectangular(&out, DEFAULT_WIDTH);
}

#[test]
fn screens_sit_side_by_side_and_wrap_to_the_budget() {
    let wf = parse_wireframe("wireframe A\n  text: a\nwireframe B\n  text: b").unwrap();
    let wide = render_wireframe_fit(&wf, false, 120);
    assert!(wide.fits && wide.text.lines().next().unwrap().contains("┌─ B"), "\n{}", wide.text);
    let narrow = render_wireframe_fit(&wf, false, 50);
    assert!(narrow.fits && narrow.text.lines().filter(|l| l.contains("┌─")).count() == 2, "\n{}", narrow.text);
    let tiny = render_wireframe_fit(&wf, false, 10);
    assert!(!tiny.fits && tiny.width == MIN_WIDTH, "\n{}", tiny.text);
}

#[test]
fn errors_name_the_line() {
    assert!(parse_wireframe("wireframe\n  text: a\n    text: b").unwrap_err().starts_with("line 3:"));
    assert!(parse_wireframe("text: a").unwrap_err().starts_with("line 1:"));
    assert!(parse_wireframe("wireframe x width=abc").unwrap_err().contains("invalid width"));
}
