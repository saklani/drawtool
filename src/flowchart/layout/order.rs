//! Ordering: each layer's nodes across the flow, for few crossings.

use super::{Kind, Lane, Layout, Node};

/// Edge crossings between consecutive layers.
pub(super) fn crossings(layers: &[Vec<usize>], nodes: &[Node]) -> usize {
    let pos = positions(layers, nodes.len());
    let pairs = |lay: &Vec<usize>| (0..lay.len()).flat_map(move |i| lay[i + 1..].iter().map(move |&b| (lay[i], b))).collect::<Vec<_>>();
    layers.iter().flat_map(pairs).map(|(a, b)| crossed(nodes, &pos, a, b, false)).sum()
}

/// Each node's place in its layer.
pub(super) fn positions(layers: &[Vec<usize>], n: usize) -> Vec<usize> {
    let mut pos = vec![0; n];
    for lay in layers {
        lay.iter().enumerate().for_each(|(i, &v)| pos[v] = i);
    }
    pos
}

/// Crossings among the edges of `a` and `b` with `a` placed first: below
/// them, and above them too if `up`.
pub(super) fn crossed(nodes: &[Node], pos: &[usize], a: usize, b: usize, up: bool) -> usize {
    let side = |na: &[usize], nb: &[usize]| na.iter().map(|&x| nb.iter().filter(|&&y| pos[x] > pos[y]).count()).sum::<usize>();
    side(&nodes[a].down, &nodes[b].down) + if up { side(&nodes[a].up, &nodes[b].up) } else { 0 }
}

impl Layout {
    /// Lanes, and each layer's nodes. Every subgraph is a lane in each row
    /// it reaches, so its frame can never enclose a stranger.
    pub(super) fn lanes(&mut self) {
        for n in self.nodes.iter().filter(|n| n.kind == Kind::Boxes || n.group.is_none()) {
            let row = self.row(n.layer);
            match self.lanes.iter_mut().find(|l| (l.group, l.row) == (n.group, row)) {
                Some(l) => (l.first, l.last) = (l.first.min(n.layer), l.last.max(n.layer)),
                None => self.lanes.push(Lane { group: n.group, row, first: n.layer, last: n.layer }),
            }
        }
        self.lanes.sort_by_key(|l| l.first);
        let lane_of = |n: &Node| {
            let row = self.row(n.layer);
            self.lanes.iter().position(|l| (l.group, l.row) == (n.group, row)).unwrap_or(0)
        };
        self.lane_of = self.nodes.iter().map(lane_of).collect();
        // A stacked box's own node is empty: the stack's first node holds them all.
        let placed = |n: &Node| n.kind != Kind::Boxes || !n.boxes.is_empty();
        for (v, n) in self.nodes.iter().enumerate().filter(|(_, n)| placed(n)) {
            self.layers[n.layer].push(v);
        }
    }

    /// Barycenter sweeps, keeping the ordering with fewest crossings. As in
    /// ELK, an exit and its entry count as neighbours while ordering (their
    /// layers are adjacent), so rows are ordered as one drawing; then each
    /// row's entries take their exits' order, so wrapping lines nest.
    pub(super) fn order(&mut self) {
        for &(x, y, _) in &self.wraps {
            self.nodes[x].down.push(y);
            self.nodes[y].up.push(x);
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
        for &(x, y, _) in &self.wraps {
            self.nodes[x].down.retain(|&v| v != y);
            self.nodes[y].up.retain(|&v| v != x);
        }
        for r in 1..self.rows.len() {
            let exits = &self.layers[self.rows[r] - 1];
            let rank = |v: &usize| self.wraps.iter().find(|w| w.1 == *v).and_then(|w| exits.iter().position(|&x| x == w.0));
            let mut entries = self.layers[self.rows[r]].clone();
            entries.sort_by_key(rank);
            self.layers[self.rows[r]] = entries;
        }
    }

    /// Swap neighbours in a layer (within a lane) while that lowers the
    /// crossings between their edges ("transpose", as in Graphviz dot).
    pub(super) fn transpose(&mut self) {
        let mut pos = positions(&self.layers, self.nodes.len());
        let cost = |nodes: &[Node], pos: &[usize], a: usize, b: usize| crossed(nodes, pos, a, b, true);
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
}
