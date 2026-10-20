//! Routing: where edges meet boxes, and the tracks they bend on.

use super::{Layout, Port, Seg, Side, overlaps};
use crate::flowchart::model::Style;
use std::collections::{HashMap, HashSet};

impl Layout {
    /// Port positions, after Ondine's `spread`: a side's ports in the order
    /// their edges go, the straightest at the centre. A back edge has ports
    /// of its own, apart from the forward edges.
    pub(super) fn ports(&mut self) {
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
            // In the order their edges go, so no two cross here, around the
            // straightest at the centre.
            let mut kinds = kinds.clone();
            kinds.sort_by_key(|k| (lean(k), k.1));
            let free: Vec<i64> = (lo..=hi).filter(|x| !taken.contains(x)).collect();
            let (count, last) = (kinds.len() as i64, free.len() as i64 - 1);
            let mid_kind = (0..count).min_by_key(|&i| lean(&kinds[i as usize]).abs()).unwrap_or(0);
            let mid = free.iter().position(|&x| x >= c).map_or(last, |i| i as i64);
            // A cell between ports where there's room for it, on each side of the centre.
            let (gl, gr) = (if mid >= 2 * mid_kind { 2 } else { 1 }, if last - mid >= 2 * (count - 1 - mid_kind) { 2 } else { 1 });
            for (i, kind) in (0..).zip(kinds) {
                let at = mid + if i < mid_kind { gl } else { gr } * (i - mid_kind);
                let x = free.get(at.max(i).min(last - (count - 1 - i)).max(0) as usize);
                self.port.insert(Port { node: n, side, style: kind.0, head: kind.1 }, x.copied().unwrap_or(c));
            }
        }
        // A two-way pair keeps one order at both ends, so it never crosses itself.
        let segs = &self.segs;
        let twins = segs.iter().filter(|s| !s.from.head).flat_map(|s| {
            let twin = |t: &&Seg| t.from.head && (t.from.node, t.to.node) == (s.from.node, s.to.node);
            segs.iter().filter(twin).map(|t| (s.from, s.to, t.from, t.to))
        });
        for (sf, st, tf, tt) in twins.collect::<Vec<_>>() {
            let (a, b, c, d) = (self.port[&sf], self.port[&st], self.port[&tf], self.port[&tt]);
            if (a - c) * (b - d) < 0 {
                self.port.insert(st, d);
                self.port.insert(tt, b);
            }
        }
        for s in &mut self.segs {
            (s.cs, s.ct) = (self.port[&s.from], self.port[&s.to]);
        }
    }

    /// Give bent segments a track (a row/column of their channel) so that
    /// unrelated horizontals never share one.
    pub(super) fn tracks(&mut self) {
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
}
