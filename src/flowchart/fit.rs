//! Fit a flowchart into a column budget without changing its direction,
//! deciding everything from [`measure`]'s estimates before laying it out
//! once. Labels are wrapped no more than needed; a left-to-right diagram
//! that is still too wide is cut into rows (see the layout module).

use super::layout::{draw, measure, Measure};
use super::model::{Direction, Graph};
use crate::text::{block_width, wrap_all, Fit, WRAP_WIDTHS};

/// A top-down estimate must leave this share of the budget free: placement
/// can spread a layer wider than its boxes packed side by side.
const TD_MARGIN_PERCENT: i64 = 15;

/// Copy of `graph` with every node and edge label wrapped at `max` columns.
#[must_use]
pub fn wrap_labels(graph: &Graph, max: usize) -> Graph {
    let mut g = graph.clone();
    g.g.node_weights_mut().for_each(|n| wrap_all(&mut n.lines, max));
    g.g.edge_weights_mut().filter_map(|e| e.label.as_mut()).for_each(|l| wrap_all(l, max));
    g
}

fn wrapped(graph: &Graph, wrap: Option<usize>) -> Graph {
    wrap.map_or_else(|| graph.clone(), |w| wrap_labels(graph, w))
}

/// Render within `max_width` columns (0 = unlimited), keeping `direction`:
/// the loosest label wrapping that fits on one row; else, left to right,
/// the loosest that can be cut into rows that each fit; else the tightest.
#[must_use]
pub fn render_fit(graph: &Graph, direction: Direction, max_width: usize) -> Fit {
    let (td, budget) = (direction == Direction::TD, max_width as i64);
    let measured = || WRAP_WIDTHS.iter().map(|&w| wrapped(graph, w)).map(|g| (measure(&g, td), g));
    let one_row = |m: &Measure| match () {
        () if max_width == 0 => true,
        () if td => m.td_width * 100 <= budget * (100 - TD_MARGIN_PERCENT),
        () => m.row(0, m.layers()) <= budget,
    };
    let (graph, cuts) = (measured().find(|(m, _)| one_row(m)).map(|(_, g)| (g, vec![])))
        .or_else(|| measured().filter(|_| !td).find_map(|(m, g)| Some((g, cuts(&m, budget)?))))
        .unwrap_or_else(|| (wrapped(graph, *WRAP_WIDTHS.last().unwrap()), vec![]));
    let text = draw(&graph, td, &cuts);
    let width = block_width(&text) as usize;
    Fit { text, width, fits: max_width == 0 || width <= max_width }
}

/// Where to cut the layers into rows that each fit in `budget`: fewest rows,
/// then the cheapest cuts. `None` if a layer is too wide on its own.
fn cuts(m: &Measure, budget: i64) -> Option<Vec<usize>> {
    let n = m.layers();
    // best[j]: the best way to lay out layers 0..j, as (rows, cost, start of the last row).
    let mut best: Vec<Option<(usize, usize, usize)>> = vec![None; n + 1];
    best[0] = Some((0, 0, 0));
    for j in 1..=n {
        for i in 0..j {
            let Some((rows, cost, _)) = best[i] else { continue };
            if m.row(i, j) > budget {
                continue;
            }
            // Ties go to the latest start, so earlier rows fill up first, like text.
            let cand = (rows + 1, cost + if i > 0 { m.cost(i) } else { 0 }, i);
            if best[j].is_none_or(|b| (cand.0, cand.1) <= (b.0, b.1)) {
                best[j] = Some(cand);
            }
        }
    }
    let (mut cuts, mut j) = (vec![], n);
    while j > 0 {
        j = best[j]?.2;
        cuts.push(j);
    }
    cuts.pop(); // the first row starts at 0
    cuts.reverse();
    Some(cuts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowchart::parse::parse;

    /// Whenever the estimates say a diagram fits (on one row, in rows, or
    /// top down within the margin), the drawing really does.
    #[test]
    fn what_the_estimates_promise_holds_for_every_example() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for entry in std::fs::read_dir(dir).unwrap() {
            let src = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let Ok(graph) = parse(&src) else { continue };
            for direction in [Direction::LR, Direction::TD] {
                let td = direction == Direction::TD;
                for budget in (30..=120).step_by(5) {
                    let tight = measure(&wrapped(&graph, *WRAP_WIDTHS.last().unwrap()), td);
                    let promised = if td {
                        tight.td_width * 100 <= budget * (100 - TD_MARGIN_PERCENT)
                    } else {
                        cuts(&tight, budget).is_some()
                    };
                    let fit = render_fit(&graph, direction, budget as usize);
                    assert!(!promised || fit.fits, "{direction:?} at {budget}:\n{}", fit.text);
                }
            }
        }
    }
}
