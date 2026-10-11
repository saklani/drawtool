//! Flowchart layout: a simplified Sugiyama pipeline.
//!
//! break cycles → layer → dummy nodes → order (barycenter) → size → place
//! (isotonic regression) → ports → channel tracks → paint.
//!
//! Coordinates are abstract: `l` runs along the flow (layers) and `c` across
//! it; `phys` maps them to x/y, so TD and LR share every line of layout.
//!
//! A *box* is something drawn with a border: a node of the graph. A layout
//! *node* is what ordering and placement move: a box, a `%% stack:` of boxes
//! laid side by side across the flow, an edge label, or a dummy standing in
//! for a long edge in a layer it passes. A subgraph is a *lane*: a strip
//! across the flow only its members occupy. Edges leave and enter boxes at
//! *ports*; a bent segment crosses the channel between two layers on a
//! *track*.
//!
//! An edge that closes a cycle is laid out reversed, as in dot: it runs
//! forward through the layers from its target to its source, with its
//! arrowhead drawn where it starts.

use crate::model::{Arrow, Direction, Graph, Mark, Shape, Style};
use crate::paint::{Corners, Layer, Painter, Pt, Stroke, Weight};
use crate::text::width;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};

/// Gap plus dashed line between a group's members and its frame.
const FRAME: i64 = 2;

/// An edge from a box to itself, drawn as a loop out of the box's flow side
/// near its far end, around that corner, and back into the far side.
#[derive(Clone, Copy)]
struct SelfLoop {
    style: Style,
    arrow: bool,
}

struct Box {
    lines: Vec<String>,
    shape: Shape,
    mark: Mark,
    looped: Option<SelfLoop>,
    /// Its layout node, and its cross-axis offset within that node.
    node: usize,
    offset: i64,
    /// Size across the flow and along it.
    csize: i64,
    lsize: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Boxes,
    Label,
    Dummy,
}

struct Node {
    kind: Kind,
    /// Its boxes (one, or a stack's in order); a label's text.
    boxes: Vec<usize>,
    lines: Vec<String>,
    layer: usize,
    group: Option<usize>,
    up: Vec<usize>,
    down: Vec<usize>,
    /// Cross-axis position of its first cell, and size across and along the flow.
    pos: i64,
    csize: i64,
    lsize: i64,
}

impl Node {
    fn new(kind: Kind, layer: usize, group: Option<usize>) -> Self {
        let (boxes, lines, up, down) = Default::default();
        Node { kind, boxes, lines, layer, group, up, down, pos: 0, csize: 0, lsize: 0 }
    }

    /// How strongly placement pulls it toward its neighbours.
    fn pull(&self) -> f64 {
        match self.kind {
            Kind::Boxes => 1.0,
            Kind::Label => 2.0,
            Kind::Dummy => 4.0,
        }
    }

    /// First and last cross-axis cell, widened by its group's frame.
    fn framed(&self) -> (i64, i64) {
        let m = FRAME * i64::from(self.group.is_some());
        (self.pos - m, self.pos + self.csize - 1 + m)
    }
}

/// An edge laid out forward: `path` is one node per layer from where it
/// starts to where it ends. `head` is an arrowhead at the start (a two-headed
/// edge, or a reversed one), `arrow` one at the end.
struct Edge {
    path: Vec<usize>,
    style: Style,
    head: bool,
    arrow: bool,
}

/// An edge between two boxes of one stack, drawn across the flow.
struct StackEdge {
    src: usize,
    dst: usize,
    label: Option<Vec<String>>,
    style: Style,
    arrow: Arrow,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Side {
    In,
    Out,
}

/// Where edges meet a box side: edges of one style share a port, another
/// style gets its own beside it, so dotted and solid stay apart. So does an
/// edge with a head at its start, whose arrowhead must not mark the others.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Port {
    node: usize,
    side: Side,
    style: Style,
    head: bool,
}

/// One edge segment between adjacent layers, `cs` → `ct` across the channel.
struct Seg {
    chan: usize,
    from: Port,
    to: Port,
    style: Style,
    arrow: bool,
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

struct Lane {
    group: Option<usize>,
    first: usize,
    last: usize,
}

fn overlaps<T: Ord + Copy>(a: (T, T), b: (T, T)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
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

// ---- Structure --------------------------------------------------------------

/// Each node's layer: the longest path to it over `edges` (source, target,
/// span), treating `rev` edges as reversed and skipping self-loops; then
/// sources are pulled down next to their nearest child.
fn layering(n: usize, edges: &[(usize, usize, usize)], rev: &[bool]) -> Vec<usize> {
    let forward: Vec<(usize, usize, usize)> = (edges.iter().zip(rev))
        .filter(|((s, t, _), _)| s != t)
        .map(|(&(s, t, span), &r)| if r { (t, s, span) } else { (s, t, span) })
        .collect();
    let mut indeg = vec![0; n];
    for &(_, t, _) in &forward {
        indeg[t] += 1;
    }
    let mut order: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut layer = vec![0; n];
    let mut i = 0;
    while i < order.len() {
        let v = order[i];
        for &(_, t, span) in forward.iter().filter(|e| e.0 == v) {
            layer[t] = layer[t].max(layer[v] + span);
            indeg[t] -= 1;
            if indeg[t] == 0 {
                order.push(t);
            }
        }
        i += 1;
    }
    assert_eq!(order.len(), n, "without back edges and self-loops the graph is acyclic");
    for &v in order.iter().rev().filter(|&&v| forward.iter().all(|e| e.1 != v)) {
        let child = |e: &(usize, usize, usize)| if e.2 == 2 { layer[v] + 1 } else { layer[e.1] };
        if let Some(nearest) = forward.iter().filter(|e| e.0 == v).map(child).min() {
            layer[v] = nearest - 1;
        }
    }
    layer
}

/// For each edge (source, target), whether it closes a cycle: depth-first
/// search in source order finds it leading back to a node still on the path.
fn back_edges(n: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    fn visit(v: usize, edges: &[(usize, usize)], seen: &mut [bool], on_path: &mut [bool], back: &mut [bool]) {
        (seen[v], on_path[v]) = (true, true);
        for (i, &(_, t)) in edges.iter().enumerate().filter(|(_, e)| e.0 == v) {
            if on_path[t] {
                back[i] = t != v;
            } else if !seen[t] {
                visit(t, edges, seen, on_path, back);
            }
        }
        on_path[v] = false;
    }
    let (mut seen, mut on_path, mut back) = (vec![false; n], vec![false; n], vec![false; edges.len()]);
    let source = |v: &usize| edges.iter().all(|e| e.1 != *v || e.0 == *v);
    for v in (0..n).filter(source).chain(0..n) {
        if !seen[v] {
            visit(v, edges, &mut seen, &mut on_path, &mut back);
        }
    }
    back
}

/// Each graph node's layer. A stack counts as one node.
pub(crate) fn layers(graph: &Graph) -> Vec<usize> {
    let n = graph.g.node_count();
    let mut unit: Vec<usize> = (0..n).collect();
    for st in &graph.stacks {
        for v in st {
            unit[v.index()] = st[0].index();
        }
    }
    let ends: Vec<(usize, usize)> =
        graph.g.edge_references().map(|e| (unit[e.source().index()], unit[e.target().index()])).collect();
    let rev = back_edges(n, &ends);
    let spans: Vec<(usize, usize, usize)> = (ends.iter().zip(graph.g.edge_weights()))
        .map(|(&(s, t), e)| (s, t, 1 + usize::from(e.label.is_some())))
        .collect();
    let layer = layering(n, &spans, &rev);
    unit.iter().map(|&u| layer[u]).collect()
}

/// Edge crossings between consecutive layers.
fn crossings(layers: &[Vec<usize>], nodes: &[Node]) -> usize {
    let mut total = 0;
    for pair in layers.windows(2) {
        let rank: HashMap<usize, usize> = pair[1].iter().enumerate().map(|(i, &v)| (v, i)).collect();
        let rank = &rank;
        let es: Vec<(usize, usize)> =
            (pair[0].iter().enumerate()).flat_map(|(i, &u)| nodes[u].down.iter().map(move |v| (i, rank[v]))).collect();
        for (i, a) in es.iter().enumerate() {
            total += es[..i].iter().filter(|b| (a.0 < b.0) != (a.1 < b.1) && a.0 != b.0 && a.1 != b.1).count();
        }
    }
    total
}

// ---- Layout -----------------------------------------------------------------

#[cfg_attr(test, derive(Default))]
struct Layout {
    td: bool,
    gap: i64,
    titles: Vec<String>,
    boxes: Vec<Box>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    stack_edges: Vec<StackEdge>,
    layers: Vec<Vec<usize>>,
    lanes: Vec<Lane>,
    lane_of: Vec<usize>,
    segs: Vec<Seg>,
    /// The port kinds at each node side in order of appearance, and where each port sits.
    port_kinds: HashMap<(usize, Side), Vec<(Style, bool)>>,
    port: HashMap<Port, i64>,
    /// Per layer: where its band starts along the flow, and the first track
    /// of the channel after it.
    bstart: Vec<i64>,
    tstart: Vec<i64>,
}

/// A laid-out graph before it becomes text: what was painted, and each
/// graph node's box (top-left, size).
pub(crate) struct Scene {
    pub painter: Painter,
    pub boxes: Vec<(Pt, Pt)>,
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
#[must_use]
pub fn render(graph: &Graph, direction: Option<Direction>, ascii: bool) -> String {
    scene(graph, direction, ascii).map_or_else(String::new, |s| s.painter.finish())
}

/// Lay out and paint `graph`; `None` if it has no nodes.
pub(crate) fn scene(graph: &Graph, direction: Option<Direction>, ascii: bool) -> Option<Scene> {
    if graph.g.node_count() == 0 {
        return None;
    }
    let mut l = Layout::new(graph, direction.unwrap_or(graph.direction) == Direction::TD);
    l.order();
    l.sizes();
    l.place();
    l.ports();
    l.tracks();
    Some(l.paint(ascii))
}

impl Layout {
    fn new(graph: &Graph, td: bool) -> Self {
        let layer = layers(graph);
        let defs: Vec<_> = graph.g.node_weights().collect();
        let mut boxes: Vec<Box> = (defs.iter().enumerate())
            .map(|(i, n)| Box {
                lines: n.lines.clone(),
                shape: n.shape,
                mark: n.mark,
                looped: None,
                node: i,
                offset: 0,
                csize: 0,
                lsize: 0,
            })
            .collect();
        // One node per box; then a stack's boxes move to the stack's first.
        let mut nodes: Vec<Node> = (defs.iter().enumerate())
            .map(|(i, n)| Node { boxes: vec![i], ..Node::new(Kind::Boxes, layer[i], n.group) })
            .collect();
        for st in &graph.stacks {
            let first = st[0].index();
            nodes[first].boxes = st.iter().map(|v| v.index()).collect();
            for &v in &st[1..] {
                boxes[v.index()].node = first;
                nodes[v.index()].boxes.clear();
            }
        }
        let ends: Vec<(usize, usize)> = (graph.g.edge_references())
            .map(|e| (boxes[e.source().index()].node, boxes[e.target().index()].node))
            .collect();
        let rev = back_edges(nodes.len(), &ends);
        let (mut edges, mut stack_edges) = (Vec::new(), Vec::new());
        for (e, &(s, t)) in graph.g.edge_references().zip(&ends) {
            let (src, dst, d) = (e.source().index(), e.target().index(), e.weight());
            if src == dst {
                boxes[src].looped = Some(SelfLoop { style: d.style, arrow: d.arrow != Arrow::None });
            } else if s == t {
                stack_edges.push(StackEdge { src, dst, label: d.label.clone(), style: d.style, arrow: d.arrow });
            } else {
                let r = rev[e.id().index()];
                let both = d.arrow == Arrow::Both;
                let mut path = if r { vec![t, s] } else { vec![s, t] };
                if let Some(label) = &d.label {
                    let mut node = Node::new(Kind::Label, nodes[path[0]].layer + 1, nodes[path[0]].group);
                    node.lines.clone_from(label);
                    nodes.push(node);
                    path.insert(1, nodes.len() - 1);
                }
                let head = both || (r && d.arrow != Arrow::None);
                edges.push(Edge { path, style: d.style, head, arrow: both || !r });
            }
        }
        let n_layers = nodes.iter().map(|n| n.layer).max().unwrap_or(0) + 1;
        let mut l = Layout {
            td,
            gap: if td { 3 } else { 1 },
            titles: graph.groups.iter().map(|g| g.title.clone()).collect(),
            boxes,
            nodes,
            edges,
            stack_edges,
            layers: vec![vec![]; n_layers],
            lanes: Vec::new(),
            lane_of: Vec::new(),
            segs: Vec::new(),
            port_kinds: HashMap::new(),
            port: HashMap::new(),
            bstart: Vec::new(),
            tstart: Vec::new(),
        };
        l.dummies();
        l
    }

    /// Dummy nodes for edges spanning several layers. Long edges of the same
    /// style into the same node share dummies, so they merge into one line as
    /// early as possible instead of each taking its own route.
    fn dummies(&mut self) {
        let mut shared: HashMap<(usize, usize, Style), usize> = HashMap::new();
        let nodes = &mut self.nodes;
        for e in &mut self.edges {
            let (mut path, group) = (vec![e.path[0]], nodes[e.path[0]].group);
            for w in e.path.windows(2) {
                let bundle = nodes[w[1]].kind == Kind::Boxes;
                for k in nodes[w[0]].layer + 1..nodes[w[1]].layer {
                    let d = match shared.get(&(w[1], k, e.style)) {
                        Some(&d) if bundle => d,
                        _ => {
                            nodes.push(Node::new(Kind::Dummy, k, group));
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
            e.path = path;
        }
    }

    /// Lanes, then barycenter sweeps (keeping the ordering with fewest
    /// crossings). Every subgraph is a lane, so its frame can never enclose
    /// a stranger.
    fn order(&mut self) {
        for n in self.nodes.iter().filter(|n| n.kind == Kind::Boxes) {
            match self.lanes.iter_mut().find(|l| l.group == n.group) {
                Some(l) => (l.first, l.last) = (l.first.min(n.layer), l.last.max(n.layer)),
                None => self.lanes.push(Lane { group: n.group, first: n.layer, last: n.layer }),
            }
        }
        self.lanes.sort_by_key(|l| l.first);
        let lane_of = |n: &Node| self.lanes.iter().position(|l| l.group == n.group).unwrap_or(0);
        self.lane_of = self.nodes.iter().map(lane_of).collect();
        // A stacked box's own node is empty: the stack's first node holds them all.
        let placed = |n: &Node| n.kind != Kind::Boxes || !n.boxes.is_empty();
        for (v, n) in self.nodes.iter().enumerate().filter(|(_, n)| placed(n)) {
            self.layers[n.layer].push(v);
        }
        let nl = self.layers.len();
        let (mut best, mut best_c) = (self.layers.clone(), usize::MAX);
        for it in 0..16 {
            let mut rank = vec![0.0; self.nodes.len()];
            let down = it % 2 == 0;
            let ks: Vec<usize> = if down { (1..nl).collect() } else { (0..nl - 1).rev().collect() };
            for k in ks {
                let adj = if down { k - 1 } else { k + 1 };
                self.layers[adj].iter().enumerate().for_each(|(i, &v)| rank[v] = i as f64);
                let key = |v: usize| {
                    let nb = if down { &self.nodes[v].up } else { &self.nodes[v].down };
                    if nb.is_empty() {
                        rank[v]
                    } else {
                        nb.iter().map(|&m| rank[m]).sum::<f64>() / nb.len() as f64
                    }
                };
                let mut lay = self.layers[k].clone();
                lay.sort_by(|&a, &b| self.lane_of[a].cmp(&self.lane_of[b]).then(key(a).total_cmp(&key(b))));
                lay.iter().enumerate().for_each(|(i, &v)| rank[v] = i as f64);
                self.layers[k] = lay;
            }
            self.transpose();
            let c = crossings(&self.layers, &self.nodes);
            if c < best_c {
                (best, best_c) = (self.layers.clone(), c);
            }
        }
        self.layers = best;
    }

    /// Swap neighbours in a layer (within a lane) while that lowers the
    /// crossings between their edges ("transpose", as in Graphviz dot).
    fn transpose(&mut self) {
        let mut pos = vec![0; self.nodes.len()];
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
                    let same_lane = self.lane_of[a] == self.lane_of[b];
                    if same_lane && cost(&self.nodes, &pos, b, a) < cost(&self.nodes, &pos, a, b) {
                        lay.swap(i - 1, i);
                        (pos[a], pos[b]) = (i, i - 1);
                        improved = true;
                    }
                }
            }
        }
    }

    /// The stack edges between boxes `a` and `b`, either way.
    fn between(&self, a: usize, b: usize) -> impl Iterator<Item = &StackEdge> {
        self.stack_edges.iter().filter(move |e| (e.src, e.dst) == (a, b) || (e.src, e.dst) == (b, a))
    }

    /// A stack edge's ends in stack order, and whether it skips over boxes
    /// (so it is drawn as a bracket beyond the layer instead of straight across).
    fn ends(&self, e: &StackEdge) -> (usize, usize, bool) {
        let at = |b: usize| self.boxes[b].offset;
        let (a, b) = if at(e.src) < at(e.dst) { (e.src, e.dst) } else { (e.dst, e.src) };
        let skips = self.nodes[self.boxes[a].node].boxes.iter().any(|&m| at(a) < at(m) && at(m) < at(b));
        (a, b, skips)
    }

    /// The gap between neighbours in a stack: room for the lines between
    /// them and their labels (beside the lines in LR, above them in TD).
    fn stack_gap(&self, a: usize, b: usize) -> i64 {
        let labels = self.between(a, b).filter_map(|e| e.label.as_ref());
        if self.between(a, b).next().is_none() {
            self.gap
        } else if self.td {
            labels.map(|l| width(&l.join(" ")) + 2).max().unwrap_or(0).max(3)
        } else {
            let both = self.between(a, b).any(|e| e.arrow == Arrow::Both);
            labels.map(|l| l.len() as i64).max().unwrap_or(0).max(2 + i64::from(both))
        }
    }

    /// Segments (a box's size depends on its ports), then box and node sizes.
    fn sizes(&mut self) {
        for e in &self.edges {
            for (j, w) in e.path.windows(2).enumerate() {
                let port = |node, side, head| Port { node, side, style: e.style, head };
                let (from, to) = (port(w[0], Side::Out, j == 0 && e.head), port(w[1], Side::In, false));
                let (chan, arrow) = (self.nodes[w[0]].layer, e.arrow && j == e.path.len() - 2);
                self.segs.push(Seg { chan, from, to, style: e.style, arrow, cs: 0, ct: 0, track: 0 });
            }
        }
        for s in &self.segs {
            for p in [s.from, s.to] {
                let kinds = self.port_kinds.entry((p.node, p.side)).or_default();
                if !kinds.contains(&(p.style, p.head)) {
                    kinds.push((p.style, p.head));
                }
            }
        }
        for v in 0..self.nodes.len() {
            let ports = |side| self.port_kinds.get(&(v, side)).map_or(0, Vec::len) as i64;
            let np = ports(Side::In).max(ports(Side::Out));
            let nd = &self.nodes[v];
            let boxes = nd.boxes.clone();
            let (textw, nlines) = (nd.lines.iter().map(|t| width(t)).max().unwrap_or(0), nd.lines.len() as i64);
            (self.nodes[v].csize, self.nodes[v].lsize) = match nd.kind {
                Kind::Dummy => (1, 0),
                Kind::Label if self.td => (textw, nlines),
                Kind::Label => (nlines, textw),
                Kind::Boxes => (0, 0),
            };
            let (mut offset, mut along) = (0, 0);
            for (i, &b) in boxes.iter().enumerate() {
                let bx = &self.boxes[b];
                let (textw, nlines) = (bx.lines.iter().map(|t| width(t)).max().unwrap_or(0), bx.lines.len() as i64);
                // Across the flow: the text, or two cells per port; a self-loop
                // adds a cell to leave by and one of padding to go round.
                // Stacked boxes are long enough along the flow for the lines
                // to each neighbour side by side.
                let loop_room = i64::from(bx.looped.is_some());
                let mut across = if self.td { textw + 4 } else { nlines + 2 }.max(2 * np + 1) + loop_room;
                let mut along_b = if self.td { nlines + 2 } else { textw + 4 };
                if boxes.len() > 1 {
                    let lines = |o: Option<&usize>| o.map_or(0, |&u| self.between(u, b).count() as i64);
                    along_b = along_b.max(2 * lines(boxes.get(i.wrapping_sub(1))).max(lines(boxes.get(i + 1))) + 1);
                    if self.stack_edges.iter().any(|e| (e.src == b || e.dst == b) && self.ends(e).2) {
                        across = across.max(5);
                    }
                }
                let bx = &mut self.boxes[b];
                (bx.offset, bx.csize, bx.lsize) = (offset, across, along_b);
                offset += across + loop_room + boxes.get(i + 1).map_or(0, |&n| self.stack_gap(b, n));
                along = along.max(along_b);
            }
            if !boxes.is_empty() {
                (self.nodes[v].csize, self.nodes[v].lsize) = (offset, along);
            }
        }
    }

    /// Space after node `a` in its layer: none before or after a dummy (a
    /// line may run right beside a label or another line), wider between
    /// two TD edge labels, so side-by-side labels don't read as one phrase.
    fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        let kind = |v: usize| self.nodes[v].kind;
        match (kind(a), b.map(|&b| kind(b))) {
            (Kind::Dummy, _) | (_, Some(Kind::Dummy)) => i64::from(self.td),
            (Kind::Label, Some(Kind::Label)) if self.td => self.gap + 3,
            _ => self.gap,
        }
    }

    /// A node's cross-axis centre: for a stack, its first box's.
    fn cen(&self, v: usize) -> i64 {
        let n = &self.nodes[v];
        match n.boxes.first() {
            Some(&b) => {
                let (lo, hi) = self.box_range(b);
                (lo + hi) / 2
            }
            None => n.pos + (n.csize - 1) / 2,
        }
    }

    /// Move nodes along their layer until none starts before the one ahead
    /// of it ends (plus spacing).
    fn pack(&mut self) {
        for lay in &self.layers {
            for w in lay.windows(2) {
                let floor = self.nodes[w[0]].pos + self.nodes[w[0]].csize + self.space(w[0], Some(&w[1]));
                self.nodes[w[1]].pos = self.nodes[w[1]].pos.max(floor);
            }
        }
    }

    /// Pull nodes toward their neighbours while keeping order and spacing,
    /// then push lanes apart so frames have room.
    fn place(&mut self) {
        self.pack();
        let nl = self.layers.len();
        for _ in 0..8 {
            (1..nl).for_each(|k| self.sweep(k, true));
            (0..nl - 1).rev().for_each(|k| self.sweep(k, false));
        }
        (1..nl).for_each(|k| self.sweep(k, true));
        // Push each lane clear of earlier lanes it shares layers with.
        let mut extent: Vec<(i64, i64)> = vec![];
        for (r, lane) in self.lanes.iter().enumerate() {
            let margin = |g: Option<usize>| FRAME * i64::from(g.is_some());
            let need = (self.lanes.iter().zip(&extent))
                .filter(|(q, _)| overlaps((q.first, q.last), (lane.first, lane.last)))
                .map(|(q, e)| e.1 + 1 + self.gap + margin(q.group) + margin(lane.group))
                .max();
            let members: Vec<usize> = (0..self.nodes.len()).filter(|&v| self.lane_of[v] == r).collect();
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

    /// Move layer `k`'s nodes toward their neighbours in the layer before
    /// (`up`) or after it, keeping their order and spacing.
    fn sweep(&mut self, k: usize, up: bool) {
        let (mut ys, mut ws, mut offs, mut c) = (vec![], vec![], vec![], 0i64);
        for (i, &v) in self.layers[k].iter().enumerate() {
            let nd = &self.nodes[v];
            let nb = if up { &nd.up } else { &nd.down };
            let (want, w) = if nb.is_empty() {
                (self.cen(v) as f64, 0.05)
            } else {
                (nb.iter().map(|&m| self.cen(m)).sum::<i64>() as f64 / nb.len() as f64, nd.pull())
            };
            ys.push(want - (self.cen(v) - nd.pos) as f64 - c as f64);
            ws.push(w);
            offs.push(c);
            c += nd.csize + self.space(v, self.layers[k].get(i + 1));
        }
        for ((&v, y), o) in self.layers[k].iter().zip(pava(&ys, &ws)).zip(offs) {
            self.nodes[v].pos = y.round_ties_even() as i64 + o;
        }
    }

    /// The cross-axis range a box covers.
    fn box_range(&self, b: usize) -> (i64, i64) {
        let bx = &self.boxes[b];
        let lo = self.nodes[bx.node].pos + bx.offset;
        (lo, lo + bx.csize - 1)
    }

    /// Port positions: the first style at the centre, others beside it on
    /// the side facing where their edges go.
    fn ports(&mut self) {
        // Where each port's edges go, on average.
        let mut ends: HashMap<Port, (i64, i64)> = HashMap::new();
        for s in &self.segs {
            for (p, other) in [(s.from, s.to.node), (s.to, s.from.node)] {
                let e = ends.entry(p).or_default();
                *e = (e.0 + self.cen(other), e.1 + 1);
            }
        }
        for (&(n, side), kinds) in &self.port_kinds {
            let (nd, c) = (&self.nodes[n], self.cen(n));
            let (lo, hi) = match nd.boxes.first() {
                Some(&b) => {
                    let (lo, hi) = self.box_range(b);
                    (lo + 1, hi - 1)
                }
                None => (c, c),
            };
            // Brackets between stacked boxes and self-loops take the far side's end cells.
            let mut taken: Vec<i64> = vec![];
            if let (Side::Out, Some(&b)) = (side, nd.boxes.first()) {
                taken.extend(self.boxes[b].looped.map(|_| hi));
                for e in self.stack_edges.iter().filter(|e| (e.src == b || e.dst == b) && self.ends(e).2) {
                    taken.push(self.bracket_at(b, self.ends(e).0 == b));
                }
            }
            let lean = |&(style, head): &(Style, bool)| {
                let (sum, count) = ends[&Port { node: n, side, style, head }];
                sum / count - c
            };
            // The kind whose edges run most nearly straight gets the centre.
            let mut kinds = kinds.clone();
            kinds.sort_by_key(|k| lean(k).abs());
            let mut used: Vec<i64> = Vec::new();
            for kind in kinds {
                let dir = lean(&kind);
                let key = |&x: &i64| (used.iter().any(|u| (x - u).abs() < 2), (x - c) * dir < 0, (x - c).abs(), x);
                let free = (lo..=hi).filter(|x| !used.contains(x) && !taken.contains(x));
                let x = if used.is_empty() { c } else { free.min_by_key(key).unwrap_or(c) };
                used.push(x);
                self.port.insert(Port { node: n, side, style: kind.0, head: kind.1 }, x);
            }
        }
        for s in &mut self.segs {
            (s.cs, s.ct) = (self.port[&s.from], self.port[&s.to]);
        }
    }

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
            let distinct = |f: fn(&Seg) -> Port| comp.iter().map(|&c| f(&segs[c])).collect::<HashSet<_>>().len();
            comp.len() == 1 || distinct(|s| s.from) == 1 || distinct(|s| s.to) == 1
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
                bent.iter()
                    .copied()
                    .filter(move |&i| segs[i].cs == b.ct && segs[i].from != b.from && segs[i].to != b.to)
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
            s.track = t.map_or(0, |t| t as i64);
        }
    }

    // ---- Painting -----------------------------------------------------------

    /// The lanes that get a frame: each subgraph, with its boxes' layer span.
    fn frames(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.lanes.iter().filter_map(|l| l.group.map(|g| (g, l.first, l.last)))
    }

    /// The stack edges of layer `k` drawn as brackets beyond its boxes.
    fn brackets(&self, k: usize) -> Vec<&StackEdge> {
        self.stack_edges.iter().filter(|e| self.nodes[self.boxes[e.src].node].layer == k && self.ends(e).2).collect()
    }

    /// The cross-axis cell where a bracket meets box `b`, on its far side:
    /// near the end of the upper box and the start of the lower one.
    fn bracket_at(&self, b: usize, upper: bool) -> i64 {
        let (lo, hi) = self.box_range(b);
        if upper {
            hi - 1
        } else {
            lo + 1
        }
    }

    fn box_len(&self, k: usize) -> i64 {
        self.layers[k].iter().map(|&v| self.nodes[v].lsize).max().unwrap_or(0)
    }

    /// Length of layer `k` along the flow: its longest box, then its
    /// brackets (a lane each, two cells apart) and their labels.
    fn band_len(&self, k: usize) -> i64 {
        let brackets = self.brackets(k);
        let label = |l: &Vec<String>| if self.td { 1 } else { l.iter().map(|t| width(t)).max().unwrap_or(0) + 1 };
        let labels = brackets.iter().filter_map(|e| e.label.as_ref()).map(label).max().unwrap_or(0);
        self.box_len(k) + 2 * brackets.len() as i64 + labels
    }

    /// Layer-axis coordinates. Channels next to a frame get FRAME extra cells.
    fn coordinates(&mut self) {
        let mut cur = if self.frames().any(|f| f.1 == 0) { FRAME } else { 0 };
        for k in 0..self.layers.len() {
            self.bstart.push(cur);
            cur += self.band_len(k);
            let segs = self.segs.iter().filter(|s| s.chan == k);
            let ntracks = segs.clone().filter(|s| s.bent()).map(|s| s.track + 1).max().unwrap_or(0);
            // A head at a segment's start takes a cell, and a line cell after
            // it keeps it apart from the tracks and other heads.
            let head = segs.clone().any(|s| s.from.head);
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

    /// A box's top-left and size.
    fn box_rect(&self, b: usize) -> (Pt, Pt) {
        let (bx, (lo, hi)) = (&self.boxes[b], self.box_range(b));
        let (l0, w) = (self.bstart[self.nodes[bx.node].layer], hi - lo + 1);
        (self.phys(l0, lo), if self.td { (w, bx.lsize) } else { (bx.lsize, w) })
    }

    fn paint(mut self, ascii: bool) -> Scene {
        self.coordinates();
        let mut p = Painter::new(ascii);
        p.mark_crossings = true;
        for b in 0..self.boxes.len() {
            let ((x, y), (w, h)) = self.box_rect(b);
            let bx = &self.boxes[b];
            let (weight, style, corners) = match (bx.mark, bx.shape) {
                (Mark::Highlight | Mark::Added, _) => (Weight::Heavy, Style::Solid, Corners::Square),
                (Mark::Removed, _) => (Weight::Light, Style::Dotted, Corners::Square),
                (_, Shape::Decision) => (Weight::Double, Style::Solid, Corners::Square),
                (_, Shape::Round) => (Weight::Light, Style::Solid, Corners::Round),
                _ => (Weight::Light, Style::Solid, Corners::Square),
            };
            if bx.shape == Shape::Database && weight == Weight::Light && style == Style::Solid {
                // Double caps, light sides: ╒═╕ … ╘═╛.
                let (x1, y1) = (x + w - 1, y + h - 1);
                let cap = Stroke::new(Weight::Double, Style::Solid);
                let sides = [((x, y), (x1, y), cap), ((x, y1), (x1, y1), cap), ((x, y), (x, y1), Stroke::LIGHT)];
                for (a, b, stroke) in sides.into_iter().chain([((x1, y), (x1, y1), Stroke::LIGHT)]) {
                    let net = p.net();
                    p.line(&[a, b], net, stroke, Layer::Main);
                }
            } else {
                p.rect((x, y), (w, h), Stroke::new(weight, style), corners, Layer::Main);
            }
            if let Some(SelfLoop { style, arrow }) = bx.looped {
                // Out of the flow side by the last cell before the far
                // corner, around that corner, and into the far side.
                let (l0, (_, cend)) = (self.bstart[self.nodes[bx.node].layer], self.box_range(b));
                let lend = l0 + bx.lsize - 1;
                let back = (lend - 2).max(l0 + 1);
                let pts =
                    [(lend, cend - 1), (lend + 1, cend - 1), (lend + 1, cend + 1), (back, cend + 1), (back, cend)];
                let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                let net = p.net();
                p.line(&pts, net, Stroke::light(style), Layer::Main);
                if arrow {
                    p.symbol(pts[4].0, pts[4].1, if self.td { '◄' } else { '▲' });
                }
            }
            let top = (h - 2 - bx.lines.len() as i64) / 2;
            for (i, t) in bx.lines.iter().enumerate() {
                p.text(x + 1 + (w - 2 - width(t)) / 2, y + 1 + top + i as i64, t);
            }
        }
        for nd in self.nodes.iter().filter(|n| n.kind == Kind::Label) {
            let (x, y) = self.phys(self.bstart[nd.layer], nd.pos);
            let (w, h) = if self.td { (nd.csize, nd.lsize) } else { (nd.lsize, nd.csize) };
            let top = (h - nd.lines.len() as i64) / 2;
            for (i, t) in nd.lines.iter().enumerate() {
                p.text(x + (w - width(t)) / 2, y + top + i as i64, t);
            }
        }

        // Edges. Each piece belongs to the port it hangs off: the vertical out
        // of a node to its out-port, the vertical into a node to its in-port,
        // and a track to the port its fan-out (or fan-in) shares. Pieces of
        // different ports that pass through each other are drawn as crossings.
        let mut nets: HashMap<Port, usize> = HashMap::new();
        let (arrow, back_arrow) = if self.td { ('▼', '▲') } else { ('►', '◄') };
        let in_l = |n: usize| self.bstart[self.nodes[n].layer];
        let out_l = |n: usize| in_l(n) + self.nodes[n].lsize - 1;
        for s in &self.segs {
            let t = self.tstart[s.chan] + s.track;
            // A head at the start sets the line a cell out, past it.
            let start = (out_l(s.from.node) + i64::from(s.from.head), s.cs);
            let end = (in_l(s.to.node) - i64::from(s.arrow), s.ct);
            let pieces = if s.bent() {
                // A track shared with a segment from another port is a fan-in.
                let fan_in = self.segs.iter().any(|o| {
                    o.bent()
                        && (o.chan, o.track) == (s.chan, s.track)
                        && o.from != s.from
                        && overlaps(o.span(), s.span())
                });
                let track_port = if fan_in { s.to } else { s.from };
                vec![
                    (s.from, vec![start, (t, s.cs)]),
                    (track_port, vec![(t, s.cs), (t, s.ct)]),
                    (s.to, vec![(t, s.ct), end]),
                ]
            } else {
                vec![(s.from, vec![start, end])]
            };
            for (port, pts) in pieces {
                let net = *nets.entry(port).or_insert_with(|| p.net());
                let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                p.line(&pts, net, Stroke::light(s.style), Layer::Main);
            }
            if s.arrow {
                let (x, y) = self.phys(in_l(s.to.node) - 1, s.ct);
                p.symbol(x, y, arrow);
            }
            if s.from.head {
                let (x, y) = self.phys(out_l(s.from.node) + 1, s.cs);
                p.symbol(x, y, back_arrow);
            }
        }
        self.paint_stack_edges(&mut p);

        // Group frames sit under everything; titles slide past crossing edges.
        for (group, lo, hi) in self.frames() {
            let members = self.nodes.iter().filter(|n| n.group == Some(group) && n.kind == Kind::Boxes);
            let (c0, c1) = members.map(Node::framed).reduce(|(a, b), (c, d)| (a.min(c), b.max(d))).unwrap();
            let (l0, l1) = (self.bstart[lo] - FRAME, self.bstart[hi] + self.band_len(hi) - 1 + FRAME);
            let ((xa, ya), (xb, yb)) = (self.phys(l0, c0), self.phys(l1, c1));
            let (x0, x1, y0, y1) = (xa.min(xb), xa.max(xb), ya.min(yb), ya.max(yb));
            p.rect((x0, y0), (x1 - x0 + 1, y1 - y0 + 1), Stroke::DOTTED, Corners::Square, Layer::Under);
            let title = format!(" {} ", self.titles[group]);
            let tw = width(&title);
            let free = |x: i64| (x..x + tw).all(|x| p.is_free((x, y0)));
            // If no gap is wide enough, the title wins over the lines it covers.
            if let Some(x) = (x0 + 1..x1 - tw).find(|&x| free(x)).or((x1 - x0 > tw).then_some(x0 + 1)) {
                p.text(x, y0, &title);
            }
        }
        let boxes = (0..self.boxes.len()).map(|b| self.box_rect(b)).collect();
        Scene { boxes, painter: p }
    }

    /// Edges inside stacks. Between neighbours: straight across the gap, side
    /// by side, labels beside (LR) or above and below (TD) the lines. Between
    /// others: out of the far side, along a lane beyond the layer's boxes,
    /// and back in.
    fn paint_stack_edges(&self, p: &mut Painter) {
        let (ahead, back, home) = if self.td { ('►', '◄', '▲') } else { ('▼', '▲', '◄') };
        let mut pairs: Vec<(usize, usize)> = vec![];
        for e in &self.stack_edges {
            let (u, w, skips) = self.ends(e);
            if !skips && !pairs.contains(&(u, w)) {
                pairs.push((u, w));
            }
        }
        for (u, w) in pairs {
            let (bu, bw) = (&self.boxes[u], &self.boxes[w]);
            let mid = self.bstart[self.nodes[bu.node].layer] + (bu.lsize.min(bw.lsize) - 1) / 2;
            let (cu, cw) = (self.box_range(u).1, self.box_range(w).0);
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
                    let (x, y) = self.phys(if i == 0 { at - 1 } else { at + 1 }, cu + 1 + (gap - width(&text)) / 2);
                    p.text(x, y, &text);
                } else {
                    // The first of several lines has its label on its left.
                    let w = label.iter().map(|t| width(t)).max().unwrap_or(0);
                    let l = if i == 0 && k > 1 { at - 1 - w } else { at + 2 };
                    let top = cu + 1 + (gap - label.len() as i64) / 2;
                    for (j, t) in label.iter().enumerate() {
                        p.text(l, top + j as i64, t);
                    }
                }
            }
        }
        for k in 0..self.layers.len() {
            for (i, e) in self.brackets(k).into_iter().enumerate() {
                let lane = self.bstart[k] + self.box_len(k) + 1 + 2 * i as i64;
                let (u, w, _) = self.ends(e);
                let far = |b: usize| self.bstart[k] + self.boxes[b].lsize - 1;
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
                    let (mid, text) = ((a.1 + b.1) / 2, label.join(" "));
                    let at = if self.td { (lane + 1, mid - width(&text) / 2) } else { (lane + 2, mid) };
                    let (x, y) = self.phys(at.0, at.1);
                    p.text(x, y, &text);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn graph(src: &str) -> Graph {
        parse(&format!("flowchart TD\n{src}")).unwrap()
    }

    fn id(g: &Graph, name: &str) -> usize {
        g.g.node_weights().position(|n| n.id == name).unwrap()
    }

    #[test]
    fn break_cycles_marks_only_the_closing_edge() {
        let g = graph("a --> b\nb --> c\nc --> a\na --> d\nd --> c");
        let l = Layout::new(&g, true);
        let ends = |e: &Edge| (e.path[0], *e.path.last().unwrap());
        let heads: Vec<(usize, usize)> = l.edges.iter().filter(|e| e.head).map(ends).collect();
        assert_eq!(heads, [(id(&g, "a"), id(&g, "c"))], "c → a is laid out as a → c with a head at a");
    }

    #[test]
    fn a_stack_is_one_node_to_cycles_and_layers() {
        let g = graph("a --> b --> c\na --> c\n%% stack: a c");
        let layer = layers(&g);
        assert_eq!(layer[id(&g, "a")], layer[id(&g, "c")]);
        // Leaving the stack (a → b) and coming back (b → c) is a loop; a → c
        // is inside the stack, so it is drawn across and not counted.
        let l = Layout::new(&g, true);
        assert_eq!(l.edges.iter().filter(|e| e.head).count(), 1);
        assert_eq!(l.stack_edges.iter().map(|e| (e.src, e.dst)).collect::<Vec<_>>(), [(id(&g, "a"), id(&g, "c"))]);
    }

    #[test]
    fn layers_of_a_chain_and_a_diamond() {
        assert_eq!(layers(&graph("a --> b\nb --> c")), [0, 1, 2]);
        let g = graph("a --> b\na --> c\nb --> d\nc --> d");
        let l = layers(&g);
        assert_eq!([l[id(&g, "a")], l[id(&g, "b")], l[id(&g, "c")], l[id(&g, "d")]], [0, 1, 1, 2]);
        // A source is pulled down next to its child; a label takes a layer.
        let g = graph("a --> b\nb --> c\nx --> c\nc -->|yes| d");
        let l = layers(&g);
        assert_eq!([l[id(&g, "x")], l[id(&g, "d")]], [1, 4]);
    }

    #[test]
    fn long_edges_into_one_node_share_dummies_by_style() {
        let g = graph("a --> b\nb --> c\nc --> d\na --> d\nz --> b\nz --> d\nz -.-> d");
        let l = Layout::new(&g, true);
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
        l.lane_of = vec![0; l.nodes.len()];
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
        let n = |s| id(&g, s);
        l.layers = vec![vec![n("a"), n("b")], vec![n("c"), n("f"), n("d"), n("e")]];
        assert_eq!(crossings(&l.layers, &l.nodes), 4);
    }

    #[test]
    fn pava_pools_adjacent_violators() {
        assert_eq!(pava(&[1.0, 2.0, 3.0], &[1.0; 3]), [1.0, 2.0, 3.0]);
        assert_eq!(pava(&[1.0, 3.0, 2.0, 4.0], &[1.0; 4]), [1.0, 2.5, 2.5, 4.0]);
        assert_eq!(pava(&[3.0, 2.0, 0.0], &[1.0, 1.0, 2.0]), [1.25; 3]);
        assert_eq!(pava(&[4.0, 1.0], &[3.0, 1.0]), [3.25; 2]);
    }

    #[test]
    fn tracks_put_the_crossing_free_order_first() {
        // By span, `a` (0..10) comes first, but if it took the upper track its
        // way down (at 10) would cross b's track and b's way up (at 5) a's.
        let port = |node, side| Port { node, side, style: Style::Solid, head: false };
        let seg = |from, to, cs, ct| Seg { chan: 0, from, to, style: Style::Solid, arrow: false, cs, ct, track: 0 };
        let mut l = Layout { layers: vec![vec![], vec![]], ..Default::default() };
        l.segs.push(seg(port(0, Side::Out), port(1, Side::In), 0, 10));
        l.segs.push(seg(port(2, Side::Out), port(3, Side::In), 5, 20));
        l.tracks();
        assert_eq!((l.segs[0].track, l.segs[1].track), (1, 0));
    }
}
