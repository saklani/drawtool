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

/// Connector number for every node that an edge from another band enters.
fn connector_numbers(graph: &Graph, band_of: &[usize]) -> HashMap<usize, usize> {
    let mut number = HashMap::new();
    for e in &graph.edges {
        if band_of[e.src] != band_of[e.dst] && !number.contains_key(&e.dst) {
            number.insert(e.dst, number.len() + 1);
        }
    }
    number
}

/// The sub-graph for band `b`: its own nodes and edges, plus a numbered
/// connector wherever an edge leaves or enters the band. Edges into the same
/// node share one connector number.
fn band_graph(graph: &Graph, band_of: &[usize], b: usize, ascii: bool) -> Graph {
    let number = connector_numbers(graph, band_of);
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
    Graph { direction: Direction::LR, nodes, edges, groups: graph.groups.clone() }
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
    let bands = parts.len();
    let text = join_bands(graph, &band_of(&cuts), parts, ascii);
    Some(Fitted { text, direction: Direction::LR, width, fits: true, bands })
}

/// Stack rendered bands. Where exactly one edge crosses a break, draw it as a
/// line that wraps around to the next band; otherwise keep the numbered
/// connectors and separate the bands with a blank line.
fn join_bands(graph: &Graph, band_of: &[usize], parts: Vec<Fitted>, ascii: bool) -> String {
    let numbers = connector_numbers(graph, band_of);
    let nb = parts.len();
    let (mut outs, mut ins) = (vec![Vec::new(); nb], vec![Vec::new(); nb]);
    for e in &graph.edges {
        let (from, to) = (band_of[e.src], band_of[e.dst]);
        if from != to {
            let k = numbers[&e.dst];
            if !outs[from].contains(&k) {
                outs[from].push(k);
            }
            if !ins[to].contains(&k) {
                ins[to].push(k);
            }
        }
    }
    let mut grids: Vec<Vec<Vec<char>>> = parts.iter().map(|p| to_grid(&p.text)).collect();
    let mut out: Vec<Vec<char>> = Vec::new();
    for b in 0..nb {
        if b > 0 {
            let k = outs[b - 1].first().copied();
            let single = outs[b - 1].len() == 1
                && ins[b] == outs[b - 1]
                && (0..nb).all(|o| o == b - 1 || !outs[o].contains(&k.unwrap()))
                && (0..nb).all(|o| o == b || !ins[o].contains(&k.unwrap()));
            let label = connector_label(k.unwrap_or(0).max(1), ascii);
            let joint = if single { wrap_around(&mut out, &mut grids[b], &label, ascii) } else { None };
            out.push(joint.unwrap_or_default());
        }
        out.append(&mut grids[b]);
    }
    from_grid(&out)
}

/// Rows of display cells; WIDE marks the right half of a double-width char.
const WIDE: char = '\0';

fn to_grid(text: &str) -> Vec<Vec<char>> {
    text.lines()
        .map(|l| {
            let mut row = Vec::new();
            for ch in l.chars() {
                row.push(ch);
                if unicode_width::UnicodeWidthChar::width(ch) == Some(2) {
                    row.push(WIDE);
                }
            }
            row
        })
        .collect()
}

fn from_grid(grid: &[Vec<char>]) -> String {
    grid.iter()
        .map(|r| r.iter().filter(|&&c| c != WIDE).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_label(grid: &[Vec<char>], label: &[char]) -> Option<(usize, usize)> {
    grid.iter().enumerate().find_map(|(r, row)| {
        row.windows(label.len()).position(|w| w == label).map(|c| (r, c))
    })
}

fn cell(grid: &[Vec<char>], r: usize, c: usize) -> char {
    grid.get(r).and_then(|row| row.get(c)).copied().unwrap_or(' ')
}

fn set(grid: &mut [Vec<char>], r: usize, c: usize, ch: char) {
    let row = &mut grid[r];
    if row.len() <= c {
        row.resize(c + 1, ' ');
    }
    row[c] = ch;
}

/// Replace the out-connector at the end of `upper` and the in-connector at
/// the start of `lower` with one line that turns down, runs back left along a
/// new row, and enters `lower`. Returns that new row, or `None` (leaving both
/// grids untouched) if anything is in the way.
fn wrap_around(upper: &mut [Vec<char>], lower: &mut [Vec<char>], label: &str, ascii: bool) -> Option<Vec<char>> {
    let label: Vec<char> = label.chars().collect();
    let lw = label.len();
    let (r, c) = find_label(upper, &label)?;
    let (r2, c2) = find_label(lower, &label)?;
    let corner = if ascii { '.' } else { '╭' };
    if r == 0 || r2 == 0 || c < 3 || c2 != 2 || cell(upper, r - 1, c - 2) != corner || cell(lower, r2 - 1, 0) != corner {
        return None;
    }
    let t = c; // column where the line turns down
    if (r + 2..upper.len()).any(|row| cell(upper, row, t) != ' ') || (0..r2 - 1).any(|row| cell(lower, row, 0) != ' ') {
        return None;
    }
    let [h, v, down_left, up_left, up_right, down_right] =
        if ascii { ['-', '|', '+', '+', '+', '+'] } else { ['─', '│', '┐', '┘', '└', '┌'] };
    for row in r - 1..=r + 1 {
        for col in c - 2..=c + lw + 1 {
            set(upper, row, col, ' ');
        }
    }
    for col in c - 3..t {
        set(upper, r, col, h);
    }
    set(upper, r, t, down_left);
    for row in r + 1..upper.len() {
        set(upper, row, t, v);
    }
    for row in r2 - 1..=r2 + 1 {
        for col in 0..=c2 + lw + 1 {
            set(lower, row, col, ' ');
        }
    }
    for row in 0..r2 {
        set(lower, row, 0, v);
    }
    set(lower, r2, 0, up_right);
    for col in 1..=c2 + lw + 1 {
        set(lower, r2, col, h);
    }
    // The connector's columns are now just line; drop them if nothing else uses them.
    let gap = 1..=c2 + lw + 1;
    if (0..lower.len()).all(|row| row == r2 || gap.clone().all(|col| cell(lower, row, col) == ' ')) {
        for row in lower.iter_mut() {
            let end = (*gap.end() + 1).min(row.len());
            if end > 1 {
                row.drain(1..end);
            }
        }
    }
    let mut joint = vec![h; t + 1];
    joint[0] = down_right;
    joint[t] = up_left;
    Some(joint)
}

fn same_labels(a: &Graph, b: &Graph) -> bool {
    a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.lines == y.lines)
        && a.edges.iter().zip(&b.edges).all(|(x, y)| x.label == y.label)
}
