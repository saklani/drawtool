//! Flowcharts: standard Mermaid, parsed by merman-core and converted to the
//! graph drawtool lays out.

use super::model::{Arrow, Direction, EdgeDef, Graph, Group, NodeDef, Shape, Style};
use crate::mermaid::label;
use crate::Error;
use merman_core::diagrams::flowchart::{FlowEdgeMarker, FlowEdgeStroke, FlowEdgeVisibility, FlowchartModel};
use merman_core::diagrams::state::StateDiagramRenderModel;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};

/// The nearest shape drawtool has for one of Mermaid's.
fn shape(name: &str) -> Shape {
    match name {
        n if n.contains("cyl") || n == "database" || n == "db" => Shape::Database,
        "diamond" | "diam" | "decision" | "question" | "hexagon" | "hex" | "prepare" => Shape::Decision,
        "roundedRect" | "rounded" | "event" | "stadium" | "pill" | "terminal" | "circle" | "circ" | "doublecircle"
        | "dbl-circ" | "double-circle" | "start" | "stop" => Shape::Round,
        _ => Shape::Box,
    }
}

/// A Mermaid flowchart as a graph. Besides Mermaid's syntax, two comment
/// lines in `text` are read: `%% highlight: a, b, c` and `%% stack: a b c`.
///
/// # Errors
///
/// What drawtool can't draw yet: a reversed direction (`RL`, `BT`) or an
/// edge to a subgraph. Also a `%% highlight:` or `%% stack:` line that names
/// a missing node.
pub(crate) fn flowchart(m: &FlowchartModel, text: &str) -> Result<Graph, Error> {
    let direction = match m.direction.as_deref().unwrap_or("TB") {
        "LR" => Direction::LR,
        "TB" | "TD" => Direction::TD,
        d => return Err(Error::new(format!("`{d}` isn't supported yet: use LR or TD"))),
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
pub(crate) fn state(m: &StateDiagramRenderModel, text: &str) -> Result<Graph, Error> {
    let direction = if m.direction == "LR" { Direction::LR } else { Direction::TD };
    let (mut g, mut groups, mut index) = (DiGraph::new(), Vec::new(), HashMap::new());
    let mut group_of = HashMap::new();
    for n in m.nodes.iter().filter(|n| n.shape != "noteGroup") {
        let text = n.label.as_ref().and_then(|l| l.as_str()).unwrap_or(&n.id);
        let (lines, shape) = match n.shape.as_str() {
            "stateStart" => (vec!["●".to_string()], Shape::Round),
            "stateEnd" => (vec!["◉".to_string()], Shape::Round),
            "choice" => (vec![String::new()], Shape::Decision),
            _ => {
                let mut lines = label(text, false);
                lines.extend(n.description.iter().flatten().flat_map(|d| label(d, false)));
                (lines, if n.shape == "note" { Shape::Box } else { Shape::Round })
            }
        };
        if n.is_group {
            group_of.insert(n.id.as_str(), groups.len());
            groups.push(Group { title: lines.join(" ") });
        }
        index.insert(n.id.as_str(), g.add_node(NodeDef::new(&n.id, lines, shape)));
    }
    // A composite state's own box goes in its frame, with the states inside it.
    for n in &m.nodes {
        let frame = if n.is_group { Some(n.id.as_str()) } else { n.parent_id.as_deref() };
        if let (Some(&v), Some(&grp)) = (index.get(n.id.as_str()), frame.and_then(|p| group_of.get(p))) {
            g[v].group = Some(grp);
        }
    }
    for e in m.edges.iter().filter(|e| index.contains_key(e.start.as_str()) && index.contains_key(e.end.as_str())) {
        let note = e.arrow_type_end.is_empty();
        let label = Some(label(&e.label, false)).filter(|l| l.iter().any(|s| !s.is_empty()));
        let (style, arrow) = if note { (Style::Dotted, Arrow::None) } else { (Style::Solid, Arrow::End) };
        g.add_edge(index[e.start.as_str()], index[e.end.as_str()], EdgeDef { label, style, arrow });
    }
    let mut graph = Graph { direction, g, groups, stacks: Vec::new() };
    directives(text, &mut graph)?;
    Ok(graph)
}

fn convert(m: &FlowchartModel) -> Result<(DiGraph<NodeDef, EdgeDef>, Vec<Group>), Error> {
    let sub_ids: HashSet<&str> = m.subgraphs.iter().map(|s| s.id.as_str()).collect();
    let mut g = DiGraph::new();
    let mut index = HashMap::new();
    for n in &m.nodes {
        if sub_ids.contains(n.id.as_str()) {
            return Err(Error::new(format!("`{}` is a subgraph: edges to subgraphs aren't supported yet", n.id)));
        }
        let lines = label(n.label.as_deref().unwrap_or(&n.id), n.label_type.as_deref() == Some("markdown"));
        let node = NodeDef::new(&n.id, lines, shape(n.layout_shape.as_deref().unwrap_or_default()));
        index.insert(n.id.as_str(), g.add_node(node));
    }
    // Mermaid lists subgraphs innermost first, so the first to list a node owns it.
    for (i, s) in m.subgraphs.iter().enumerate() {
        for id in &s.nodes {
            if let Some(&v) = index.get(id.as_str()) {
                g[v].group.get_or_insert(i);
            }
        }
    }
    let groups = m.subgraphs.iter().map(|s| Group { title: label(&s.title, false).join(" ") }).collect();
    for e in m.edges.iter().filter(|e| e.visibility == FlowEdgeVisibility::Visible) {
        let style = match e.stroke_kind {
            FlowEdgeStroke::Normal => Style::Solid,
            FlowEdgeStroke::Dotted => Style::Dotted,
            FlowEdgeStroke::Thick => Style::Thick,
        };
        let arrow = match (e.start_marker, e.end_marker) {
            (FlowEdgeMarker::None, FlowEdgeMarker::None) => Arrow::None,
            (FlowEdgeMarker::None, _) => Arrow::End,
            _ => Arrow::Both,
        };
        let text = e.label.as_deref().filter(|l| !l.trim().is_empty());
        let label = text.map(|l| label(l, e.label_type.as_deref() == Some("markdown")));
        g.add_edge(index[e.from.as_str()], index[e.to.as_str()], EdgeDef { label, style, arrow });
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
