//! Flowchart layout: a simplified Sugiyama pipeline, one method per stage.
//!
//! break cycles → layer → dummy nodes → lanes → crossing reduction → sizes →
//! placement (isotonic regression) → ports → channel tracks → paint.
//!
//! Coordinates are abstract: `l` runs along the flow (layers) and `c` across
//! it; `phys` maps them to x/y, so TD and LR share every line of layout.

use crate::model::{Direction, Graph, Mark, Shape, Style};
use crate::paint::{Corners, Layer, Painter, Pt, Stroke, Weight};
use crate::text::width;
use std::collections::{HashMap, HashSet, VecDeque};

/// Gap plus dashed line between a group's members and its frame.
const FRAME: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Real,
    Label,
    Dummy,
}

struct Node {
    lines: Vec<String>,
    shape: Shape,
    mark: Mark,
    kind: Kind,
    layer: usize,
    /// 0 = no group, g + 1 = `Graph::groups[g]`.
    lane: usize,
    up: Vec<usize>,
    down: Vec<usize>,
    /// Cross-axis position of the left/top edge.
    pos: i64,
    /// Size across the flow and along it.
    csize: i64,
    lsize: i64,
    /// A self-loop (its style, and whether it has an arrowhead), drawn as a
    /// hook in one cell of padding on the far side of the box.
    looped: Option<(Style, bool)>,
}

impl Node {
    fn new(lines: Vec<String>, kind: Kind) -> Self {
        let (shape, mark) = (Shape::Box, Mark::None);
        let (up, down) = (vec![], vec![]);
        Node { lines, shape, mark, kind, layer: 0, lane: 0, up, down, pos: 0, csize: 0, lsize: 0, looped: None }
    }

    /// Cross-axis size of the box itself, without the self-loop's padding.
    fn box_csize(&self) -> i64 {
        self.csize - i64::from(self.looped.is_some())
    }

    fn cen(&self) -> i64 {
        self.pos + (self.box_csize() - 1).div_euclid(2)
    }
}

struct Edge {
    src: usize,
    dst: usize,
    label: Option<Vec<String>>,
    style: Style,
    arrow: bool,
    /// A back edge (it closes a cycle): routed around the outside instead
    /// of through the layers.
    rev: bool,
    /// Layer-ordered nodes, including the label node and dummies.
    path: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    In,
    Out,
}

/// (node, side, style): edges of one style share a port on a box side; a
/// second style gets its own port beside it, so dotted and solid stay apart.
/// Node indices past the real nodes stand for a back edge's outer lane.
type PortKey = (usize, Side, Style);

/// One edge segment between adjacent layers, `cs` → `ct` across the channel.
struct Seg {
    u: usize,
    v: usize,
    /// The channel (between layer `chan` and `chan + 1`) it crosses.
    chan: usize,
    /// For the two halves of a back edge: the edge's index.
    back: Option<usize>,
    style: Style,
    arrow_end: bool,
    sk: PortKey,
    tk: PortKey,
    cs: i64,
    ct: i64,
    track: i64,
}

impl Seg {
    fn span(&self) -> (i64, i64) {
        (self.cs.min(self.ct), self.cs.max(self.ct))
    }
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
    blocks.iter().flat_map(|&(v, _, c)| std::iter::repeat_n(v, c)).collect()
}

/// Mark edges that close a cycle (found by depth-first search) as reversed.
fn break_cycles(n: usize, edges: &mut [Edge]) {
    let mut out = vec![Vec::new(); n];
    let mut indeg = vec![0; n];
    for (i, e) in edges.iter().enumerate() {
        out[e.src].push(i);
        indeg[e.dst] += 1;
    }
    let mut state = vec![0u8; n]; // 0 new, 1 on the stack, 2 done
    for root in (0..n).filter(|&v| indeg[v] == 0).chain(0..n) {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (u, idx) = *top;
            let Some(&ei) = out[u].get(idx) else {
                state[u] = 2;
                stack.pop();
                continue;
            };
            top.1 += 1;
            match state[edges[ei].dst] {
                1 => edges[ei].rev = true,
                0 => {
                    state[edges[ei].dst] = 1;
                    stack.push((edges[ei].dst, 0));
                }
                _ => {}
            }
        }
    }
}

fn crossings(layers: &[Vec<usize>], nodes: &[Node]) -> usize {
    let mut total = 0;
    for pair in layers.windows(2) {
        let rank: HashMap<usize, i64> = pair[1].iter().enumerate().map(|(i, &v)| (v, i as i64)).collect();
        let es: Vec<(i64, i64)> = pair[0]
            .iter()
            .enumerate()
            .flat_map(|(i, &u)| nodes[u].down.iter().map(move |v| (i as i64, v)))
            .map(|(i, v)| (i, rank[v]))
            .collect();
        for (a, ea) in es.iter().enumerate() {
            total += es[a + 1..].iter().filter(|eb| (ea.0 - eb.0) * (ea.1 - eb.1) < 0).count();
        }
    }
    total
}

/// Break cycles, turn edge labels into nodes, and assign each node a layer
/// (longest path, then sources pulled down next to their children).
fn layer_graph(graph: &Graph) -> (Vec<Node>, Vec<Edge>, Vec<Vec<usize>>) {
    let mut nodes: Vec<Node> = graph
        .nodes
        .iter()
        .map(|n| Node {
            shape: n.shape,
            mark: n.mark,
            lane: n.group.map_or(0, |g| g + 1),
            ..Node::new(n.lines.clone(), Kind::Real)
        })
        .collect();
    for e in graph.edges.iter().filter(|e| e.src == e.dst) {
        nodes[e.src].looped = Some((e.style, e.arrow));
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
            path: vec![],
        })
        .collect();
    break_cycles(nodes.len(), &mut edges);

    let (mut succ, mut pred) = (vec![vec![]; nodes.len()], vec![vec![]; nodes.len()]);
    let mut chains = Vec::new();
    for e in &edges {
        let mut chain = if e.rev { vec![e.dst, e.src] } else { vec![e.src, e.dst] };
        if let Some(label) = e.label.as_ref().filter(|_| !e.rev) {
            nodes.push(Node::new(label.clone(), Kind::Label));
            succ.push(vec![]);
            pred.push(vec![]);
            chain.insert(1, nodes.len() - 1);
        }
        for w in chain.windows(2) {
            succ[w[0]].push(w[1]);
            pred[w[1]].push(w[0]);
        }
        chains.push(chain);
    }
    let mut indeg: Vec<usize> = pred.iter().map(Vec::len).collect();
    let mut queue: VecDeque<usize> = (0..nodes.len()).filter(|&v| indeg[v] == 0).collect();
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
    for &v in &topo {
        nodes[v].layer = pred[v].iter().map(|&p| nodes[p].layer + 1).max().unwrap_or(0);
    }
    for &v in topo.iter().rev() {
        if pred[v].is_empty() && !succ[v].is_empty() {
            nodes[v].layer = succ[v].iter().map(|&s| nodes[s].layer).min().unwrap() - 1;
        }
    }
    (nodes, edges, chains)
}

/// Layer index of every real node, as the layout would assign it. Labelled
/// edges take an extra layer for the label.
#[must_use]
pub fn node_layers(graph: &Graph) -> Vec<usize> {
    layer_graph(graph).0.iter().take(graph.nodes.len()).map(|n| n.layer).collect()
}

#[derive(Default)]
struct Layout {
    /// Number of real (declared) nodes, and the subgraph titles.
    real: usize,
    titles: Vec<String>,
    td: bool,
    gap: i64,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    layers: Vec<Vec<usize>>,
    /// Lanes in cross-axis order, and each lane's span of layers.
    lanes: Vec<usize>,
    span: HashMap<usize, (usize, usize)>,
    segs: Vec<Seg>,
    groups: HashMap<(usize, Side), Vec<Style>>,
    port: HashMap<PortKey, i64>,
    /// Cross-axis position of each back edge's outer lane.
    back_lane: HashMap<usize, i64>,
    /// Per layer: band start, channel start, channel lead before the tracks.
    bstart: Vec<i64>,
    cstart: Vec<i64>,
    lead: Vec<i64>,
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
#[must_use]
pub fn render(graph: &Graph, direction: Option<Direction>, ascii: bool) -> String {
    if graph.nodes.is_empty() {
        return String::new();
    }
    let td = direction.unwrap_or(graph.direction) == Direction::TD;
    let (mut nodes, edges, chains) = layer_graph(graph);
    // A back edge enters its target from the channel before it, so the first
    // layer may need one in front of it.
    if edges.iter().any(|e| e.rev && nodes[e.dst].layer == 0) {
        for n in &mut nodes {
            n.layer += 1;
        }
    }
    let titles = graph.groups.iter().map(|g| g.title.clone()).collect();
    let mut l =
        Layout { real: graph.nodes.len(), titles, td, gap: if td { 3 } else { 1 }, nodes, edges, ..Default::default() };
    l.dummies(&chains);
    l.order();
    l.segments();
    l.sizes();
    l.place();
    l.ports();
    l.back_lanes();
    l.tracks();
    l.coordinates();
    l.paint(ascii)
}

impl Layout {
    fn lane_rank(&self, v: usize) -> usize {
        self.lanes.iter().position(|&l| l == self.nodes[v].lane).unwrap_or(0)
    }

    /// A cross-axis point far out on the side a back edge's lane will use.
    fn lane_side(&self, e: usize) -> i64 {
        let (src, dst) = (self.nodes[self.edges[e].src].cen(), self.nodes[self.edges[e].dst].cen());
        let (lo, hi) = self.nodes.iter().fold((i64::MAX, i64::MIN), |(a, b), n| (a.min(n.pos), b.max(n.pos + n.csize)));
        if src + dst < lo + hi {
            -(1 << 30)
        } else {
            1 << 30
        }
    }

    /// Space after `a` in its layer: wider between two TD edge labels, so
    /// side-by-side labels don't read as one phrase.
    fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        let labels = |v: usize| self.nodes[v].kind == Kind::Label;
        self.gap + if self.td && labels(a) && b.is_some_and(|&b| labels(b)) { 3 } else { 0 }
    }

    fn framed(&self, v: usize) -> i64 {
        if self.nodes[v].lane > 0 {
            FRAME
        } else {
            0
        }
    }

    /// Dummy nodes for edges spanning several layers. Long edges of the same
    /// style into the same node share dummies, so they merge into one line as
    /// early as possible instead of each taking its own route.
    fn dummies(&mut self, chains: &[Vec<usize>]) {
        let mut shared: HashMap<(usize, usize, Style), usize> = HashMap::new();
        for (ei, chain) in chains.iter().enumerate() {
            let (style, src) = (self.edges[ei].style, self.edges[ei].src);
            if self.edges[ei].rev {
                continue;
            }
            let mut path = vec![chain[0]];
            for w in chain.windows(2) {
                let bundle = self.nodes[w[1]].kind == Kind::Real;
                for k in self.nodes[w[0]].layer + 1..self.nodes[w[1]].layer {
                    let d = if let Some(&d) = shared.get(&(w[1], k, style)).filter(|_| bundle) {
                        d
                    } else {
                        self.nodes.push(Node { layer: k, ..Node::new(vec![], Kind::Dummy) });
                        self.nodes.len() - 1
                    };
                    if bundle {
                        shared.insert((w[1], k, style), d);
                    }
                    path.push(d);
                }
                path.push(w[1]);
            }
            for w in path.windows(2) {
                if !self.nodes[w[0]].down.contains(&w[1]) {
                    self.nodes[w[0]].down.push(w[1]);
                    self.nodes[w[1]].up.push(w[0]);
                }
            }
            // Labels and dummies live in the lane of the edge's source.
            for &v in &path {
                if self.nodes[v].kind != Kind::Real {
                    self.nodes[v].lane = self.nodes[src].lane;
                }
            }
            self.edges[ei].path = path;
        }
    }

    /// Lanes, then barycenter sweeps (keeping the ordering with fewest
    /// crossings). Every subgraph is a lane: a strip across the flow only its
    /// members occupy, so its frame can never enclose a stranger.
    fn order(&mut self) {
        let real = self.real;
        for v in 0..real {
            let (lane, layer) = (self.nodes[v].lane, self.nodes[v].layer);
            let e = self.span.entry(lane).or_insert((layer, layer));
            *e = (e.0.min(layer), e.1.max(layer));
            if !self.lanes.contains(&lane) {
                self.lanes.push(lane);
            }
        }
        self.lanes.sort_by_key(|l| self.span[l].0);
        let last = self.nodes.iter().map(|n| n.layer).max().unwrap();
        let nl = last + 1 + usize::from(self.edges.iter().any(|e| e.rev && self.nodes[e.src].layer == last));
        self.layers = vec![vec![]; nl];
        for v in 0..self.nodes.len() {
            self.layers[self.nodes[v].layer].push(v);
        }
        let rank_of: Vec<usize> = (0..self.nodes.len()).map(|v| self.lane_rank(v)).collect();
        for lay in &mut self.layers {
            lay.sort_by_key(|&v| rank_of[v]);
        }
        let (mut best, mut best_c) = (self.layers.clone(), crossings(&self.layers, &self.nodes));
        let mut rank = vec![0usize; self.nodes.len()];
        for it in 0..16 {
            for lay in &self.layers {
                lay.iter().enumerate().for_each(|(i, &v)| rank[v] = i);
            }
            let down = it % 2 == 0;
            let ks: Vec<usize> = if down { (1..nl).collect() } else { (0..nl - 1).rev().collect() };
            for k in ks {
                let adj = if down { k - 1 } else { k + 1 };
                self.layers[adj].iter().enumerate().for_each(|(i, &v)| rank[v] = i);
                let key = |v: usize| {
                    let nb = if down { &self.nodes[v].up } else { &self.nodes[v].down };
                    if nb.is_empty() {
                        rank[v] as f64
                    } else {
                        nb.iter().map(|&m| rank[m]).sum::<usize>() as f64 / nb.len() as f64
                    }
                };
                let mut keyed: Vec<(f64, usize)> = self.layers[k].iter().map(|&v| (key(v), v)).collect();
                keyed.sort_by(|a, b| rank_of[a.1].cmp(&rank_of[b.1]).then(a.0.partial_cmp(&b.0).unwrap()));
                self.layers[k] = keyed.into_iter().map(|(_, v)| v).collect();
            }
            self.transpose(&rank_of);
            let c = crossings(&self.layers, &self.nodes);
            if c < best_c {
                (best, best_c) = (self.layers.clone(), c);
            }
        }
        self.layers = best;
    }

    /// Swap neighbours in a layer (within a lane) while that lowers the
    /// crossings between their edges ("transpose", as in Graphviz dot).
    fn transpose(&mut self, lane_rank: &[usize]) {
        let mut pos = vec![0usize; self.nodes.len()];
        for lay in &self.layers {
            lay.iter().enumerate().for_each(|(i, &v)| pos[v] = i);
        }
        // Crossings among the edges of `a` and `b` when `a` is placed first.
        let cost = |nodes: &[Node], pos: &[usize], a: usize, b: usize| -> usize {
            let side = |na: &[usize], nb: &[usize]| {
                na.iter().map(|&x| nb.iter().filter(|&&y| pos[x] > pos[y]).count()).sum::<usize>()
            };
            side(&nodes[a].up, &nodes[b].up) + side(&nodes[a].down, &nodes[b].down)
        };
        let mut improved = true;
        while improved {
            improved = false;
            for lay in &mut self.layers {
                for i in 1..lay.len() {
                    let (a, b) = (lay[i - 1], lay[i]);
                    if lane_rank[a] == lane_rank[b] && cost(&self.nodes, &pos, b, a) < cost(&self.nodes, &pos, a, b) {
                        lay.swap(i - 1, i);
                        (pos[a], pos[b]) = (i, i - 1);
                        improved = true;
                    }
                }
            }
        }
    }

    /// One segment per layer gap. A back edge becomes two: out of its source
    /// into the outer lane, and from the lane into its target.
    fn segments(&mut self) {
        let real = self.nodes.len();
        for (ei, e) in self.edges.iter().enumerate() {
            let mut add =
                |segs: &mut Vec<Seg>, (u, v, chan): (usize, usize, usize), sk: PortKey, tk: PortKey, arrow_end| {
                    for (n, side, style) in [sk, tk].into_iter().filter(|k| k.0 < real) {
                        let styles = self.groups.entry((n, side)).or_default();
                        if !styles.contains(&style) {
                            styles.push(style);
                        }
                    }
                    let back = e.rev.then_some(ei);
                    segs.push(Seg { u, v, chan, back, style: e.style, arrow_end, sk, tk, cs: 0, ct: 0, track: 0 });
                };
            let (out, inn) = (|n| (n, Side::Out, e.style), |n| (n, Side::In, e.style));
            if e.rev {
                let (src, dst) = (self.nodes[e.src].layer, self.nodes[e.dst].layer);
                add(&mut self.segs, (e.src, e.src, src), out(e.src), inn(real + ei), false);
                add(&mut self.segs, (e.dst, e.dst, dst - 1), out(real + ei), inn(e.dst), e.arrow);
                continue;
            }
            for (j, w) in e.path.windows(2).enumerate() {
                let chan = self.nodes[w[0]].layer;
                add(&mut self.segs, (w[0], w[1], chan), out(w[0]), inn(w[1]), e.arrow && j == e.path.len() - 2);
            }
        }
    }

    /// Boxes are wide (and in LR tall) enough for their text and ports.
    fn sizes(&mut self) {
        for v in 0..self.nodes.len() {
            let ports = |side| self.groups.get(&(v, side)).map_or(0, Vec::len) as i64;
            let np = ports(Side::In).max(ports(Side::Out));
            let nd = &mut self.nodes[v];
            let (textw, nlines) = (nd.lines.iter().map(|t| width(t)).max().unwrap_or(0), nd.lines.len() as i64);
            let (w, h) = match nd.kind {
                Kind::Dummy => (1, 0),
                Kind::Label => (textw, nlines),
                // A TD self-loop needs two rows inside the box to hook between.
                Kind::Real if self.td => ((textw + 4).max(2 * np + 1), nlines + 2 + i64::from(nd.looped.is_some())),
                Kind::Real => ((textw + 4).max(2 * np + 1), (nlines + 2).max(2 * np + 1)),
            };
            (nd.csize, nd.lsize) = if self.td || nd.kind == Kind::Dummy { (w, h) } else { (h, w) };
            nd.csize += i64::from(nd.looped.is_some());
        }
    }

    /// Pull nodes toward their neighbours while keeping order and spacing,
    /// then push lanes apart so frames have room.
    fn place(&mut self) {
        for k in 0..self.layers.len() {
            let mut c = 0;
            for (i, &v) in self.layers[k].iter().enumerate() {
                self.nodes[v].pos = c;
                c += self.nodes[v].csize + self.space(v, self.layers[k].get(i + 1));
            }
        }
        let sweep = |l: &mut Self, k: usize, up: bool| {
            let (mut ys, mut ws, mut offs, mut c) = (vec![], vec![], vec![], 0i64);
            for (i, &v) in l.layers[k].iter().enumerate() {
                let nd = &l.nodes[v];
                let nb = if up { &nd.up } else { &nd.down };
                let (want, w) = match nd.kind {
                    _ if nb.is_empty() => (nd.cen() as f64, 0.05),
                    kind => {
                        let sum: i64 = nb.iter().map(|&m| l.nodes[m].cen()).sum();
                        (sum as f64 / nb.len() as f64, [1.0, 2.0, 4.0][kind as usize])
                    }
                };
                ys.push(want - (nd.box_csize() - 1).div_euclid(2) as f64 - c as f64);
                ws.push(w);
                offs.push(c);
                c += nd.csize + l.space(v, l.layers[k].get(i + 1));
            }
            for ((&v, y), o) in l.layers[k].clone().iter().zip(pava(&ys, &ws)).zip(offs) {
                l.nodes[v].pos = y.round_ties_even() as i64 + o;
            }
        };
        let nl = self.layers.len();
        for _ in 0..8 {
            (1..nl).for_each(|k| sweep(self, k, true));
            (0..nl - 1).rev().for_each(|k| sweep(self, k, false));
        }
        (1..nl).for_each(|k| sweep(self, k, true));

        // Push each lane clear of earlier lanes it shares layers with.
        let mut extent: Vec<Option<(i64, i64)>> = vec![None; self.lanes.len()];
        for (v, nd) in self.nodes.iter().enumerate() {
            let e = &mut extent[self.lane_rank(v)];
            let (lo, hi) = (nd.pos, nd.pos + nd.csize - 1);
            *e = Some(e.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
        let margin = |l: usize| if l > 0 { FRAME } else { 0 };
        let overlaps = |a: (usize, usize), b: (usize, usize)| a.0 <= b.1 && b.0 <= a.1;
        for r in 1..self.lanes.len() {
            let lr = self.lanes[r];
            let need = (0..r)
                .filter(|&q| overlaps(self.span[&self.lanes[q]], self.span[&lr]))
                .filter_map(|q| extent[q].map(|e| e.1 + 1 + self.gap + margin(self.lanes[q]) + margin(lr)))
                .max();
            let Some((lo, hi)) = extent[r] else { continue };
            let push = need.map_or(0, |n| (n - lo).max(0));
            for v in 0..self.nodes.len() {
                if self.lane_rank(v) == r {
                    self.nodes[v].pos += push;
                }
            }
            extent[r] = Some((lo + push, hi + push));
        }
        // Pushing a lane can run it into another lane's dummies in a shared layer.
        for lay in &self.layers {
            for w in lay.windows(2) {
                let floor = self.nodes[w[0]].pos + self.nodes[w[0]].csize + self.space(w[0], Some(&w[1]));
                self.nodes[w[1]].pos = self.nodes[w[1]].pos.max(floor);
            }
        }
        let shift = (0..self.nodes.len()).map(|v| self.nodes[v].pos - self.framed(v)).min().unwrap();
        self.nodes.iter_mut().for_each(|n| n.pos -= shift);
    }

    /// Port positions: the first style at the centre, others beside it on
    /// the side facing where their edges go (back edges: the outer lane).
    fn ports(&mut self) {
        let real = self.nodes.len();
        // Where each port's edges go, on average (back edges: far outside).
        let mut ends: HashMap<PortKey, (i64, i64)> = HashMap::new();
        for s in &self.segs {
            let other = |n: usize| if n < real { self.nodes[n].cen() } else { self.lane_side(n - real) };
            for (key, end) in [(s.sk, other(s.tk.0)), (s.tk, other(s.sk.0))] {
                let e = ends.entry(key).or_default();
                *e = (e.0 + end, e.1 + 1);
            }
        }
        let toward = |key: PortKey| ends.get(&key).map_or(0, |&(sum, n)| sum / n);
        for (&(n, side), styles) in &self.groups {
            let nd = &self.nodes[n];
            let c = nd.cen();
            let (lo, hi) = if nd.kind == Kind::Real { (nd.pos + 1, nd.pos + nd.box_csize() - 2) } else { (c, c) };
            let mut used: Vec<i64> = Vec::new();
            // The style whose edges run most nearly straight gets the centre.
            let mut styles = styles.clone();
            styles.sort_by_key(|&st| (toward((n, side, st)) - c).abs());
            for style in styles {
                let free: Vec<i64> = (lo..=hi).filter(|x| !used.contains(x)).collect();
                let dir = toward((n, side, style)) - c;
                let key = |&x: &i64| (used.iter().any(|u| (x - u).abs() < 2), (x - c) * dir < 0, (x - c).abs(), x);
                let x = if used.is_empty() { c } else { free.into_iter().min_by_key(key).unwrap_or(c) };
                used.push(x);
                self.port.insert((n, side, style), x);
            }
        }
        for s in &mut self.segs {
            (s.cs, s.ct) = (self.port.get(&s.sk).copied().unwrap_or(0), self.port.get(&s.tk).copied().unwrap_or(0));
        }
    }

    /// Outer lanes for back edges, beyond every node and frame on the side
    /// nearer the edge's ends; shorter edges run inside longer ones.
    fn back_lanes(&mut self) {
        let real = self.nodes.len();
        let lo = (0..real).map(|v| self.nodes[v].pos - self.framed(v)).min().unwrap_or(0) - 2;
        let hi = (0..real).map(|v| self.nodes[v].pos + self.nodes[v].csize - 1 + self.framed(v)).max().unwrap_or(0) + 2;
        let (mut near, mut far) = (lo, hi);
        let mut backs: Vec<usize> = (0..self.edges.len()).filter(|&e| self.edges[e].rev).collect();
        backs.sort_by_key(|&e| self.nodes[self.edges[e].src].layer - self.nodes[self.edges[e].dst].layer);
        for e in backs {
            let (src, dst) = (self.edges[e].src, self.edges[e].dst);
            let low = self.nodes[src].cen() + self.nodes[dst].cen() < lo + hi;
            // In TD the label sits beside the (vertical) lane, so leave room.
            let label = self.edges[e].label.as_ref().map_or(0, |l| width(&l.join(" ")) + 1);
            let step = 2 + if self.td { label } else { 0 };
            let lane = if low { near } else { far };
            if low {
                near -= step;
            } else {
                far += step;
            }
            self.back_lane.insert(e, if low && self.td { lane - label } else { lane });
            for s in self.segs.iter_mut().filter(|s| s.back == Some(e)) {
                if s.sk.0 >= real {
                    s.cs = self.back_lane[&e];
                } else {
                    s.ct = self.back_lane[&e];
                }
            }
        }
    }

    /// Give bent segments a track (a row/column of their channel) so that
    /// unrelated horizontals never share one.
    fn tracks(&mut self) {
        let segs = &self.segs;
        // Segments overlapping on a track must all share a source or a target port.
        let compatible = |s: usize, track: &[usize]| {
            let (mut comp, mut rest) = (vec![s], track.to_vec());
            while let Some(i) = rest.iter().position(|&f| {
                comp.iter().any(|&c| segs[f].span().0 <= segs[c].span().1 && segs[c].span().0 <= segs[f].span().1)
            }) {
                comp.push(rest.remove(i));
            }
            let distinct = |f: fn(&Seg) -> PortKey| comp.iter().map(|&c| f(&segs[c])).collect::<HashSet<_>>().len();
            comp.len() == 1 || distinct(|s| s.sk) == 1 || distinct(|s| s.tk) == 1
        };
        // Crossings between `a` and `b` if `a` takes the higher track: `a`'s
        // way down and `b`'s way up each cross the other's horizontal.
        let above = |a: usize, b: usize| -> i64 {
            let inside = |x: i64, (lo, hi): (i64, i64)| lo < x && x < hi;
            i64::from(inside(segs[a].ct, segs[b].span())) + i64::from(inside(segs[b].cs, segs[a].span()))
        };
        let mut assigned: Vec<(usize, i64)> = Vec::new();
        for k in 0..self.layers.len() {
            let mut bent: Vec<usize> =
                (0..segs.len()).filter(|&i| segs[i].chan == k && segs[i].cs != segs[i].ct).collect();
            bent.sort_by_key(|&i| segs[i].span());
            // A segment leaving column x must turn before one arriving at column x.
            let nb = bent.len();
            let preds: Vec<HashSet<usize>> = (0..nb)
                .map(|j| {
                    let (b, others) = (&segs[bent[j]], (0..nb).filter(|&i| i != j));
                    others
                        .filter(|&i| segs[bent[i]].cs == b.ct && segs[bent[i]].sk != b.sk && segs[bent[i]].tk != b.tk)
                        .collect()
                })
                .collect();
            let (mut order, mut done, mut remaining) = (vec![], HashSet::new(), (0..nb).collect::<Vec<_>>());
            while !remaining.is_empty() {
                let mut ready: Vec<usize> = remaining.iter().copied().filter(|&i| preds[i].is_subset(&done)).collect();
                if ready.is_empty() {
                    ready = vec![*remaining.iter().min_by_key(|&&i| preds[i].difference(&done).count()).unwrap()];
                }
                // Within a ready batch, put first (higher) the segment whose
                // verticals cross the fewest horizontals of the others.
                while !ready.is_empty() {
                    let score = |i: usize| -> i64 {
                        remaining
                            .iter()
                            .filter(|&&r| r != i)
                            .map(|&r| above(bent[i], bent[r]) - above(bent[r], bent[i]))
                            .sum()
                    };
                    let k = (0..ready.len()).min_by_key(|&k| score(ready[k])).unwrap();
                    let i = ready.remove(k);
                    order.push(i);
                    done.insert(i);
                    remaining.retain(|&r| r != i);
                }
            }
            let (mut tracks, mut tof): (Vec<Vec<usize>>, HashMap<usize, usize>) = (vec![], HashMap::new());
            for i in order {
                let mut t = preds[i].iter().filter_map(|p| tof.get(p)).map(|t| t + 1).max().unwrap_or(0);
                while t < tracks.len() && !compatible(bent[i], &tracks[t]) {
                    t += 1;
                }
                if t == tracks.len() {
                    tracks.push(vec![]);
                }
                tracks[t].push(bent[i]);
                tof.insert(i, t);
                assigned.push((bent[i], t as i64));
            }
        }
        for (s, t) in assigned {
            self.segs[s].track = t;
        }
    }

    /// The real-node layer span of each framed lane.
    fn frames(&self) -> Vec<(usize, usize, usize)> {
        self.lanes.iter().filter(|&&l| l > 0).map(|&l| (l, self.span[&l].0, self.span[&l].1)).collect()
    }

    fn band_len(&self, k: usize) -> i64 {
        self.layers[k].iter().map(|&v| self.nodes[v].lsize).max().unwrap_or(0)
    }

    /// Layer-axis coordinates. Channels next to a frame get FRAME extra cells.
    fn coordinates(&mut self) {
        let frames = self.frames();
        let nl = self.layers.len();
        let mut cur = if frames.iter().any(|f| f.1 == 0) { FRAME } else { 0 };
        (self.bstart, self.cstart, self.lead) = (vec![0; nl], vec![0; nl], vec![0; nl]);
        for k in 0..nl {
            self.bstart[k] = cur;
            cur += self.band_len(k);
            if k + 1 < nl {
                let bent = self.segs.iter().filter(|s| s.chan == k && s.cs != s.ct);
                let ntracks = bent.map(|s| s.track + 1).max().unwrap_or(0);
                self.lead[k] = i64::from(!self.td) + if frames.iter().any(|f| f.2 == k) { FRAME } else { 0 };
                let before_tail = if frames.iter().any(|f| f.1 == k + 1) { FRAME } else { 0 };
                self.cstart[k] = cur;
                cur += self.lead[k] + ntracks + before_tail + 1;
            }
        }
    }

    fn phys(&self, l: i64, c: i64) -> Pt {
        if self.td {
            (c, l)
        } else {
            (l, c)
        }
    }

    fn paint(&self, ascii: bool) -> String {
        let mut p = Painter::new(ascii);
        p.mark_crossings = true;
        let in_l = |n: usize| self.bstart[self.nodes[n].layer];
        let out_l = |n: usize| in_l(n) + self.nodes[n].lsize - 1;
        for nd in &self.nodes {
            let l0 = self.bstart[nd.layer];
            let textw = nd.lines.iter().map(|t| width(t)).max().unwrap_or(0);
            match nd.kind {
                Kind::Dummy => {}
                Kind::Label => {
                    for (i, t) in nd.lines.iter().enumerate() {
                        let (l, c) = ((textw - width(t)).div_euclid(2), i as i64);
                        p.text(if self.td { nd.pos + l } else { l0 + l }, if self.td { l0 + c } else { nd.pos + c }, t);
                    }
                }
                Kind::Real => {
                    let (x, y) = self.phys(l0, nd.pos);
                    let (w, h) = if self.td { (nd.box_csize(), nd.lsize) } else { (nd.lsize, nd.box_csize()) };
                    let (weight, style, corners) = match (nd.mark, nd.shape) {
                        (Mark::Highlight | Mark::Added, _) => (Weight::Heavy, Style::Solid, Corners::Square),
                        (Mark::Removed, _) => (Weight::Light, Style::Dotted, Corners::Square),
                        (_, Shape::Decision) => (Weight::Double, Style::Solid, Corners::Square),
                        (_, Shape::Round) => (Weight::Light, Style::Solid, Corners::Round),
                        _ => (Weight::Light, Style::Solid, Corners::Square),
                    };
                    if nd.shape == Shape::Database && weight == Weight::Light && style == Style::Solid {
                        // Double caps, light sides: ╒═╕ … ╘═╛.
                        let (x1, y1) = (x + w - 1, y + h - 1);
                        for (pts, weight) in
                            [([(x, y), (x1, y)], Weight::Double), ([(x, y1), (x1, y1)], Weight::Double)]
                                .into_iter()
                                .chain([([(x, y), (x, y1)], Weight::Light), ([(x1, y), (x1, y1)], Weight::Light)])
                        {
                            let net = p.net();
                            p.line(&pts, net, Stroke::new(weight, Style::Solid), Layer::Main);
                        }
                    } else {
                        p.rect((x, y), (w, h), Stroke::new(weight, style), corners, Layer::Main);
                    }
                    if let Some((style, arrow)) = nd.looped {
                        // A hook on the far side: out of the box near its end,
                        // back along the side, and in again with an arrowhead.
                        let (cend, a) = (nd.pos + nd.box_csize() - 1, l0 + nd.lsize - 2);
                        let b = if self.td { a - 1 } else { (a - 4).max(l0 + 1) };
                        let mut pts = vec![(a, cend), (a, cend + 1), (b, cend + 1)];
                        if !arrow {
                            pts.push((b, cend));
                        }
                        let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                        let net = p.net();
                        p.line(&pts, net, Stroke::light(style), Layer::Main);
                        if arrow {
                            let (ax, ay) = pts[2];
                            p.symbol(ax, ay, if self.td { '◄' } else { '▲' });
                        }
                    }
                    let top = (h - 2 - nd.lines.len() as i64).div_euclid(2);
                    for (i, t) in nd.lines.iter().enumerate() {
                        p.text(x + 1 + (w - 2 - width(t)).div_euclid(2), y + 1 + top + i as i64, t);
                    }
                }
            }
        }

        // Edges. Each piece belongs to the port it hangs off: the vertical out
        // of a node to its out-port, the vertical into a node to its in-port,
        // and a track to the port its fan-out (or fan-in) shares. Pieces of
        // different ports that pass through each other are drawn as crossings.
        let mut nets: HashMap<PortKey, usize> = HashMap::new();
        let mut net = |p: &mut Painter, key: PortKey| *nets.entry(key).or_insert_with(|| p.net());
        let (arrow, real) = (if self.td { '▼' } else { '►' }, self.nodes.len());
        let track = |s: &Seg| self.cstart[s.chan] + self.lead[s.chan] + s.track;
        for (i, s) in self.segs.iter().enumerate() {
            let t = track(s);
            let (start, end) = ((out_l(s.u), s.cs), (in_l(s.v) - i64::from(s.arrow_end), s.ct));
            let pieces = if let Some(e) = s.back {
                // A back edge: out of its source to the outer lane, back along
                // the lane, and in to its target like any other edge.
                let (lane, lane_key) = (self.back_lane[&e], (real + e, Side::In, s.style));
                if s.sk.0 < real {
                    let turn = self.segs.iter().find(|o| o.back == Some(e) && o.sk.0 >= real).map_or(t, track);
                    vec![(s.sk, vec![start, (t, s.cs)]), (lane_key, vec![(t, s.cs), (t, lane), (turn, lane)])]
                } else {
                    vec![(lane_key, vec![(t, lane), (t, s.ct)]), (s.tk, vec![(t, s.ct), end])]
                }
            } else if s.cs == s.ct {
                vec![(s.sk, vec![start, end])]
            } else {
                // A track shared with a segment from another port is a fan-in.
                let fan_in = self.segs.iter().enumerate().any(|(j, o)| {
                    let overlap = s.span().0 <= o.span().1 && o.span().0 <= s.span().1;
                    j != i && o.chan == s.chan && o.track == s.track && o.cs != o.ct && o.sk != s.sk && overlap
                });
                let track_key = if fan_in { s.tk } else { s.sk };
                vec![
                    (s.sk, vec![start, (t, s.cs)]),
                    (track_key, vec![(t, s.cs), (t, s.ct)]),
                    (s.tk, vec![(t, s.ct), end]),
                ]
            };
            for (key, pts) in pieces {
                let id = net(&mut p, key);
                let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                p.line(&pts, id, Stroke::light(s.style), Layer::Main);
            }
            if s.arrow_end {
                let (x, y) = self.phys(in_l(s.v) - 1, s.ct);
                p.symbol(x, y, arrow);
            }
        }
        // Back-edge labels: on the lane in LR, beside it in TD.
        for (&e, &lane) in &self.back_lane {
            let Some(label) = &self.edges[e].label else { continue };
            let text = label.join(" ");
            let ts: Vec<i64> = self.segs.iter().filter(|s| s.back == Some(e)).map(track).collect();
            let mid = (ts.iter().min().unwrap_or(&0) + ts.iter().max().unwrap_or(&0)) / 2;
            let (x, y) = if self.td { (lane + 2, mid) } else { (mid - width(&text) / 2, lane) };
            p.text(x, y, &text);
        }

        // Group frames sit under everything; titles slide past crossing edges.
        for (lane, lo, hi) in self.frames() {
            let members = self.nodes.iter().filter(|n| n.lane == lane && n.kind == Kind::Real);
            let (c0, c1) = members.fold((i64::MAX, i64::MIN), |(a, b), n| (a.min(n.pos), b.max(n.pos + n.csize - 1)));
            let (l0, l1) = (self.bstart[lo] - FRAME, self.bstart[hi] + self.band_len(hi) - 1 + FRAME);
            let ((xa, ya), (xb, yb)) = (self.phys(l0, c0 - FRAME), self.phys(l1, c1 + FRAME));
            let (x0, x1, y0, y1) = (xa.min(xb), xa.max(xb), ya.min(yb), ya.max(yb));
            p.rect((x0, y0), (x1 - x0 + 1, y1 - y0 + 1), Stroke::DOTTED, Corners::Square, Layer::Under);
            let title = format!(" {} ", self.titles[lane - 1]);
            let tw = width(&title);
            let free = |x: i64| (x..x + tw).all(|x| p.is_free((x, y0)));
            // If no gap is wide enough, the title wins over the lines it covers.
            if let Some(x) = (x0 + 1..x1 - tw).find(|&x| free(x)).or((x1 - x0 > tw).then_some(x0 + 1)) {
                p.text(x, y0, &title);
            }
        }
        p.finish()
    }
}
