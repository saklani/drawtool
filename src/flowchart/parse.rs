//! Flowcharts: standard Mermaid, parsed by Ondine and converted to the
//! graph drawtool lays out.

use super::model::{Arrow, Direction, EdgeDef, Graph, Group, NodeDef, Shape, Style};
use crate::mermaid::{get, label, list};
use crate::Error;
use petgraph::graph::{DiGraph, NodeIndex};
use serde_json::Value;
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
pub(crate) fn flowchart(m: &Value, text: &str) -> Result<Graph, Error> {
    let direction = match get(m, "direction") {
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
pub(crate) fn state(m: &Value, text: &str) -> Result<Graph, Error> {
    let direction = if get(m, "direction") == "LR" { Direction::LR } else { Direction::TD };
    let (mut g, mut groups, mut index) = (DiGraph::new(), Vec::new(), HashMap::new());
    let mut group_of = HashMap::new();
    let (nodes, edges) = (list(&m["graph"]["nodes"]), list(&m["graph"]["edges"]));
    for n in nodes.iter().filter(|n| get(n, "shape") != "noteGroup") {
        let (id, shape) = (get(n, "id"), get(n, "shape"));
        let (lines, shape) = match shape {
            "stateStart" => (vec!["●".to_string()], Shape::Round),
            "stateEnd" => (vec!["◉".to_string()], Shape::Round),
            "choice" => (vec![String::new()], Shape::Decision),
            _ => {
                let mut lines = label(n["label"].as_str().unwrap_or(id), false);
                lines.extend(list(&n["description"]).iter().flat_map(|d| label(d.as_str().unwrap_or_default(), false)));
                (lines, if shape == "note" { Shape::Box } else { Shape::Round })
            }
        };
        if n["isGroup"] == true {
            group_of.insert(id, groups.len());
            groups.push(Group { title: lines.join(" ") });
        }
        index.insert(id, g.add_node(NodeDef::new(id, lines, shape)));
    }
    // A composite state's own box goes in its frame, with the states inside it.
    for n in nodes {
        let frame = if n["isGroup"] == true { get(n, "id") } else { get(n, "parentId") };
        if let (Some(&v), Some(&grp)) = (index.get(get(n, "id")), group_of.get(frame)) {
            g[v].group = Some(grp);
        }
    }
    for e in edges {
        let (Some(&s), Some(&t)) = (index.get(get(e, "start")), index.get(get(e, "end"))) else { continue };
        let label = Some(label(get(e, "label"), false)).filter(|l| l.iter().any(|s| !s.is_empty()));
        // The line from a note to its state is dashed.
        let (style, arrow) = if e["pattern"].is_string() { (Style::Dotted, Arrow::None) } else { (Style::Solid, Arrow::End) };
        g.add_edge(s, t, EdgeDef { label, style, arrow });
    }
    let mut graph = Graph { direction, g, groups, stacks: Vec::new() };
    directives(text, &mut graph)?;
    Ok(graph)
}

fn convert(m: &Value) -> Result<(DiGraph<NodeDef, EdgeDef>, Vec<Group>), Error> {
    let subgraphs = list(&m["subgraphs"]);
    let sub_ids: HashSet<&str> = subgraphs.iter().map(|s| get(s, "id")).collect();
    let mut g = DiGraph::new();
    let mut index = HashMap::new();
    for n in m["nodes"].as_object().into_iter().flat_map(|o| o.values()) {
        let id = get(n, "id");
        if sub_ids.contains(id) {
            return Err(Error::new(format!("`{id}` is a subgraph: edges to subgraphs aren't supported yet")));
        }
        let lines = label(n["text"].as_str().unwrap_or(id), get(n, "labelType") == "markdown");
        index.insert(id, g.add_node(NodeDef::new(id, lines, shape(get(n, "type")))));
    }
    // Mermaid lists subgraphs innermost first, so the first to list a node owns it.
    for (i, s) in subgraphs.iter().enumerate() {
        for id in list(&s["nodes"]) {
            if let Some(&v) = id.as_str().and_then(|id| index.get(id)) {
                g[v].group.get_or_insert(i);
            }
        }
    }
    let groups = subgraphs.iter().map(|s| Group { title: label(get(s, "title"), false).join(" ") }).collect();
    for e in list(&m["edges"]).iter().filter(|e| get(e, "stroke") != "invisible") {
        let style = match get(e, "stroke") {
            "dotted" => Style::Dotted,
            "thick" => Style::Thick,
            _ => Style::Solid,
        };
        let arrow = match get(e, "type") {
            "arrow_open" => Arrow::None,
            a if a.starts_with("double_") => Arrow::Both,
            _ => Arrow::End,
        };
        let text = Some(get(e, "text")).filter(|l| !l.trim().is_empty());
        let label = text.map(|l| label(l, get(e, "labelType") == "markdown"));
        g.add_edge(index[get(e, "start")], index[get(e, "end")], EdgeDef { label, style, arrow });
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
