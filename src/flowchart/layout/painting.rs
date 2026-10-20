//! Painting: layer-axis coordinates, then boxes, labels, edges and frames
//! onto the character grid.

use super::{FRAME, Kind, Layout, Node, Port, SelfLoop, StackEdge, overlaps};
use crate::flowchart::model::{Arrow, Mark, Shape, Style};
use crate::paint::{Corners, Layer, Painter, Pt, Stroke, Weight};
use crate::text::width;
use std::collections::HashMap;

impl Layout {
    /// The lanes that get a frame: each subgraph, with its boxes' layer span.
    pub(super) fn frames(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.lanes.iter().filter_map(|l| l.group.map(|g| (g, l.first, l.last)))
    }

    /// The stack edges of layer `k` drawn as brackets beyond its boxes.
    pub(super) fn brackets(&self, k: usize) -> Vec<&StackEdge> {
        self.stack_edges.iter().filter(|e| self.nodes[self.boxes[e.src].node].layer == k && self.ends(e).2).collect()
    }

    /// The cross-axis cell where a bracket meets box `b`, on its far side:
    /// near the end of the upper box and the start of the lower one.
    pub(super) fn bracket_at(&self, b: usize, upper: bool) -> i64 {
        let (lo, hi) = self.box_range(b);
        if upper {
            hi - 1
        } else {
            lo + 1
        }
    }

    pub(super) fn box_len(&self, k: usize) -> i64 {
        self.layers[k].iter().map(|&v| self.nodes[v].lsize).max().unwrap_or(0)
    }

    /// Length of layer `k` along the flow: its longest box, then its
    /// brackets (a lane each, two cells apart) and their labels; for a layer
    /// of exits, the columns its wrapping lines turn down.
    pub(super) fn band_len(&self, k: usize) -> i64 {
        let brackets = self.brackets(k);
        let label = |l: &Vec<String>| if self.td { 1 } else { l.iter().map(|t| width(t)).max().unwrap_or(0) + 1 };
        let labels = brackets.iter().filter_map(|e| e.label.as_ref()).map(label).max().unwrap_or(0);
        let exits = self.wraps.iter().filter(|w| self.nodes[w.0].layer == k).count() as i64;
        self.box_len(k) + 2 * brackets.len() as i64 + labels + (2 * exits - 1).max(0)
    }

    /// Layer-axis coordinates. Channels next to a frame get FRAME extra cells.
    pub(super) fn coordinates(&mut self) {
        let mut cur = 0;
        for k in 0..self.layers.len() {
            // A row starts at the last column its wrapping lines come down.
            if let Some(r) = self.rows.iter().position(|&s| s == k) {
                let lines = 2 * self.wraps_into(r).len() as i64 - 1;
                cur = lines.max(0) + if self.frames().any(|f| f.1 == k) { FRAME } else { 0 };
            }
            self.bstart.push(cur);
            cur += self.band_len(k);
            let segs = self.segs.iter().filter(|s| s.chan == k);
            let ntracks = segs.clone().filter(|s| s.bent()).map(|s| s.track + 1).max().unwrap_or(0);
            // A head at a segment's start takes a cell, and a line cell after
            // it keeps it apart from the tracks and other heads.
            let head = segs.clone().any(|s| s.tail);
            let lead = i64::from(!self.td)
                + if head { 1 + i64::from(self.td) } else { 0 }
                + if self.frames().any(|f| f.2 == k) { FRAME } else { 0 };
            let before_tail = if self.frames().any(|f| f.1 == k + 1) { FRAME } else { 0 };
            self.tstart.push(cur + lead);
            cur += lead + ntracks + before_tail + 1;
        }
    }

    pub(super) fn phys(&self, l: i64, c: i64) -> Pt {
        if self.td {
            (c, l)
        } else {
            (l, c)
        }
    }

    /// A box's top-left and size.
    pub(super) fn box_rect(&self, b: usize) -> (Pt, Pt) {
        let (bx, (lo, hi)) = (&self.boxes[b], self.box_range(b));
        let (l0, w) = (self.bstart[self.nodes[bx.node].layer], hi - lo + 1);
        (self.phys(l0, lo), if self.td { (w, bx.lsize) } else { (bx.lsize, w) })
    }

    pub(super) fn paint(mut self) -> Painter {
        self.coordinates();
        let mut p = Painter::new();
        p.mark_crossings = true;
        for b in 0..self.boxes.len() {
            let ((x, y), (w, h)) = self.box_rect(b);
            let bx = &self.boxes[b];
            let (weight, style, corners) = match (bx.mark, bx.shape) {
                (Mark::Highlight, _) => (Weight::Heavy, Style::Solid, Corners::Square),
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
            let start = (out_l(s.from.node) + i64::from(s.tail), s.cs);
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
            if s.tail {
                let (x, y) = self.phys(out_l(s.from.node) + 1, s.cs);
                p.symbol(x, y, back_arrow);
            }
        }
        self.paint_stack_edges(&mut p);
        self.paint_wraps(&mut p);

        // Group frames sit under everything; titles slide past crossing edges.
        for (group, lo, hi) in self.frames() {
            let members = (self.layers[lo..=hi].iter().flatten().map(|&v| &self.nodes[v]))
                .filter(|n| n.group == Some(group) && n.kind == Kind::Boxes);
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
        p
    }

    /// Edges wrapping into each row: out past the exits, down to a line of
    /// their own under the row above, back to the row's start, and down into
    /// their entries. The top exit runs out farthest, along the lowest line,
    /// and in nearest to the row, so the lines nest without crossing.
    pub(super) fn paint_wraps(&self, p: &mut Painter) {
        for r in 1..self.rows.len() {
            let wraps = self.wraps_into(r);
            let m = wraps.len() as i64;
            for (i, &(exit, entry, style)) in wraps.iter().enumerate() {
                let (out, line) = (2 * (m - 1 - i as i64), self.row_span[r].0 - 1 - i as i64);
                let (lx, cx, cy) = (self.bstart[self.nodes[exit].layer], self.cen(exit), self.cen(entry));
                let mut pts = vec![(lx, cx), (lx + out, cx), (lx + out, line), (out, line), (out, cy)];
                pts.push((self.bstart[self.nodes[entry].layer] - 1, cy));
                pts.dedup();
                let pts: Vec<Pt> = pts.into_iter().map(|(l, c)| self.phys(l, c)).collect();
                let net = p.net();
                p.line(&pts, net, Stroke::light(style), Layer::Main);
            }
        }
    }

    /// Edges inside stacks. Between neighbours: straight across the gap, side
    /// by side, labels beside (LR) or above and below (TD) the lines. Between
    /// others: out of the far side, along a lane beyond the layer's boxes,
    /// and back in.
    pub(super) fn paint_stack_edges(&self, p: &mut Painter) {
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
