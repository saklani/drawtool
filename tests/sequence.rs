use drawtool::{is_sequence, parse_sequence, render_sequence, render_sequence_fit};
use drawtool::{Head, Item, Message, Note, NoteSide, Participant, Sequence};
use unicode_width::UnicodeWidthStr;

fn draw(src: &str) -> String {
    render_sequence(&parse_sequence(src).unwrap()).unwrap()
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
    let out = render_sequence(&seq).unwrap();
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
    assert_eq!(parse_sequence("sequenceDiagram\na->>b: ok\nnonsense here").unwrap_err().line, Some(3));
    assert_eq!(parse_sequence("sequenceDiagram\nloop x\na->>b").unwrap_err().line, Some(2));
    assert_eq!(parse_sequence("sequenceDiagram\nend").unwrap_err().line, Some(2));
    assert_eq!(parse_sequence("sequenceDiagram\nelse x").unwrap_err().line, Some(2));
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
    let wide = render_sequence_fit(&seq, 0).unwrap();
    let fitted = render_sequence_fit(&seq, 60).unwrap();
    assert!(wide.width > 60, "\n{}", wide.text);
    assert!(fitted.fits && fitted.width <= 60, "\n{}", fitted.text);
}

#[test]
fn a_long_label_never_covers_a_lifeline_it_spans() {
    let out = draw("sequenceDiagram\na->>b: hi\nb->>c: yo\na->>c: a much longer label that spans");
    let row = out.lines().find(|l| l.contains("a much longer")).unwrap();
    assert_eq!(row.matches('│').count(), 3, "a, b and c lifelines all visible:\n{out}");
}

#[test]
fn box_groups_participants_without_drawing_a_frame() {
    let src = "sequenceDiagram\nbox Purple Team\nparticipant a\nparticipant b\nend\na->>b: hi";
    let seq = parse_sequence(src).unwrap();
    let ids: Vec<&str> = seq.participants.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    assert_eq!(seq.items.len(), 1, "only the message: box is not a frame");
    let out = render_sequence(&seq).unwrap();
    assert!(!out.contains('┄') && !out.contains("Purple"), "\n{out}");
}

#[test]
fn rect_frames_drop_their_colour() {
    let out = draw("sequenceDiagram\nrect rgb(0, 0, 255)\na->>b: x\nend");
    assert!(out.contains("┌ rect ┄"), "\n{out}");
    assert!(!out.contains("rgb"), "\n{out}");
}

#[test]
fn par_critical_and_break_blocks() {
    let src = "sequenceDiagram\npar fan out\na->>b: one\nand\na->>c: two\nend\ncritical commit\na->>b: write\noption timeout\na->>b: roll back\nend\nbreak on error\nb-->>a: stop\nend";
    let out = draw(src);
    for title in ["┌ par fan out", "├ and ", "┌ critical commit", "├ option timeout", "┌ break on error"] {
        assert!(out.contains(title), "missing {title:?}:\n{out}");
    }
    assert_eq!(out.matches("└┄").count(), 3, "one bottom border per block:\n{out}");
}

#[test]
fn note_over_non_adjacent_participants_spans_the_middle_one() {
    let out = draw("sequenceDiagram\na->>b: x\nb->>c: y\nNote over a,c: shared note across three");
    let note_top = out.lines().find(|l| l.contains("┌──") && !l.contains("┌───┐")).unwrap();
    let (left, right) = (note_top.find('┌').unwrap(), note_top.rfind('┐').unwrap());
    assert!(left <= col(&out, "│ a │") && note_top[..right].width() >= col(&out, "│ c │"), "\n{out}");
}

#[test]
fn labels_keep_colons_and_drop_trailing_comments() {
    let seq =
        parse_sequence("sequenceDiagram\nparticipant a as Alice %% the user\na->>b: at 10:30 sharp %% why").unwrap();
    assert_eq!(seq.participants[0].lines, ["Alice"]);
    match &seq.items[0] {
        Item::Message(m) => assert_eq!(m.lines, ["at 10:30 sharp"]),
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn errors_point_at_line_and_column() {
    let at = |src: &str| {
        let e = parse_sequence(src).unwrap_err();
        (e.line, e.column)
    };
    assert_eq!(at("sequenceDiagram\n  nonsense here"), (Some(2), Some(3)));
    assert_eq!(at("sequenceDiagram\nNote left of a, b: x"), (Some(2), Some(17)));
    assert_eq!(at("sequenceDiagram\n    end"), (Some(2), Some(5)));
    assert_eq!(at("sequenceDiagram\n  loop x\na->>b"), (Some(2), Some(3)));
}

#[test]
fn hand_built_sequences_render() {
    let part = |id: &str| Participant { id: id.into(), lines: vec![id.into()], actor: false };
    let msg = Message { from: 0, to: 1, lines: vec!["hi".into()], dotted: false, head: Head::Arrow };
    let note = Note { side: NoteSide::Over, first: 0, last: 1, lines: vec!["both".into()] };
    let loop_ = Item::Start("loop again".into());
    let seq = Sequence {
        participants: vec![part("a"), part("b")],
        items: vec![loop_, Item::Message(msg), Item::Note(note), Item::End],
    };
    let out = render_sequence(&seq).unwrap();
    assert!(
        out.contains("│ a │") && out.contains("hi") && out.contains("both") && out.contains("loop again"),
        "\n{out}"
    );
    assert!(render_sequence_fit(&seq, 0).unwrap().fits);
}

#[test]
fn hand_built_sequences_are_validated() {
    let base = parse_sequence(
        "sequenceDiagram
a->>b: x",
    )
    .unwrap();
    let err = |edit: &dyn Fn(&mut Sequence)| {
        let mut seq = base.clone();
        edit(&mut seq);
        assert!(render_sequence_fit(&seq, 0).is_err());
        render_sequence(&seq).unwrap_err().message
    };
    let note = |side, first, last| Item::Note(Note { side, first, last, lines: vec!["n".into()] });
    let message = |from, to| Item::Message(Message { from, to, lines: vec![], dotted: false, head: Head::Open });
    assert!(err(&|s| s.items.push(message(0, 2))).contains("no participant 2"));
    assert!(err(&|s| s.items.push(message(5, 0))).contains("no participant 5"));
    assert!(err(&|s| s.items.push(note(NoteSide::Over, 0, 9))).contains("no participant 9"));
    assert!(err(&|s| s.items.push(note(NoteSide::Over, 1, 0))).contains("after its `last`"));
    assert!(err(&|s| s.items.push(note(NoteSide::Left, 0, 1))).contains("only `Over`"));
    assert!(err(&|s| s.items.insert(0, Item::End)).contains("item 0: outside a frame"));
    let divider = Item::Divider("else".into());
    assert!(err(&|s| s.items.push(divider.clone())).contains("item 1: outside a frame"));
    let start = Item::Start("loop".into());
    assert!(err(&|s| s.items.push(start.clone())).contains("never closed"));
    assert!(err(&|s| s.participants.clear()).contains("no participant 1"));
}
