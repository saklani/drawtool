use drawtool::{parse, render};
use petgraph::graph::{EdgeIndex, NodeIndex};

#[test]
fn parse_error_names_the_line() {
    let err = parse("a --> b\nb -->\n").unwrap_err();
    assert_eq!(err.line, Some(2), "{err}");
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
fn last_explicit_label_wins_as_in_mermaid() {
    let g = parse("a[First] --> b\na[Second] --> c\na --> d").unwrap();
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
    let unwrapped = render(&g, None, false).lines().map(unicode_width::UnicodeWidthStr::width).max().unwrap_or(0);
    let fitted = drawtool::render_fit(&g, drawtool::Direction::LR, false, unwrapped - 10);
    assert!(same_row(&fitted.text, "a fairly", "another"), "still LR:\n{}", fitted.text);
    assert!(fitted.fits && fitted.width < unwrapped, "\n{}", fitted.text);
}

#[test]
fn long_lr_chain_breaks_into_rows_and_stays_lr() {
    let src = "flowchart LR\na --> b --> c --> d --> e --> f --> g --> h";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 30);
    assert!(same_row(&fitted.text, "│ a ", "│ b "), "still LR:\n{}", fitted.text);
    assert!(fitted.text.contains("└─►") && fitted.width <= 30, "wrapped onto a new row:\n{}", fitted.text);
}

#[test]
fn lr_never_switches_to_td_on_its_own() {
    // Each box is 12 wide: no LR layout fits in 12 columns, but TD would.
    let src = "flowchart LR\na[abcdefgh] --> b[ijklmnop] --> c[qrstuvwx]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 12);
    assert!(same_row(&fitted.text, "abcdefgh", "ijklmnop"), "still LR:\n{}", fitted.text);
    assert!(!fitted.fits);
}

#[test]
fn td_is_kept_when_asked_for() {
    let src = "flowchart TD\na --> b --> c";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::TD, false, 120);
    assert!(!same_row(&fitted.text, "│ a ", "│ b "), "stacked, not side by side:\n{}", fitted.text);
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
    assert_eq!(g.g[NodeIndex::new(0)].lines, vec!["supercalifragilistic", "word"]);
}

#[test]
fn single_crossing_edge_wraps_around_like_text() {
    let src = "flowchart LR\na[first step] --> b[second step] --> c[third step] --> d[fourth step]";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 50);
    assert!(fitted.width <= 50, "\n{}", fitted.text);
    assert!(!fitted.text.contains('①'), "a lone crossing edge is drawn, not numbered:\n{}", fitted.text);
    assert!(fitted.text.contains("└─►"), "\n{}", fitted.text);
}

#[test]
fn several_crossing_edges_keep_connectors() {
    let src = "flowchart LR\na --> b --> c --> d --> e\nb --> e";
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 20);
    assert!(fitted.text.contains('①'), "\n{}", fitted.text);
}

#[test]
fn highlight_marks_nodes_and_thickens_edges() {
    let g = parse("flowchart LR\n%% highlight: a, b\na --> b --> c").unwrap();
    assert_eq!(g.g[NodeIndex::new(0)].mark, drawtool::Mark::Highlight);
    assert_eq!(g.g[NodeIndex::new(2)].mark, drawtool::Mark::None);
    assert_eq!(g.g[EdgeIndex::new(0)].style, drawtool::Style::Thick);
    assert_eq!(g.g[EdgeIndex::new(1)].style, drawtool::Style::Solid);
    let out = render(&g, None, false);
    assert!(out.contains('┏') && out.contains("━►"), "\n{out}");
}

#[test]
fn highlight_rejects_unknown_nodes_and_missing_edges() {
    let err = parse("a --> b\n%% highlight: a, zz").unwrap_err();
    assert!(err.line == Some(2) && err.message.contains("zz"), "{err}");
    let mut g = parse("a --> b --> c").unwrap();
    assert!(drawtool::highlight(&mut g, &["a".into(), "c".into()]).unwrap_err().message.contains("no edge"));
}

#[test]
fn diff_marks_added_removed_and_changed() {
    let before = parse("a[A] --> b[B] --> c[C]").unwrap();
    let after = parse("a[A] --> b[B two] --> d[D]").unwrap();
    let g = drawtool::diff(&before, &after);
    let mark = |id: &str| g.g.node_weights().find(|n| n.id == id).unwrap().mark;
    assert_eq!(mark("a"), drawtool::Mark::None);
    assert_eq!(mark("b"), drawtool::Mark::Changed);
    assert_eq!(mark("d"), drawtool::Mark::Added);
    assert_eq!(mark("c"), drawtool::Mark::Removed);
    let styles: Vec<_> = g.g.edge_weights().map(|e| e.style).collect();
    use drawtool::Style::*;
    assert_eq!(styles, vec![Solid, Thick, Dotted]);
    let out = render(&g, None, false);
    assert!(out.contains("+ D") && out.contains("- C") && out.contains("~ B two"), "\n{out}");
}

#[test]
fn subgraph_membership_and_errors() {
    let g = parse("subgraph api [API server]\n  a --> b\n  subgraph inner\n    c\n  end\nend\nd --> a").unwrap();
    assert_eq!(g.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), vec!["API server", "inner"]);
    let group = |id: &str| g.g.node_weights().find(|n| n.id == id).unwrap().group;
    assert_eq!((group("a"), group("b"), group("c"), group("d")), (Some(0), Some(0), Some(1), None));
    assert!(parse("a --> b\nend").unwrap_err().message.contains("without a matching"));
    assert!(parse("subgraph x\na").unwrap_err().message.contains("never closed"));
}

#[test]
fn group_frame_holds_members_and_excludes_others() {
    for dir in ["LR", "TD"] {
        let src = format!("flowchart {dir}\nsubgraph g [Grp]\n  a --> b\nend\nc --> a\nb --> d");
        let out = render(&parse(&src).unwrap(), None, false);
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
    let fitted = drawtool::render_fit(&parse(src).unwrap(), drawtool::Direction::LR, false, 120);
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
    let out = render(&parse(&src).unwrap(), None, false);
    assert_eq!(crossings(&out), 0, "\n{out}");
}

#[test]
fn unrelated_edges_are_never_drawn_as_joined() {
    // Edges that only share ports transitively (b→e, c→e, c→g) used to merge
    // into one net, so their crossings rendered as ┼ joins.
    let out = render(&parse("flowchart LR\nb --> e & f\nc --> e & g\nd --> f & g\nb --> g").unwrap(), None, false);
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
    let out = render(&parse("flowchart LR\nx --> a\nx --> b\na --> b\n%% stack: a b").unwrap(), None, false);
    let ((ya, xa), (yb, xb)) = (find(&out, "│ a"), find(&out, "│ b"));
    assert!(ya < yb && xa == xb, "a above b, same column:\n{out}");
    assert!(out.contains('▼'), "a → b runs down:\n{out}");
    // The order is the stack's: b first means b on top, and a → b runs up.
    let out = render(&parse("flowchart LR\nx --> a\nx --> b\na --> b\n%% stack: b a").unwrap(), None, false);
    assert!(find(&out, "│ b").0 < find(&out, "│ a").0 && out.contains('▲'), "\n{out}");
}

#[test]
fn a_stacked_two_way_pair_has_no_outer_loop() {
    let src = "flowchart LR\na --> b\na --> c\nb --> c\nc --> b";
    let looped = render(&parse(src).unwrap(), None, false);
    let stacked = render(&parse(&format!("{src}\n%% stack: b c")).unwrap(), None, false);
    assert!(looped.lines().count() > find(&looped, "│ c").0 + 2, "unstacked, c → b loops below:\n{looped}");
    assert_eq!(stacked.lines().count(), find(&stacked, "│ c").0 + 2, "nothing below c:\n{stacked}");
    assert!(stacked.contains('▼') && stacked.contains('▲'), "\n{stacked}");
}

#[test]
fn stacks_run_across_in_td_and_skip_with_a_bracket() {
    let out = render(&parse("flowchart TD\nx --> a & b\na -->|go| b\n%% stack: a b").unwrap(), None, false);
    assert!(same_row(&out, "│ a", "│ b") && out.contains('►') && out.contains("go"), "\n{out}");
    // a → c skips over b: out of a's far side, along a lane, into c's.
    let out = render(&parse("flowchart LR\nx --> a & b & c\na --> c\n%% stack: a b c").unwrap(), None, false);
    assert!(out.contains('◄'), "\n{out}");
}

#[test]
fn two_headed_edges_get_a_head_at_each_end() {
    let draw = |src: &str, ascii| render(&parse(src).unwrap(), None, ascii);
    let lr = draw("flowchart LR\na <--> b", false);
    assert!(lr.contains("│◄─►│"), "\n{lr}");
    assert!(draw("flowchart LR\na <-.-> b", true).contains("|<.>|"), "ascii");
    let td = draw("flowchart TD\na <--> b", false);
    let col: Vec<char> = td.lines().filter_map(|l| l.chars().nth(2)).collect();
    assert!(col.windows(3).any(|w| w == ['▲', '│', '▼']), "\n{td}");
    // The head into `a` marks only the two-headed edge: `a --> c` leaves by
    // its own port, without one.
    let mixed = draw("flowchart LR\na <--> b\na --> c", false);
    assert_eq!(mixed.matches('◄').count(), 1, "\n{mixed}");
    assert_eq!(mixed.matches('►').count(), 2, "\n{mixed}");
    // A back edge keeps its head into its source too.
    let back = draw("flowchart LR\na --> b --> c\nc <--> a", false);
    assert!(back.contains("│ c │◄"), "\n{back}");
}
