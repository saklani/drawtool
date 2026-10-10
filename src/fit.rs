//! Fit a diagram into a column budget without changing its direction. LR
//! first wraps long labels, then breaks into stacked bands joined by
//! numbered connectors (①). TD only ever wraps labels.

use crate::layout::{node_layers, render};
use crate::parse::{Direction, EdgeDef, Graph, NodeDef, Shape, Style};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

/// Label wrap widths tried in order; `None` leaves labels as written.
const WRAPS: [Option<usize>; 4] = [None, Some(24), Some(16), Some(12)];

pub struct Fitted {
    pub text: String,
    pub direction: Direction,
    pub width: usize,
    pub fits: bool,
    /// Number of stacked LR bands (1 = not broken up).
    pub bands: usize,
}

pub fn text_width(text: &str) -> usize {
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

/// Greedy word wrap. Words longer than `max` stay whole.
fn wrap_line(line: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        if !cur.is_empty() && cur.width() + 1 + word.width() > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn wrap_lines(lines: &[String], max: usize) -> Vec<String> {
    lines.iter().flat_map(|l| wrap_line(l, max)).collect()
}

/// Copy of `graph` with every node and edge label wrapped at `max` columns.
pub fn wrap_labels(graph: &Graph, max: usize) -> Graph {
    let mut g = graph.clone();
    for n in &mut g.nodes {
        n.lines = wrap_lines(&n.lines, max);
    }
    for e in &mut g.edges {
        if let Some(label) = &mut e.label {
            *label = wrap_lines(label, max);
        }
    }
    g
}

/// Render within `max_width` columns (0 = unlimited), keeping `direction`.
/// If nothing fits, returns the narrowest single-band attempt.
pub fn render_fit(graph: &Graph, direction: Direction, ascii: bool, max_width: usize) -> Fitted {
    let first = fit_labels(graph, direction, ascii, max_width);
    if first.fits || direction == Direction::TD {
        return first;
    }
    for wrap in WRAPS {
        let g = match wrap {
            None => graph.clone(),
            Some(w) => wrap_labels(graph, w),
        };
        if let Some(banded) = fit_bands(&g, ascii, max_width) {
            return banded;
        }
    }
    first
}

/// One direction, progressively tighter label wrapping.
fn fit_labels(graph: &Graph, direction: Direction, ascii: bool, max_width: usize) -> Fitted {
    let mut best: Option<Fitted> = None;
    let mut previous: Option<Graph> = None;
    for wrap in WRAPS {
        let g = match wrap {
            None => graph.clone(),
            Some(w) => wrap_labels(graph, w),
        };
        if previous.as_ref().is_some_and(|p| same_labels(p, &g)) {
            continue;
        }
        let text = render(&g, Some(direction), ascii);
        let width = text_width(&text);
        if max_width == 0 || width <= max_width {
            return Fitted { text, direction, width, fits: true, bands: 1 };
        }
        if best.as_ref().map_or(true, |b| width < b.width) {
            best = Some(Fitted { text, direction, width, fits: false, bands: 1 });
        }
        previous = Some(g);
    }
    best.expect("at least one render attempt")
}

fn connector_label(n: usize, ascii: bool) -> String {
    match char::from_u32(0x2460 + n as u32 - 1) {
        Some(c) if !ascii && n <= 20 => c.to_string(),
        _ => format!("({n})"),
    }
}

/// The sub-graph for band `b`: its own nodes and edges, plus a numbered
/// connector wherever an edge leaves or enters the band. Edges into the same
/// node share one connector number.
fn band_graph(graph: &Graph, band_of: &[usize], b: usize, ascii: bool) -> Graph {
    let mut number: HashMap<usize, usize> = HashMap::new();
    for e in &graph.edges {
        if band_of[e.src] != band_of[e.dst] && !number.contains_key(&e.dst) {
            number.insert(e.dst, number.len() + 1);
        }
    }
    let mut nodes = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut connectors: HashMap<usize, usize> = HashMap::new();
    let mut connector = |nodes: &mut Vec<NodeDef>, n: usize| {
        *connectors.entry(n).or_insert_with(|| {
            nodes.push(NodeDef::new(&format!("\0c{n}"), vec![connector_label(n, ascii)], Shape::Round));
            nodes.len() - 1
        })
    };
    let mut edges = Vec::new();
    let mut entered = std::collections::HashSet::new();
    for e in &graph.edges {
        if band_of[e.dst] == b && band_of[e.src] != b && entered.insert(e.dst) {
            let c = connector(&mut nodes, number[&e.dst]);
            edges.push(EdgeDef { src: c, dst: e.dst, label: None, style: Style::Solid, arrow: true });
        }
    }
    for (i, n) in graph.nodes.iter().enumerate() {
        if band_of[i] == b {
            index.insert(i, nodes.len());
            nodes.push(n.clone());
        }
    }
    for e in edges.iter_mut() {
        e.dst = index[&e.dst];
    }
    for e in &graph.edges {
        match (band_of[e.src] == b, band_of[e.dst] == b) {
            (true, true) => edges.push(EdgeDef { src: index[&e.src], dst: index[&e.dst], ..e.clone() }),
            (true, false) => {
                let c = connector(&mut nodes, number[&e.dst]);
                edges.push(EdgeDef { src: index[&e.src], dst: c, ..e.clone() });
            }
            _ => {}
        }
    }
    Graph { direction: Direction::LR, nodes, edges }
}

/// Greedily pack consecutive layers into LR bands that each fit, without
/// re-wrapping labels. `None` if a single layer is too wide on its own.
fn fit_bands(graph: &Graph, ascii: bool, max_width: usize) -> Option<Fitted> {
    let layers = node_layers(graph);
    let mut distinct = layers.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let band_of = |cuts: &[usize]| -> Vec<usize> {
        layers.iter().map(|l| cuts.iter().filter(|&&c| distinct[c] <= *l).count() - 1).collect()
    };
    let fits = |cuts: &[usize], b: usize| {
        let text = render(&band_graph(graph, &band_of(cuts), b, ascii), Some(Direction::LR), ascii);
        let width = text_width(&text);
        (width <= max_width).then_some(Fitted { text, direction: Direction::LR, width, fits: true, bands: 1 })
    };
    let mut cuts = vec![0];
    for i in 1..distinct.len() {
        // Try extending the current band through layer i.
        let b = cuts.len() - 1;
        let mut trial = cuts.clone();
        trial.push(i + 1);
        if i + 1 < distinct.len() && fits(&trial, b).is_some() || i + 1 == distinct.len() && fits(&cuts, b).is_some() {
            continue;
        }
        cuts.push(i);
    }
    if cuts.len() < 2 {
        return None;
    }
    let mut parts = Vec::new();
    for b in 0..cuts.len() {
        parts.push(fits(&cuts, b)?);
    }
    let width = parts.iter().map(|p| p.width).max().unwrap_or(0);
    let text = parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n");
    Some(Fitted { text, direction: Direction::LR, width, fits: true, bands: parts.len() })
}

fn same_labels(a: &Graph, b: &Graph) -> bool {
    a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.lines == y.lines)
        && a.edges.iter().zip(&b.edges).all(|(x, y)| x.label == y.label)
}
