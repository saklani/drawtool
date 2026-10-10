//! Emphasis on top of a parsed graph: highlighting a path, and diffing two
//! versions of a diagram. Both only change node marks, labels and edge styles,
//! so layout stays unaware of them.

use crate::model::{EdgeDef, Graph, Group, Mark, NodeDef, Style};
use crate::Error;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};

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

/// `lines` with `prefix` in front of the first line (or just `prefix`).
fn prefixed(prefix: &str, lines: &[String]) -> Vec<String> {
    let mut out = lines.to_vec();
    match out.first_mut() {
        Some(first) if !first.is_empty() => *first = format!("{prefix} {first}"),
        _ => out = vec![prefix.to_string()],
    }
    out
}

/// An edge label with `prefix` (an unlabelled edge gets `prefix` alone).
fn edge_label(prefix: &str, label: Option<&[String]>) -> Option<Vec<String>> {
    Some(prefixed(prefix, label.unwrap_or_default()))
}

fn mark(node: &mut NodeDef, mark: Mark, prefix: &str) {
    node.mark = mark;
    node.lines = prefixed(prefix, &node.lines);
}

/// An edge's identity across versions: its end ids, and which of the edges
/// between that pair it is (so parallel edges are matched in order).
type EdgeKey = (String, String, usize);

fn edge_keys(graph: &Graph) -> Vec<EdgeKey> {
    let mut seen: HashMap<(NodeIndex, NodeIndex), usize> = HashMap::new();
    let ends = graph.g.edge_references().map(|e| (e.source(), e.target()));
    let key = |(s, t)| {
        let nth = seen.entry((s, t)).or_default();
        *nth += 1;
        (graph.g[s].id.clone(), graph.g[t].id.clone(), *nth)
    };
    ends.map(key).collect()
}

/// The index of the group titled `title` in `graph`, adding it if needed.
fn group_titled(graph: &mut Graph, title: &str) -> usize {
    graph.groups.iter().position(|g| g.title == title).unwrap_or_else(|| {
        graph.groups.push(Group { title: title.to_string() });
        graph.groups.len() - 1
    })
}

/// One graph showing how `before` became `after`: added nodes and edges are
/// thick with a `+`, removed ones dashed/dotted with a `-`, and nodes or edges
/// whose label, shape, style or arrowheads changed get a `~`. Nodes are matched by id,
/// edges by their end ids (parallel edges in order of appearance).
#[must_use]
pub fn diff(before: &Graph, after: &Graph) -> Graph {
    let mut g = after.clone();
    let old: HashMap<&str, &NodeDef> = before.g.node_weights().map(|n| (n.id.as_str(), n)).collect();
    for n in g.g.node_weights_mut() {
        match old.get(n.id.as_str()) {
            None => mark(n, Mark::Added, "+"),
            Some(o) if o.lines != n.lines || o.shape != n.shape => mark(n, Mark::Changed, "~"),
            Some(_) => {}
        }
    }
    let mut index: HashMap<String, NodeIndex> = g.g.node_indices().map(|v| (g.g[v].id.clone(), v)).collect();
    for n in before.g.node_weights().filter(|n| !after.g.node_weights().any(|a| a.id == n.id)) {
        let mut removed = n.clone();
        mark(&mut removed, Mark::Removed, "-");
        // Keep it inside its frame: group indices differ between versions, titles don't.
        removed.group = n.group.map(|i| group_titled(&mut g, &before.groups[i].title));
        index.insert(n.id.clone(), g.g.add_node(removed));
    }

    let old_edges: HashMap<EdgeKey, &EdgeDef> = edge_keys(before).into_iter().zip(before.g.edge_weights()).collect();
    let new_keys = edge_keys(after);
    for (e, key) in g.g.edge_weights_mut().zip(&new_keys) {
        match old_edges.get(key) {
            None => {
                e.style = Style::Thick;
                e.label = edge_label("+", e.label.as_deref());
            }
            Some(o) if o.label != e.label || o.style != e.style || o.arrow != e.arrow => {
                e.label = edge_label("~", e.label.as_deref());
            }
            Some(_) => {}
        }
    }
    let new_keys: HashSet<EdgeKey> = new_keys.into_iter().collect();
    for (e, key) in before.g.edge_weights().zip(edge_keys(before)) {
        if !new_keys.contains(&key) {
            let label = edge_label("-", e.label.as_deref());
            g.g.add_edge(index[&key.0], index[&key.1], EdgeDef { label, style: Style::Dotted, arrow: e.arrow });
        }
    }
    g
}
