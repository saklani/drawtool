//! Placement: node and box sizes, then cross-axis positions.

use super::{FRAME, Kind, Layout, Port, Seg, Side, overlaps};
use crate::text::width;

/// Weighted isotonic (non-decreasing) regression.
pub(super) fn pava(ys: &[f64], ws: &[f64]) -> Vec<f64> {
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

impl Layout {
    /// Segments (a box's size depends on its ports), then box and node sizes.
    pub(super) fn sizes(&mut self) {
        for e in &self.edges {
            for (j, w) in e.path.windows(2).enumerate() {
                let port = |node, side, head| Port { node, side, style: e.style, head };
                // Lines that end in an arrowhead share a port; a line without one keeps apart.
                let (from, to) = (port(w[0], Side::Out, e.head), port(w[1], Side::In, !e.arrow));
                let (chan, arrow) = (self.nodes[w[0]].layer, e.arrow && j == e.path.len() - 2);
                self.segs.push(Seg { chan, tail: j == 0 && e.tail, from, to, style: e.style, arrow, cs: 0, ct: 0, track: 0 });
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
    pub(super) fn space(&self, a: usize, b: Option<&usize>) -> i64 {
        let kind = |v: usize| self.nodes[v].kind;
        match (kind(a), b.map(|&b| kind(b))) {
            (Kind::Dummy, _) | (_, Some(Kind::Dummy)) => i64::from(self.td),
            (Kind::Label, Some(Kind::Label)) if self.td => self.gap + 3,
            _ => self.gap,
        }
    }

    /// A node's cross-axis centre: for a stack, its first box's.
    pub(super) fn cen(&self, v: usize) -> i64 {
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
    pub(super) fn pack(&mut self) {
        for lay in &self.layers {
            for w in lay.windows(2) {
                let floor = self.nodes[w[0]].pos + self.nodes[w[0]].csize + self.space(w[0], Some(&w[1]));
                self.nodes[w[1]].pos = self.nodes[w[1]].pos.max(floor);
            }
        }
    }

    /// Pull nodes toward their neighbours while keeping order and spacing,
    /// then push lanes apart so frames have room.
    pub(super) fn place(&mut self) {
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
                let (lay, v) = (&self.layers[k], self.layers[k][i]);
                let nd = &self.nodes[v];
                if nd.kind == Kind::Boxes || nd.up.len() != 1 || nd.down.len() != 1 {
                    continue;
                }
                // Clear of its neighbours, and of their frames when in another lane.
                let margin = |w: usize| if self.lane_of[w] == self.lane_of[v] { 0 } else { 2 * FRAME };
                let lo = i.checked_sub(1).map_or(i64::MIN, |j| {
                    let p = lay[j];
                    self.nodes[p].pos + self.nodes[p].csize + self.space(p, Some(&v)) + margin(p)
                });
                let hi = lay.get(i + 1).map_or(i64::MAX, |&q| self.nodes[q].pos - self.space(v, Some(&q)) - margin(q) - nd.csize);
                let off = self.cen(v) - nd.pos;
                let ends = [nd.up[0], nd.down[0]].map(|m| self.cen(m) - off);
                if let Some(p) = ends.into_iter().filter(|p| (lo..=hi).contains(p)).min_by_key(|p| (p - nd.pos).abs()) {
                    self.nodes[v].pos = p;
                }
            }
        }
    }

    /// Move layer `k`'s nodes toward their neighbours in the layer before
    /// (`up`) or after it, keeping their order and spacing.
    pub(super) fn sweep(&mut self, k: usize, up: bool) {
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
    pub(super) fn box_range(&self, b: usize) -> (i64, i64) {
        let bx = &self.boxes[b];
        let lo = self.nodes[bx.node].pos + bx.offset;
        (lo, lo + bx.csize - 1)
    }
}
