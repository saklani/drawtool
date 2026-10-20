//! Flowcharts: standard Mermaid, parsed by Ondine and converted to the
//! graph drawtool lays out.

use super::model::{Arrow, Direction, EdgeDef, Graph, Group, NodeDef, Shape, Style};
use crate::mermaid::{self, label};
use crate::Error;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};

/// The nearest shape drawtool has for one of Mermaid's.
fn shape(name: &str) -> Shape {
    match name {
        n if n.contains("cyl") || n == "database" || n == "db" => Shape::Database,
        "diamond" | "diam" | "decision" | "question" | "hexagon" | "hex" | "prepare" => Shape::Decision,
        "round" | "roundedRect" | "rounded" | "event" | "stadium" | "pill" | "terminal" | "circle" | "circ" | "doublecircle"
        | "dbl-circ" | "double-circle" | "start" | "stop" => Shape::Round,
        _ => Shape::Box,
    }
}

/// A Mermaid flowchart as a graph. Besides Mermaid's syntax, two comment
/// lines in `text` are read: `%% highlight: a, b, c` and `%% stack: a b c`.
///
/// # Errors
///
/// What drawtool can't draw yet: an edge to a subgraph. Also a `%% highlight:` or `%% stack:` line that names
/// a missing node.
pub(crate) fn flowchart(m: &mermaid::Flowchart, text: &str) -> Result<Graph, Error> {
    let direction = match m.direction.as_deref().unwrap_or_default() {
        // Drawn the usual way round: right-to-left and bottom-to-top read the same.
        "LR" | "RL" => Direction::LR,
        _ => Direction::TD,
    };
    let (g, groups) = convert(m)?;
    let mut graph = Graph { direction, g, groups, stacks: Vec::new() };
    directives(text, &mut graph)?;
    Ok(graph)
}

/// A Mermaid `stateDiagram-v2` as a graph: states are boxes, `[*]`
/// the ● start and ◉ end, a `<<choice>>` a diamond, a note a box on a dotted
/// line. A composite state is a box for the arrows into it, framed with the
/// states inside it. `%% highlight:` and `%% stack:`
/// work as in flowcharts.
///
/// # Errors
///
/// A bad `%% highlight:` / `%% stack:` line.
pub(crate) fn state(m: &mermaid::State, text: &str) -> Result<Graph, Error> {
    let direction = if m.direction.as_deref() == Some("LR") { Direction::LR } else { Direction::TD };
    let (mut g, mut groups, mut index) = (DiGraph::new(), Vec::new(), HashMap::new());
    let mut group_of = HashMap::new();
    let (nodes, edges) = (&m.graph.nodes, &m.graph.edges);
    for n in nodes.iter().filter(|n| n.shape.as_deref() != Some("noteGroup")) {
        let (id, shape) = (n.id.as_str(), n.shape.as_deref().unwrap_or_default());
        let (lines, shape) = match shape {
            "stateStart" => (vec!["●".to_string()], Shape::Round),
            "stateEnd" => (vec!["◉".to_string()], Shape::Round),
            "choice" => (vec![String::new()], Shape::Decision),
            _ => {
                let mut lines = label(n.label.as_deref().unwrap_or(id), false);
                lines.extend(n.description.iter().flat_map(|d| label(d, false)));
                (lines, if shape == "note" { Shape::Box } else { Shape::Round })
            }
        };
        if n.is_group {
            group_of.insert(id, groups.len());
            groups.push(Group { title: lines.join(" ") });
        }
        index.insert(id, g.add_node(NodeDef::new(id, lines, shape)));
    }
    // A composite state's own box goes in its frame, with the states inside it.
    for n in nodes {
        let frame = if n.is_group { Some(&n.id) } else { n.parent_id.as_ref() };
        if let (Some(&v), Some(&grp)) = (index.get(n.id.as_str()), frame.and_then(|f| group_of.get(f.as_str()))) {
            g[v].group = Some(grp);
        }
    }
    for e in edges {
        let (Some(&s), Some(&t)) = (index.get(e.start.as_str()), index.get(e.end.as_str())) else { continue };
        let label = Some(label(&e.label, false)).filter(|l| l.iter().any(|s| !s.is_empty()));
        // The line from a note to its state is dashed.
        let (style, arrow) = if e.pattern.is_some() { (Style::Dotted, Arrow::None) } else { (Style::Solid, Arrow::End) };
        g.add_edge(s, t, EdgeDef { label, style, arrow });
    }
    let mut graph = Graph { direction, g, groups, stacks: Vec::new() };
    directives(text, &mut graph)?;
    Ok(graph)
}

fn convert(m: &mermaid::Flowchart) -> Result<(DiGraph<NodeDef, EdgeDef>, Vec<Group>), Error> {
    let subgraphs = &m.subgraphs;
    let sub_ids: HashSet<&str> = subgraphs.iter().map(|s| s.id.as_str()).collect();
    let mut g = DiGraph::new();
    let mut index = HashMap::new();
    for n in m.nodes.values() {
        let id = n.id.as_str();
        if sub_ids.contains(id) {
            return Err(Error::new(format!("`{id}` is a subgraph: edges to subgraphs aren't supported yet")));
        }
        let lines = label(n.text.as_deref().unwrap_or(id), n.label_type.as_deref() == Some("markdown"));
        index.insert(id, g.add_node(NodeDef::new(id, lines, shape(n.shape.as_deref().unwrap_or_default()))));
    }
    // Mermaid lists subgraphs innermost first, so the first to list a node owns it.
    for (i, s) in subgraphs.iter().enumerate() {
        for id in &s.nodes {
            if let Some(&v) = index.get(id.as_str()) {
                g[v].group.get_or_insert(i);
            }
        }
    }
    let groups = subgraphs.iter().map(|s| Group { title: label(&s.title, false).join(" ") }).collect();
    for e in m.edges.iter().filter(|e| e.stroke.as_deref() != Some("invisible")) {
        let style = match e.stroke.as_deref() {
            Some("dotted") => Style::Dotted,
            Some("thick") => Style::Thick,
            _ => Style::Solid,
        };
        let arrow = match e.arrow.as_deref().unwrap_or_default() {
            "arrow_open" => Arrow::None,
            a if a.starts_with("double_") => Arrow::Both,
            _ => Arrow::End,
        };
        let text = Some(&e.text).filter(|l| !l.trim().is_empty());
        let label = text.map(|l| label(l, e.label_type.as_deref() == Some("markdown")));
        g.add_edge(index[e.start.as_str()], index[e.end.as_str()], EdgeDef { label, style, arrow });
    }
    Ok((g, groups))
}

/// The `%% highlight:` and `%% stack:` lines, which Mermaid reads as comments.
fn directives(text: &str, graph: &mut Graph) -> Result<(), Error> {
    let index: HashMap<String, NodeIndex> = graph.g.node_indices().map(|v| (graph.g[v].id.clone(), v)).collect();
    let mut seen = HashSet::new();
    for (i, raw) in text.lines().enumerate() {
        let comment = raw.trim().strip_prefix("%%").map(str::trim).unwrap_or_default();
        let at = |e: Error| Error::at(i + 1, e.message);
        if let Some(ids) = comment.strip_prefix("highlight:") {
            let ids = super::marks::highlight_ids(ids).map_err(at)?;
            super::marks::highlight(graph, &ids).map_err(at)?;
        } else if let Some(ids) = comment.strip_prefix("stack:") {
            let ids = super::marks::split_ids(ids);
            if ids.len() < 2 {
                return Err(Error::at(i + 1, "`%% stack:` needs two or more boxes"));
            }
            let mut stack = Vec::new();
            for id in &ids {
                let &v = index.get(id).ok_or_else(|| Error::at(i + 1, format!("stack: no node {id:?}")))?;
                if !seen.insert(v) {
                    return Err(Error::at(i + 1, format!("stack: {id:?} is already in a stack")));
                }
                stack.push(v);
            }
            if stack.iter().any(|&v| graph.g[v].group != graph.g[stack[0]].group) {
                return Err(Error::at(i + 1, "stack: its boxes must all be in the same subgraph"));
            }
            graph.stacks.push(stack);
        }
    }
    Ok(())
}

/// A flowchart's graph, for unit tests.
#[cfg(test)]
pub(crate) fn parse(text: &str) -> Result<Graph, Error> {
    match crate::parse(text)? {
        crate::Diagram::Flowchart(g) => Ok(g),
        _ => Err(Error::new("not a flowchart")),
    }
}
