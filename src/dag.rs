//! DAG conversion and cycle-breaking algorithms for Digraph Explorer.
//!
//! All fast transforms use only the graph topology.  The exact feedback-set
//! solvers are deliberately bounded: they return a valid heuristic DAG when
//! the Nintendo DS work budget is exhausted instead of blocking the UI.

extern crate alloc;

use alloc::{vec, vec::Vec};
use serde::{Deserialize, Serialize};

use crate::graph::{COLOR_MARK, Digraph, Edge, EdgeId, Vertex, VertexId};

/// Hard cap for exact feedback-arc search.  The algorithm is still exponential;
/// this cap and the node budget make it safe to expose on DS hardware.
pub const EXACT_FAS_MAX_VERTICES: usize = 24;
pub const EXACT_FAS_MAX_ARCS: usize = 128;
/// Hard cap for exact directed feedback-vertex search.
pub const EXACT_FVS_MAX_VERTICES: usize = 28;
/// Recursive search-node budget shared by iterative-deepening passes.
pub const EXACT_SEARCH_BUDGET: u32 = 60_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DagTransformKind {
    /// Contract every strongly connected component to one super-vertex.
    SccCondensation,
    /// Keep only arcs that point forward in the stable vertex-ID order.
    VertexOrder,
    /// Run DFS and remove exactly the DFS back arcs.
    DfsBackEdgeRemoval,
    /// Minimum feedback arc set when the bounded exact search completes;
    /// otherwise a valid DFS feedback-arc set.
    MinimumFeedbackArcSet,
    /// Minimum directed feedback vertex set when the bounded exact search
    /// completes; otherwise a valid greedy feedback-vertex set.
    MinimumFeedbackVertexSet,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArcRef {
    pub edge_id: EdgeId,
    pub from: VertexId,
    pub to: VertexId,
    /// 0 for the stored orientation.  1 is the reverse orientation contributed
    /// by an undirected edge.  This lets overlays represent an undirected edge
    /// as two graph-theoretic arcs without changing the original graph.
    pub orientation: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CondensationNode {
    pub component: u16,
    pub x: i16,
    pub y: i16,
    pub members: Vec<VertexId>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CondensationArc {
    pub from_component: u16,
    pub to_component: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DagTransform {
    pub kind: DagTransformKind,
    /// True only when an optimization result is proven minimum.  Linear
    /// transforms are deterministic rather than optimization problems and are
    /// also marked exact.
    pub exact: bool,
    /// True if a bounded exact solver stopped because of the size/work cap.
    pub truncated: bool,
    /// Search nodes consumed by bounded exact solvers.
    pub work: u32,
    /// A topological ordering of the resulting DAG when the result uses the
    /// original vertices.  Condensation uses component IDs instead.
    pub ordering: Vec<VertexId>,
    /// Directed arcs retained by the resulting acyclic subgraph.
    pub kept_arcs: Vec<ArcRef>,
    /// Arcs deleted by feedback-arc/order/DFS transforms.
    pub removed_arcs: Vec<ArcRef>,
    /// Vertices deleted by feedback-vertex-set transforms.
    pub removed_vertices: Vec<VertexId>,
    /// SCC super-vertices for condensation.
    pub components: Vec<CondensationNode>,
    /// Edges of the condensation DAG.
    pub condensation_arcs: Vec<CondensationArc>,
}

impl DagTransform {
    fn empty(kind: DagTransformKind) -> Self {
        Self {
            kind,
            exact: true,
            truncated: false,
            work: 0,
            ordering: Vec::new(),
            kept_arcs: Vec::new(),
            removed_arcs: Vec::new(),
            removed_vertices: Vec::new(),
            components: Vec::new(),
            condensation_arcs: Vec::new(),
        }
    }

    pub fn resulting_arc_count(&self) -> usize {
        if self.kind == DagTransformKind::SccCondensation {
            self.condensation_arcs.len()
        } else {
            self.kept_arcs.len()
        }
    }
}

#[derive(Clone, Copy)]
struct ArcIx {
    arc: ArcRef,
    from: usize,
    to: usize,
}

fn expanded_arcs(graph: &Digraph) -> Vec<ArcIx> {
    let mut out = Vec::with_capacity(graph.edges.len().saturating_mul(2));
    for edge in &graph.edges {
        let Some(from) = graph.vertex_index(edge.from) else {
            continue;
        };
        let Some(to) = graph.vertex_index(edge.to) else {
            continue;
        };
        out.push(ArcIx {
            arc: ArcRef {
                edge_id: edge.id,
                from: edge.from,
                to: edge.to,
                orientation: 0,
            },
            from,
            to,
        });
        if !edge.directed {
            out.push(ArcIx {
                arc: ArcRef {
                    edge_id: edge.id,
                    from: edge.to,
                    to: edge.from,
                    orientation: 1,
                },
                from: to,
                to: from,
            });
        }
    }
    out
}

fn adjacency(n: usize, arcs: &[ArcIx]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); n];
    for (i, arc) in arcs.iter().enumerate() {
        adj[arc.from].push(i);
    }
    adj
}

fn topological_order_filtered(
    graph: &Digraph,
    arcs: &[ArcIx],
    removed_arc: Option<&[bool]>,
    removed_vertex: Option<&[bool]>,
) -> Option<Vec<VertexId>> {
    let n = graph.vertices.len();
    let mut indegree = vec![0u16; n];
    let mut adj = vec![Vec::<usize>::new(); n];

    for (ai, arc) in arcs.iter().enumerate() {
        if removed_arc.is_some_and(|r| r[ai])
            || removed_vertex.is_some_and(|r| r[arc.from] || r[arc.to])
        {
            continue;
        }
        indegree[arc.to] = indegree[arc.to].saturating_add(1);
        adj[arc.from].push(arc.to);
    }

    let mut queue = Vec::with_capacity(n);
    for i in 0..n {
        if removed_vertex.is_some_and(|r| r[i]) {
            continue;
        }
        if indegree[i] == 0 {
            queue.push(i);
        }
    }

    // A small vector queue avoids VecDeque's extra bookkeeping and remains O(V+E)
    // because `head` only moves forward.
    let mut head = 0usize;
    let mut order = Vec::with_capacity(n);
    while head < queue.len() {
        let v = queue[head];
        head += 1;
        order.push(graph.vertices[v].id);
        for &w in &adj[v] {
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push(w);
            }
        }
    }

    let wanted = (0..n)
        .filter(|&i| !removed_vertex.is_some_and(|r| r[i]))
        .count();
    (order.len() == wanted).then_some(order)
}

pub fn is_dag(graph: &Digraph) -> bool {
    let arcs = expanded_arcs(graph);
    topological_order_filtered(graph, &arcs, None, None).is_some()
}

// ---------------------------------------------------------------------------
// SCC condensation -- Tarjan, O(V + E)
// ---------------------------------------------------------------------------

struct Tarjan<'a> {
    arcs: &'a [ArcIx],
    adj: Vec<Vec<usize>>,
    next_index: i16,
    index: Vec<i16>,
    low: Vec<i16>,
    stack: Vec<usize>,
    on_stack: Vec<bool>,
    component_of: Vec<u16>,
    components: Vec<Vec<usize>>,
}

impl<'a> Tarjan<'a> {
    fn new(n: usize, arcs: &'a [ArcIx]) -> Self {
        Self {
            arcs,
            adj: adjacency(n, arcs),
            next_index: 0,
            index: vec![-1; n],
            low: vec![0; n],
            stack: Vec::with_capacity(n),
            on_stack: vec![false; n],
            component_of: vec![u16::MAX; n],
            components: Vec::new(),
        }
    }

    fn visit(&mut self, v: usize) {
        let idx = self.next_index;
        self.next_index += 1;
        self.index[v] = idx;
        self.low[v] = idx;
        self.stack.push(v);
        self.on_stack[v] = true;

        // Clone only the tiny index list so recursive mutable access to `self`
        // remains simple and predictable on no_std Rust.
        let outgoing = self.adj[v].clone();
        for ai in outgoing {
            let w = self.arcs[ai].to;
            if self.index[w] < 0 {
                self.visit(w);
                self.low[v] = self.low[v].min(self.low[w]);
            } else if self.on_stack[w] {
                self.low[v] = self.low[v].min(self.index[w]);
            }
        }

        if self.low[v] == self.index[v] {
            let cid = self.components.len() as u16;
            let mut members = Vec::new();
            loop {
                let w = self.stack.pop().expect("Tarjan stack cannot be empty");
                self.on_stack[w] = false;
                self.component_of[w] = cid;
                members.push(w);
                if w == v {
                    break;
                }
            }
            self.components.push(members);
        }
    }

    fn run(mut self) -> (Vec<u16>, Vec<Vec<usize>>) {
        for v in 0..self.index.len() {
            if self.index[v] < 0 {
                self.visit(v);
            }
        }
        (self.component_of, self.components)
    }
}

pub fn scc_condensation(graph: &Digraph) -> DagTransform {
    let arcs = expanded_arcs(graph);
    let (component_of, raw_components) = Tarjan::new(graph.vertices.len(), &arcs).run();
    let component_count = raw_components.len();

    let mut result = DagTransform::empty(DagTransformKind::SccCondensation);
    result.components.reserve(component_count);

    for (cid, members) in raw_components.iter().enumerate() {
        let mut ids = Vec::with_capacity(members.len());
        let mut sx = 0i32;
        let mut sy = 0i32;
        for &vi in members {
            let v = graph.vertices[vi];
            ids.push(v.id);
            sx += i32::from(v.x);
            sy += i32::from(v.y);
        }
        ids.sort_unstable();
        let denom = members.len().max(1) as i32;
        result.components.push(CondensationNode {
            component: cid as u16,
            x: (sx / denom) as i16,
            y: (sy / denom) as i16,
            members: ids,
        });
    }

    // Deduplicate component edges in O(V+E+C) with a generation-mark array.
    let mut by_component = vec![Vec::<usize>::new(); component_count];
    for (ai, arc) in arcs.iter().enumerate() {
        let c = component_of[arc.from] as usize;
        by_component[c].push(ai);
    }
    let mut mark = vec![usize::MAX; component_count];
    for from_c in 0..component_count {
        for &ai in &by_component[from_c] {
            let to_c = component_of[arcs[ai].to] as usize;
            if from_c == to_c || mark[to_c] == from_c {
                continue;
            }
            mark[to_c] = from_c;
            result.condensation_arcs.push(CondensationArc {
                from_component: from_c as u16,
                to_component: to_c as u16,
            });
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Order-induced acyclic subgraph -- O(V + E)
// ---------------------------------------------------------------------------

pub fn vertex_order_dag(graph: &Digraph) -> DagTransform {
    let arcs = expanded_arcs(graph);
    let mut result = DagTransform::empty(DagTransformKind::VertexOrder);
    result.ordering = graph.vertices.iter().map(|v| v.id).collect();

    for arc in &arcs {
        if arc.from < arc.to {
            result.kept_arcs.push(arc.arc);
        } else {
            result.removed_arcs.push(arc.arc);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// DFS back-edge removal -- O(V + E)
// ---------------------------------------------------------------------------

fn dfs_back_edges(n: usize, arcs: &[ArcIx]) -> Vec<bool> {
    fn visit(v: usize, arcs: &[ArcIx], adj: &[Vec<usize>], color: &mut [u8], removed: &mut [bool]) {
        color[v] = 1;
        for &ai in &adj[v] {
            let w = arcs[ai].to;
            match color[w] {
                0 => visit(w, arcs, adj, color, removed),
                1 => removed[ai] = true,
                _ => {}
            }
        }
        color[v] = 2;
    }

    let adj = adjacency(n, arcs);
    let mut color = vec![0u8; n];
    let mut removed = vec![false; arcs.len()];
    for v in 0..n {
        if color[v] == 0 {
            visit(v, arcs, &adj, &mut color, &mut removed);
        }
    }
    removed
}

pub fn dfs_acyclic_subgraph(graph: &Digraph) -> DagTransform {
    let arcs = expanded_arcs(graph);
    let removed = dfs_back_edges(graph.vertices.len(), &arcs);
    let mut result = DagTransform::empty(DagTransformKind::DfsBackEdgeRemoval);

    for (i, arc) in arcs.iter().enumerate() {
        if removed[i] {
            result.removed_arcs.push(arc.arc);
        } else {
            result.kept_arcs.push(arc.arc);
        }
    }
    result.ordering =
        topological_order_filtered(graph, &arcs, Some(&removed), None).unwrap_or_default();
    result
}

// ---------------------------------------------------------------------------
// Cycle finder shared by bounded exact feedback-set solvers.
// ---------------------------------------------------------------------------

fn find_cycle(
    n: usize,
    arcs: &[ArcIx],
    removed_arc: &[bool],
    removed_vertex: &[bool],
) -> Option<Vec<usize>> {
    fn visit(
        v: usize,
        arcs: &[ArcIx],
        adj: &[Vec<usize>],
        removed_arc: &[bool],
        removed_vertex: &[bool],
        color: &mut [u8],
        parent_arc: &mut [usize],
    ) -> Option<Vec<usize>> {
        color[v] = 1;
        for &ai in &adj[v] {
            if removed_arc[ai] {
                continue;
            }
            let arc = arcs[ai];
            if removed_vertex[arc.to] || removed_vertex[arc.from] {
                continue;
            }
            let w = arc.to;
            if color[w] == 0 {
                parent_arc[w] = ai;
                if let Some(cycle) =
                    visit(w, arcs, adj, removed_arc, removed_vertex, color, parent_arc)
                {
                    return Some(cycle);
                }
            } else if color[w] == 1 {
                let mut cycle = Vec::new();
                cycle.push(ai);
                let mut cur = v;
                while cur != w {
                    let pa = parent_arc[cur];
                    if pa == usize::MAX {
                        break;
                    }
                    cycle.push(pa);
                    cur = arcs[pa].from;
                }
                cycle.sort_unstable();
                cycle.dedup();
                return Some(cycle);
            }
        }
        color[v] = 2;
        None
    }

    let adj = adjacency(n, arcs);
    let mut color = vec![0u8; n];
    let mut parent_arc = vec![usize::MAX; n];
    for v in 0..n {
        if removed_vertex[v] || color[v] != 0 {
            continue;
        }
        if let Some(cycle) = visit(
            v,
            arcs,
            &adj,
            removed_arc,
            removed_vertex,
            &mut color,
            &mut parent_arc,
        ) {
            return Some(cycle);
        }
    }
    None
}

#[derive(Default)]
struct SearchBudget {
    used: u32,
    limit: u32,
    exhausted: bool,
}

impl SearchBudget {
    fn new(limit: u32) -> Self {
        Self {
            used: 0,
            limit,
            exhausted: false,
        }
    }

    fn tick(&mut self) -> bool {
        if self.used >= self.limit {
            self.exhausted = true;
            return false;
        }
        self.used += 1;
        true
    }
}

fn fas_decide(
    n: usize,
    arcs: &[ArcIx],
    removed: &mut [bool],
    depth_left: usize,
    budget: &mut SearchBudget,
) -> bool {
    if !budget.tick() {
        return false;
    }
    let removed_vertex = vec![false; n];
    let Some(cycle) = find_cycle(n, arcs, removed, &removed_vertex) else {
        return true;
    };
    if depth_left == 0 {
        return false;
    }

    for ai in cycle {
        if removed[ai] {
            continue;
        }
        removed[ai] = true;
        if fas_decide(n, arcs, removed, depth_left - 1, budget) {
            return true;
        }
        removed[ai] = false;
        if budget.exhausted {
            return false;
        }
    }
    false
}

fn collect_arc_result(
    graph: &Digraph,
    kind: DagTransformKind,
    arcs: &[ArcIx],
    removed: &[bool],
) -> DagTransform {
    let mut result = DagTransform::empty(kind);
    for (i, arc) in arcs.iter().enumerate() {
        if removed[i] {
            result.removed_arcs.push(arc.arc);
        } else {
            result.kept_arcs.push(arc.arc);
        }
    }
    result.ordering =
        topological_order_filtered(graph, arcs, Some(removed), None).unwrap_or_default();
    result
}

/// Find a minimum feedback arc set when the graph is small enough and the
/// search finishes inside the DS work budget.  Otherwise return the linear-time
/// DFS back-edge set, which is always a valid (not necessarily minimum) FAS.
pub fn minimum_feedback_arc_set(graph: &Digraph) -> DagTransform {
    let arcs = expanded_arcs(graph);
    let n = graph.vertices.len();
    let heuristic = dfs_back_edges(n, &arcs);
    let heuristic_count = heuristic.iter().filter(|&&x| x).count();

    if heuristic_count == 0 {
        return collect_arc_result(
            graph,
            DagTransformKind::MinimumFeedbackArcSet,
            &arcs,
            &heuristic,
        );
    }

    if n > EXACT_FAS_MAX_VERTICES || arcs.len() > EXACT_FAS_MAX_ARCS {
        let mut result = collect_arc_result(
            graph,
            DagTransformKind::MinimumFeedbackArcSet,
            &arcs,
            &heuristic,
        );
        result.exact = false;
        result.truncated = true;
        return result;
    }

    let mut budget = SearchBudget::new(EXACT_SEARCH_BUDGET);
    for k in 0..=heuristic_count {
        let mut removed = vec![false; arcs.len()];
        if fas_decide(n, &arcs, &mut removed, k, &mut budget) {
            let mut result = collect_arc_result(
                graph,
                DagTransformKind::MinimumFeedbackArcSet,
                &arcs,
                &removed,
            );
            result.work = budget.used;
            result.exact = true;
            return result;
        }
        if budget.exhausted {
            break;
        }
    }

    let mut result = collect_arc_result(
        graph,
        DagTransformKind::MinimumFeedbackArcSet,
        &arcs,
        &heuristic,
    );
    result.exact = false;
    result.truncated = true;
    result.work = budget.used;
    result
}

fn cycle_vertices(arcs: &[ArcIx], cycle: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(cycle.len());
    for &ai in cycle {
        out.push(arcs[ai].from);
        out.push(arcs[ai].to);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn greedy_feedback_vertices(n: usize, arcs: &[ArcIx]) -> Vec<bool> {
    let no_removed_arc = vec![false; arcs.len()];
    let mut removed_vertex = vec![false; n];

    while let Some(cycle) = find_cycle(n, arcs, &no_removed_arc, &removed_vertex) {
        let candidates = cycle_vertices(arcs, &cycle);
        let mut best = candidates[0];
        let mut best_degree = 0usize;
        for &v in &candidates {
            let mut degree = 0usize;
            for arc in arcs {
                if removed_vertex[arc.from] || removed_vertex[arc.to] {
                    continue;
                }
                if arc.from == v || arc.to == v {
                    degree += 1;
                }
            }
            if degree > best_degree {
                best_degree = degree;
                best = v;
            }
        }
        removed_vertex[best] = true;
    }
    removed_vertex
}

fn fvs_decide(
    n: usize,
    arcs: &[ArcIx],
    removed_vertex: &mut [bool],
    depth_left: usize,
    budget: &mut SearchBudget,
) -> bool {
    if !budget.tick() {
        return false;
    }
    let no_removed_arc = vec![false; arcs.len()];
    let Some(cycle) = find_cycle(n, arcs, &no_removed_arc, removed_vertex) else {
        return true;
    };
    if depth_left == 0 {
        return false;
    }

    let candidates = cycle_vertices(arcs, &cycle);
    for v in candidates {
        if removed_vertex[v] {
            continue;
        }
        removed_vertex[v] = true;
        if fvs_decide(n, arcs, removed_vertex, depth_left - 1, budget) {
            return true;
        }
        removed_vertex[v] = false;
        if budget.exhausted {
            return false;
        }
    }
    false
}

fn collect_vertex_result(graph: &Digraph, arcs: &[ArcIx], removed_vertex: &[bool]) -> DagTransform {
    let mut result = DagTransform::empty(DagTransformKind::MinimumFeedbackVertexSet);
    for (vi, &removed) in removed_vertex.iter().enumerate() {
        if removed {
            result.removed_vertices.push(graph.vertices[vi].id);
        }
    }
    for arc in arcs {
        if !removed_vertex[arc.from] && !removed_vertex[arc.to] {
            result.kept_arcs.push(arc.arc);
        }
    }
    result.ordering =
        topological_order_filtered(graph, arcs, None, Some(removed_vertex)).unwrap_or_default();
    result
}

/// Minimum directed feedback vertex set, bounded for DS hardware.  If the exact
/// branch search cannot safely finish, returns a valid greedy FVS and marks the
/// result `exact = false, truncated = true`.
pub fn minimum_feedback_vertex_set(graph: &Digraph) -> DagTransform {
    let arcs = expanded_arcs(graph);
    let n = graph.vertices.len();
    let heuristic = greedy_feedback_vertices(n, &arcs);
    let heuristic_count = heuristic.iter().filter(|&&x| x).count();

    if heuristic_count == 0 {
        return collect_vertex_result(graph, &arcs, &heuristic);
    }

    if n > EXACT_FVS_MAX_VERTICES {
        let mut result = collect_vertex_result(graph, &arcs, &heuristic);
        result.exact = false;
        result.truncated = true;
        return result;
    }

    let mut budget = SearchBudget::new(EXACT_SEARCH_BUDGET);
    for k in 0..=heuristic_count {
        let mut removed_vertex = vec![false; n];
        if fvs_decide(n, &arcs, &mut removed_vertex, k, &mut budget) {
            let mut result = collect_vertex_result(graph, &arcs, &removed_vertex);
            result.exact = true;
            result.work = budget.used;
            return result;
        }
        if budget.exhausted {
            break;
        }
    }

    let mut result = collect_vertex_result(graph, &arcs, &heuristic);
    result.exact = false;
    result.truncated = true;
    result.work = budget.used;
    result
}

// ---------------------------------------------------------------------------
// Materialization helpers.  Analysis is non-destructive by default; this is
// the explicit path for replacing the current graph with the displayed DAG.
// ---------------------------------------------------------------------------

fn color_for_arc(graph: &Digraph, edge_id: EdgeId) -> u16 {
    graph.edge(edge_id).map(|e| e.color).unwrap_or(COLOR_MARK)
}

/// Convert an analysis result into a standalone directed graph.  This never
/// mutates `graph`; callers can assign the returned value after an explicit UI
/// confirmation.  Undirected source edges become the surviving directed arcs.
pub fn materialize(graph: &Digraph, transform: &DagTransform) -> Digraph {
    if transform.kind == DagTransformKind::SccCondensation {
        let vertices = transform
            .components
            .iter()
            .map(|c| Vertex {
                id: c.component,
                x: c.x,
                y: c.y,
                color: COLOR_MARK,
            })
            .collect::<Vec<_>>();
        let edges = transform
            .condensation_arcs
            .iter()
            .enumerate()
            .map(|(i, a)| Edge {
                id: i as EdgeId,
                from: a.from_component,
                to: a.to_component,
                directed: true,
                color: COLOR_MARK,
            })
            .collect::<Vec<_>>();
        return Digraph::from_parts(vertices, edges);
    }

    let mut vertices = Vec::new();
    for v in &graph.vertices {
        if !transform.removed_vertices.contains(&v.id) {
            vertices.push(*v);
        }
    }
    let mut edges = Vec::with_capacity(transform.kept_arcs.len());
    for (i, arc) in transform.kept_arcs.iter().enumerate() {
        edges.push(Edge {
            id: i as EdgeId,
            from: arc.from,
            to: arc.to,
            directed: true,
            color: color_for_arc(graph, arc.edge_id),
        });
    }
    Digraph::from_parts(vertices, edges)
}

/// Compact dispatcher used by the analysis menu.
pub fn run(graph: &Digraph, kind: DagTransformKind) -> DagTransform {
    match kind {
        DagTransformKind::SccCondensation => scc_condensation(graph),
        DagTransformKind::VertexOrder => vertex_order_dag(graph),
        DagTransformKind::DfsBackEdgeRemoval => dfs_acyclic_subgraph(graph),
        DagTransformKind::MinimumFeedbackArcSet => minimum_feedback_arc_set(graph),
        DagTransformKind::MinimumFeedbackVertexSet => minimum_feedback_vertex_set(graph),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(vertices: usize, edges: &[(usize, usize)]) -> Digraph {
        let mut g = Digraph::new();
        for i in 0..vertices {
            g.add_vertex((20 + 20 * i) as i16, 60).unwrap();
        }
        for &(a, b) in edges {
            g.add_edge(a as VertexId, b as VertexId, true).unwrap();
        }
        g
    }

    #[test]
    fn condensation_of_cycle_is_one_node() {
        let g = graph(3, &[(0, 1), (1, 2), (2, 0)]);
        let d = scc_condensation(&g);
        assert_eq!(d.components.len(), 1);
        assert!(d.condensation_arcs.is_empty());
    }

    #[test]
    fn condensation_is_dag() {
        let g = graph(5, &[(0, 1), (1, 0), (1, 2), (2, 3), (3, 2), (3, 4)]);
        let d = scc_condensation(&g);
        let m = materialize(&g, &d);
        assert!(is_dag(&m));
    }

    #[test]
    fn vertex_order_is_always_dag() {
        let g = graph(4, &[(0, 1), (1, 2), (2, 0), (3, 1), (2, 3)]);
        let d = vertex_order_dag(&g);
        let m = materialize(&g, &d);
        assert!(is_dag(&m));
    }

    #[test]
    fn dfs_removes_a_feedback_set() {
        let g = graph(4, &[(0, 1), (1, 2), (2, 0), (2, 3)]);
        let d = dfs_acyclic_subgraph(&g);
        assert!(!d.removed_arcs.is_empty());
        assert!(is_dag(&materialize(&g, &d)));
    }

    #[test]
    fn exact_fas_on_triangle_is_one() {
        let g = graph(3, &[(0, 1), (1, 2), (2, 0)]);
        let d = minimum_feedback_arc_set(&g);
        assert!(d.exact);
        assert_eq!(d.removed_arcs.len(), 1);
        assert!(is_dag(&materialize(&g, &d)));
    }

    #[test]
    fn exact_fvs_on_triangle_is_one() {
        let g = graph(3, &[(0, 1), (1, 2), (2, 0)]);
        let d = minimum_feedback_vertex_set(&g);
        assert!(d.exact);
        assert_eq!(d.removed_vertices.len(), 1);
        assert!(is_dag(&materialize(&g, &d)));
    }
}
