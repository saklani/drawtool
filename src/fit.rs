//! Fit a flowchart into a column budget without changing its direction. LR
//! first wraps long labels, then breaks into stacked bands: a single edge
//! crossing a break wraps around like text, several are joined by numbered
//! connectors (①). TD only ever wraps labels.

use crate::layout::{node_layers, render};
use crate::model::{Direction, EdgeDef, Graph, NodeDef, Shape, Style};
use crate::text::{first_fit, wrap, Fit};
use std::collections::{BTreeSet, HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Label wrap widths tried in order; `None` leaves labels as written.
const WRAPS: [Option<usize>; 4] = [None, Some(24), Some(16), Some(12)];

pub fn text_width(text: &str) -> usize {
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

/// Copy of `graph` with every node and edge label wrapped at `max` columns.
#[must_use]
pub fn wrap_labels(graph: &Graph, max: usize) -> Graph {
    let mut g = graph.clone();
    let wrap_all = |lines: &mut Vec<String>| *lines = lines.iter().flat_map(|l| wrap(l, max, false)).collect();
    g.nodes.iter_mut().for_each(|n| wrap_all(&mut n.lines));
    g.edges.iter_mut().filter_map(|e| e.label.as_mut()).for_each(wrap_all);
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

/// Connector number for every node that an edge from another band enters.
fn connector_numbers(graph: &Graph, band_of: &[usize]) -> HashMap<usize, usize> {
    let mut number = HashMap::new();
    for e in graph.edges.iter().filter(|e| band_of[e.src] != band_of[e.dst]) {
        let next = number.len() + 1;
        number.entry(e.dst).or_insert(next);
    }
    number
}

/// The sub-graph for band `b`: its own nodes and edges, plus a numbered
/// connector wherever an edge leaves or enters the band.
fn band_graph(graph: &Graph, band_of: &[usize], b: usize, ascii: bool) -> Graph {
    let number = connector_numbers(graph, band_of);
    let (mut nodes, mut edges, mut connectors) = (Vec::new(), Vec::new(), HashMap::new());
    let mut connector = |nodes: &mut Vec<NodeDef>, n: usize| {
        *connectors.entry(n).or_insert_with(|| {
            nodes.push(NodeDef::new(&format!("\0c{n}"), vec![connector_label(n, ascii)], Shape::Round));
            nodes.len() - 1
        })
    };
    let mut entered = HashSet::new();
    for e in graph.edges.iter().filter(|e| band_of[e.dst] == b && band_of[e.src] != b) {
        if entered.insert(e.dst) {
            let c = connector(&mut nodes, number[&e.dst]);
            edges.push(EdgeDef { src: c, dst: e.dst, label: None, style: Style::Solid, arrow: true });
        }
    }
    let mut index = HashMap::new();
    for (i, n) in graph.nodes.iter().enumerate().filter(|(i, _)| band_of[*i] == b) {
        index.insert(i, nodes.len());
        nodes.push(n.clone());
    }
    for e in &mut edges {
        e.dst = index[&e.dst];
    }
    for e in graph.edges.iter().filter(|e| band_of[e.src] == b) {
        let dst = if band_of[e.dst] == b { index[&e.dst] } else { connector(&mut nodes, number[&e.dst]) };
        edges.push(EdgeDef { src: index[&e.src], dst, ..e.clone() });
    }
    Graph { direction: Direction::LR, nodes, edges, groups: graph.groups.clone() }
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
        (text_width(&text) <= max_width).then_some(text)
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
        let crossing = graph.edges.iter().filter(|e| (at[e.src] < p) != (at[e.dst] < p)).count();
        let has = |g, before| graph.nodes.iter().zip(&at).any(|(n, &a)| n.group == Some(g) && (a < p) == before);
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
    let parts = (0..cuts.len()).map(|b| fits(&cuts, b)).collect::<Option<Vec<_>>>()?;
    let width = parts.iter().map(|p| text_width(p)).max().unwrap_or(0);
    Some(Fit { text: join_bands(graph, &band_of(&cuts), &parts, ascii), width, fits: true })
}

/// Stack rendered bands, joining each break by a wrap-around line when
/// exactly one edge crosses it and by a blank line (keeping connectors) else.
fn join_bands(graph: &Graph, band_of: &[usize], parts: &[String], ascii: bool) -> String {
    let numbers = connector_numbers(graph, band_of);
    let nb = parts.len();
    let (mut outs, mut ins) = (vec![HashSet::new(); nb], vec![HashSet::new(); nb]);
    for e in graph.edges.iter().filter(|e| band_of[e.src] != band_of[e.dst]) {
        outs[band_of[e.src]].insert(numbers[&e.dst]);
        ins[band_of[e.dst]].insert(numbers[&e.dst]);
    }
    let mut grids: Vec<Grid> = parts.iter().map(|p| p.lines().map(cells).collect()).collect();
    let mut out: Grid = Vec::new();
    for b in 0..nb {
        if b > 0 {
            let only = outs[b - 1].iter().next().copied().filter(|_| outs[b - 1].len() == 1 && ins[b] == outs[b - 1]);
            let single = only
                .filter(|k| (0..nb).all(|o| (o == b - 1 || !outs[o].contains(k)) && (o == b || !ins[o].contains(k))));
            let joint = single.and_then(|k| wrap_around(&mut out, &mut grids[b], &connector_label(k, ascii), ascii));
            out.push(joint.unwrap_or_default());
        }
        out.append(&mut grids[b]);
    }
    out.iter()
        .map(|r| r.iter().filter(|&&c| c != WIDE).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rows of display cells; WIDE marks the right half of a double-width char.
type Grid = Vec<Vec<char>>;
const WIDE: char = '\0';

fn cells(line: &str) -> Vec<char> {
    line.chars().flat_map(|ch| if ch.width() == Some(2) { vec![ch, WIDE] } else { vec![ch] }).collect()
}

fn cell(grid: &Grid, r: usize, c: usize) -> char {
    grid.get(r).and_then(|row| row.get(c)).copied().unwrap_or(' ')
}

fn set(grid: &mut Grid, r: usize, cols: impl IntoIterator<Item = usize>, ch: char) {
    for c in cols {
        if grid[r].len() <= c {
            grid[r].resize(c + 1, ' ');
        }
        grid[r][c] = ch;
    }
}

/// Replace the out-connector at the end of `upper` and the in-connector at
/// the start of `lower` with one line that turns down, runs back left along a
/// new row, and enters `lower`. Returns that new row, or `None` (leaving both
/// grids untouched) if anything is in the way.
fn wrap_around(upper: &mut Grid, lower: &mut Grid, label: &str, ascii: bool) -> Option<Vec<char>> {
    let label: Vec<char> = label.chars().collect();
    let find = |g: &Grid| {
        g.iter().enumerate().find_map(|(r, row)| row.windows(label.len()).position(|w| w == label).map(|c| (r, c)))
    };
    let ((r, c), (r2, c2), lw) = (find(upper)?, find(lower)?, label.len());
    let corner = if ascii { '.' } else { '╭' };
    let blocked = (r + 2..upper.len()).any(|row| cell(upper, row, c) != ' ')
        || (0..r2.saturating_sub(1)).any(|row| cell(lower, row, 0) != ' ');
    let anchored = r > 0 && r2 > 0 && c >= 3 && c2 == 2;
    if !anchored || cell(upper, r - 1, c - 2) != corner || cell(lower, r2 - 1, 0) != corner || blocked {
        return None;
    }
    let [h, v, down_left, up_left, up_right, down_right] =
        if ascii { ['-', '|', '+', '+', '+', '+'] } else { ['─', '│', '┐', '┘', '└', '┌'] };
    (r - 1..=r + 1).for_each(|row| set(upper, row, c - 2..=c + lw + 1, ' '));
    set(upper, r, c - 3..c, h);
    set(upper, r, [c], down_left);
    (r + 1..upper.len()).for_each(|row| set(upper, row, [c], v));
    (r2 - 1..=r2 + 1).for_each(|row| set(lower, row, 0..=c2 + lw + 1, ' '));
    (0..r2).for_each(|row| set(lower, row, [0], v));
    set(lower, r2, [0], up_right);
    set(lower, r2, 1..=c2 + lw + 1, h);
    // The connector's columns are now just line; drop them if nothing else uses them.
    let gap = 1..=c2 + lw + 1;
    if (0..lower.len()).all(|row| row == r2 || gap.clone().all(|col| cell(lower, row, col) == ' ')) {
        for row in lower.iter_mut() {
            let end = (gap.end() + 1).min(row.len());
            if end > 1 {
                row.drain(1..end);
            }
        }
    }
    let mut joint = vec![h; c + 1];
    (joint[0], joint[c]) = (down_right, up_left);
    Some(joint)
}
