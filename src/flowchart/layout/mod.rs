//! Flowchart layout: a simplified Sugiyama pipeline.
//!
//! break cycles → layer → dummy nodes → order (barycenter) → size → place
//! (isotonic regression) → ports → channel tracks → paint.
//!
//! Coordinates are abstract: `l` runs along the flow (layers) and `c` across
//! it; `to_xy` maps them to x/y, so TD and LR share every line of layout.
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
//!
//! A left-to-right drawing too wide for the terminal is cut into *rows*
//! between layers, as in ELK's graph wrapping: each row after the first opens
//! with a layer of *entries* and each row but the last closes with a layer of
//! *exits*, dummies where an edge leaves one row and comes back in the next.
//! Everything is laid out once; then the rows are stacked, and each exit is
//! joined to its entry by a line that runs out past the row's end, back
//! along a line of its own under the row, and in at the next row's start.


mod order;
mod painting;
mod place;
mod rank;
mod route;

pub(crate) use rank::layers;
#[cfg(test)]
use {order::crossings, place::pava};

use crate::flowchart::model::{Arrow, Direction, Graph, Mark, Shape, Style};
use crate::text::width;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

/// Gap plus dashed line between a group's members and its frame.
const FRAME: i64 = 2;

/// An edge from a box to itself, drawn as a loop out of the box's flow side
/// near its far end, around that corner, and back into the far side.
#[derive(Clone, Copy)]
struct SelfLoop {
    style: Style,
    arrow: bool,
}

struct NodeBox {
    lines: Vec<String>,
    shape: Shape,
    mark: Mark,
    looped: Option<SelfLoop>,
    /// Its layout node, and its cross-axis offset within that node.
    node: usize,
    offset: i64,
    /// Size across the flow and along it.
    across: i64,
    along: i64,
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
    across: i64,
    along: i64,
}

impl Node {
    fn new(kind: Kind, layer: usize, group: Option<usize>) -> Self {
        let (boxes, lines, up, down) = Default::default();
        Node { kind, boxes, lines, layer, group, up, down, pos: 0, across: 0, along: 0 }
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
        (self.pos - m, self.pos + self.across - 1 + m)
    }
}

/// An edge laid out forward: `path` is one node per layer from where it
/// starts to where it ends. `head` marks an edge with an arrowhead at its
/// start (a two-headed edge, or a reversed one), which gets ports of its
/// own; `tail` draws that head, on the part of the edge where it starts;
/// `arrow` is one at the end.
struct Edge {
    path: Vec<usize>,
    style: Style,
    head: bool,
    tail: bool,
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

/// One edge segment between adjacent layers, `from_at` → `to_at` across the channel.
struct Segment {
    channel: usize,
    tail: bool,
    from: Port,
    to: Port,
    style: Style,
    arrow: bool,
    from_at: i64,
    to_at: i64,
    track: i64,
}

impl Segment {
    fn span(&self) -> (i64, i64) {
        (self.from_at.min(self.to_at), self.from_at.max(self.to_at))
    }

    fn bent(&self) -> bool {
        self.from_at != self.to_at
    }
}

struct Lane {
    group: Option<usize>,
    row: usize,
    first: usize,
    last: usize,
}

fn overlaps<T: Ord + Copy>(a: (T, T), b: (T, T)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

#[cfg_attr(test, derive(Default))]
struct Layout {
    td: bool,
    gap: i64,
    titles: Vec<String>,
    boxes: Vec<NodeBox>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    stack_edges: Vec<StackEdge>,
    layers: Vec<Vec<usize>>,
    /// The layer each row starts at, and the (exit, entry, style) of every
    /// edge wrapping from one row to the next.
    rows: Vec<usize>,
    wraps: Vec<(usize, usize, Style)>,
    lanes: Vec<Lane>,
    lane_of: Vec<usize>,
    segments: Vec<Segment>,
    /// The port kinds at each node side in order of appearance, and where each port sits.
    port_kinds: HashMap<(usize, Side), Vec<(Style, bool)>>,
    port: HashMap<Port, i64>,
    /// Per layer: where its band starts along the flow, and the first track
    /// of the channel after it.
    band_start: Vec<i64>,
    track_start: Vec<i64>,
    /// Per row: its first and last cell across the flow.
    row_span: Vec<(i64, i64)>,
}

/// Render a parsed graph as text. `direction` overrides the graph's own.
#[must_use]
pub fn render(graph: &Graph, direction: Option<Direction>) -> String {
    draw(graph, direction.unwrap_or(graph.direction) == Direction::TD, &[]).0
}

/// Lay out and paint `graph`, cut into rows before each layer in `row_breaks`
/// (layers as [`layers`] numbers them; left to right only).
/// The drawing, and how wide each of its rows is.
pub(crate) fn draw(graph: &Graph, td: bool, row_breaks: &[usize]) -> (String, Vec<i64>) {
    if graph.g.node_count() == 0 {
        return (String::new(), vec![0]);
    }
    let mut l = Layout::new(graph, td, row_breaks);
    l.lanes();
    l.order();
    l.sizes();
    l.place();
    l.ports();
    l.tracks();
    let spans = l.row_span.clone();
    let p = l.paint();
    let (text, top) = (p.finish(), p.bounds().map_or(0, |b| b.0 .1));
    let lines: Vec<i64> = text.lines().map(width).collect();
    let row = |&(a, b): &(i64, i64)| (a..=b).filter_map(|y| lines.get((y - top) as usize)).copied().max().unwrap_or(0);
    let widths = if td { vec![lines.iter().copied().max().unwrap_or(0)] } else { spans.iter().map(row).collect() };
    (text, widths)
}

impl Layout {
    fn new(graph: &Graph, td: bool, row_breaks: &[usize]) -> Self {
        // Each cut adds two layers: the exits closing one row, the entries opening the next.
        let at = |l: usize| l + 2 * row_breaks.iter().filter(|&&c| c <= l).count();
        let base_layer = layers(graph);
        let layer: Vec<usize> = base_layer.iter().map(|&l| at(l)).collect();
        let defs: Vec<_> = graph.g.node_weights().collect();
        let mut boxes: Vec<NodeBox> = (defs.iter().enumerate())
            .map(|(i, n)| NodeBox {
                lines: n.lines.clone(),
                shape: n.shape,
                mark: n.mark,
                looped: None,
                node: i,
                offset: 0,
                across: 0,
                along: 0,
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
        // Back edges are the ones the layering turned round.
        let reversed: Vec<bool> = ends.iter().map(|&(s, t)| base_layer[s] > base_layer[t]).collect();
        let (mut edges, mut stack_edges) = (Vec::new(), Vec::new());
        for (e, &(s, t)) in graph.g.edge_references().zip(&ends) {
            let (src, dst, d) = (e.source().index(), e.target().index(), e.weight());
            if src == dst {
                boxes[src].looped = Some(SelfLoop { style: d.style, arrow: d.arrow != Arrow::None });
            } else if s == t {
                stack_edges.push(StackEdge { src, dst, label: d.label.clone(), style: d.style, arrow: d.arrow });
            } else {
                let r = reversed[e.id().index()];
                let both = d.arrow == Arrow::Both;
                let mut path = if r { vec![t, s] } else { vec![s, t] };
                if let Some(label) = &d.label {
                    let mut node = Node::new(Kind::Label, at(base_layer[path[0]] + 1), nodes[path[0]].group);
                    node.lines.clone_from(label);
                    nodes.push(node);
                    path.insert(1, nodes.len() - 1);
                }
                let head = both || (r && d.arrow != Arrow::None);
                edges.push(Edge { path, style: d.style, head, tail: head, arrow: both || (!r && d.arrow != Arrow::None) });
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
            rows: std::iter::once(0).chain(row_breaks.iter().map(|&c| at(c) - 1)).collect(),
            wraps: Vec::new(),
            lanes: Vec::new(),
            lane_of: Vec::new(),
            segments: Vec::new(),
            port_kinds: HashMap::new(),
            port: HashMap::new(),
            band_start: Vec::new(),
            track_start: Vec::new(),
            row_span: Vec::new(),
        };
        l.dummies();
        l
    }

    /// Dummy nodes for edges spanning several layers. Long edges of the same
    /// style into the same node share dummies, so they merge into one line as
    /// early as possible instead of each taking its own route. An edge that
    /// crosses into another row is split there, between its exit and entry.
    fn dummies(&mut self) {
        let mut shared: HashMap<(usize, usize, Style, bool), usize> = HashMap::new();
        let (nodes, rows) = (&mut self.nodes, &self.rows);
        let mut parts = Vec::new();
        for e in std::mem::take(&mut self.edges) {
            // A line between subgraphs belongs to neither frame.
            let (first, last) = (nodes[e.path[0]].group, nodes[e.path[e.path.len() - 1]].group);
            let (mut path, group) = (vec![e.path[0]], if first == last { first } else { None });
            for w in e.path.windows(2) {
                let bundle = nodes[w[1]].kind == Kind::Boxes;
                for k in nodes[w[0]].layer + 1..nodes[w[1]].layer {
                    let d = match shared.get(&(w[1], k, e.style, e.head)) {
                        Some(&d) if bundle => d,
                        _ => {
                            nodes.push(Node::new(Kind::Dummy, k, group));
                            nodes.len() - 1
                        }
                    };
                    if bundle {
                        shared.insert((w[1], k, e.style, e.head), d);
                    }
                    path.push(d);
                }
                path.push(w[1]);
            }
            let entries = (1..path.len()).filter(|&i| rows[1..].contains(&nodes[path[i]].layer));
            let mut from = 0;
            for i in entries.chain([path.len()]) {
                let (tail, arrow) = (e.tail && from == 0, e.arrow && i == path.len());
                parts.push(Edge { path: path[from..i].to_vec(), style: e.style, head: e.head, tail, arrow });
                if let Some(&entry) = path.get(i) {
                    let wrap = (path[i - 1], entry, e.style);
                    if !self.wraps.contains(&wrap) {
                        self.wraps.push(wrap);
                    }
                }
                from = i;
            }
        }
        for e in &parts {
            for w in e.path.windows(2) {
                if !nodes[w[0]].down.contains(&w[1]) {
                    nodes[w[0]].down.push(w[1]);
                    nodes[w[1]].up.push(w[0]);
                }
            }
        }
        self.edges = parts;
    }

    /// The row layer `k` is in.
    fn row(&self, k: usize) -> usize {
        self.rows.iter().filter(|&&s| s <= k).count() - 1
    }

    /// The layers of row `r`.
    fn row_layers(&self, r: usize) -> std::ops::Range<usize> {
        self.rows[r]..self.rows.get(r + 1).copied().unwrap_or(self.layers.len())
    }

    /// The edges wrapping into row `r`, top to bottom.
    fn wraps_into(&self, r: usize) -> Vec<(usize, usize, Style)> {
        let mut w: Vec<_> = self.wraps.iter().copied().filter(|w| r > 0 && self.nodes[w.1].layer == self.rows[r]).collect();
        w.sort_by_key(|w| self.nodes[w.0].pos);
        w
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowchart::parse::parse;

    fn graph(src: &str) -> Graph {
        parse(&format!("flowchart TD\n{src}")).unwrap()
    }

    fn id(g: &Graph, name: &str) -> usize {
        g.g.node_weights().position(|n| n.id == name).unwrap()
    }

    #[test]
    fn break_cycles_marks_only_the_closing_edge() {
        let g = graph("a --> b\nb --> c\nc --> a\na --> d\nd --> c");
        let l = Layout::new(&g, true, &[]);
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
        let l = Layout::new(&g, true, &[]);
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
        let l = Layout::new(&g, true, &[]);
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
        let mut l = Layout::new(&g, true, &[]);
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
        let mut l = Layout::new(&g, true, &[]);
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
        let seg = |from, to, from_at, to_at| Segment { channel: 0, tail: false, from, to, style: Style::Solid, arrow: false, from_at, to_at, track: 0 };
        let mut l = Layout { layers: vec![vec![], vec![]], ..Default::default() };
        l.segments.push(seg(port(0, Side::Out), port(1, Side::In), 0, 10));
        l.segments.push(seg(port(2, Side::Out), port(3, Side::In), 5, 20));
        l.tracks();
        assert_eq!((l.segments[0].track, l.segments[1].track), (1, 0));
    }
}
