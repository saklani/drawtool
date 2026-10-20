//! Placement: node and box sizes, then cross-axis positions.

use super::{FRAME, Kind, Layout, Port, Segment, Side, overlaps};
use crate::text::width;

/// Weighted isotonic (non-decreasing) regression.
pub(super) fn pava(targets: &[f64], weights: &[f64]) -> Vec<f64> {
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for (&y, &w) in targets.iter().zip(weights) {
        blocks.push((y, w, 1));
        while blocks.len() > 1 && blocks[blocks.len() - 2].0 > blocks[blocks.len() - 1].0 {
            let (v2, w2, c2) = blocks.pop().unwrap();
            let (v1, w1, c1) = blocks.pop().unwrap();
            blocks.push(((v1 * w1 + v2 * w2) / (w1 + w2), w1 + w2, c1 + c2));
        }
    }
    blocks.iter().flat_map(|&(v, _, c)| std::iter::repeat_n(v, c)).collect()
}

impl Layout {
    /// Segments (a box's size depends on its ports), then box and node sizes.
    pub(super) fn sizes(&mut self) {
        for e in &self.edges {
            for (j, w) in e.path.windows(2).enumerate() {
                let port = |node, side, head| Port { node, side, style: e.style, head };
                // Lines that end in an arrowhead share a port; a line without one keeps apart.
                let (from, to) = (port(w[0], Side::Out, e.head), port(w[1], Side::In, !e.arrow));
                let (channel, arrow) = (self.nodes[w[0]].layer, e.arrow && j == e.path.len() - 2);
                self.segments.push(Segment { channel, tail: j == 0 && e.tail, from, to, style: e.style, arrow, from_at: 0, to_at: 0, track: 0 });
            }
        }
        for s in &self.segments {
            for p in [s.from, s.to] {
                let kinds = self.port_kinds.entry((p.node, p.side)).or_default();
                if !kinds.contains(&(p.style, p.head)) {
                    kinds.push((p.style, p.head));
                }
            }
        }
        for v in 0..self.nodes.len() {
            let ports = |side| self.port_kinds.get(&(v, side)).map_or(0, Vec::len) as i64;
            let port_count = ports(Side::In).max(ports(Side::Out));
            let node = &self.nodes[v];
            let boxes = node.boxes.clone();
            let (text_width, line_count) = (node.lines.iter().map(|t| width(t)).max().unwrap_or(0), node.lines.len() as i64);
            (self.nodes[v].across, self.nodes[v].along) = match node.kind {
                Kind::Dummy => (1, 0),
                Kind::Label if self.td => (text_width, line_count),
                Kind::Label => (line_count, text_width),
                Kind::Boxes => (0, 0),
            };
            let (mut offset, mut node_along) = (0, 0);
            for (i, &b) in boxes.iter().enumerate() {
                let bx = &self.boxes[b];
                let (text_width, line_count) = (bx.lines.iter().map(|t| width(t)).max().unwrap_or(0), bx.lines.len() as i64);
                // Across the flow: the text, or two cells per port; a self-loop
                // adds a cell to leave by and one of padding to go round.
                // Stacked boxes are long enough along the flow for the lines
                // to each neighbour side by side.
                let loop_room = i64::from(bx.looped.is_some());
                let mut box_across = if self.td { text_width + 4 } else { line_count + 2 }.max(2 * port_count + 1) + loop_room;
                let mut box_along = if self.td { line_count + 2 } else { text_width + 4 };
                if boxes.len() > 1 {
                    let lines = |o: Option<&usize>| o.map_or(0, |&u| self.between(u, b).count() as i64);
                    box_along = box_along.max(2 * lines(boxes.get(i.wrapping_sub(1))).max(lines(boxes.get(i + 1))) + 1);
                    if self.stack_edges.iter().any(|e| (e.src == b || e.dst == b) && self.ends(e).2) {
                        box_across = box_across.max(5);
                    }
                }
                let bx = &mut self.boxes[b];
                (bx.offset, bx.across, bx.along) = (offset, box_across, box_along);
                offset += box_across + loop_room + boxes.get(i + 1).map_or(0, |&n| self.stack_gap(b, n));
                node_along = node_along.max(box_along);
            }
            if !boxes.is_empty() {
                (self.nodes[v].across, self.nodes[v].along) = (offset, node_along);
            }
        }
    }

    /// Space after node `a` in its layer: none before or after a dummy (a
    /// line may run right beside a label or another line), wider between
    /// two TD edge labels, so side-by-side labels don't read as one phrase.
    pub(super) fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        let kind = |v: usize| self.nodes[v].kind;
        match (kind(a), b.map(|&b| kind(b))) {
            (Kind::Dummy, _) | (_, Some(Kind::Dummy)) => i64::from(self.td),
            (Kind::Label, Some(Kind::Label)) if self.td => self.gap + 3,
            _ => self.gap,
        }
    }

    /// A node's cross-axis centre: for a stack, its first box's.
    pub(super) fn centre(&self, v: usize) -> i64 {
        let n = &self.nodes[v];
        match n.boxes.first() {
            Some(&b) => {
                let (lo, hi) = self.box_range(b);
                (lo + hi) / 2
            }
            None => n.pos + (n.across - 1) / 2,
        }
    }

    /// Move nodes along their layer until none starts before the one ahead
    /// of it ends (plus spacing).
    pub(super) fn pack(&mut self) {
        for layer_nodes in &self.layers {
            for w in layer_nodes.windows(2) {
                let floor = self.nodes[w[0]].pos + self.nodes[w[0]].across + self.space(w[0], Some(&w[1]));
                self.nodes[w[1]].pos = self.nodes[w[1]].pos.max(floor);
            }
        }
    }

    /// Pull nodes toward their neighbours while keeping order and spacing,
    /// then push lanes apart so frames have room.
    pub(super) fn place(&mut self) {
        self.pack();
        let n_layers = self.layers.len();
        for _ in 0..8 {
            (1..n_layers).for_each(|k| self.sweep(k, true));
            (0..n_layers - 1).rev().for_each(|k| self.sweep(k, false));
        }
        (1..n_layers).for_each(|k| self.sweep(k, true));
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
                .map(|&v| (self.nodes[v].pos, self.nodes[v].pos + self.nodes[v].across - 1))
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
        self.align();
        // Rows one under another, with a line between them per edge wrapping.
        let mut bottom = 0;
        for r in 0..self.rows.len() {
            let members: Vec<usize> = self.layers[self.row_layers(r)].concat();
            let (lo, hi) = (members.iter().map(|&v| self.nodes[v].framed()))
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                .unwrap_or((0, -1));
            let top = if r == 0 { 0 } else { bottom + 1 + self.wraps_into(r).len().max(1) as i64 };
            for &v in &members {
                self.nodes[v].pos += top - lo;
            }
            bottom = hi + top - lo;
            self.row_span.push((top, bottom));
        }
    }

    /// Line each label and bend up with one end of its edge, as Ondine's
    /// `align_labels` does, where its layer has room: the edge then turns
    /// once instead of twice.
    pub(super) fn align(&mut self) {
        for k in 0..self.layers.len() {
            for i in 0..self.layers[k].len() {
                let (layer_nodes, v) = (&self.layers[k], self.layers[k][i]);
                let node = &self.nodes[v];
                if node.kind == Kind::Boxes || node.up.len() != 1 || node.down.len() != 1 {
                    continue;
                }
                // Clear of its neighbours, and of their frames when in another lane.
                let margin = |w: usize| if self.lane_of[w] == self.lane_of[v] { 0 } else { 2 * FRAME };
                let lo = i.checked_sub(1).map_or(i64::MIN, |j| {
                    let p = layer_nodes[j];
                    self.nodes[p].pos + self.nodes[p].across + self.space(p, Some(&v)) + margin(p)
                });
                let hi = layer_nodes.get(i + 1).map_or(i64::MAX, |&q| self.nodes[q].pos - self.space(v, Some(&q)) - margin(q) - node.across);
                let off = self.centre(v) - node.pos;
                let ends = [node.up[0], node.down[0]].map(|m| self.centre(m) - off);
                if let Some(p) = ends.into_iter().filter(|p| (lo..=hi).contains(p)).min_by_key(|p| (p - node.pos).abs()) {
                    self.nodes[v].pos = p;
                }
            }
        }
    }

    /// Move layer `k`'s nodes toward their neighbours in the layer before
    /// (`up`) or after it, keeping their order and spacing.
    pub(super) fn sweep(&mut self, k: usize, up: bool) {
        let (mut targets, mut weights, mut offsets, mut c) = (vec![], vec![], vec![], 0i64);
        for (i, &v) in self.layers[k].iter().enumerate() {
            let node = &self.nodes[v];
            let neighbours = if up { &node.up } else { &node.down };
            let (want, w) = if neighbours.is_empty() {
                (self.centre(v) as f64, 0.05)
            } else {
                (neighbours.iter().map(|&m| self.centre(m)).sum::<i64>() as f64 / neighbours.len() as f64, node.pull())
            };
            targets.push(want - (self.centre(v) - node.pos) as f64 - c as f64);
            weights.push(w);
            offsets.push(c);
            c += node.across + self.space(v, self.layers[k].get(i + 1));
        }
        for ((&v, y), o) in self.layers[k].iter().zip(pava(&targets, &weights)).zip(offsets) {
            self.nodes[v].pos = y.round_ties_even() as i64 + o;
        }
    }

    /// The cross-axis range a box covers.
    pub(super) fn box_range(&self, b: usize) -> (i64, i64) {
        let bx = &self.boxes[b];
        let lo = self.nodes[bx.node].pos + bx.offset;
        (lo, lo + bx.across - 1)
    }
}
