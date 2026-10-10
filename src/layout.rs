//! Layout and rendering: a simplified Sugiyama pipeline.
//!
//! break cycles -> layer -> dummy nodes -> crossing reduction -> placement
//! (isotonic regression) -> ports -> channel track routing -> character grid.

use crate::parse::{Direction, Graph, Shape, Style};
use std::collections::{HashMap, HashSet, VecDeque};
use unicode_width::UnicodeWidthChar;

const U: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const R: u8 = 8;
const VERT: u8 = U | D;
const HORZ: u8 = L | R;
/// Placeholder for the right half of a double-width character.
const WIDE_TAIL: char = '\0';

fn char_width(c: char) -> i64 {
    c.width().unwrap_or(1).max(1) as i64
}

fn text_width(s: &str) -> i64 {
    s.chars().map(char_width).sum()
}

struct Glyphs {
    ascii: bool,
}

impl Glyphs {
    fn junction(&self, bits: u8) -> char {
        if self.ascii {
            return '+';
        }
        match bits {
            b if b == D | R => '┌',
            b if b == D | L => '┐',
            b if b == U | R => '└',
            b if b == U | L => '┘',
            b if b == U | D | R => '├',
            b if b == U | D | L => '┤',
            b if b == D | L | R => '┬',
            b if b == U | L | R => '┴',
            _ => '┼',
        }
    }

    fn straight(&self, style: Style, vertical: bool) -> char {
        let (v, h) = match (self.ascii, style) {
            (false, Style::Solid) => ('│', '─'),
            (false, Style::Dotted) => ('┆', '┄'),
            (false, Style::Thick) => ('┃', '━'),
            (true, Style::Solid) => ('|', '-'),
            (true, Style::Dotted) => (':', '.'),
            (true, Style::Thick) => ('|', '='),
        };
        if vertical {
            v
        } else {
            h
        }
    }

    /// Corners tl, tr, bl, br, then horizontal and vertical.
    fn border(&self, shape: Shape) -> [char; 6] {
        match (self.ascii, shape) {
            (false, Shape::Box) => ['┌', '┐', '└', '┘', '─', '│'],
            (false, Shape::Round) => ['╭', '╮', '╰', '╯', '─', '│'],
            (false, Shape::Decision) => ['╔', '╗', '╚', '╝', '═', '║'],
            (true, Shape::Box) => ['+', '+', '+', '+', '-', '|'],
            (true, Shape::Round) => ['.', '.', '\'', '\'', '-', '|'],
            (true, Shape::Decision) => ['+', '+', '+', '+', '=', '|'],
        }
    }

    /// Tees where an edge leaves a border: top, bottom, left, right.
    fn tee(&self, shape: Shape) -> [char; 4] {
        match (self.ascii, shape) {
            (true, _) => ['+'; 4],
            (false, Shape::Decision) => ['╧', '╤', '╢', '╟'],
            (false, _) => ['┴', '┬', '┤', '├'],
        }
    }

    fn arrow(&self, forward: bool, td: bool) -> char {
        match (self.ascii, td, forward) {
            (false, true, true) => '▼',
            (false, true, false) => '▲',
            (false, false, true) => '►',
            (false, false, false) => '◄',
            (true, true, true) => 'v',
            (true, true, false) => '^',
            (true, false, true) => '>',
            (true, false, false) => '<',
        }
    }

    fn loop_mark(&self) -> &'static str {
        if self.ascii {
            " (loop)"
        } else {
            " ↻"
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Real,
    Label,
    Dummy,
}

struct LNode {
    lines: Vec<String>,
    shape: Shape,
    kind: Kind,
    pos: i64,   // cross-axis coordinate (left / top edge)
    csize: i64, // size across the flow
    lsize: i64, // size along the flow
}

impl LNode {
    fn new(lines: Vec<String>, shape: Shape, kind: Kind) -> Self {
        LNode { lines, shape, kind, pos: 0, csize: 0, lsize: 0 }
    }

    fn cen(&self) -> i64 {
        self.pos + (self.csize - 1).div_euclid(2)
    }
}

struct Edge {
    src: usize,
    dst: usize,
    label: Option<Vec<String>>,
    style: Style,
    arrow: bool,
    rev: bool,
    path: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Side {
    In,
    Out,
}

/// (node, side, group): group is `None` for the shared port, or the edge
/// index for a back edge that gets its own port.
type PortKey = (usize, Side, Option<usize>);

/// One edge segment between adjacent layers.
struct Seg {
    u: usize,
    v: usize,
    style: Style,
    arrow_end: bool,
    arrow_start: bool,
    sk: PortKey,
    tk: PortKey,
    cs: i64,
    ct: i64,
    lo: i64,
    hi: i64,
    track: i64,
}

/// Weighted isotonic (non-decreasing) regression.
fn pava(ys: &[f64], ws: &[f64]) -> Vec<f64> {
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for (&y, &w) in ys.iter().zip(ws) {
        blocks.push((y, w, 1));
        while blocks.len() > 1 && blocks[blocks.len() - 2].0 > blocks[blocks.len() - 1].0 {
            let (v2, w2, c2) = blocks.pop().unwrap();
            let (v1, w1, c1) = blocks.pop().unwrap();
            blocks.push(((v1 * w1 + v2 * w2) / (w1 + w2), w1 + w2, c1 + c2));
        }
    }
    blocks.iter().flat_map(|&(v, _, c)| std::iter::repeat(v).take(c)).collect()
}

fn break_cycles(n: usize, edges: &mut [Edge]) {
    let mut out = vec![Vec::new(); n];
    let mut indeg = vec![0; n];
    for (i, e) in edges.iter().enumerate() {
        out[e.src].push(i);
        indeg[e.dst] += 1;
    }
    let mut state = vec![0u8; n];
    let roots: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).chain(0..n).collect();
    for root in roots {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (u, idx) = *top;
            if idx == out[u].len() {
                state[u] = 2;
                stack.pop();
                continue;
            }
            top.1 += 1;
            let ei = out[u][idx];
            let v = edges[ei].dst;
            if state[v] == 1 {
                edges[ei].rev = true;
            } else if state[v] == 0 {
                state[v] = 1;
                stack.push((v, 0));
            }
        }
    }
}

fn crossings(layers: &[Vec<usize>], down: &[Vec<usize>]) -> usize {
    let mut total = 0;
    let mut rank = HashMap::new();
    for k in 0..layers.len().saturating_sub(1) {
        rank.clear();
        for (i, &v) in layers[k + 1].iter().enumerate() {
            rank.insert(v, i as i64);
        }
        let es: Vec<(i64, i64)> = layers[k]
            .iter()
            .enumerate()
            .flat_map(|(i, &u)| down[u].iter().map(move |v| (i as i64, *v)))
            .map(|(i, v)| (i, rank[&v]))
            .collect();
        for a in 0..es.len() {
            for b in a + 1..es.len() {
                if (es[a].0 - es[b].0) * (es[a].1 - es[b].1) < 0 {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Break cycles, turn edge labels into nodes, and assign each node a layer.
fn layer_graph(graph: &Graph, gl: &Glyphs) -> (Vec<LNode>, Vec<Edge>, Vec<Vec<usize>>, Vec<i64>) {
    let mut ln: Vec<LNode> =
        graph.nodes.iter().map(|n| LNode::new(n.lines.clone(), n.shape, Kind::Real)).collect();
    let nreal = ln.len();
    let mut looped = vec![false; nreal];
    for e in &graph.edges {
        if e.src == e.dst && !looped[e.src] {
            if let Some(last) = ln[e.src].lines.last_mut() {
                last.push_str(gl.loop_mark());
            }
            looped[e.src] = true;
        }
    }
    let mut edges: Vec<Edge> = graph
        .edges
        .iter()
        .filter(|e| e.src != e.dst)
        .map(|e| Edge {
            src: e.src,
            dst: e.dst,
            label: e.label.clone(),
            style: e.style,
            arrow: e.arrow,
            rev: false,
            path: Vec::new(),
        })
        .collect();
    break_cycles(nreal, &mut edges);

    // Expand labelled edges into chains through a label node.
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); nreal];
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); nreal];
    let mut chains = Vec::new();
    for e in &edges {
        let mut chain = if e.rev { vec![e.dst, e.src] } else { vec![e.src, e.dst] };
        if let Some(label) = &e.label {
            ln.push(LNode::new(label.clone(), Shape::Box, Kind::Label));
            succ.push(Vec::new());
            pred.push(Vec::new());
            chain.insert(1, ln.len() - 1);
        }
        for w in chain.windows(2) {
            succ[w[0]].push(w[1]);
            pred[w[1]].push(w[0]);
        }
        chains.push(chain);
    }

    // Layering: longest path, then pull sources down next to their children.
    let n1 = ln.len();
    let mut indeg: Vec<usize> = (0..n1).map(|v| pred[v].len()).collect();
    let mut queue: VecDeque<usize> = (0..n1).filter(|&v| indeg[v] == 0).collect();
    let mut topo = Vec::new();
    while let Some(v) = queue.pop_front() {
        topo.push(v);
        for &s in &succ[v] {
            indeg[s] -= 1;
            if indeg[s] == 0 {
                queue.push_back(s);
            }
        }
    }
    let mut layer = vec![0i64; n1];
    for &v in &topo {
        layer[v] = pred[v].iter().map(|&p| layer[p] + 1).max().unwrap_or(0);
    }
    for &v in topo.iter().rev() {
        if pred[v].is_empty() && !succ[v].is_empty() {
            layer[v] = succ[v].iter().map(|&s| layer[s]).min().unwrap() - 1;
        }
    }

    (ln, edges, chains, layer)
}

/// Layer index of every real node, as the layout would assign it. Labelled
/// edges take an extra layer for the label.
pub fn node_layers(graph: &Graph) -> Vec<i64> {
    let (_, _, _, layer) = layer_graph(graph, &Glyphs { ascii: false });
    layer[..graph.nodes.len()].to_vec()
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
pub fn render(graph: &Graph, direction: Option<Direction>, ascii: bool) -> String {
    if graph.nodes.is_empty() {
        return String::new();
    }
    let td = direction.unwrap_or(graph.direction) == Direction::TD;
    let gl = Glyphs { ascii };

    let (mut ln, mut edges, chains, mut layer) = layer_graph(graph, &gl);

    // Dummy nodes for segments spanning several layers.
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); ln.len()];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); ln.len()];
    for (e, chain) in edges.iter_mut().zip(&chains) {
        let mut path = vec![chain[0]];
        for w in chain.windows(2) {
            for k in layer[w[0]] + 1..layer[w[1]] {
                ln.push(LNode::new(Vec::new(), Shape::Box, Kind::Dummy));
                layer.push(k);
                up.push(Vec::new());
                down.push(Vec::new());
                path.push(ln.len() - 1);
            }
            path.push(w[1]);
        }
        for w in path.windows(2) {
            down[w[0]].push(w[1]);
            up[w[1]].push(w[0]);
        }
        e.path = path;
    }

    let nl = (*layer.iter().max().unwrap() + 1) as usize;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); nl];
    for v in 0..ln.len() {
        layers[layer[v] as usize].push(v);
    }

    // Crossing reduction (barycenter sweeps, keep the best ordering).
    let mut best = layers.clone();
    let mut best_c = crossings(&layers, &down);
    let mut rank = vec![0usize; ln.len()];
    for it in 0..16 {
        for lay in &layers {
            for (i, &v) in lay.iter().enumerate() {
                rank[v] = i;
            }
        }
        let even = it % 2 == 0;
        let ks: Vec<usize> = if even { (1..nl).collect() } else { (0..nl - 1).rev().collect() };
        let nbrs = if even { &up } else { &down };
        for k in ks {
            let adj = if even { k - 1 } else { k + 1 };
            for (i, &v) in layers[adj].iter().enumerate() {
                rank[v] = i;
            }
            let key = |v: usize| -> f64 {
                if nbrs[v].is_empty() {
                    rank[v] as f64
                } else {
                    nbrs[v].iter().map(|&m| rank[m]).sum::<usize>() as f64 / nbrs[v].len() as f64
                }
            };
            let mut keyed: Vec<(f64, usize)> = layers[k].iter().map(|&v| (key(v), v)).collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            layers[k] = keyed.into_iter().map(|(_, v)| v).collect();
        }
        let c = crossings(&layers, &down);
        if c < best_c {
            best = layers.clone();
            best_c = c;
        }
    }
    let layers = best;

    // Segments and ports. Back edges get their own port so their arrowhead
    // doesn't sit on top of the shared port.
    let mut segs: Vec<Seg> = Vec::new();
    for (ei, e) in edges.iter().enumerate() {
        let p = &e.path;
        for j in 0..p.len() - 1 {
            let (u, v) = (p[j], p[j + 1]);
            let group = |n: usize| if e.rev && ln[n].kind == Kind::Real { Some(ei) } else { None };
            segs.push(Seg {
                u,
                v,
                style: e.style,
                arrow_end: e.arrow && !e.rev && j == p.len() - 2,
                arrow_start: e.arrow && e.rev && j == 0,
                sk: (u, Side::Out, group(u)),
                tk: (v, Side::In, group(v)),
                cs: 0,
                ct: 0,
                lo: 0,
                hi: 0,
                track: 0,
            });
        }
    }
    let mut side_groups: HashMap<(usize, Side), Vec<Option<usize>>> = HashMap::new();
    for s in &segs {
        for (n, side, g) in [s.sk, s.tk] {
            let gs = side_groups.entry((n, side)).or_default();
            if !gs.contains(&g) {
                gs.push(g);
            }
        }
    }
    let nports = |n: usize, side: Side| side_groups.get(&(n, side)).map_or(0, |g| g.len()) as i64;

    // Node sizes: csize runs across the flow, lsize along it.
    for (i, nd) in ln.iter_mut().enumerate() {
        let textw = nd.lines.iter().map(|t| text_width(t)).max().unwrap_or(0);
        let nlines = nd.lines.len() as i64;
        let (w, h) = match nd.kind {
            Kind::Dummy => {
                nd.csize = 1;
                nd.lsize = 0;
                continue;
            }
            Kind::Label => (textw, nlines),
            Kind::Real => {
                let np = nports(i, Side::In).max(nports(i, Side::Out));
                let h = if td { nlines + 2 } else { (nlines + 2).max(2 * np + 1) };
                ((textw + 4).max(2 * np + 1), h)
            }
        };
        if td {
            nd.csize = w;
            nd.lsize = h;
        } else {
            nd.csize = h;
            nd.lsize = w;
        }
    }

    // Cross-axis placement: pull nodes toward neighbours while keeping order.
    let gap: i64 = if td { 3 } else { 1 };
    for lay in &layers {
        let mut p = 0;
        for &v in lay {
            ln[v].pos = p;
            p += ln[v].csize + gap;
        }
    }
    let weight = |k: Kind| match k {
        Kind::Real => 1.0,
        Kind::Label => 2.0,
        Kind::Dummy => 4.0,
    };
    let place = |ln: &mut Vec<LNode>, lay: &[usize], nbrs: &[Vec<usize>]| {
        let (mut ys, mut ws, mut offs) = (Vec::new(), Vec::new(), Vec::new());
        let mut p = 0i64;
        for &v in lay {
            let nd = &ln[v];
            let (want, w) = if nbrs[v].is_empty() {
                (nd.cen() as f64, 0.05)
            } else {
                let sum: i64 = nbrs[v].iter().map(|&m| ln[m].cen()).sum();
                (sum as f64 / nbrs[v].len() as f64, weight(nd.kind))
            };
            ys.push(want - (nd.csize - 1).div_euclid(2) as f64 - p as f64);
            ws.push(w);
            offs.push(p);
            p += nd.csize + gap;
        }
        for ((&v, y), o) in lay.iter().zip(pava(&ys, &ws)).zip(offs) {
            ln[v].pos = y.round_ties_even() as i64 + o;
        }
    };
    for _ in 0..8 {
        for k in 1..nl {
            place(&mut ln, &layers[k], &up);
        }
        for k in (0..nl - 1).rev() {
            place(&mut ln, &layers[k], &down);
        }
    }
    for k in 1..nl {
        place(&mut ln, &layers[k], &up);
    }
    let shift = ln.iter().map(|n| n.pos).min().unwrap();
    for nd in ln.iter_mut() {
        nd.pos -= shift;
    }

    // Port positions. Back-edge ports prefer the side facing their other end.
    let mut other_end: HashMap<PortKey, usize> = HashMap::new();
    for s in &segs {
        other_end.insert(s.sk, s.v);
        other_end.insert(s.tk, s.u);
    }
    let mut port: HashMap<PortKey, i64> = HashMap::new();
    for (&(n, side), groups) in &side_groups {
        let nd = &ln[n];
        let c = nd.cen();
        let (lo, hi) = if nd.kind == Kind::Real { (nd.pos + 1, nd.pos + nd.csize - 2) } else { (c, c) };
        let mut used: Vec<i64> = Vec::new();
        let mut groups = groups.clone();
        groups.sort_by_key(|g| g.is_some());
        for g in groups {
            let x = match g {
                None => c,
                Some(_) => {
                    let toward = ln[other_end[&(n, side, g)]].cen() - c;
                    let mut free: Vec<i64> = (lo..=hi).filter(|x| !used.contains(x)).collect();
                    if free.is_empty() {
                        free.push(c);
                    }
                    *free
                        .iter()
                        .min_by_key(|&&x| {
                            (used.iter().any(|u| (x - u).abs() < 2), (x - c) * toward < 0, (x - c).abs())
                        })
                        .unwrap()
                }
            };
            used.push(x);
            port.insert((n, side, g), x);
        }
    }

    // Nets: segments sharing a port are drawn as one connected line.
    let mut parent: HashMap<PortKey, PortKey> = HashMap::new();
    fn find(parent: &HashMap<PortKey, PortKey>, mut x: PortKey) -> PortKey {
        while let Some(&p) = parent.get(&x) {
            if p == x {
                break;
            }
            x = p;
        }
        x
    }
    for s in &segs {
        let (a, b) = (find(&parent, s.sk), find(&parent, s.tk));
        if a != b {
            parent.insert(a, b);
        }
    }

    // Track assignment within each channel between layers.
    let mut chan: Vec<Vec<usize>> = vec![Vec::new(); nl];
    for (i, s) in segs.iter_mut().enumerate() {
        s.cs = port[&s.sk];
        s.ct = port[&s.tk];
        s.lo = s.cs.min(s.ct);
        s.hi = s.cs.max(s.ct);
        chan[layer[s.u] as usize].push(i);
    }
    let compatible = |segs: &[Seg], s: usize, track: &[usize]| -> bool {
        let mut comp = vec![s];
        let mut rest: Vec<usize> = track.to_vec();
        let mut grown = true;
        while grown {
            grown = false;
            let mut i = 0;
            while i < rest.len() {
                let f = &segs[rest[i]];
                if comp.iter().any(|&c| f.lo <= segs[c].hi && segs[c].lo <= f.hi) {
                    comp.push(rest.remove(i));
                    grown = true;
                } else {
                    i += 1;
                }
            }
        }
        let sks: HashSet<PortKey> = comp.iter().map(|&c| segs[c].sk).collect();
        let tks: HashSet<PortKey> = comp.iter().map(|&c| segs[c].tk).collect();
        comp.len() == 1 || sks.len() == 1 || tks.len() == 1
    };
    for ss in &chan {
        let mut bent: Vec<usize> = ss.iter().copied().filter(|&i| segs[i].cs != segs[i].ct).collect();
        bent.sort_by_key(|&i| (segs[i].lo, segs[i].hi));
        // A segment leaving column x must turn above one arriving at column x.
        let nb = bent.len();
        let mut preds: Vec<HashSet<usize>> = vec![HashSet::new(); nb];
        for i in 0..nb {
            for j in 0..nb {
                let (a, b) = (&segs[bent[i]], &segs[bent[j]]);
                if i != j && a.cs == b.ct && a.sk != b.sk && a.tk != b.tk {
                    preds[j].insert(i);
                }
            }
        }
        let (mut order, mut done) = (Vec::new(), HashSet::new());
        let mut remaining: Vec<usize> = (0..nb).collect();
        while !remaining.is_empty() {
            let mut ready: Vec<usize> =
                remaining.iter().copied().filter(|&i| preds[i].is_subset(&done)).collect();
            if ready.is_empty() {
                ready = vec![*remaining.iter().min_by_key(|&&i| preds[i].difference(&done).count()).unwrap()];
            }
            for i in ready {
                order.push(i);
                done.insert(i);
                remaining.retain(|&r| r != i);
            }
        }
        let mut tracks: Vec<Vec<usize>> = Vec::new();
        let mut tof: HashMap<usize, i64> = HashMap::new();
        for i in order {
            let s = bent[i];
            let mut t = preds[i].iter().filter_map(|p| tof.get(p)).map(|t| t + 1).max().unwrap_or(0) as usize;
            loop {
                if t >= tracks.len() {
                    tracks.push(Vec::new());
                }
                if compatible(&segs, s, &tracks[t]) {
                    break;
                }
                t += 1;
            }
            tracks[t].push(s);
            segs[s].track = t as i64;
            tof.insert(i, t as i64);
        }
    }

    // Layer-axis coordinates.
    let (mut bstart, mut cstart, mut lead) = (vec![0i64; nl], vec![0i64; nl], vec![0i64; nl]);
    let mut cur = 0i64;
    for k in 0..nl {
        bstart[k] = cur;
        cur += layers[k].iter().map(|&v| ln[v].lsize).max().unwrap_or(0);
        if k < nl - 1 {
            let ss = &chan[k];
            let ntracks = ss.iter().filter(|&&i| segs[i].cs != segs[i].ct).map(|&i| segs[i].track + 1).max().unwrap_or(0);
            lead[k] = if !td { 1 } else if ss.iter().any(|&i| segs[i].arrow_start) { 1 } else { 0 };
            cstart[k] = cur;
            cur += lead[k] + ntracks + 1;
        }
    }
    let phys = |l: i64, c: i64| if td { (c, l) } else { (l, c) };
    let in_l = |n: usize| bstart[layer[n] as usize];
    let out_l = |n: usize| bstart[layer[n] as usize] + ln[n].lsize - 1;

    // ---- Drawing ----
    let mut glyph: HashMap<(i64, i64), char> = HashMap::new();
    let mut mask: HashMap<(i64, i64), Vec<(PortKey, u8, Style)>> = HashMap::new();

    fn put_text(glyph: &mut HashMap<(i64, i64), char>, mut x: i64, y: i64, text: &str) {
        for ch in text.chars() {
            glyph.insert((x, y), ch);
            let w = char_width(ch);
            if w == 2 {
                glyph.insert((x + 1, y), WIDE_TAIL);
            }
            x += w;
        }
    }
    let line = |mask: &mut HashMap<(i64, i64), Vec<(PortKey, u8, Style)>>,
                    p: (i64, i64),
                    q: (i64, i64),
                    net: PortKey,
                    st: Style| {
        let vertical = p.0 == q.0;
        let (a, b) = if vertical { (p.1.min(q.1), p.1.max(q.1)) } else { (p.0.min(q.0), p.0.max(q.0)) };
        for t in a..=b {
            let (cell, bits) = if vertical {
                ((p.0, t), (if t > a { U } else { 0 }) | (if t < b { D } else { 0 }))
            } else {
                ((t, p.1), (if t > a { L } else { 0 }) | (if t < b { R } else { 0 }))
            };
            let entry = mask.entry(cell).or_default();
            match entry.iter_mut().find(|e| e.0 == net) {
                Some(e) => {
                    e.1 |= bits;
                    e.2 = st;
                }
                None => entry.push((net, bits, st)),
            }
        }
    };

    for (v, nd) in ln.iter().enumerate() {
        let l0 = bstart[layer[v] as usize];
        match nd.kind {
            Kind::Dummy => {}
            Kind::Label => {
                let w = nd.lines.iter().map(|t| text_width(t)).max().unwrap_or(0);
                for (i, t) in nd.lines.iter().enumerate() {
                    let off = (w - text_width(t)).div_euclid(2);
                    let (x, y) = if td { (nd.pos + off, l0 + i as i64) } else { (l0 + off, nd.pos + i as i64) };
                    put_text(&mut glyph, x, y, t);
                }
            }
            Kind::Real => {
                let (x, y) = phys(l0, nd.pos);
                let (w, h) = if td { (nd.csize, nd.lsize) } else { (nd.lsize, nd.csize) };
                let [tl, tr, bl, br, hz, vt] = gl.border(nd.shape);
                let hline: String = std::iter::repeat(hz).take((w - 2) as usize).collect();
                put_text(&mut glyph, x, y, &format!("{tl}{hline}{tr}"));
                put_text(&mut glyph, x, y + h - 1, &format!("{bl}{hline}{br}"));
                let top = (h - 2 - nd.lines.len() as i64).div_euclid(2);
                for i in 1..h - 1 {
                    let li = i - 1 - top;
                    let t = if li >= 0 && (li as usize) < nd.lines.len() { nd.lines[li as usize].as_str() } else { "" };
                    let tw = text_width(t);
                    let pad = (w - 2 - tw).div_euclid(2);
                    let row = format!(
                        "{vt}{}{t}{}{vt}",
                        " ".repeat(pad.max(0) as usize),
                        " ".repeat((w - 2 - pad - tw).max(0) as usize)
                    );
                    put_text(&mut glyph, x, y + i, &row);
                }
            }
        }
    }

    let mut arrow_in: HashSet<(i64, i64)> = HashSet::new(); // arrowheads pointing at an in-border
    let mut arrow_out: HashSet<(i64, i64)> = HashSet::new(); // ... at an out-border
    for s in &segs {
        let k = layer[s.u] as usize;
        let (a, b) = (out_l(s.u), in_l(s.v));
        let pts: Vec<(i64, i64)> = if s.cs == s.ct {
            vec![(a, s.cs), (b, s.ct)]
        } else {
            let t = cstart[k] + lead[k] + s.track;
            vec![(a, s.cs), (t, s.cs), (t, s.ct), (b, s.ct)]
        };
        let net = find(&parent, s.sk);
        for w in pts.windows(2) {
            if w[0] != w[1] {
                line(&mut mask, phys(w[0].0, w[0].1), phys(w[1].0, w[1].1), net, s.style);
            }
        }
        if s.arrow_end {
            let cell = phys(b - 1, s.ct);
            glyph.insert(cell, gl.arrow(true, td));
            arrow_in.insert(cell);
        }
        if s.arrow_start {
            let cell = phys(a + 1, s.cs);
            glyph.insert(cell, gl.arrow(false, td));
            arrow_out.insert(cell);
        }
    }

    let mut out = glyph.clone();
    for (cell, nets) in &mask {
        if glyph.contains_key(cell) {
            continue;
        }
        let vert: Vec<_> = nets.iter().filter(|n| n.1 & HORZ == 0).collect();
        let horz: Vec<_> = nets.iter().filter(|n| n.1 & VERT == 0).collect();
        if nets.len() > 1 && !vert.is_empty() && !horz.is_empty() && vert.len() + horz.len() == nets.len() {
            out.insert(*cell, gl.straight(vert[0].2, true)); // a crossing, not a join
            continue;
        }
        let bits = nets.iter().fold(0, |acc, n| acc | n.1);
        let st = nets[0].2;
        let ch = if bits & HORZ == 0 {
            gl.straight(st, true)
        } else if bits & VERT == 0 {
            gl.straight(st, false)
        } else {
            gl.junction(bits)
        };
        out.insert(*cell, ch);
    }

    // Tees where edges attach to box borders without an arrowhead.
    for (&(n, side, _), &c) in &port {
        let nd = &ln[n];
        if nd.kind != Kind::Real {
            continue;
        }
        let tees = gl.tee(nd.shape);
        let (border, outside, tee, arrows) = match side {
            Side::Out => (out_l(n), out_l(n) + 1, tees[if td { 1 } else { 3 }], &arrow_out),
            Side::In => (in_l(n), in_l(n) - 1, tees[if td { 0 } else { 2 }], &arrow_in),
        };
        let o = phys(outside, c);
        if mask.contains_key(&o) && !arrows.contains(&o) {
            out.insert(phys(border, c), tee);
        }
    }

    let width = out.keys().map(|c| c.0).max().unwrap() + 1;
    let height = out.keys().map(|c| c.1).max().unwrap() + 1;
    let mut rows = vec![vec![' '; width as usize]; height as usize];
    for (&(x, y), &ch) in &out {
        rows[y as usize][x as usize] = ch;
    }
    rows.iter()
        .map(|r| r.iter().filter(|&&c| c != WIDE_TAIL).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
