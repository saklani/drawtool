//! Ondine's layout cases (tests/vectors/layout-cases.json, from wevm/ondine,
//! MIT): every one must draw, and the whole set is scored on what looks
//! wrong in a drawing, so a layout change can't quietly make things worse.

use drawtool::{parse, render_fit, render_sequence_fit, Diagram, Direction};
use serde_json::Value;

/// Lines crossing (`╂`), and arrowheads touching a junction (`├►┤`): an
/// arrow that runs into another edge instead of a box.
#[derive(Debug, Default, PartialEq, PartialOrd)]
struct Score {
    crossings: usize,
    arrow_tees: usize,
    cells: usize,
}

fn score(text: &str, s: &mut Score) {
    const TEES: &str = "├┤┬┴┼╂╟╢╤╧┠┨┯┷";
    let grid: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
    let at = |r: usize, c: usize| grid.get(r).and_then(|l| l.get(c)).copied().unwrap_or(' ');
    for (r, line) in grid.iter().enumerate() {
        s.cells += line.len();
        for (c, &ch) in line.iter().enumerate() {
            s.crossings += usize::from(ch == '╂' || ch == '┼');
            // The cell an arrowhead points into should be a box, not a junction.
            let ahead = match ch {
                '►' => at(r, c + 1),
                '◄' => c.checked_sub(1).map_or(' ', |c| at(r, c)),
                '▼' => at(r + 1, c),
                '▲' => r.checked_sub(1).map_or(' ', |r| at(r, c)),
                _ => continue,
            };
            s.arrow_tees += usize::from(TEES.contains(ahead));
        }
    }
}

#[test]
fn ondine_layout_cases_draw_and_score_no_worse() {
    let cases: Value = serde_json::from_str(include_str!("vectors/layout-cases.json")).unwrap();
    let mut total = Score::default();
    let mut count = 0;
    for kind in ["flowchart", "state", "sequence"] {
        for group in cases[kind].as_array().unwrap() {
            for case in group["cases"].as_array().unwrap() {
                let source = case["source"].as_str().unwrap();
                let title = case["title"].as_str().unwrap();
                let diagram = match parse(source) {
                    Err(e) if e.message.contains("edges to subgraphs") => continue,
                    d => d.unwrap_or_else(|e| panic!("{title}: {e}\n{source}")),
                };
                let texts = match diagram {
                    Diagram::Flowchart(g) => {
                        // Drawn as written and left-to-right, the direction drawtool prefers.
                        let mut texts = vec![render_fit(&g, g.direction, 120).text];
                        if g.direction != Direction::LR {
                            texts.push(render_fit(&g, Direction::LR, 120).text);
                        }
                        texts
                    }
                    Diagram::Sequence(s) => vec![render_sequence_fit(&s, 120).unwrap().text],
                    Diagram::Wireframe(_) => unreachable!(),
                };
                for text in texts {
                    score(&text, &mut total);
                    count += 1;
                }
            }
        }
    }
    eprintln!("{count} drawings: {total:?}");
    // Lower these as layouts improve.
    let baseline = Score { crossings: 254, arrow_tees: 226, cells: 118_792 };
    assert!(total.crossings <= baseline.crossings, "more crossings: {total:?}");
    assert!(total.arrow_tees <= baseline.arrow_tees, "more arrows into junctions: {total:?}");
    assert!(total.cells * 100 <= baseline.cells * 105, "drawings over 5% bigger: {total:?}");
}
