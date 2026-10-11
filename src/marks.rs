//! Highlighting a path on top of a parsed graph. It only changes node marks
//! and edge styles, so layout stays unaware of it.

use crate::model::{Graph, Mark, Style};
use crate::Error;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

/// Split `a, b c` / `a,b,c` into ids.
pub(crate) fn split_ids(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Mark the nodes on `path` (ids, in order) and draw the edges between
/// consecutive ones thick. Every id must exist and every consecutive pair must
/// be joined by an edge, in either direction.
///
/// # Errors
///
/// An id that isn't a node, or two consecutive ids with no edge between them.
pub fn highlight(graph: &mut Graph, path: &[String]) -> Result<(), Error> {
    let g = &mut graph.g;
    let index: HashMap<&str, NodeIndex> = g.node_indices().map(|v| (g[v].id.as_str(), v)).collect();
    let mut ids = Vec::new();
    for id in path {
        ids.push(*index.get(id.as_str()).ok_or_else(|| Error::new(format!("highlight: no node {id:?}")))?);
    }
    for &v in &ids {
        g[v].mark = Mark::Highlight;
    }
    for pair in ids.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let between: Vec<_> = g.edges_connecting(a, b).chain(g.edges_connecting(b, a)).map(|e| e.id()).collect();
        if between.is_empty() {
            let (a, b) = (&g[a].id, &g[b].id);
            return Err(Error::new(format!("highlight: no edge between {a:?} and {b:?}")));
        }
        for e in between {
            g[e].style = Style::Thick;
        }
    }
    Ok(())
}
