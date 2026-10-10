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

#[test]
fn highlight_marks_nodes_and_thickens_edges() {
    let g = parse("flowchart LR\n%% highlight: a, b\na --> b --> c").unwrap();
    assert_eq!(g.nodes[0].mark, drawtool::Mark::Highlight);
    assert_eq!(g.nodes[2].mark, drawtool::Mark::None);
    assert_eq!(g.edges[0].style, drawtool::parse::Style::Thick);
    assert_eq!(g.edges[1].style, drawtool::parse::Style::Solid);
    let out = render(&g, None, false);
    assert!(out.contains('┏') && out.contains("━►"), "\n{out}");
}

#[test]
fn highlight_rejects_unknown_nodes_and_missing_edges() {
    let err = parse("a --> b\n%% highlight: a, zz").unwrap_err();
    assert!(err.starts_with("line 2:") && err.contains("zz"), "{err}");
    let mut g = parse("a --> b --> c").unwrap();
    assert!(drawtool::highlight(&mut g, &["a".into(), "c".into()]).unwrap_err().contains("no edge"));
}

#[test]
fn diff_marks_added_removed_and_changed() {
    let before = parse("a[A] --> b[B] --> c[C]").unwrap();
    let after = parse("a[A] --> b[B two] --> d[D]").unwrap();
    let g = drawtool::diff(&before, &after);
    let mark = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().mark;
    assert_eq!(mark("a"), drawtool::Mark::None);
    assert_eq!(mark("b"), drawtool::Mark::Changed);
    assert_eq!(mark("d"), drawtool::Mark::Added);
    assert_eq!(mark("c"), drawtool::Mark::Removed);
    let styles: Vec<_> = g.edges.iter().map(|e| e.style).collect();
    use drawtool::parse::Style::*;
    assert_eq!(styles, vec![Solid, Thick, Dotted]);
    let out = render(&g, None, false);
    assert!(out.contains("+ D") && out.contains("- C") && out.contains("~ B two"), "\n{out}");
}

#[test]
fn subgraph_membership_and_errors() {
    let g = parse("subgraph api [API server]\n  a --> b\n  subgraph inner\n    c\n  end\nend\nd --> a").unwrap();
    assert_eq!(g.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), vec!["API server", "inner"]);
    let group = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().group;
    assert_eq!((group("a"), group("b"), group("c"), group("d")), (Some(0), Some(0), Some(1), None));
    assert!(parse("a --> b\nend").unwrap_err().contains("without a matching"));
    assert!(parse("subgraph x\na").unwrap_err().contains("never closed"));
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
