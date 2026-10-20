//! Fit a flowchart into a column budget without changing its direction, by
//! laying it out: labels are wrapped no more than needed; a left-to-right
//! diagram that is still too wide is cut into rows (see the layout module).

use super::layout::{draw, layers};
use super::model::{Direction, Graph};
use crate::text::{block_width, wrap_all, Fit, WRAP_WIDTHS};

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
    let (td, budget) = (direction == Direction::TD, if max_width == 0 { i64::MAX } else { max_width as i64 });
    let fit = |text: String| {
        let width = block_width(&text) as usize;
        Fit { fits: width as i64 <= budget, text, width }
    };
    let graphs = || WRAP_WIDTHS.iter().map(|&w| wrapped(graph, w));
    let mut tightest = None;
    for g in graphs() {
        let one_row = fit(draw(&g, td, &[]).0);
        if one_row.fits {
            return one_row;
        }
        tightest = Some(one_row);
    }
    let rows = graphs().filter(|_| !td).find_map(|g| Some(fit(draw(&g, td, &cuts(&g, budget)?).0)));
    rows.or(tightest).unwrap_or_else(|| fit(String::new()))
}

/// Where to cut the layers into rows that each fit in `budget`, each row as
/// long as it can be, found by laying them out. `None` if a layer is too
/// wide on its own.
fn cuts(graph: &Graph, budget: i64) -> Option<Vec<usize>> {
    let n = layers(graph).into_iter().max().map_or(0, |l| l + 1);
    let mut cuts: Vec<usize> = vec![];
    for _ in 0..2 * n {
        let Some(r) = draw(graph, false, &cuts).1.iter().position(|&w| w > budget) else { return Some(cuts) };
        cuts.truncate(r);
        // The latest cut after which this row fits.
        let fits = |j: usize| draw(graph, false, &[&cuts[..], &[j]].concat()).1[r] <= budget;
        let (mut lo, mut hi, mut best) = (cuts.last().map_or(0, |c| c + 1), n, None);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if mid > 0 && fits(mid) {
                (best, lo) = (Some(mid), mid + 1);
            } else {
                hi = mid;
            }
        }
        cuts.push(best?);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowchart::parse::parse;

    /// A diagram that fits a budget fits every larger one.
    #[test]
    fn a_looser_budget_never_does_worse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for entry in std::fs::read_dir(dir).unwrap() {
            let src = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let Ok(graph) = parse(&src) else { continue };
            for direction in [Direction::LR, Direction::TD] {
                let fits: Vec<bool> = (30..=120).step_by(5).map(|b| render_fit(&graph, direction, b).fits).collect();
                assert!(fits.windows(2).all(|w| w[1] || !w[0]), "{direction:?}: {fits:?}\n{src}");
            }
        }
    }
}
