//! Fit a flowchart into a column budget without changing its direction. LR
//! first wraps long labels, then breaks into stacked bands: a single edge
//! crossing a break wraps around like text, several are joined by numbered
//! connectors (①). TD only ever wraps labels.

use crate::layout::{node_layers, render, scene, Scene};
use crate::model::{Direction, EdgeDef, Graph, NodeDef, Shape, Style};
use crate::paint::{Layer, Painter, Pt, Stroke};
use crate::text::{block_width, first_fit, wrap, Fit};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Label wrap widths tried in order; `None` leaves labels as written.
const WRAPS: [Option<usize>; 4] = [None, Some(24), Some(16), Some(12)];

/// Copy of `graph` with every node and edge label wrapped at `max` columns.
#[must_use]
pub fn wrap_labels(graph: &Graph, max: usize) -> Graph {
    let mut g = graph.clone();
    let wrap_all = |lines: &mut Vec<String>| *lines = lines.iter().flat_map(|l| wrap(l, max, false)).collect();
    g.g.node_weights_mut().for_each(|n| wrap_all(&mut n.lines));
    g.g.edge_weights_mut().filter_map(|e| e.label.as_mut()).for_each(wrap_all);
    g
}

fn wrapped(graph: &Graph, wrap: Option<usize>) -> Graph {
    wrap.map_or_else(|| graph.clone(), |w| wrap_labels(graph, w))
}

/// Render within `max_width` columns (0 = unlimited), keeping `direction`.
/// If nothing fits, returns the narrowest single-band attempt.
#[must_use]
pub fn render_fit(graph: &Graph, direction: Direction, ascii: bool, max_width: usize) -> Fit {
    let first = first_fit(max_width, WRAPS.iter().map(|&w| render(&wrapped(graph, w), Some(direction), ascii)));
    if first.fits || direction == Direction::TD {
        return first;
    }
    WRAPS.iter().find_map(|&w| fit_bands(&wrapped(graph, w), ascii, max_width)).unwrap_or(first)
}

fn connector_label(n: usize, ascii: bool) -> String {
    match char::from_u32(0x2460 + n as u32 - 1) {
        Some(c) if !ascii && n <= 20 => c.to_string(),
        _ => format!("({n})"),
    }
}

/// The edges between different bands, as (source, target) indices.
fn crossing(graph: &Graph, band_of: &[usize]) -> Vec<(usize, usize)> {
    let ends = graph.g.edge_references().map(|e| (e.source().index(), e.target().index()));
    ends.filter(|&(s, t)| band_of[s] != band_of[t]).collect()
}

/// Connector number for every node that an edge from another band enters.
fn connector_numbers(graph: &Graph, band_of: &[usize]) -> HashMap<usize, usize> {
    let mut number = HashMap::new();
    for (_, t) in crossing(graph, band_of) {
        let next = number.len() + 1;
        number.entry(t).or_insert(next);
    }
    number
}

/// The sub-graph for band `b`: its own nodes and edges, plus a numbered
/// connector wherever an edge leaves or enters the band.
fn band_graph(graph: &Graph, band_of: &[usize], b: usize, ascii: bool) -> Graph {
    let number = connector_numbers(graph, band_of);
    let (mut g, mut connectors) = (DiGraph::new(), HashMap::new());
    let mut connector = |g: &mut DiGraph<NodeDef, EdgeDef>, n: usize| {
        *connectors.entry(n).or_insert_with(|| {
            g.add_node(NodeDef::new(&format!("\0c{n}"), vec![connector_label(n, ascii)], Shape::Round))
        })
    };
    // In-connectors come first, one per node entered from another band.
    let mut entered = Vec::new();
    for (_, t) in crossing(graph, band_of).into_iter().filter(|&(_, t)| band_of[t] == b) {
        if !entered.iter().any(|&(_, u)| u == t) {
            entered.push((connector(&mut g, number[&t]), t));
        }
    }
    let mut index = HashMap::new();
    for v in graph.g.node_indices().filter(|v| band_of[v.index()] == b) {
        index.insert(v.index(), g.add_node(graph.g[v].clone()));
    }
    for (c, t) in entered {
        g.add_edge(c, index[&t], EdgeDef { label: None, style: Style::Solid, arrow: true });
    }
    for e in graph.g.edge_references().filter(|e| band_of[e.source().index()] == b) {
        let t = e.target().index();
        let dst = if band_of[t] == b { index[&t] } else { connector(&mut g, number[&t]) };
        g.add_edge(index[&e.source().index()], dst, e.weight().clone());
    }
    Graph { direction: Direction::LR, g, groups: graph.groups.clone() }
}

/// Split the layers into consecutive LR bands that each fit, choosing the
/// cuts by dynamic programming. `None` if a single layer is too wide on its own.
fn fit_bands(graph: &Graph, ascii: bool, max_width: usize) -> Option<Fit> {
    let layers = node_layers(graph);
    let distinct: Vec<usize> = layers.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let band_of = |cuts: &[usize]| -> Vec<usize> {
        layers.iter().map(|l| cuts.iter().filter(|&&c| distinct[c] <= *l).count() - 1).collect()
    };
    let fits = |cuts: &[usize], b: usize| {
        let text = render(&band_graph(graph, &band_of(cuts), b, ascii), Some(Direction::LR), ascii);
        (block_width(&text) as usize <= max_width).then_some(text)
    };
    // Height of band [i, j) of the distinct layers, if it fits on its own.
    let n = distinct.len();
    let mut memo: HashMap<(usize, usize), Option<usize>> = HashMap::new();
    let mut band_height = |i: usize, j: usize| {
        let cuts: Vec<usize> = [0].into_iter().chain((i > 0).then_some(i)).chain((j < n).then_some(j)).collect();
        *memo.entry((i, j)).or_insert_with(|| fits(&cuts, usize::from(i > 0)).map(|t| t.lines().count()))
    };
    // A cut before layer p costs one per edge crossing it (so one edge wraps
    // around like text) and three per subgraph it splits.
    let at: Vec<usize> = layers.iter().map(|l| distinct.binary_search(l).unwrap_or(0)).collect();
    let cut_cost = |p: usize| {
        let before = |v: NodeIndex| at[v.index()] < p;
        let crossing = graph.g.edge_references().filter(|e| before(e.source()) != before(e.target())).count();
        let has = |g, side| graph.g.node_weights().zip(&at).any(|(n, &a)| n.group == Some(g) && (a < p) == side);
        let split = (0..graph.groups.len()).filter(|&g| has(g, true) && has(g, false)).count();
        crossing + 3 * split
    };
    // Fewest bands, then least height, then the cheapest cuts. best[j] is the
    // best way to lay out layers [0, j): (bands, height, cut cost, start of last band).
    let mut best: Vec<Option<(usize, usize, usize, usize)>> = vec![None; n + 1];
    best[0] = Some((0, 0, 0, 0));
    for j in 1..=n {
        for i in (0..j).rev() {
            let Some((bands, height, cost, _)) = best[i] else { continue };
            let Some(h) = band_height(i, j) else { break };
            let cand = (bands + 1, height + h, cost + if i > 0 { cut_cost(i) } else { 0 }, i);
            if best[j].is_none_or(|b| (cand.0, cand.1, cand.2) < (b.0, b.1, b.2)) {
                best[j] = Some(cand);
            }
        }
    }
    let (mut cuts, mut j) = (vec![], n);
    while j > 0 {
        j = best[j]?.3;
        cuts.push(j);
    }
    cuts.reverse();
    if cuts.len() < 2 {
        return None;
    }
    let band_of = band_of(&cuts);
    let graphs: Vec<Graph> = (0..cuts.len()).map(|b| band_graph(graph, &band_of, b, ascii)).collect();
    let scenes = graphs.iter().map(|g| scene(g, Some(Direction::LR), ascii)).collect::<Option<Vec<_>>>()?;
    let width = scenes.iter().map(|s| block_width(&s.painter.finish()) as usize).max().unwrap_or(0);
    let text = join_bands(graph, &band_of, &graphs, scenes, ascii);
    (width <= max_width).then_some(Fit { text, width, fits: true })
}

/// Stack the bands, joining each break by a wrap-around line when exactly
/// one edge crosses it and by a blank row (keeping connectors) else.
fn join_bands(graph: &Graph, band_of: &[usize], graphs: &[Graph], scenes: Vec<Scene>, ascii: bool) -> String {
    let numbers = connector_numbers(graph, band_of);
    let nb = graphs.len();
    let (mut outs, mut ins) = (vec![HashSet::new(); nb], vec![HashSet::new(); nb]);
    for (s, t) in crossing(graph, band_of) {
        outs[band_of[s]].insert(numbers[&t]);
        ins[band_of[t]].insert(numbers[&t]);
    }
    let connector = |b: usize, k: usize| graphs[b].g.node_weights().position(|n| n.id == format!("\0c{k}"));
    let mut canvas = Painter::new(ascii);
    canvas.mark_crossings = true;
    // The next free row, and the previous band's boxes where they sit on the canvas.
    let (mut top, mut above): (i64, Vec<(Pt, Pt)>) = (0, vec![]);
    for (b, scene) in scenes.into_iter().enumerate() {
        let Some(((x0, y0), (_, y1))) = scene.painter.bounds() else { continue };
        // The band on its own, with its top-left at (0, 0).
        let mut band = Scene { painter: Painter::new(ascii), boxes: scene.boxes };
        band.painter.paste(&scene.painter, (-x0, -y0));
        band.boxes.iter_mut().for_each(|((x, y), _)| (*x, *y) = (*x - x0, *y - y0));
        let mut route = None;
        if b > 0 {
            let only = outs[b - 1].iter().next().copied().filter(|_| outs[b - 1].len() == 1 && ins[b] == outs[b - 1]);
            let single = only
                .filter(|k| (0..nb).all(|o| (o == b - 1 || !outs[o].contains(k)) && (o == b || !ins[o].contains(k))));
            route = single.and_then(|k| {
                let (from, to) = (connector(b - 1, k)?, connector(b, k)?);
                wrap_around(&mut canvas, above[from], &mut band, to, top)
            });
            top += 1;
        }
        canvas.paste(&band.painter, (0, top));
        if let Some(pts) = route {
            let net = canvas.net();
            canvas.line(&pts, net, Stroke::LIGHT, Layer::Main);
        }
        above = band.boxes.iter().map(|&((x, y), size)| ((x, y + top), size)).collect();
        top += y1 - y0 + 1;
    }
    canvas.finish()
}

/// Make way for one line in place of the out-connector `upper` (a box on
/// `canvas`) and the in-connector `lower.boxes[k]` at the start of the next
/// band: out of `upper`, down to the row `joint` below everything drawn,
/// back left along it, and down into `lower`, which goes under that row.
/// Erases both connectors and returns the line's points on the canvas, or
/// `None` (changing nothing) if anything is in the way.
fn wrap_around(canvas: &mut Painter, upper: (Pt, Pt), lower: &mut Scene, k: usize, joint: i64) -> Option<Vec<Pt>> {
    let (((x, y), (w, h)), ((lx, ly), (lw, lh))) = (upper, lower.boxes[k]);
    // Down from the upper label's first column; into the lower one on its label's row.
    let (r, c, r2) = (y + 1, x + 2, ly + 1);
    let blocked = (y + h..joint).any(|row| canvas.cell((c, row)) != ' ')
        || (0..ly).any(|row| lower.painter.cell((0, row)) != ' ');
    // A taller connector has edges of several styles entering it on their own ports.
    if (h, lh) != (3, 3) || x < 1 || lx != 0 || blocked {
        return None;
    }
    // Both connectors, and the arrowhead into the upper one.
    canvas.erase((x, y), (x + w - 1, y + h - 1));
    canvas.erase((x - 1, r), (x - 1, r));
    lower.painter.erase((0, ly), (lw - 1, ly + lh - 1));
    // The lower connector's columns are now just line: drop them if nothing else uses them.
    let ((_, top), (_, bottom)) = lower.painter.bounds()?;
    let unused = (top..=bottom).all(|row| row == r2 || (1..lw).all(|col| lower.painter.cell((col, row)) == ' '));
    let end = if unused {
        lower.painter.remove_columns(1, lw - 1);
        lower.boxes.iter_mut().filter(|((x, _), _)| *x >= lw).for_each(|((x, _), _)| *x -= lw - 1);
        1
    } else {
        lw
    };
    let below = joint + 1;
    Some(vec![(x - 1, r), (c, r), (c, joint), (0, joint), (0, below + r2), (end, below + r2)])
}
