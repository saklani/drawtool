//! Emphasis on top of a parsed graph: highlighting a path, and diffing two
//! versions of a diagram. Both only change node marks, labels and edge styles,
//! so layout stays unaware of them.

use crate::parse::{EdgeDef, Graph, Mark, Style};
use crate::Error;
use std::collections::{HashMap, HashSet};

/// Split `a, b c` / `a,b,c` into ids.
pub fn split_ids(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Mark the nodes on `path` (ids, in order) and draw the edges between
/// consecutive ones thick. Every id must exist and every consecutive pair must
/// be joined by an edge, in either direction.
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
            return Err(Error::new(format!(
                "highlight: no edge between {:?} and {:?}",
                graph.nodes[a].id, graph.nodes[b].id
            )));
        }
    }
    Ok(())
}

fn prefixed(prefix: &str, lines: &[String]) -> Vec<String> {
    let mut out = lines.to_vec();
    match out.first_mut() {
        Some(first) if !first.is_empty() => *first = format!("{prefix} {first}"),
        _ => out = vec![prefix.to_string()],
    }
    out
}

/// One graph showing how `before` became `after`: added nodes and edges are
/// thick with a `+`, removed ones dashed/dotted with a `-`, and nodes whose
/// label changed get a `~`. Nodes and edges are matched by id.
#[must_use]
pub fn diff(before: &Graph, after: &Graph) -> Graph {
    let mut g = after.clone();
    let old: HashMap<&str, usize> = before.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    for n in &mut g.nodes {
        match old.get(n.id.as_str()) {
            None => {
                n.mark = Mark::Added;
                n.lines = prefixed("+", &n.lines);
            }
            Some(&i) if before.nodes[i].lines != n.lines => {
                n.mark = Mark::Changed;
                n.lines = prefixed("~", &n.lines);
            }
            Some(_) => {}
        }
    }
    let mut index: HashMap<String, usize> = g.nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i)).collect();
    for n in &before.nodes {
        if !index.contains_key(&n.id) {
            let mut removed = n.clone();
            removed.mark = Mark::Removed;
            removed.group = None;
            removed.lines = prefixed("-", &n.lines);
            index.insert(n.id.clone(), g.nodes.len());
            g.nodes.push(removed);
        }
    }

    let key = |graph: &Graph, e: &EdgeDef| (graph.nodes[e.src].id.clone(), graph.nodes[e.dst].id.clone());
    let old_edges: HashSet<(String, String)> = before.edges.iter().map(|e| key(before, e)).collect();
    let new_edges: HashSet<(String, String)> = after.edges.iter().map(|e| key(after, e)).collect();
    for e in &mut g.edges {
        if !old_edges.contains(&key(after, e)) {
            e.style = Style::Thick;
            e.label = Some(prefixed("+", e.label.as_deref().unwrap_or(&[])));
        }
    }
    for e in &before.edges {
        let (src, dst) = key(before, e);
        if !new_edges.contains(&(src.clone(), dst.clone())) {
            g.edges.push(EdgeDef {
                src: index[&src],
                dst: index[&dst],
                label: Some(prefixed("-", e.label.as_deref().unwrap_or(&[]))),
                style: Style::Dotted,
                arrow: e.arrow,
            });
        }
    }
    g
}
