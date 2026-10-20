//! Layering: which layer each node goes in, with the edges that close
//! cycles turned round.

use crate::flowchart::model::Graph;
use petgraph::visit::EdgeRef;

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
        let child = |e: &(usize, usize, usize)| if e.2 > 1 { layer[v] + 1 } else { layer[e.1] };
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

/// Each graph node's layer. A stack counts as one node, and a subgraph is
/// laid out as one (as Ondine's compound layout does), so its frame spans
/// only the layers its own boxes need.
pub(crate) fn layers(graph: &Graph) -> Vec<usize> {
    let n = graph.g.node_count();
    let mut unit: Vec<usize> = (0..n).collect();
    for st in &graph.stacks {
        for v in st {
            unit[v.index()] = st[0].index();
        }
    }
    let spans: Vec<(usize, usize, usize)> = (graph.g.edge_references())
        .map(|e| (unit[e.source().index()], unit[e.target().index()], 1 + usize::from(e.weight().label.is_some())))
        .collect();
    let layer_all = |n: usize, edges: &[(usize, usize, usize)]| {
        let ends: Vec<(usize, usize)> = edges.iter().map(|e| (e.0, e.1)).collect();
        layering(n, edges, &back_edges(n, &ends))
    };
    // Inside each subgraph, over its own edges; then each subgraph as one node.
    let outer = |v: usize| graph.g[petgraph::graph::NodeIndex::new(v)].group.map_or(v, |g| n + g);
    let inner = layer_all(n, &spans.iter().filter(|e| outer(e.0) == outer(e.1) && outer(e.0) >= n).copied().collect::<Vec<_>>());
    let cross: Vec<&(usize, usize, usize)> = spans.iter().filter(|e| outer(e.0) != outer(e.1)).collect();
    let ends: Vec<(usize, usize)> = cross.iter().map(|e| (outer(e.0), outer(e.1))).collect();
    let units = n + graph.groups.len();
    // An edge out of a subgraph starts past the box it leaves.
    let oriented: Vec<_> = (cross.iter().zip(back_edges(units, &ends)))
        .map(|(&&(s, t, span), r)| if r { (outer(t), outer(s), span + inner[t]) } else { (outer(s), outer(t), span + inner[s]) })
        .collect();
    let top = layering(units, &oriented, &vec![false; oriented.len()]);
    unit.iter().map(|&u| top[outer(u)] + inner[u]).collect()
}
