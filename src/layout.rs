//! Flowchart layout: a simplified Sugiyama pipeline, one method per stage.
//!
//! break cycles → layer → dummy nodes → lanes → crossing reduction → sizes →
//! placement (isotonic regression) → ports → back-edge lanes → channel tracks
//! → paint.
//!
//! The stages are grouped into phases, and a `Layout<Phase>` offers only the
//! stages of its phase and the step to the next, so they can't run out of
//! order: `Layout::new(..).order().place().route().paint(..)`.
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

use crate::model::{Arrow, Direction, EdgeDef, Graph, Mark, Shape, Style};
use crate::paint::{Corners, Layer, Painter, Pt, Stroke, Weight};
use crate::text::width;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, EdgeIndex, EdgeReference, NodeIndex};
use petgraph::visit::{
    depth_first_search, DfsEvent, EdgeFiltered, EdgeRef, GraphBase, GraphRef, IntoEdgesDirected, IntoNeighbors,
    Visitable,
};
use petgraph::Direction::{Incoming, Outgoing};
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

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
    arrow: Arrow,
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

/// (node, side, (style, head)): edges of one style share a port on a box
/// side; a second style gets its own port beside it, so dotted and solid stay
/// apart. So does the start of a two-headed edge (`head`), whose arrowhead
/// into its source must not mark the other edges leaving it. Node indices
/// from `nodes.len()` on stand for a back edge's outer lane.
type PortKey = (usize, Side, (Style, bool));

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

    /// Whether it starts with a two-headed edge's head into its source.
    fn head(&self) -> bool {
        let (_, _, (_, head)) = self.sk;
        head
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

/// An edge between two boxes of one stack. Neighbours in the stack are
/// joined by a short straight line across the gap between them; others by a
/// bracket on the far side of the stack.
struct StackEdge {
    src: usize,
    dst: usize,
    label: Option<Vec<String>>,
    style: Style,
    arrow: Arrow,
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

/// `graph` with each node's out-edges in the order they were written:
/// petgraph lists the newest first, and for a depth-first search the order
/// decides which edge of a cycle is the one that closes it.
struct SourceOrder<'a, N>(&'a DiGraph<N, EdgeDef>);

impl<N> Clone for SourceOrder<'_, N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N> Copy for SourceOrder<'_, N> {}

impl<N> GraphBase for SourceOrder<'_, N> {
    type NodeId = NodeIndex;
    type EdgeId = EdgeIndex;
}

impl<N> Visitable for SourceOrder<'_, N> {
    type Map = <DiGraph<N, EdgeDef> as Visitable>::Map;
    fn visit_map(&self) -> Self::Map {
        self.0.visit_map()
    }
    fn reset_map(&self, map: &mut Self::Map) {
        self.0.reset_map(map);
    }
}

impl<N> GraphRef for SourceOrder<'_, N> {}

impl<N> IntoNeighbors for SourceOrder<'_, N> {
    type Neighbors = std::iter::Rev<std::vec::IntoIter<NodeIndex>>;
    fn neighbors(self, v: NodeIndex) -> Self::Neighbors {
        self.0.neighbors(v).collect::<Vec<_>>().into_iter().rev()
    }
}

/// What layout takes from a graph's connections alone. Wrapping labels
/// doesn't change it, so it is worked out once per graph.
pub(crate) struct Structure {
    /// For each edge, whether it closes a cycle (it is drawn as a back edge).
    back: Vec<bool>,
    /// Each node's layer.
    pub layer: Vec<usize>,
}

/// Find the back edges, then layer the nodes. A stack counts as one node:
/// edges into or out of any of its boxes are its edges, and edges between
/// its boxes are self-loops, which neither step counts.
pub(crate) fn structure(graph: &Graph) -> Structure {
    if graph.stacks.is_empty() {
        let back = back_edges(&graph.g);
        return Structure { layer: layers(&graph.g, &back), back };
    }
    let mut node: Vec<Option<NodeIndex>> = vec![None; graph.g.node_count()];
    let mut contracted: DiGraph<(), EdgeDef> = DiGraph::new();
    for v in graph.g.node_indices() {
        if node[v.index()].is_none() {
            let c = contracted.add_node(());
            let stack = graph.stacks.iter().find(|st| st.contains(&v));
            for m in stack.map_or(&[v][..], Vec::as_slice) {
                node[m.index()] = Some(c);
            }
        }
    }
    let node: Vec<NodeIndex> = node.into_iter().flatten().collect();
    for e in graph.g.edge_references() {
        contracted.add_edge(node[e.source().index()], node[e.target().index()], e.weight().clone());
    }
    let back = back_edges(&contracted);
    let layer = layers(&contracted, &back);
    Structure { layer: node.iter().map(|c| layer[c.index()]).collect(), back }
}

/// For each edge, whether it closes a cycle: depth-first search, from the
/// sources first and then from any node not yet reached, finds it leading
/// back to a node still on the search path. Self-loops don't count.
fn back_edges<N>(g: &DiGraph<N, EdgeDef>) -> Vec<bool> {
    let source = |v: &NodeIndex| g.edges_directed(*v, Incoming).all(|e| e.source() == *v);
    let mut back = HashSet::new();
    depth_first_search(SourceOrder(g), g.node_indices().filter(source).chain(g.node_indices()), |event| {
        if let DfsEvent::BackEdge(u, v) = event {
            back.insert((u, v));
        }
    });
    g.edge_references().map(|e| e.source() != e.target() && back.contains(&(e.source(), e.target()))).collect()
}

/// Each node's layer: the longest path to it, ignoring back edges and
/// self-loops, where a labelled edge spans two layers (its label sits in the
/// one between); then sources are pulled down next to their nearest child.
fn layers<N>(g: &DiGraph<N, EdgeDef>, back: &[bool]) -> Vec<usize> {
    let forward = EdgeFiltered::from_fn(g, |e| !back[e.id().index()] && e.source() != e.target());
    let topo = toposort(&forward, None).expect("without back edges and self-loops the graph is acyclic");
    let span = |e: &EdgeReference<EdgeDef>| 1 + usize::from(e.weight().label.is_some());
    let mut layer = vec![0; g.node_count()];
    for &v in &topo {
        let after = forward.edges_directed(v, Incoming).map(|e| layer[e.source().index()] + span(&e));
        layer[v.index()] = after.max().unwrap_or(0);
    }
    for &v in topo.iter().rev() {
        if forward.edges_directed(v, Incoming).next().is_none() {
            // A labelled edge's first child is its label, one layer on.
            let (here, out) = (layer[v.index()], forward.edges_directed(v, Outgoing));
            let child = |e: EdgeReference<EdgeDef>| if span(&e) == 2 { here + 1 } else { layer[e.target().index()] };
            if let Some(nearest) = out.map(child).min() {
                layer[v.index()] = nearest - 1;
            }
        }
    }
    layer
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

/// Layout's own nodes and edges, with the layers and back edges of `s`, and
/// each edge label a node of its own. Edges between the boxes of a stack are
/// set aside: they are drawn across the flow, not through the layers.
fn layer_graph(graph: &Graph, s: &Structure) -> (Vec<Node>, Vec<Edge>, Vec<StackEdge>) {
    let mut nodes: Vec<Node> = (graph.g.node_weights().zip(&s.layer))
        .map(|(n, &layer)| Node {
            shape: n.shape,
            mark: n.mark,
            layer,
            lane: n.group.map_or(0, |g| g + 1),
            ..Node::new(n.lines.clone(), Kind::Real)
        })
        .collect();
    let stack_of = |v: usize| graph.stacks.iter().position(|st| st.iter().any(|m| m.index() == v));
    let (mut edges, mut stacked) = (Vec::new(), Vec::new());
    for e in graph.g.edge_references() {
        let (src, dst, rev, d) = (e.source().index(), e.target().index(), s.back[e.id().index()], e.weight());
        if src == dst {
            // Both ends are the same box, so a second head would add nothing.
            nodes[src].looped = Some((d.style, d.arrow != Arrow::None));
            continue;
        }
        if stack_of(src).is_some() && stack_of(src) == stack_of(dst) {
            stacked.push(StackEdge { src, dst, label: d.label.clone(), style: d.style, arrow: d.arrow });
            continue;
        }
        // The layering chain: source, label node, target (reversed for a back edge).
        let mut path = if rev { vec![dst, src] } else { vec![src, dst] };
        if let Some(label) = d.label.as_ref().filter(|_| !rev) {
            nodes.push(Node { layer: nodes[src].layer + 1, ..Node::new(label.clone(), Kind::Label) });
            path.insert(1, nodes.len() - 1);
        }
        edges.push(Edge { src, dst, label: d.label.clone(), style: d.style, arrow: d.arrow, rev, path });
    }
    (nodes, edges, stacked)
}

// Phases of a `Layout`, named for what has been decided so far.
/// Layers, with dummy nodes on long edges.
#[derive(Default)]
struct Layered;
/// The order of the nodes within each layer, and the lanes.
#[derive(Default)]
struct Ordered;
/// Edge segments, box sizes, and positions across the flow.
#[derive(Default)]
struct Placed;
/// Every segment's route: ports, back-edge lanes and channel tracks.
#[derive(Default)]
struct Routed;

#[derive(Default)]
struct Layout<Phase> {
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
    /// The port kinds (see [`PortKey`]) at each box side, in order of appearance.
    groups: HashMap<(usize, Side), Vec<(Style, bool)>>,
    port: HashMap<PortKey, i64>,
    backs: Vec<BackEdge>,
    /// Each stack's boxes, in order across the flow; for each stacked box,
    /// its stack and place in it; and the edges between stacked boxes.
    stacks: Vec<Vec<usize>>,
    stacked: HashMap<usize, (usize, usize)>,
    stack_edges: Vec<StackEdge>,
    /// Per layer: where its band starts, and the first track of the channel after it.
    bstart: Vec<i64>,
    tstart: Vec<i64>,
    phase: PhantomData<Phase>,
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
#[must_use]
pub fn render(graph: &Graph, direction: Option<Direction>, ascii: bool) -> String {
    draw(graph, &structure(graph), direction, ascii)
}

/// [`render`] with the graph's structure already worked out.
pub(crate) fn draw(graph: &Graph, s: &Structure, direction: Option<Direction>, ascii: bool) -> String {
    scene(graph, s, direction, ascii).map_or_else(String::new, |s| s.painter.finish())
}

/// A laid-out graph before it becomes text: what was painted, and each
/// node's box (top-left, size) by its index in the graph.
pub(crate) struct Scene {
    pub painter: Painter,
    pub boxes: Vec<(Pt, Pt)>,
}

/// Lay out and paint `graph`, whose structure is `s`; `None` if it has no nodes.
pub(crate) fn scene(graph: &Graph, s: &Structure, direction: Option<Direction>, ascii: bool) -> Option<Scene> {
    if graph.g.node_count() == 0 {
        return None;
    }
    let td = direction.unwrap_or(graph.direction) == Direction::TD;
    Some(Layout::new(graph, s, td).order().place().route().paint(ascii))
}

impl<P> Layout<P> {
    /// The same layout, in the next phase.
    fn to<Next>(self) -> Layout<Next> {
        Layout {
            titles: self.titles,
            td: self.td,
            gap: self.gap,
            nodes: self.nodes,
            edges: self.edges,
            layers: self.layers,
            lanes: self.lanes,
            rank: self.rank,
            segs: self.segs,
            groups: self.groups,
            port: self.port,
            backs: self.backs,
            stacks: self.stacks,
            stacked: self.stacked,
            stack_edges: self.stack_edges,
            bstart: self.bstart,
            tstart: self.tstart,
            phase: PhantomData,
        }
    }

    // ---- Stacks -------------------------------------------------------------

    /// `lay` with each stack's boxes together, in stack order, where the
    /// first of them was.
    fn glue(&self, lay: &[usize]) -> Vec<usize> {
        let mut out = Vec::with_capacity(lay.len());
        for &v in lay {
            match self.stacked.get(&v) {
                None => out.push(v),
                Some(&(s, _)) if !out.contains(&self.stacks[s][0]) => out.extend(&self.stacks[s]),
                Some(_) => {}
            }
        }
        out
    }

    /// Whether `b` directly follows `a` in a stack.
    fn follows(&self, a: usize, b: usize) -> bool {
        matches!((self.stacked.get(&a), self.stacked.get(&b)), (Some(&(s, i)), Some(&(t, j))) if s == t && j == i + 1)
    }

    /// The stack edges between `a` and `b`, either way.
    fn between(&self, a: usize, b: usize) -> impl Iterator<Item = &StackEdge> {
        self.stack_edges.iter().filter(move |e| (e.src, e.dst) == (a, b) || (e.src, e.dst) == (b, a))
    }

    /// A stack edge's ends in stack order: (upper, lower) in LR.
    fn ends(&self, e: &StackEdge) -> (usize, usize) {
        if self.stacked[&e.src].1 < self.stacked[&e.dst].1 {
            (e.src, e.dst)
        } else {
            (e.dst, e.src)
        }
    }

    /// Whether a stack edge skips over boxes, so it is drawn as a bracket.
    fn bracket(&self, e: &StackEdge) -> bool {
        let (a, b) = self.ends(e);
        self.stacked[&b].1 > self.stacked[&a].1 + 1
    }

    /// The cross-axis cell where a bracket meets box `n`, on its far side:
    /// near the end of the upper box and the start of the lower one.
    fn bracket_at(&self, n: usize, upper: bool) -> i64 {
        let nd = &self.nodes[n];
        if upper {
            nd.pos + nd.box_csize() - 2
        } else {
            nd.pos + 1
        }
    }
}

impl Layout<Layered> {
    fn new(graph: &Graph, s: &Structure, td: bool) -> Self {
        let (mut nodes, edges, stack_edges) = layer_graph(graph, s);
        let layers = vec![vec![]; make_room_for_back_edges(&mut nodes, &edges)];
        let titles = graph.groups.iter().map(|g| g.title.clone()).collect();
        let gap = if td { 3 } else { 1 };
        let stacks: Vec<Vec<usize>> = graph.stacks.iter().map(|st| st.iter().map(|v| v.index()).collect()).collect();
        let stacked = (stacks.iter().enumerate())
            .flat_map(|(s, st)| st.iter().enumerate().map(move |(i, &v)| (v, (s, i))))
            .collect();
        let mut l =
            Layout { titles, td, gap, nodes, edges, layers, stacks, stacked, stack_edges, ..Default::default() };
        l.dummies();
        l
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
    fn order(mut self) -> Layout<Ordered> {
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
        for k in 0..nl {
            self.layers[k].sort_by_key(|&v| self.rank[v]);
            let lay = self.glue(&self.layers[k]);
            self.layers[k] = lay;
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
                // A stack moves as one, at the mean of its boxes' keys.
                for st in &self.stacks {
                    let ks: Vec<f64> = keyed.iter().filter(|(_, v)| st.contains(v)).map(|k| k.0).collect();
                    let mean = ks.iter().sum::<f64>() / ks.len() as f64;
                    keyed.iter_mut().filter(|(_, v)| st.contains(v)).for_each(|k| k.0 = mean);
                }
                keyed.sort_by(|a, b| self.rank[a.1].cmp(&self.rank[b.1]).then(a.0.partial_cmp(&b.0).unwrap()));
                let lay: Vec<usize> = keyed.into_iter().map(|(_, v)| v).collect();
                self.layers[k] = self.glue(&lay);
            }
            self.transpose();
            let c = crossings(&self.layers, &self.nodes);
            if c < best_c {
                (best, best_c) = (self.layers.clone(), c);
            }
        }
        self.layers = best;
        self.to()
    }

    /// Each node's index within its layer.
    fn positions(&self) -> Vec<usize> {
        let mut pos = vec![0; self.nodes.len()];
        for lay in &self.layers {
            lay.iter().enumerate().for_each(|(i, &v)| pos[v] = i);
        }
        pos
    }

    /// Swap neighbours in a layer (within a lane, never a stacked box) while
    /// that lowers the crossings between their edges ("transpose", as in
    /// Graphviz dot).
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
                    let free = !self.stacked.contains_key(&a) && !self.stacked.contains_key(&b);
                    if free
                        && self.rank[a] == self.rank[b]
                        && cost(&self.nodes, &pos, b, a) < cost(&self.nodes, &pos, a, b)
                    {
                        lay.swap(i - 1, i);
                        (pos[a], pos[b]) = (i, i - 1);
                        improved = true;
                    }
                }
            }
        }
    }
}

impl Layout<Ordered> {
    /// Segments first: a box's size depends on its ports.
    fn place(mut self) -> Layout<Placed> {
        self.segments();
        self.sizes();
        self.position();
        self.to()
    }

    /// Space after `a` in its layer: wider between two TD edge labels, so
    /// side-by-side labels don't read as one phrase.
    fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        if let Some(&b) = b.filter(|&&b| self.follows(a, b)) {
            return self.stack_gap(a, b);
        }
        let labels = |v: usize| self.nodes[v].kind == Kind::Label;
        self.gap + if self.td && labels(a) && b.is_some_and(|&b| labels(b)) { 3 } else { 0 }
    }

    /// The gap between neighbours in a stack: room for the lines between
    /// them and their labels (beside the lines in LR, above them in TD).
    fn stack_gap(&self, a: usize, b: usize) -> i64 {
        if self.between(a, b).next().is_none() {
            return self.gap;
        }
        let labels = self.between(a, b).filter_map(|e| e.label.as_ref());
        if self.td {
            labels.map(|l| width(&l.join(" ")) + 2).max().unwrap_or(0).max(3)
        } else {
            // A two-headed line needs a cell of line between its heads.
            let both = self.between(a, b).any(|e| e.arrow == Arrow::Both);
            labels.map(|l| l.len() as i64).max().unwrap_or(0).max(2 + i64::from(both))
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
            // Only the edge's first segment starts at its source, so only it
            // carries the head of a two-headed edge there.
            let head = |first: bool| first && e.arrow == Arrow::Both;
            let (out, inn) = (|n, first| (n, Side::Out, (e.style, head(first))), |n| (n, Side::In, (e.style, false)));
            let arrow = e.arrow != Arrow::None;
            if e.rev {
                let (lane, out_seg) = (lanes_from + ei, self.segs.len());
                self.segs.push(seg(self.nodes[e.src].layer, out(e.src, true), inn(lane), false));
                self.segs.push(seg(self.nodes[e.dst].layer - 1, out(lane, false), inn(e.dst), arrow));
                self.backs.push(BackEdge { edge: ei, out_seg, in_seg: out_seg + 1, lane: 0 });
            } else {
                for (j, w) in e.path.windows(2).enumerate() {
                    let arrow_end = arrow && j == e.path.len() - 2;
                    self.segs.push(seg(self.nodes[w[0]].layer, out(w[0], j == 0), inn(w[1]), arrow_end));
                }
            }
        }
        for s in &self.segs {
            for (n, side, kind) in [s.sk, s.tk].into_iter().filter(|k| k.0 < lanes_from) {
                let kinds = self.groups.entry((n, side)).or_default();
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
        }
    }

    /// Boxes are wide (and in LR tall) enough for their text and ports. A
    /// stacked box is long enough along the flow for the lines to each
    /// neighbour side by side, and a bracket's end leaves its middle free.
    fn sizes(&mut self) {
        for v in 0..self.nodes.len() {
            let ports = |side| self.groups.get(&(v, side)).map_or(0, Vec::len) as i64;
            let np = ports(Side::In).max(ports(Side::Out));
            let (mut along, mut across) = (0, 0);
            if let Some(&(s, i)) = self.stacked.get(&v) {
                let st = &self.stacks[s];
                let next = [i.checked_sub(1).map(|j| st[j]), st.get(i + 1).copied()];
                let lines = next.into_iter().flatten().map(|u| self.between(u, v).count() as i64).max();
                along = 2 * lines.unwrap_or(0) + 1;
                let mut ends = self.stack_edges.iter().filter(|e| e.src == v || e.dst == v);
                across = if ends.any(|e| self.bracket(e)) { 5 } else { 0 };
            }
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
            (nd.csize, nd.lsize) = (nd.csize.max(across), nd.lsize.max(along));
            nd.csize += i64::from(nd.looped.is_some());
        }
    }

    /// Pull nodes toward their neighbours while keeping order and spacing,
    /// then push lanes apart so frames have room.
    fn position(&mut self) {
        self.pack();
        let sweep = |l: &mut Self, k: usize, up: bool| {
            let (mut ys, mut ws, mut offs, mut c) = (vec![], vec![], vec![], 0i64);
            // A stack moves as one unit, where its boxes want to be on average.
            let mut units: Vec<Vec<usize>> = vec![];
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
                let y = want - (nd.box_csize() - 1).div_euclid(2) as f64 - c as f64;
                match units.last_mut() {
                    Some(unit) if l.follows(*unit.last().unwrap(), v) => {
                        unit.push(v);
                        let (last, n) = (ys.len() - 1, unit.len() as f64);
                        // The mean so far, of every box's wish weighted alike.
                        ys[last] += (y - ys[last]) / n;
                        ws[last] += w;
                    }
                    _ => {
                        units.push(vec![v]);
                        ys.push(y);
                        ws.push(w);
                        offs.push(c);
                    }
                }
                c += nd.csize + l.space(v, l.layers[k].get(i + 1));
            }
            for ((unit, y), o) in units.into_iter().zip(pava(&ys, &ws)).zip(offs) {
                let mut pos = y.round_ties_even() as i64 + o;
                for (j, &v) in unit.iter().enumerate() {
                    l.nodes[v].pos = pos;
                    pos += l.nodes[v].csize + l.space(v, unit.get(j + 1));
                }
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
}

impl Layout<Placed> {
    fn route(mut self) -> Layout<Routed> {
        self.ports();
        self.back_lanes();
        self.tracks();
        self.to()
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
            // Brackets between stacked boxes meet the far side off-centre.
            let mut taken: Vec<i64> = vec![];
            if side == Side::Out {
                for e in self.stack_edges.iter().filter(|e| (e.src == n || e.dst == n) && self.bracket(e)) {
                    taken.push(self.bracket_at(n, self.ends(e).0 == n));
                }
            }
            // The style whose edges run most nearly straight gets the centre.
            let mut styles = styles.clone();
            styles.sort_by_key(|&st| (toward((n, side, st)) - c).abs());
            for style in styles {
                let free: Vec<i64> = (lo..=hi).filter(|x| !used.contains(x) && !taken.contains(x)).collect();
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

    /// Every node's cross-axis extent, frames included.
    fn extent(&self) -> (i64, i64) {
        let lo = self.nodes.iter().map(|n| n.framed().0).min().unwrap_or(0);
        (lo, self.nodes.iter().map(|n| n.framed().1).max().unwrap_or(0))
    }

    /// Whether a back edge's lane goes on the low side (before every node):
    /// the side nearer the middle of its ends.
    fn low_side(&self, edge: usize) -> bool {
        let (e, (lo, hi)) = (&self.edges[edge], self.extent());
        self.nodes[e.src].cen() + self.nodes[e.dst].cen() < lo + hi
    }

    /// A cross-axis point far out on the side a back edge's lane will use,
    /// for choosing which side of a box its ports go on.
    fn lane_hint(&self, edge: usize) -> i64 {
        if self.low_side(edge) {
            -(1 << 30)
        } else {
            1 << 30
        }
    }

    /// Outer lanes for back edges, beyond every node and frame on the side
    /// nearer the edge's ends; shorter edges run inside longer ones.
    fn back_lanes(&mut self) {
        let (lo, hi) = self.extent();
        let (mut near, mut far) = (lo - 2, hi + 2);
        let mut order: Vec<usize> = (0..self.backs.len()).collect();
        order.sort_by_key(|&b| {
            let e = &self.edges[self.backs[b].edge];
            self.nodes[e.src].layer - self.nodes[e.dst].layer
        });
        for b in order {
            let (e, low) = (&self.edges[self.backs[b].edge], self.low_side(self.backs[b].edge));
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
}

impl Layout<Routed> {
    /// The real-node layer span of each framed lane.
    fn frames(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.lanes.iter().copied().filter(|l| l.0 > 0)
    }

    /// The longest box in layer `k`, along the flow.
    fn box_len(&self, k: usize) -> i64 {
        self.layers[k].iter().map(|&v| self.nodes[v].lsize).max().unwrap_or(0)
    }

    fn band_len(&self, k: usize) -> i64 {
        self.box_len(k) + self.bracket_room(k)
    }

    /// The stack edges of layer `k` drawn as brackets beyond its boxes.
    fn brackets(&self, k: usize) -> Vec<&StackEdge> {
        self.stack_edges.iter().filter(|e| self.nodes[e.src].layer == k && self.bracket(e)).collect()
    }

    /// Room after layer `k`'s boxes for its brackets: a lane each, two cells
    /// apart, and their labels beside them.
    fn bracket_room(&self, k: usize) -> i64 {
        let brackets = self.brackets(k);
        let label = |l: &Vec<String>| if self.td { 1 } else { l.iter().map(|t| width(t)).max().unwrap_or(0) + 1 };
        let labels = brackets.iter().filter_map(|e| e.label.as_ref()).map(label).max().unwrap_or(0);
        2 * brackets.len() as i64 + labels
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
            // A two-headed edge's head into its source takes a cell, and a
            // line cell after it keeps it apart from the tracks and other head.
            let head = self.segs.iter().any(|s| s.chan == k && s.head());
            let lead = i64::from(!self.td)
                + if head { 1 + i64::from(self.td) } else { 0 }
                + if self.frames().any(|f| f.2 == k) { FRAME } else { 0 };
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

    /// Each real node's box (top-left, size); real nodes come first.
    fn boxes(&self) -> Vec<(Pt, Pt)> {
        let real = self.nodes.iter().take_while(|n| n.kind == Kind::Real);
        real.map(|n| {
            let size = if self.td { (n.box_csize(), n.lsize) } else { (n.lsize, n.box_csize()) };
            (self.phys(self.bstart[n.layer], n.pos), size)
        })
        .collect()
    }

    fn paint(mut self, ascii: bool) -> Scene {
        self.coordinates();
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
        let (arrow, back_arrow) = if self.td { ('▼', '▲') } else { ('►', '◄') };
        let lanes_from = self.nodes.len();
        let track = |s: &Seg| self.tstart[s.chan] + s.track;
        for (i, s) in self.segs.iter().enumerate() {
            let t = track(s);
            // Back edges have a lane, not a node, at one end, so only look up the other.
            // A two-headed edge starts a cell out, past its head into the source.
            let start = || (out_l(s.sk.0) + i64::from(s.head()), s.cs);
            let end = || (in_l(s.tk.0) - i64::from(s.arrow_end), s.ct);
            let pieces = if let Some(b) = s.back {
                // A back edge: out of its source to the outer lane, back along
                // the lane, and in to its target like any other edge.
                let back = &self.backs[b];
                let lane_key = (lanes_from + back.edge, Side::In, (s.style, false));
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
            if s.head() {
                let (x, y) = self.phys(out_l(s.sk.0) + 1, s.cs);
                p.symbol(x, y, back_arrow);
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
        self.paint_stack_edges(&mut p);

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
        Scene { boxes: self.boxes(), painter: p }
    }

    /// Edges inside stacks. Between neighbours: straight across the gap, side
    /// by side, labels beside (LR) or above and below (TD) the lines. Between
    /// others: out of the far side, along a lane beyond the layer's boxes,
    /// and back in.
    fn paint_stack_edges(&self, p: &mut Painter) {
        let (ahead, back, home) = if self.td { ('►', '◄', '▲') } else { ('▼', '▲', '◄') };
        let mut pairs: Vec<(usize, usize)> = vec![];
        for e in self.stack_edges.iter().filter(|e| !self.bracket(e)) {
            if !pairs.contains(&self.ends(e)) {
                pairs.push(self.ends(e));
            }
        }
        for (u, w) in pairs {
            let (nu, nw) = (&self.nodes[u], &self.nodes[w]);
            let mid = self.bstart[nu.layer] + (nu.lsize.min(nw.lsize) - 1).div_euclid(2);
            let (cu, cw) = (nu.pos + nu.box_csize() - 1, nw.pos);
            let lines: Vec<&StackEdge> = self.between(u, w).collect();
            let k = lines.len() as i64;
            for (i, e) in lines.into_iter().enumerate() {
                let at = mid - (k - 1) + 2 * i as i64;
                let (from, to, step) = if e.src == u { (cu, cw, 1) } else { (cw, cu, -1) };
                let (head, tail) = (e.arrow != Arrow::None, e.arrow == Arrow::Both);
                let (start, end) = (from + step * i64::from(tail), to - step * i64::from(head));
                let net = p.net();
                p.line(&[self.phys(at, start), self.phys(at, end)], net, Stroke::light(e.style), Layer::Main);
                for (on, c, forward) in [(head, end, step == 1), (tail, start, step != 1)] {
                    if on {
                        let (x, y) = self.phys(at, c);
                        p.symbol(x, y, if forward { ahead } else { back });
                    }
                }
                let Some(label) = &e.label else { continue };
                let gap = cw - cu - 1;
                if self.td {
                    let text = label.join(" ");
                    let l = if i == 0 { at - 1 } else { at + 1 };
                    let (x, y) = self.phys(l, cu + 1 + (gap - width(&text)).div_euclid(2));
                    p.text(x, y, &text);
                } else {
                    // The first of several lines has its label on its left.
                    let w = label.iter().map(|t| width(t)).max().unwrap_or(0);
                    let l = if i == 0 && k > 1 { at - 1 - w } else { at + 2 };
                    let top = cu + 1 + (gap - label.len() as i64).div_euclid(2);
                    for (j, t) in label.iter().enumerate() {
                        p.text(l, top + j as i64, t);
                    }
                }
            }
        }
        for k in 0..self.layers.len() {
            for (i, e) in self.brackets(k).into_iter().enumerate() {
                let lane = self.bstart[k] + self.box_len(k) + 1 + 2 * i as i64;
                let (u, w) = self.ends(e);
                let far = |n: usize| self.bstart[k] + self.nodes[n].lsize - 1;
                let (mut a, mut b) = ((far(u), self.bracket_at(u, true)), (far(w), self.bracket_at(w, false)));
                if e.src != u {
                    (a, b) = (b, a);
                }
                let (head, tail) = (e.arrow != Arrow::None, e.arrow == Arrow::Both);
                let (start, end) = ((a.0 + i64::from(tail), a.1), (b.0 + i64::from(head), b.1));
                let pts = [start, (lane, a.1), (lane, b.1), end].map(|(l, c)| self.phys(l, c));
                let net = p.net();
                p.line(&pts, net, Stroke::light(e.style), Layer::Main);
                for (on, (l, c)) in [(head, end), (tail, start)] {
                    if on {
                        let (x, y) = self.phys(l, c);
                        p.symbol(x, y, home);
                    }
                }
                if let Some(label) = &e.label {
                    let mid = (a.1 + b.1).div_euclid(2);
                    let text = label.join(" ");
                    let (x, y) =
                        if self.td { self.phys(lane + 1, mid - width(&text) / 2) } else { self.phys(lane + 2, mid) };
                    p.text(x, y, &text);
                }
            }
        }
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
        g.g.node_weights().position(|n| n.id == name).unwrap()
    }

    fn node(pos: i64, csize: i64, layer: usize) -> Node {
        Node { pos, csize, layer, lsize: 3, ..Node::new(vec!["x".into()], Kind::Real) }
    }

    fn edge(src: usize, dst: usize, style: Style, rev: bool) -> Edge {
        Edge { src, dst, label: None, style, arrow: Arrow::End, rev, path: vec![] }
    }

    fn seg(sk: PortKey, tk: PortKey, cs: i64, ct: i64) -> Seg {
        let (_, _, (style, _)) = sk;
        Seg { chan: 0, back: None, style, arrow_end: false, sk, tk, cs, ct, track: 0 }
    }

    #[test]
    fn break_cycles_marks_only_the_closing_edge() {
        let g = graph("a --> b\nb --> c\nc --> a\na --> d\nd --> c");
        let (_, edges, _) = layer_graph(&g, &structure(&g));
        let rev: Vec<(usize, usize)> = edges.iter().filter(|e| e.rev).map(|e| (e.src, e.dst)).collect();
        assert_eq!(rev, [(id(&g, "c"), id(&g, "a"))]);
    }

    #[test]
    fn a_stack_is_one_node_to_cycles_and_layers() {
        let g = graph("a --> b --> c\na --> c\n%% stack: a c");
        let s = structure(&g);
        assert_eq!(s.layer[id(&g, "a")], s.layer[id(&g, "c")]);
        // Leaving the stack (a → b) and coming back (b → c) is a loop; a → c
        // is inside the stack, so it is drawn across and not counted.
        let (_, edges, stacked) = layer_graph(&g, &s);
        assert_eq!(edges.iter().filter(|e| e.rev).count(), 1);
        assert_eq!(stacked.iter().map(|e| (e.src, e.dst)).collect::<Vec<_>>(), [(id(&g, "a"), id(&g, "c"))]);
    }

    #[test]
    fn layers_of_a_chain_and_a_diamond() {
        let g = graph("a --> b\nb --> c");
        assert_eq!(structure(&g).layer, [0, 1, 2]);
        let g = graph("a --> b\na --> c\nb --> d\nc --> d");
        let layers = structure(&g).layer;
        assert_eq!([layers[id(&g, "a")], layers[id(&g, "b")], layers[id(&g, "c")], layers[id(&g, "d")]], [0, 1, 1, 2]);
        // A source is pulled down next to its child; a label takes a layer.
        let g = graph("a --> b\nb --> c\nx --> c\nc -->|yes| d");
        let layers = structure(&g).layer;
        assert_eq!([layers[id(&g, "x")], layers[id(&g, "d")]], [1, 4]);
    }

    #[test]
    fn long_edges_into_one_node_share_dummies_by_style() {
        let g = graph("a --> b\nb --> c\nc --> d\na --> d\nz --> b\nz --> d\nz -.-> d");
        let l = Layout::new(&g, &structure(&g), true);
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
        let mut l = Layout::new(&g, &structure(&g), true);
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
        let mut l = Layout::new(&g, &structure(&g), true);
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
        let mut l: Layout<Placed> = Layout { layers: vec![vec![], vec![]], ..Default::default() };
        let key = |n, side| (n, side, (Style::Solid, false));
        l.segs.push(seg(key(0, Side::Out), key(1, Side::In), 0, 10));
        l.segs.push(seg(key(2, Side::Out), key(3, Side::In), 5, 20));
        l.tracks();
        assert_eq!((l.segs[0].track, l.segs[1].track), (1, 0));
    }

    #[test]
    fn ports_put_the_straightest_style_at_the_centre() {
        // `a` (centre 5) has a dotted edge to far-off `c`, listed first, and a
        // solid one to `b` straight below.
        let mut l: Layout<Placed> =
            Layout { nodes: vec![node(0, 11, 0), node(0, 11, 1), node(40, 11, 1)], ..Default::default() };
        let (dotted, solid) = ((Style::Dotted, false), (Style::Solid, false));
        l.groups.insert((0, Side::Out), vec![dotted, solid]);
        l.segs.push(seg((0, Side::Out, dotted), (2, Side::In, dotted), 0, 0));
        l.segs.push(seg((0, Side::Out, solid), (1, Side::In, solid), 0, 0));
        l.ports();
        assert_eq!(l.port[&(0, Side::Out, solid)], 5);
        assert!(l.port[&(0, Side::Out, dotted)] > 5, "the dotted port leans toward c");
    }

    #[test]
    fn back_lanes_take_the_nearer_side() {
        // Two cycles, one at each end of the cross axis (0..55).
        let nodes = vec![node(0, 5, 1), node(0, 5, 2), node(50, 5, 1), node(50, 5, 2)];
        let edges = vec![edge(1, 0, Style::Solid, true), edge(3, 2, Style::Solid, true)];
        let mut l: Layout<Ordered> = Layout { nodes, edges, ..Default::default() };
        l.segments();
        let mut l: Layout<Placed> = l.to();
        l.back_lanes();
        let lanes: Vec<i64> = l.backs.iter().map(|b| b.lane).collect();
        assert_eq!(lanes, [-2, 56]);
        for b in &l.backs {
            assert_eq!((l.segs[b.out_seg].ct, l.segs[b.in_seg].cs), (b.lane, b.lane));
        }
    }
}
