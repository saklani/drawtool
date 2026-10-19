use drawtool::{parse, render};
use petgraph::graph::{EdgeIndex, NodeIndex};

#[test]
fn kind_comes_from_the_first_words_of_the_first_line() {
    use drawtool::{kind, Kind};
    assert_eq!(kind("%% hi\n\nflowchart LR\na --> b"), Ok(Kind::Flowchart));
    assert_eq!(kind("graph"), Ok(Kind::Flowchart));
    assert_eq!(kind("  sequenceDiagram\na->>b: x"), Ok(Kind::Sequence));
    assert_eq!(kind("Wireframe Home width=40"), Ok(Kind::Wireframe));
    let err = kind("\nsequence\na->>b: hi").unwrap_err();
    assert_eq!(err.line, Some(2));
    assert!(err.message.contains("`sequenceDiagram` or `wireframe`"), "names every kind: {err}");
    assert!(kind("%% only a comment\n").unwrap_err().message.starts_with("no diagram"));
}

#[test]
fn parse_error_names_the_line() {
    let err = parse("flowchart LR\na --> b\nb -->\n").unwrap_err();
    assert_eq!(err.line, Some(3), "{err}");
}

#[test]
fn a_header_alone_renders_nothing() {
    assert_eq!(render(&parse("flowchart LR\n%% nothing here\n").unwrap(), None), "");
}

#[test]
fn wide_characters_keep_boxes_aligned() {
    let out = render(&parse("flowchart LR\na[数据] --> b[ok 🚀]").unwrap(), None);
    let widths: Vec<usize> = out.lines().map(unicode_width::UnicodeWidthStr::width).collect();
    // Top border, text row and bottom border of the first box are equally wide.
    assert_eq!(widths[0], widths[1], "\n{out}");
    assert_eq!(widths[1], widths[2], "\n{out}");
}

#[test]
fn last_explicit_label_wins_as_in_mermaid() {
    let g = parse("flowchart LR\na[First] --> b\na[Second] --> c\na --> d").unwrap();
    assert_eq!(g.g[NodeIndex::new(0)].lines, vec!["Second"]);
}

/// Some line of `out` contains both `a` and `b`: in LR, consecutive boxes sit
/// side by side; in TD they stack.
fn same_row(out: &str, a: &str, b: &str) -> bool {
    out.lines().any(|l| l.contains(a) && l.contains(b))
}

#[test]
fn wide_lr_wraps_labels_before_giving_up_on_lr() {
    let src = "flowchart LR\na[a fairly long label here] --> b[another quite long label] --> c[and a third long one]";
    let g = parse(src).unwrap();
    let unwrapped = render(&g, None).lines().map(unicode_width::UnicodeWidthStr::width).max().unwrap_or(0);
    let fitted = drawtool::render_fit(&g, drawtool::Direction::LR, unwrapped - 10);
    assert!(same_row(&fitted.text, "a fairly", "another"), "still LR:\n{}", fitted.text);
    assert!(fitted.fits && fitted.width < unwrapped, "\n{}", fitted.text);
}

#[test]
fn long_lr_chain_breaks_into_rows_and_stays_lr() {
    let src = "flowchart LR\na --> b --> c --> d --> e --> f --> g --> h";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, 30);
    assert!(same_row(&fitted.text, "│ a ", "│ b "), "still LR:\n{}", fitted.text);
    assert!(fitted.text.contains("└─►") && fitted.width <= 30, "wrapped onto a new row:\n{}", fitted.text);
}

#[test]
fn lr_never_switches_to_td_on_its_own() {
    // Each box is 12 wide: no LR layout fits in 12 columns, but TD would.
    let src = "flowchart LR\na[abcdefgh] --> b[ijklmnop] --> c[qrstuvwx]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, 12);
    assert!(same_row(&fitted.text, "abcdefgh", "ijklmnop"), "still LR:\n{}", fitted.text);
    assert!(!fitted.fits);
}

#[test]
fn td_is_kept_when_asked_for() {
    let src = "flowchart TD\na --> b --> c";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::TD, 120);
    assert!(!same_row(&fitted.text, "│ a ", "│ b "), "stacked, not side by side:\n{}", fitted.text);
}

#[test]
fn max_width_zero_means_unlimited() {
    let g = parse("flowchart LR\na --> b --> c --> d --> e --> f --> g --> h").unwrap();
    let fitted = drawtool::render_fit(&g, drawtool::Direction::LR, 0);
    assert_eq!(fitted.text, render(&g, None));
}

#[test]
fn wrapping_keeps_long_words_whole() {
    let g = drawtool::wrap_labels(&parse("flowchart LR\na[supercalifragilistic word]").unwrap(), 8);
    assert_eq!(g.g[NodeIndex::new(0)].lines, vec!["supercalifragilistic", "word"]);
}

#[test]
fn single_crossing_edge_wraps_around_like_text() {
    let src = "flowchart LR\na[first step] --> b[second step] --> c[third step] --> d[fourth step]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, 50);
    assert!(fitted.width <= 50, "\n{}", fitted.text);
    assert!(!fitted.text.contains('①'), "a lone crossing edge is drawn, not numbered:\n{}", fitted.text);
    assert!(fitted.text.contains("└─►"), "\n{}", fitted.text);
}

#[test]
fn several_crossing_edges_wrap_as_nested_lines() {
    let src = "flowchart LR\nreq[request] --> auth{auth ok?}\nauth -->|no| deny[401]\nauth -->|yes| p(parse body)\n\
               auth --> log(audit log)\np --> v{valid?} -->|yes| db[(postgres)] --> ok[201]\nlog --> ok";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, 90);
    assert!(fitted.fits, "\n{}", fitted.text);
    assert!(fitted.text.lines().any(|l| l.starts_with("│ ┌")), "two lines wrap, one inside the other:\n{}", fitted.text);
    assert!(!fitted.text.contains('╂'), "wrapping lines never cross:\n{}", fitted.text);
}

#[test]
fn highlight_marks_nodes_and_thickens_edges() {
    let g = parse("flowchart LR\n%% highlight: a, b\na --> b --> c").unwrap();
    assert_eq!(g.g[NodeIndex::new(0)].mark, drawtool::Mark::Highlight);
    assert_eq!(g.g[NodeIndex::new(2)].mark, drawtool::Mark::None);
    assert_eq!(g.g[EdgeIndex::new(0)].style, drawtool::Style::Thick);
    assert_eq!(g.g[EdgeIndex::new(1)].style, drawtool::Style::Solid);
    let out = render(&g, None);
    assert!(out.contains('┏') && out.contains("━►"), "\n{out}");
}

#[test]
fn highlight_rejects_unknown_nodes_and_missing_edges() {
    let err = parse("flowchart LR\na --> b\n%% highlight: a, zz").unwrap_err();
    assert!(err.line == Some(3) && err.message.contains("zz"), "{err}");
    let err = parse("flowchart LR\na --> b\n%% highlight: a,,b").unwrap_err();
    assert!(err.line == Some(3) && err.message.contains("empty id"), "{err}");
    assert!(drawtool::highlight(&mut parse("flowchart LR\na --> b").unwrap(), &["a".into(), String::new()]).is_err());
    let mut g = parse("flowchart LR\na --> b --> c").unwrap();
    assert!(drawtool::highlight(&mut g, &["a".into(), "c".into()]).unwrap_err().message.contains("no edge"));
}

#[test]
fn subgraph_membership_and_errors() {
    let g = parse("flowchart LR\nsubgraph api [API server]\n  a --> b\n  subgraph inner\n    c\n  end\nend\nd --> a")
        .unwrap();
    // Mermaid lists subgraphs innermost first.
    assert_eq!(g.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), vec!["inner", "API server"]);
    let group = |id: &str| g.g.node_weights().find(|n| n.id == id).unwrap().group;
    assert_eq!((group("a"), group("b"), group("c"), group("d")), (Some(1), Some(1), Some(0), None));
    assert_eq!(parse("flowchart LR\na --> b\nend").unwrap_err().line, Some(3));
    assert!(parse("flowchart LR\nsubgraph x\na").unwrap_err().message.contains("expected \"end\""));
}

#[test]
fn group_frame_holds_members_and_excludes_others() {
    for dir in ["LR", "TD"] {
        let src = format!("flowchart {dir}\nsubgraph g [Grp]\n  a --> b\nend\nc --> a\nb --> d");
        let out = render(&parse(&src).unwrap(), None);
        let rows: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();
        let find = |s: &str| {
            rows.iter().enumerate().find_map(|(y, r)| {
                r.iter().collect::<String>().find(s).map(|i| (r.iter().collect::<String>()[..i].chars().count(), y))
            })
        };
        let (fx, fy) = find("┌ Grp").expect("frame title");
        let fx1 = rows[fy].iter().rposition(|&c| c == '┐').unwrap();
        let fy1 = (fy..rows.len()).find(|&y| rows[y].get(fx) == Some(&'└')).unwrap();
        let inside = |(x, y): (usize, usize)| x > fx && x < fx1 && y > fy && y < fy1;
        assert!(inside(find("│ a ").unwrap()) && inside(find("│ b ").unwrap()), "{dir}:\n{out}");
        assert!(!inside(find("│ c ").unwrap()) && !inside(find("│ d ").unwrap()), "{dir}:\n{out}");
    }
}

#[test]
fn groups_with_back_edges_fit_without_hanging() {
    // Regression: a lane overlapping no earlier lane underflowed `need - start`
    // inside band sub-graphs, and release builds wrapped it into a hang.
    let src = "flowchart LR
subgraph onboarding [Onboarding]
  welcome[Welcome] -->|Get started| email[Enter email] -->|code sent| verify[Verify code]
end
subgraph app [App]
  home[Home] -->|Add money| amount[Amount] --> method{Pay with?}
  method -->|card| card[Card checkout]
  method -->|bank| bank[Bank details]
  card & bank --> done[Success]
end
verify -->|ok| home
verify -.->|wrong code| email
done -.->|back| home";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, 120);
    assert!(fitted.text.contains("Onboarding") && fitted.text.contains("Success"));
}

/// Cells where a vertical line passes straight through a horizontal one.
fn crossings(out: &str) -> usize {
    let rows: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();
    let (v, h) = ("│┆┃║╂", "─┄━═");
    rows.iter()
        .map(|r| {
            (1..r.len().saturating_sub(1))
                .filter(|&x| v.contains(r[x]) && h.contains(r[x - 1]) && h.contains(r[x + 1]))
                .count()
        })
        .sum()
}

#[test]
fn channel_tracks_are_ordered_to_avoid_crossings() {
    // A decision tree whose yes/no edges used to take their channel rows in
    // the wrong order and cross twice.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/decisions.mmd")).unwrap();
    let out = render(&parse(&src).unwrap(), None);
    assert_eq!(crossings(&out), 0, "\n{out}");
}

#[test]
fn unrelated_edges_are_never_drawn_as_joined() {
    // Edges that only share ports transitively (b→e, c→e, c→g) used to merge
    // into one net, so their crossings rendered as ┼ joins.
    let out = render(&parse("flowchart LR\nb --> e & f\nc --> e & g\nd --> f & g\nb --> g").unwrap(), None);
    assert!(!out.contains('┼'), "\n{out}");
    assert!(out.contains('╂'), "crossings are marked:\n{out}");
}

/// (row, column) of the first cell where `s` starts.
fn find(out: &str, s: &str) -> (usize, usize) {
    let at = |(y, l): (usize, &str)| l.find(s).map(|x| (y, l[..x].chars().count()));
    out.lines().enumerate().find_map(at).unwrap_or_else(|| panic!("{s:?} not in\n{out}"))
}

#[test]
fn stacked_boxes_sit_in_one_column_joined_straight() {
    let out = render(&parse("flowchart LR\nx --> a\nx --> b\na --> b\n%% stack: a b").unwrap(), None);
    let ((ya, xa), (yb, xb)) = (find(&out, "│ a"), find(&out, "│ b"));
    assert!(ya < yb && xa == xb, "a above b, same column:\n{out}");
    assert!(out.contains('▼'), "a → b runs down:\n{out}");
    // The order is the stack's: b first means b on top, and a → b runs up.
    let out = render(&parse("flowchart LR\nx --> a\nx --> b\na --> b\n%% stack: b a").unwrap(), None);
    assert!(find(&out, "│ b").0 < find(&out, "│ a").0 && out.contains('▲'), "\n{out}");
}

#[test]
fn a_stacked_two_way_pair_has_no_outer_loop() {
    let src = "flowchart LR\na --> b\na --> c\nb --> c\nc --> b";
    let looped = render(&parse(src).unwrap(), None);
    let stacked = render(&parse(&format!("{src}\n%% stack: b c")).unwrap(), None);
    assert!(looped.contains('◄'), "unstacked, c → b runs back into b:\n{looped}");
    assert_eq!(stacked.lines().count(), find(&stacked, "│ c").0 + 2, "nothing below c:\n{stacked}");
    assert!(stacked.contains('▼') && stacked.contains('▲') && !stacked.contains('◄'), "\n{stacked}");
}

#[test]
fn stacks_run_across_in_td_and_skip_with_a_bracket() {
    let out = render(&parse("flowchart TD\nx --> a & b\na -->|go| b\n%% stack: a b").unwrap(), None);
    assert!(same_row(&out, "│ a", "│ b") && out.contains('►') && out.contains("go"), "\n{out}");
    // a → c skips over b: out of a's far side, along a lane, into c's.
    let out = render(&parse("flowchart LR\nx --> a & b & c\na --> c\n%% stack: a b c").unwrap(), None);
    assert!(out.contains('◄'), "\n{out}");
}

#[test]
fn two_headed_edges_get_a_head_at_each_end() {
    let draw = |src: &str| render(&parse(src).unwrap(), None);
    let lr = draw("flowchart LR\na <--> b");
    assert!(lr.contains("│◄─►│"), "\n{lr}");
    let td = draw("flowchart TD\na <--> b");
    let col: Vec<char> = td.lines().filter_map(|l| l.chars().nth(2)).collect();
    assert!(col.windows(3).any(|w| w == ['▲', '│', '▼']), "\n{td}");
    // The head into `a` marks only the two-headed edge: `a --> c` leaves by
    // its own port, without one.
    let mixed = draw("flowchart LR\na <--> b\na --> c");
    assert_eq!(mixed.matches('◄').count(), 1, "\n{mixed}");
    assert_eq!(mixed.matches('►').count(), 2, "\n{mixed}");
    // A back edge is laid out reversed (a → c) and keeps both heads.
    let back = draw("flowchart LR\na --> b --> c\nc <--> a");
    assert!(back.contains("│ a │◄") && back.contains("►│ c │"), "\n{back}");
}
