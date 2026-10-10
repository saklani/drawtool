//! Emphasis on top of a parsed graph: highlighting a path, and diffing two
//! versions of a diagram. Both only change node marks, labels and edge styles,
//! so layout stays unaware of them.

use crate::model::{EdgeDef, Graph, Group, Mark, NodeDef, Style};
use crate::Error;
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
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut ids = Vec::new();
    for id in path {
        ids.push(*index.get(id.as_str()).ok_or_else(|| Error::new(format!("highlight: no node {id:?}")))?);
    }
    for &i in &ids {
        graph.nodes[i].mark = Mark::Highlight;
    }
    for pair in ids.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mut found = false;
        for e in graph.edges.iter_mut().filter(|e| (e.src, e.dst) == (a, b) || (e.src, e.dst) == (b, a)) {
            e.style = Style::Thick;
            found = true;
        }
        if !found {
            let (a, b) = (&graph.nodes[a].id, &graph.nodes[b].id);
            return Err(Error::new(format!("highlight: no edge between {a:?} and {b:?}")));
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

fn mark(node: &mut NodeDef, mark: Mark, prefix: &str) {
    node.mark = mark;
    node.lines = prefixed(prefix, &node.lines);
}

/// An edge's identity across versions: its end ids, and which of the edges
/// between that pair it is (so parallel edges are matched in order).
type EdgeKey = (String, String, usize);

fn edge_keys(graph: &Graph) -> Vec<EdgeKey> {
    let mut seen: HashMap<(usize, usize), usize> = HashMap::new();
    let nth = |e: &EdgeDef, seen: &mut HashMap<_, usize>| {
        let n = seen.entry((e.src, e.dst)).or_default();
        *n += 1;
        *n
    };
    graph
        .edges
        .iter()
        .map(|e| (graph.nodes[e.src].id.clone(), graph.nodes[e.dst].id.clone(), nth(e, &mut seen)))
        .collect()
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
/// whose label, shape or style changed get a `~`. Nodes are matched by id,
/// edges by their end ids (parallel edges in order of appearance).
#[must_use]
pub fn diff(before: &Graph, after: &Graph) -> Graph {
    let mut g = after.clone();
    let old: HashMap<&str, &NodeDef> = before.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for n in &mut g.nodes {
        match old.get(n.id.as_str()) {
            None => mark(n, Mark::Added, "+"),
            Some(o) if o.lines != n.lines || o.shape != n.shape => mark(n, Mark::Changed, "~"),
            Some(_) => {}
        }
    }
    let mut index: HashMap<String, usize> = g.nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i)).collect();
    for n in before.nodes.iter().filter(|n| !after.nodes.iter().any(|a| a.id == n.id)) {
        let mut removed = n.clone();
        mark(&mut removed, Mark::Removed, "-");
        // Keep it inside its frame: group indices differ between versions, titles don't.
        removed.group = n.group.map(|i| group_titled(&mut g, &before.groups[i].title));
        index.insert(n.id.clone(), g.nodes.len());
        g.nodes.push(removed);
    }

    let old_edges: HashMap<EdgeKey, &EdgeDef> = edge_keys(before).into_iter().zip(&before.edges).collect();
    let new_keys = edge_keys(after);
    for (e, key) in g.edges.iter_mut().zip(&new_keys) {
        match old_edges.get(key) {
            None => {
                e.style = Style::Thick;
                e.label = Some(prefixed("+", e.label.as_deref().unwrap_or(&[])));
            }
            Some(o) if o.label != e.label || o.style != e.style => {
                e.label = Some(prefixed("~", e.label.as_deref().unwrap_or(&[])));
            }
            Some(_) => {}
        }
    }
    let new_keys: HashSet<EdgeKey> = new_keys.into_iter().collect();
    for (e, key) in before.edges.iter().zip(edge_keys(before)) {
        if !new_keys.contains(&key) {
            let (src, dst) = (index[&key.0], index[&key.1]);
            let label = Some(prefixed("-", e.label.as_deref().unwrap_or(&[])));
            g.edges.push(EdgeDef { src, dst, label, style: Style::Dotted, arrow: e.arrow });
        }
    }
    g
}
