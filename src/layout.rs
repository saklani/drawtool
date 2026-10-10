//! Flowchart layout: a simplified Sugiyama pipeline, one method per stage.
//!
//! break cycles → layer → dummy nodes → lanes → crossing reduction → sizes →
//! placement (isotonic regression) → ports → back-edge lanes → channel tracks
//! → paint.
//!
//! Coordinates are abstract: `l` runs along the flow (layers) and `c` across
//! it; `phys` maps them to x/y, so TD and LR share every line of layout.
//!
//! Back edges (edges that close a cycle) don't run backwards through the
//! layers. Layering treats them as reversed, so they still shape the layers,
//! but they get no dummies and take no part in ordering. Afterwards each one
//! leaves its source like any edge, turns into an outer lane beyond every node
//! (on whichever side is nearer its ends), runs back along the lane, and
//! enters its target from the channel before it with an ordinary arrowhead.
//! Everything about them is in the "Back edges" section below.

use crate::model::{Direction, Graph, Mark, Shape, Style};
use crate::paint::{Corners, Layer, Painter, Pt, Stroke, Weight};
use crate::text::width;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction::{Incoming, Outgoing};
use std::collections::{HashMap, HashSet};

/// Gap plus dashed line between a group's members and its frame.
const FRAME: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// First and last cross-axis cell, widened by its lane's frame.
    fn framed(&self) -> (i64, i64) {
        (self.pos - margin(self.lane), self.pos + self.csize - 1 + margin(self.lane))
    }
}

/// Room a lane needs on each side for its frame (none outside any group).
fn margin(lane: usize) -> i64 {
    FRAME * i64::from(lane > 0)
}

fn overlaps<T: Ord + Copy>(a: (T, T), b: (T, T)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
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
    /// Layer-ordered nodes: the layering chain (source, label node, target,
    /// reversed for a back edge), then with dummies filled in by `dummies`.
    path: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Side {
    In,
    Out,
}

/// (node, side, style): edges of one style share a port on a box side; a
/// second style gets its own port beside it, so dotted and solid stay apart.
/// Node indices from `nodes.len()` on stand for a back edge's outer lane.
type PortKey = (usize, Side, Style);

/// One edge segment between adjacent layers, `cs` → `ct` across the channel.
struct Seg {
    /// The channel (between layer `chan` and `chan + 1`) it crosses.
    chan: usize,
    /// For the two halves of a back edge: its index in `Layout::backs`.
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

    fn bent(&self) -> bool {
        self.cs != self.ct
    }
}

/// A back edge's route: the half out of its source, the half into its
/// target, and the cross-axis position of the outer lane between them.
struct BackEdge {
    edge: usize,
    out_seg: usize,
    in_seg: usize,
    lane: i64,
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
/// Our own DFS rather than petgraph's: it visits out-edges in source order,
/// which decides which edge of a cycle counts as the back edge.
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

/// Edge crossings between consecutive layers, counted as inversions with a
/// Fenwick tree: with edges sorted by their upper end (ties by lower end),
/// each earlier edge whose lower end lies further right crosses.
fn crossings(layers: &[Vec<usize>], nodes: &[Node]) -> usize {
    let mut total = 0;
    for pair in layers.windows(2) {
        let rank: HashMap<usize, usize> = pair[1].iter().enumerate().map(|(i, &v)| (v, i)).collect();
        let mut es: Vec<(usize, usize)> = pair[0]
            .iter()
            .enumerate()
            .flat_map(|(i, &u)| nodes[u].down.iter().map(move |v| (i, v)))
            .map(|(i, v)| (i, rank[v]))
            .collect();
        es.sort_unstable();
        let mut tree = vec![0usize; pair[1].len() + 1];
        for (seen, &(_, b)) in es.iter().enumerate() {
            let (mut at_most, mut i) = (0, b + 1);
            while i > 0 {
                at_most += tree[i];
                i &= i - 1;
            }
            total += seen - at_most;
            let mut i = b + 1;
            while i < tree.len() {
                tree[i] += 1;
                i += i & i.wrapping_neg();
            }
        }
    }
    total
}

/// Break cycles, turn edge labels into nodes, and assign each node a layer
/// (longest path, then sources pulled down next to their children).
fn layer_graph(graph: &Graph) -> (Vec<Node>, Vec<Edge>) {
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

    // The layering graph: back edges reversed, each label a node of its own
    // between its edge's ends. Node indices match `nodes`.
    let mut g: DiGraph<(), ()> = DiGraph::with_capacity(nodes.len(), edges.len());
    for _ in &nodes {
        g.add_node(());
    }
    for e in &mut edges {
        e.path = if e.rev { vec![e.dst, e.src] } else { vec![e.src, e.dst] };
        if let Some(label) = e.label.as_ref().filter(|_| !e.rev) {
            nodes.push(Node::new(label.clone(), Kind::Label));
            e.path.insert(1, g.add_node(()).index());
        }
        for w in e.path.windows(2) {
            g.add_edge(NodeIndex::new(w[0]), NodeIndex::new(w[1]), ());
        }
    }
    let topo = toposort(&g, None).expect("back edges are reversed, so the layering graph is acyclic");
    let near = |v: NodeIndex, way| g.neighbors_directed(v, way).map(NodeIndex::index);
    for &v in &topo {
        nodes[v.index()].layer = near(v, Incoming).map(|p| nodes[p].layer + 1).max().unwrap_or(0);
    }
    for &v in topo.iter().rev() {
        if near(v, Incoming).next().is_none() && near(v, Outgoing).next().is_some() {
            nodes[v.index()].layer = near(v, Outgoing).map(|s| nodes[s].layer).min().unwrap() - 1;
        }
    }
    (nodes, edges)
}

/// Layer index of every real node, as the layout would assign it. Labelled
/// edges take an extra layer for the label.
#[must_use]
pub fn node_layers(graph: &Graph) -> Vec<usize> {
    layer_graph(graph).0.iter().take(graph.nodes.len()).map(|n| n.layer).collect()
}

#[derive(Default)]
struct Layout {
    /// Subgraph titles.
    titles: Vec<String>,
    td: bool,
    gap: i64,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    layers: Vec<Vec<usize>>,
    /// Lanes in cross-axis order as (lane, first layer, last layer) of their
    /// real nodes, and each node's lane rank (its lane's position in `lanes`).
    lanes: Vec<(usize, usize, usize)>,
    rank: Vec<usize>,
    segs: Vec<Seg>,
    /// The styles of the edges at each box side, in order of appearance.
    groups: HashMap<(usize, Side), Vec<Style>>,
    port: HashMap<PortKey, i64>,
    backs: Vec<BackEdge>,
    /// Per layer: where its band starts, and the first track of the channel after it.
    bstart: Vec<i64>,
    tstart: Vec<i64>,
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
#[must_use]
pub fn render(graph: &Graph, direction: Option<Direction>, ascii: bool) -> String {
    if graph.nodes.is_empty() {
        return String::new();
    }
    let mut l = Layout::new(graph, direction.unwrap_or(graph.direction) == Direction::TD);
    l.dummies();
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
    fn new(graph: &Graph, td: bool) -> Self {
        let (mut nodes, edges) = layer_graph(graph);
        let layers = vec![vec![]; make_room_for_back_edges(&mut nodes, &edges)];
        let titles = graph.groups.iter().map(|g| g.title.clone()).collect();
        let gap = if td { 3 } else { 1 };
        Layout { titles, td, gap, nodes, edges, layers, ..Default::default() }
    }

    /// Space after `a` in its layer: wider between two TD edge labels, so
    /// side-by-side labels don't read as one phrase.
    fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        let labels = |v: usize| self.nodes[v].kind == Kind::Label;
        self.gap + if self.td && labels(a) && b.is_some_and(|&b| labels(b)) { 3 } else { 0 }
    }

    /// Dummy nodes for edges spanning several layers. Long edges of the same
    /// style into the same node share dummies, so they merge into one line as
    /// early as possible instead of each taking its own route.
    fn dummies(&mut self) {
        let mut shared: HashMap<(usize, usize, Style), usize> = HashMap::new();
        let nodes = &mut self.nodes;
        for e in self.edges.iter_mut().filter(|e| !e.rev) {
            let mut path = vec![e.path[0]];
            for w in e.path.windows(2) {
                let bundle = nodes[w[1]].kind == Kind::Real;
                for k in nodes[w[0]].layer + 1..nodes[w[1]].layer {
                    let d = match shared.get(&(w[1], k, e.style)) {
                        Some(&d) if bundle => d,
                        _ => {
                            nodes.push(Node { layer: k, ..Node::new(vec![], Kind::Dummy) });
                            nodes.len() - 1
                        }
                    };
                    if bundle {
                        shared.insert((w[1], k, e.style), d);
                    }
                    path.push(d);
                }
                path.push(w[1]);
            }
            for w in path.windows(2) {
                if !nodes[w[0]].down.contains(&w[1]) {
                    nodes[w[0]].down.push(w[1]);
                    nodes[w[1]].up.push(w[0]);
                }
            }
            // Labels and dummies live in the lane of the edge's source.
            for &v in &path {
                if nodes[v].kind != Kind::Real {
                    nodes[v].lane = nodes[e.src].lane;
                }
            }
            e.path = path;
        }
    }

    /// Lanes, then barycenter sweeps (keeping the ordering with fewest
    /// crossings). Every subgraph is a lane: a strip across the flow only its
    /// members occupy, so its frame can never enclose a stranger.
    fn order(&mut self) {
        for n in self.nodes.iter().filter(|n| n.kind == Kind::Real) {
            match self.lanes.iter_mut().find(|l| l.0 == n.lane) {
                Some(l) => (l.1, l.2) = (l.1.min(n.layer), l.2.max(n.layer)),
                None => self.lanes.push((n.lane, n.layer, n.layer)),
            }
        }
        self.lanes.sort_by_key(|l| l.1);
        self.rank = self.nodes.iter().map(|n| self.lanes.iter().position(|l| l.0 == n.lane).unwrap_or(0)).collect();
        let nl = self.layers.len();
        for v in 0..self.nodes.len() {
            self.layers[self.nodes[v].layer].push(v);
        }
        for lay in &mut self.layers {
            lay.sort_by_key(|&v| self.rank[v]);
        }
        let (mut best, mut best_c) = (self.layers.clone(), crossings(&self.layers, &self.nodes));
        for it in 0..16 {
            let mut rank = self.positions();
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
                keyed.sort_by(|a, b| self.rank[a.1].cmp(&self.rank[b.1]).then(a.0.partial_cmp(&b.0).unwrap()));
                self.layers[k] = keyed.into_iter().map(|(_, v)| v).collect();
            }
            self.transpose();
            let c = crossings(&self.layers, &self.nodes);
            if c < best_c {
                (best, best_c) = (self.layers.clone(), c);
            }
        }
        self.layers = best;
    }

    /// Each node's index within its layer.
    fn positions(&self) -> Vec<usize> {
        let mut pos = vec![0; self.nodes.len()];
        for lay in &self.layers {
            lay.iter().enumerate().for_each(|(i, &v)| pos[v] = i);
        }
        pos
    }

    /// Swap neighbours in a layer (within a lane) while that lowers the
    /// crossings between their edges ("transpose", as in Graphviz dot).
    fn transpose(&mut self) {
        let mut pos = self.positions();
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
                    if self.rank[a] == self.rank[b] && cost(&self.nodes, &pos, b, a) < cost(&self.nodes, &pos, a, b) {
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
        let lanes_from = self.nodes.len();
        for (ei, e) in self.edges.iter().enumerate() {
            let back = e.rev.then_some(self.backs.len());
            let seg =
                |chan, sk, tk, arrow_end| Seg { chan, back, style: e.style, arrow_end, sk, tk, cs: 0, ct: 0, track: 0 };
            let (out, inn) = (|n| (n, Side::Out, e.style), |n| (n, Side::In, e.style));
            if e.rev {
                let (lane, out_seg) = (lanes_from + ei, self.segs.len());
                self.segs.push(seg(self.nodes[e.src].layer, out(e.src), inn(lane), false));
                self.segs.push(seg(self.nodes[e.dst].layer - 1, out(lane), inn(e.dst), e.arrow));
                self.backs.push(BackEdge { edge: ei, out_seg, in_seg: out_seg + 1, lane: 0 });
            } else {
                for (j, w) in e.path.windows(2).enumerate() {
                    let arrow_end = e.arrow && j == e.path.len() - 2;
                    self.segs.push(seg(self.nodes[w[0]].layer, out(w[0]), inn(w[1]), arrow_end));
                }
            }
        }
        for s in &self.segs {
            for (n, side, style) in [s.sk, s.tk].into_iter().filter(|k| k.0 < lanes_from) {
                let styles = self.groups.entry((n, side)).or_default();
                if !styles.contains(&style) {
                    styles.push(style);
                }
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
        self.pack();
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
        let mut extent: Vec<(i64, i64)> = vec![];
        for (r, &(lane, first, last)) in self.lanes.iter().enumerate() {
            let need = (self.lanes.iter().zip(&extent))
                .filter(|(q, _)| overlaps((q.1, q.2), (first, last)))
                .map(|(q, e)| e.1 + 1 + self.gap + margin(q.0) + margin(lane))
                .max();
            let members: Vec<usize> = (0..self.nodes.len()).filter(|&v| self.rank[v] == r).collect();
            let (lo, hi) = (members.iter())
                .map(|&v| (self.nodes[v].pos, self.nodes[v].pos + self.nodes[v].csize - 1))
                .reduce(|(a, b), (lo, hi)| (a.min(lo), b.max(hi)))
                .unwrap();
            let push = need.map_or(0, |n| (n - lo).max(0));
            for &v in &members {
                self.nodes[v].pos += push;
            }
            extent.push((lo + push, hi + push));
        }
        // Pushing a lane can run it into another lane's dummies in a shared layer.
        self.pack();
        let shift = self.nodes.iter().map(|n| n.framed().0).min().unwrap();
        self.nodes.iter_mut().for_each(|n| n.pos -= shift);
    }

    /// Move nodes along their layer until none starts before the one ahead of
    /// it ends (plus spacing). On nodes still at 0, this lays out each layer.
    fn pack(&mut self) {
        for lay in &self.layers {
            for w in lay.windows(2) {
                let floor = self.nodes[w[0]].pos + self.nodes[w[0]].csize + self.space(w[0], Some(&w[1]));
                self.nodes[w[1]].pos = self.nodes[w[1]].pos.max(floor);
            }
        }
    }

    /// Port positions: the first style at the centre, others beside it on
    /// the side facing where their edges go (back edges: the outer lane).
    fn ports(&mut self) {
        let lanes_from = self.nodes.len();
        // Where each port's edges go, on average.
        let mut ends: HashMap<PortKey, (i64, i64)> = HashMap::new();
        for s in &self.segs {
            let other = |n: usize| if n < lanes_from { self.nodes[n].cen() } else { self.lane_hint(n - lanes_from) };
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

    // ---- Back edges ---------------------------------------------------------

    /// A cross-axis point far out on the side a back edge's lane will use,
    /// for choosing which side of a box its ports go on.
    fn lane_hint(&self, edge: usize) -> i64 {
        let e = &self.edges[edge];
        let (src, dst) = (self.nodes[e.src].cen(), self.nodes[e.dst].cen());
        let (lo, hi) = self.nodes.iter().fold((i64::MAX, i64::MIN), |(a, b), n| (a.min(n.pos), b.max(n.pos + n.csize)));
        if src + dst < lo + hi {
            -(1 << 30)
        } else {
            1 << 30
        }
    }

    /// Outer lanes for back edges, beyond every node and frame on the side
    /// nearer the edge's ends; shorter edges run inside longer ones.
    fn back_lanes(&mut self) {
        let lo = self.nodes.iter().map(|n| n.framed().0).min().unwrap_or(0) - 2;
        let hi = self.nodes.iter().map(|n| n.framed().1).max().unwrap_or(0) + 2;
        let (mut near, mut far) = (lo, hi);
        let mut order: Vec<usize> = (0..self.backs.len()).collect();
        order.sort_by_key(|&b| {
            let e = &self.edges[self.backs[b].edge];
            self.nodes[e.src].layer - self.nodes[e.dst].layer
        });
        for b in order {
            let e = &self.edges[self.backs[b].edge];
            let low = self.nodes[e.src].cen() + self.nodes[e.dst].cen() < lo + hi;
            // In TD the label sits beside the (vertical) lane, so leave room.
            let label = e.label.as_ref().map_or(0, |l| width(&l.join(" ")) + 1);
            let step = 2 + if self.td { label } else { 0 };
            let (next, step) = if low { (&mut near, -step) } else { (&mut far, step) };
            let lane = *next;
            *next += step;
            let back = &mut self.backs[b];
            back.lane = if low && self.td { lane - label } else { lane };
            self.segs[back.out_seg].ct = back.lane;
            self.segs[back.in_seg].cs = back.lane;
        }
    }

    // ---- Routing ------------------------------------------------------------

    /// Give bent segments a track (a row/column of their channel) so that
    /// unrelated horizontals never share one.
    fn tracks(&mut self) {
        let segs = &self.segs;
        // Segments overlapping on a track must all share a source or a target port.
        let compatible = |s: usize, track: &[usize]| {
            let (mut comp, mut rest) = (vec![s], track.to_vec());
            while let Some(i) =
                rest.iter().position(|&f| comp.iter().any(|&c| overlaps(segs[f].span(), segs[c].span())))
            {
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
        let mut track_of: Vec<Option<usize>> = vec![None; segs.len()];
        for chan in 0..self.layers.len() {
            let mut bent: Vec<usize> = (0..segs.len()).filter(|&i| segs[i].bent() && segs[i].chan == chan).collect();
            bent.sort_by_key(|&i| segs[i].span());
            // A segment leaving column x must turn before one arriving at column x.
            let preds = |j: usize| {
                let b = &segs[j];
                bent.iter().copied().filter(move |&i| segs[i].cs == b.ct && segs[i].sk != b.sk && segs[i].tk != b.tk)
            };
            let (mut remaining, mut tracks): (Vec<usize>, Vec<Vec<usize>>) = (bent.clone(), vec![]);
            while !remaining.is_empty() {
                let waiting = |i: usize| preds(i).filter(|p| remaining.contains(p)).count();
                let mut ready: Vec<usize> = remaining.iter().copied().filter(|&i| waiting(i) == 0).collect();
                if ready.is_empty() {
                    ready = vec![*remaining.iter().min_by_key(|&&i| waiting(i)).unwrap()];
                }
                // Within a ready batch, put first (higher) the segment whose
                // verticals cross the fewest horizontals of the others; then
                // give it the first compatible track below its predecessors.
                while !ready.is_empty() {
                    let score = |i: usize| remaining.iter().map(|&r| above(i, r) - above(r, i)).sum::<i64>();
                    let i = ready.remove((0..ready.len()).min_by_key(|&k| score(ready[k])).unwrap());
                    remaining.retain(|&r| r != i);
                    let mut t = preds(i).filter_map(|p| track_of[p]).map(|t| t + 1).max().unwrap_or(0);
                    while t < tracks.len() && !compatible(i, &tracks[t]) {
                        t += 1;
                    }
                    if t == tracks.len() {
                        tracks.push(vec![]);
                    }
                    tracks[t].push(i);
                    track_of[i] = Some(t);
                }
            }
        }
        for (s, t) in self.segs.iter_mut().zip(track_of) {
            s.track = t.map_or(s.track, |t| t as i64);
        }
    }

    /// The real-node layer span of each framed lane.
    fn frames(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.lanes.iter().copied().filter(|l| l.0 > 0)
    }

    fn band_len(&self, k: usize) -> i64 {
        self.layers[k].iter().map(|&v| self.nodes[v].lsize).max().unwrap_or(0)
    }

    /// Layer-axis coordinates. Channels next to a frame get FRAME extra cells.
    fn coordinates(&mut self) {
        let mut ntracks = vec![0i64; self.layers.len()];
        for s in self.segs.iter().filter(|s| s.bent()) {
            ntracks[s.chan] = ntracks[s.chan].max(s.track + 1);
        }
        let mut cur = if self.frames().any(|f| f.1 == 0) { FRAME } else { 0 };
        for (k, ntracks) in ntracks.into_iter().enumerate() {
            self.bstart.push(cur);
            cur += self.band_len(k);
            let lead = i64::from(!self.td) + if self.frames().any(|f| f.2 == k) { FRAME } else { 0 };
            let before_tail = if self.frames().any(|f| f.1 == k + 1) { FRAME } else { 0 };
            self.tstart.push(cur + lead);
            cur += lead + ntracks + before_tail + 1;
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
            let (x, y) = self.phys(l0, nd.pos);
            let (w, h) = if self.td { (nd.box_csize(), nd.lsize) } else { (nd.lsize, nd.box_csize()) };
            if nd.kind == Kind::Real {
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
                    let (cap, side) = (Stroke::new(Weight::Double, Style::Solid), Stroke::LIGHT);
                    for (a, b, stroke) in [
                        ((x, y), (x1, y), cap),
                        ((x, y1), (x1, y1), cap),
                        ((x, y), (x, y1), side),
                        ((x1, y), (x1, y1), side),
                    ] {
                        let net = p.net();
                        p.line(&[a, b], net, stroke, Layer::Main);
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
                        p.symbol(pts[2].0, pts[2].1, if self.td { '◄' } else { '▲' });
                    }
                }
            }
            // Text is centred in the box, inside its border; labels have none.
            let inset = i64::from(nd.kind == Kind::Real);
            let top = (h - 2 * inset - nd.lines.len() as i64).div_euclid(2);
            for (i, t) in nd.lines.iter().enumerate() {
                p.text(x + inset + (w - 2 * inset - width(t)).div_euclid(2), y + inset + top + i as i64, t);
            }
        }

        // Edges. Each piece belongs to the port it hangs off: the vertical out
        // of a node to its out-port, the vertical into a node to its in-port,
        // and a track to the port its fan-out (or fan-in) shares. Pieces of
        // different ports that pass through each other are drawn as crossings.
        let mut nets: HashMap<PortKey, usize> = HashMap::new();
        let mut net = |p: &mut Painter, key: PortKey| *nets.entry(key).or_insert_with(|| p.net());
        let (arrow, lanes_from) = (if self.td { '▼' } else { '►' }, self.nodes.len());
        let track = |s: &Seg| self.tstart[s.chan] + s.track;
        for (i, s) in self.segs.iter().enumerate() {
            let t = track(s);
            // Back edges have a lane, not a node, at one end, so only look up the other.
            let start = || (out_l(s.sk.0), s.cs);
            let end = || (in_l(s.tk.0) - i64::from(s.arrow_end), s.ct);
            let pieces = if let Some(b) = s.back {
                // A back edge: out of its source to the outer lane, back along
                // the lane, and in to its target like any other edge.
                let back = &self.backs[b];
                let lane_key = (lanes_from + back.edge, Side::In, s.style);
                if i == back.out_seg {
                    let turn = track(&self.segs[back.in_seg]);
                    vec![
                        (s.sk, vec![start(), (t, s.cs)]),
                        (lane_key, vec![(t, s.cs), (t, back.lane), (turn, back.lane)]),
                    ]
                } else {
                    vec![(lane_key, vec![(t, back.lane), (t, s.ct)]), (s.tk, vec![(t, s.ct), end()])]
                }
            } else if !s.bent() {
                vec![(s.sk, vec![start(), end()])]
            } else {
                // A track shared with a segment from another port is a fan-in.
                let fan_in = self.segs.iter().any(|o| {
                    o.bent() && (o.chan, o.track) == (s.chan, s.track) && o.sk != s.sk && overlaps(o.span(), s.span())
                });
                let track_key = if fan_in { s.tk } else { s.sk };
                vec![
                    (s.sk, vec![start(), (t, s.cs)]),
                    (track_key, vec![(t, s.cs), (t, s.ct)]),
                    (s.tk, vec![(t, s.ct), end()]),
                ]
            };
            for (key, pts) in pieces {
                let id = net(&mut p, key);
                let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                p.line(&pts, id, Stroke::light(s.style), Layer::Main);
            }
            if s.arrow_end {
                let (x, y) = self.phys(in_l(s.tk.0) - 1, s.ct);
                p.symbol(x, y, arrow);
            }
        }
        // Back-edge labels: on the lane in LR, beside it in TD.
        for back in &self.backs {
            let Some(label) = &self.edges[back.edge].label else { continue };
            let text = label.join(" ");
            let (a, b) = (track(&self.segs[back.out_seg]), track(&self.segs[back.in_seg]));
            let mid = (a.min(b) + a.max(b)) / 2;
            let (x, y) = if self.td { (back.lane + 2, mid) } else { (mid - width(&text) / 2, back.lane) };
            p.text(x, y, &text);
        }

        // Group frames sit under everything; titles slide past crossing edges.
        for (lane, lo, hi) in self.frames() {
            let members = self.nodes.iter().filter(|n| n.lane == lane && n.kind == Kind::Real);
            let (c0, c1) = members.map(Node::framed).reduce(|(a, b), (c, d)| (a.min(c), b.max(d))).unwrap();
            let (l0, l1) = (self.bstart[lo] - FRAME, self.bstart[hi] + self.band_len(hi) - 1 + FRAME);
            let ((xa, ya), (xb, yb)) = (self.phys(l0, c0), self.phys(l1, c1));
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

/// Back edges enter their target from the channel before it and leave their
/// source into the channel after it. Shift every layer down when one targets
/// the first layer, and add an empty layer when one leaves the last. Returns
/// the number of layers.
fn make_room_for_back_edges(nodes: &mut [Node], edges: &[Edge]) -> usize {
    if edges.iter().any(|e| e.rev && nodes[e.dst].layer == 0) {
        for n in nodes.iter_mut() {
            n.layer += 1;
        }
    }
    let last = nodes.iter().map(|n| n.layer).max().unwrap_or(0);
    last + 1 + usize::from(edges.iter().any(|e| e.rev && nodes[e.src].layer == last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn graph(src: &str) -> Graph {
        parse(&format!("flowchart TD\n{src}")).unwrap()
    }

    /// The index of the node with this id.
    fn id(g: &Graph, name: &str) -> usize {
        g.nodes.iter().position(|n| n.id == name).unwrap()
    }

    fn node(pos: i64, csize: i64, layer: usize) -> Node {
        Node { pos, csize, layer, lsize: 3, ..Node::new(vec!["x".into()], Kind::Real) }
    }

    fn edge(src: usize, dst: usize, style: Style, rev: bool) -> Edge {
        Edge { src, dst, label: None, style, arrow: true, rev, path: vec![] }
    }

    fn seg(sk: PortKey, tk: PortKey, cs: i64, ct: i64) -> Seg {
        let style = sk.2;
        Seg { chan: 0, back: None, style, arrow_end: false, sk, tk, cs, ct, track: 0 }
    }

    #[test]
    fn break_cycles_marks_only_the_closing_edge() {
        let g = graph("a --> b\nb --> c\nc --> a\na --> d\nd --> c");
        let (_, edges) = layer_graph(&g);
        let rev: Vec<(usize, usize)> = edges.iter().filter(|e| e.rev).map(|e| (e.src, e.dst)).collect();
        assert_eq!(rev, [(id(&g, "c"), id(&g, "a"))]);
    }

    #[test]
    fn layers_of_a_chain_and_a_diamond() {
        let g = graph("a --> b\nb --> c");
        assert_eq!(node_layers(&g), [0, 1, 2]);
        let g = graph("a --> b\na --> c\nb --> d\nc --> d");
        let layers = node_layers(&g);
        assert_eq!([layers[id(&g, "a")], layers[id(&g, "b")], layers[id(&g, "c")], layers[id(&g, "d")]], [0, 1, 1, 2]);
        // A source is pulled down next to its child; a label takes a layer.
        let g = graph("a --> b\nb --> c\nx --> c\nc -->|yes| d");
        let layers = node_layers(&g);
        assert_eq!([layers[id(&g, "x")], layers[id(&g, "d")]], [1, 4]);
    }

    #[test]
    fn long_edges_into_one_node_share_dummies_by_style() {
        let g = graph("a --> b\nb --> c\nc --> d\na --> d\nz --> b\nz --> d\nz -.-> d");
        let mut l = Layout::new(&g, true);
        l.dummies();
        let inner = |e: &Edge| e.path[1..e.path.len() - 1].to_vec();
        let (a_d, z_d, z_dotted) = (&l.edges[3], &l.edges[5], &l.edges[6]);
        assert_eq!(inner(a_d).len(), 2);
        assert_eq!(inner(a_d), inner(z_d));
        assert!(inner(z_dotted).iter().all(|d| !inner(a_d).contains(d)));
        assert_eq!(l.nodes.iter().filter(|n| n.kind == Kind::Dummy).count(), 4);
    }

    #[test]
    fn transpose_removes_a_simple_crossing() {
        let g = graph("a --> c\nb --> d");
        let (a, b, c, d) = (id(&g, "a"), id(&g, "b"), id(&g, "c"), id(&g, "d"));
        let mut l = Layout::new(&g, true);
        l.dummies();
        l.rank = vec![0; l.nodes.len()];
        l.layers = vec![vec![a, b], vec![d, c]];
        assert_eq!(crossings(&l.layers, &l.nodes), 1);
        l.transpose();
        assert_eq!(crossings(&l.layers, &l.nodes), 0);
    }

    #[test]
    fn crossings_count_every_pair_once() {
        // Each of a's two edges crosses each of b's: four crossings.
        let g = graph("a --> d\na --> e\nb --> c\nb --> f");
        let mut l = Layout::new(&g, true);
        l.dummies();
        let n = |s| id(&g, s);
        l.layers = vec![vec![n("a"), n("b")], vec![n("c"), n("f"), n("d"), n("e")]];
        assert_eq!(crossings(&l.layers, &l.nodes), 4);
    }

    #[test]
    fn pava_pools_adjacent_violators() {
        assert_eq!(pava(&[1.0, 2.0, 3.0], &[1.0; 3]), [1.0, 2.0, 3.0]);
        assert_eq!(pava(&[1.0, 3.0, 2.0, 4.0], &[1.0; 4]), [1.0, 2.5, 2.5, 4.0]);
        // Weighted, and a later block pulling an earlier pool down with it.
        assert_eq!(pava(&[3.0, 2.0, 0.0], &[1.0, 1.0, 2.0]), [1.25; 3]);
        assert_eq!(pava(&[4.0, 1.0], &[3.0, 1.0]), [3.25; 2]);
    }

    #[test]
    fn tracks_put_the_crossing_free_order_first() {
        // By span, `a` (0..10) comes first, but if it took the upper track its
        // way down (at 10) would cross b's track and b's way up (at 5) a's.
        let mut l = Layout { layers: vec![vec![], vec![]], ..Default::default() };
        let key = |n, side| (n, side, Style::Solid);
        l.segs.push(seg(key(0, Side::Out), key(1, Side::In), 0, 10));
        l.segs.push(seg(key(2, Side::Out), key(3, Side::In), 5, 20));
        l.tracks();
        assert_eq!((l.segs[0].track, l.segs[1].track), (1, 0));
    }

    #[test]
    fn ports_put_the_straightest_style_at_the_centre() {
        // `a` (centre 5) has a dotted edge to far-off `c`, listed first, and a
        // solid one to `b` straight below.
        let mut l = Layout { nodes: vec![node(0, 11, 0), node(0, 11, 1), node(40, 11, 1)], ..Default::default() };
        l.groups.insert((0, Side::Out), vec![Style::Dotted, Style::Solid]);
        l.segs.push(seg((0, Side::Out, Style::Dotted), (2, Side::In, Style::Dotted), 0, 0));
        l.segs.push(seg((0, Side::Out, Style::Solid), (1, Side::In, Style::Solid), 0, 0));
        l.ports();
        assert_eq!(l.port[&(0, Side::Out, Style::Solid)], 5);
        assert!(l.port[&(0, Side::Out, Style::Dotted)] > 5, "the dotted port leans toward c");
    }

    #[test]
    fn back_lanes_take_the_nearer_side() {
        // Two cycles, one at each end of the cross axis (0..55).
        let nodes = vec![node(0, 5, 1), node(0, 5, 2), node(50, 5, 1), node(50, 5, 2)];
        let edges = vec![edge(1, 0, Style::Solid, true), edge(3, 2, Style::Solid, true)];
        let mut l = Layout { nodes, edges, ..Default::default() };
        l.segments();
        l.back_lanes();
        let lanes: Vec<i64> = l.backs.iter().map(|b| b.lane).collect();
        assert_eq!(lanes, [-2, 56]);
        for b in &l.backs {
            assert_eq!((l.segs[b.out_seg].ct, l.segs[b.in_seg].cs), (b.lane, b.lane));
        }
    }
}
