use drawtool::sequence::{Head, Item, NoteSide};
use drawtool::{is_sequence, parse_sequence, render_sequence, render_sequence_fit};
use unicode_width::UnicodeWidthStr;

fn draw(src: &str) -> String {
    render_sequence(&parse_sequence(src).unwrap(), false)
}

/// Column of the first occurrence of `needle` in each line containing it.
fn col(out: &str, needle: &str) -> usize {
    let line = out.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} not in\n{out}"));
    line[..line.find(needle).unwrap()].width()
}

#[test]
fn detects_sequence_diagrams() {
    assert!(is_sequence("%% hi\n\nsequenceDiagram\na->>b: x"));
    assert!(!is_sequence("flowchart LR\na --> b"));
}

#[test]
fn two_party_exchange() {
    let out = draw("sequenceDiagram\nalice->>bob: hello\nbob-->>alice: hi back");
    assert!(out.contains("│ hello"), "\n{out}");
    assert!(out.contains("├─────") && out.contains("►│"), "\n{out}");
    assert!(out.contains("│◄┄"), "dotted reply arrow:\n{out}");
    // Boxes appear at the top and again at the bottom.
    assert_eq!(out.matches("│ alice │").count(), 2, "\n{out}");
}

#[test]
fn participants_are_ordered_by_first_mention_and_aliased() {
    let seq = parse_sequence("sequenceDiagram\nparticipant b as Bob\na->>b: x\nc->>a: y").unwrap();
    let ids: Vec<&str> = seq.participants.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["b", "a", "c"]);
    assert_eq!(seq.participants[0].lines, ["Bob"]);
}

#[test]
fn arrow_kinds() {
    let seq = parse_sequence("sequenceDiagram\na->>b\na-->>b\na->b\na-->b\na-xb\na--xb\na-)b\na->>+b: go").unwrap();
    let heads: Vec<(Head, bool)> = seq
        .items
        .iter()
        .map(|i| match i {
            Item::Message(m) => (m.head, m.dotted),
            _ => panic!("expected only messages"),
        })
        .collect();
    use Head::*;
    assert_eq!(
        heads,
        [
            (Arrow, false),
            (Arrow, true),
            (Open, false),
            (Open, true),
            (Cross, false),
            (Cross, true),
            (Arrow, false),
            (Arrow, false)
        ]
    );
}

#[test]
fn self_message_loops_right_of_the_lifeline() {
    let out = draw("sequenceDiagram\napi->>api: validate");
    assert!(out.contains("├──┐") && out.contains("│◄─┘"), "\n{out}");
    assert!(col(&out, "validate") > col(&out, "├──┐"), "\n{out}");
}

#[test]
fn notes_on_every_side() {
    let seq = parse_sequence(
        "sequenceDiagram\nNote left of a: L\nNote right of a: R\nNote over a: O\nNote over a,b: span\na->>b: x",
    )
    .unwrap();
    let sides: Vec<NoteSide> =
        seq.items.iter().filter_map(|i| if let Item::Note(n) = i { Some(n.side) } else { None }).collect();
    assert_eq!(sides, [NoteSide::Left, NoteSide::Right, NoteSide::Over, NoteSide::Over]);
    let out = render_sequence(&seq, false);
    assert!(col(&out, "│ L │") < col(&out, "┬"), "left note sits left of the lifeline:\n{out}");
    assert!(col(&out, "│ R │") > col(&out, "┬"), "\n{out}");
    assert!(out.contains("span"), "\n{out}");
}

#[test]
fn nested_loop_and_alt_frames() {
    let src = "sequenceDiagram\nloop retry\nalt ok\na->>b: yes\nelse fail\na->>b: no\nend\nend";
    let out = draw(src);
    assert!(out.contains("┌ loop retry"), "\n{out}");
    assert!(out.contains("┌ alt ok"), "\n{out}");
    assert!(out.contains("├ else fail"), "\n{out}");
    // The inner frame sits inside the outer one.
    assert!(col(&out, "┌ alt ok") > col(&out, "┌ loop retry"), "\n{out}");
    assert_eq!(out.matches("└┄").count(), 2, "one bottom border per frame:\n{out}");
}

#[test]
fn ascii_output_is_pure_ascii() {
    let out = render_sequence(
        &parse_sequence("sequenceDiagram\nloop x\na->>b: hi\nb-->>a: ok\nend\na->>a: self").unwrap(),
        true,
    );
    assert!(out.is_ascii(), "\n{out}");
    assert!(out.contains("|<....") && out.contains("--->|"), "\n{out}");
}

#[test]
fn wide_characters_keep_lifelines_aligned() {
    let out = draw("sequenceDiagram\n用户->>服务: 数据流\n服务-->>用户: 好");
    let lifelines: Vec<usize> = out
        .lines()
        .filter(|l| !l.contains('─') && !l.contains('┄') && l.matches('│').count() == 2 && !l.contains("用户"))
        .map(|l| l.find('│').map(|b| l[..b].width()).unwrap())
        .collect();
    assert!(lifelines.windows(2).all(|w| w[0] == w[1]), "\n{out}");
}

#[test]
fn errors_name_the_line() {
    assert!(parse_sequence("sequenceDiagram\na->>b: ok\nnonsense here").unwrap_err().starts_with("line 3:"));
    assert!(parse_sequence("sequenceDiagram\nloop x\na->>b").unwrap_err().starts_with("line 2:"));
    assert!(parse_sequence("sequenceDiagram\nend").unwrap_err().starts_with("line 2:"));
    assert!(parse_sequence("sequenceDiagram\nelse x").unwrap_err().starts_with("line 2:"));
}

#[test]
fn ignored_lines() {
    let seq =
        parse_sequence("sequenceDiagram\nautonumber\ntitle Checkout\n%% note\nactivate a\na->>b: x\ndeactivate a")
            .unwrap();
    assert_eq!(seq.items.len(), 1);
}

#[test]
fn long_labels_wrap_to_fit_the_budget() {
    let src = "sequenceDiagram\nclient->>api: POST /checkout with a long description of the payload\napi->>pay: create a session for the destination wallet";
    let seq = parse_sequence(src).unwrap();
    let wide = render_sequence_fit(&seq, false, 0);
    let fitted = render_sequence_fit(&seq, false, 60);
    assert!(wide.width > 60, "\n{}", wide.text);
    assert!(fitted.fits && fitted.width <= 60, "\n{}", fitted.text);
}
