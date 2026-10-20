//! Fit a flowchart into a column budget without changing its direction, by
//! laying it out: labels are wrapped no more than needed; a left-to-right
//! diagram that is still too wide is cut into rows (see the layout module).

use super::layout::{draw, layers};
use super::model::{Direction, Graph};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use crate::text::{first_fit, wrap_all, Fit, WRAP_WIDTHS};

/// Copy of `graph` with every node and edge label wrapped at `max` columns.
#[must_use]
pub fn wrap_labels(graph: &Graph, max: usize) -> Graph {
    let mut g = graph.clone();
    g.g.node_weights_mut().for_each(|n| wrap_all(&mut n.lines, max));
    g.g.edge_weights_mut().filter_map(|e| e.label.as_mut()).for_each(|l| wrap_all(l, max));
    g
}

/// Render within `max_width` columns (0 = unlimited), keeping `direction`:
/// the loosest label wrapping that fits on one row; else, left to right,
/// the loosest that can be cut into rows that each fit; else the narrowest.
#[must_use]
pub fn render_fit(graph: &Graph, direction: Direction, max_width: usize) -> Fit {
    let (td, budget) = (direction == Direction::TD, if max_width == 0 { i64::MAX } else { max_width as i64 });
    let graphs = || WRAP_WIDTHS.iter().map(|&w| w.map_or_else(|| graph.clone(), |w| wrap_labels(graph, w)));
    let one_row = graphs().map(|g| draw(&g, td, &[]).0);
    let rows = graphs().filter(|_| !td).filter_map(|g| Some(draw(&g, false, &row_breaks(&g, budget)?).0));
    first_fit(max_width, one_row.chain(rows))
}

/// Where to cut the layers into rows that each fit in `budget`, found by
/// laying them out: each row as long as fits, or a layer shorter where that
/// cut is cheaper. `None` if a layer is too wide on its own.
fn row_breaks(graph: &Graph, budget: i64) -> Option<Vec<usize>> {
    let layer = layers(graph);
    let n = layer.iter().max().map_or(0, |l| l + 1);
    // A cut before layer `j` costs a line per edge wrapping across it, and three per frame it splits.
    let span = |v: NodeIndex| layer[v.index()];
    let crosses = |j: usize, a: usize, b: usize| a.min(b) < j && j <= a.max(b);
    let frames: Vec<(usize, usize)> = (0..graph.groups.len())
        .filter_map(|g| {
            let ls = graph.g.node_indices().filter(|&v| graph.g[v].group == Some(g)).map(span);
            ls.clone().min().zip(ls.max())
        })
        .collect();
    let cost = |j: usize| {
        let edges = graph.g.edge_references().filter(|e| crosses(j, span(e.source()), span(e.target()))).count();
        edges + 3 * frames.iter().filter(|f| crosses(j, f.0, f.1)).count()
    };
    let mut breaks: Vec<usize> = vec![];
    // A row cut again can grow past the budget once the next cut is in, so give up eventually.
    for _ in 0..2 * n {
        let Some(r) = draw(graph, false, &breaks).1.iter().position(|&w| w > budget) else { return Some(breaks) };
        breaks.truncate(r);
        let fits = |j: usize| draw(graph, false, &[&breaks[..], &[j]].concat()).1[r] <= budget;
        // A row holds at least one layer.
        let first = breaks.last().map_or(1, |c| c + 1);
        let (mut lo, mut hi, mut latest) = (first, n, None);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if fits(mid) {
                (latest, lo) = (Some(mid), mid + 1);
            } else {
                hi = mid;
            }
        }
        let latest = latest?;
        // A shorter row is no wider, so it fits too.
        breaks.push((latest.saturating_sub(1).max(first)..=latest).rev().min_by_key(|&j| cost(j)).unwrap_or(latest));
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
