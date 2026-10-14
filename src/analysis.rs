//! Graph-only analysis catalogue exposed by the bottom-screen menu.
//!
//! All entries use only graph topology (plus the optional source/target vertex
//! chosen by the user). Expensive transformations reuse the bounded DAG code.

extern crate alloc;

use alloc::{vec, vec::Vec};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    dag::{self, DagTransform, DagTransformKind},
    graph::{Digraph, EdgeId, VertexId},
};

const MAX_CYCLES_SHOWN: usize = 64;
const MAX_CYCLE_STEPS: usize = 250_000;
const MAX_UNILATERAL_COMPONENTS: usize = 64;
const MAX_UNILATERAL_STEPS: usize = 200_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AnalysisKind {
    Reachability,
    ShortestPath,
    TopologicalOrder,
    FindCycle,
    EulerTrail,
    DagLongestPath,
    PnPositions,
    SpragueGrundy,
    WinLoseDraw,
    DagKernel,
    Bipartite,
    BipartiteMatching,
    SpanningForest,
    KCore,
    Dominators,
    UnitMinCut,
    EdgeDisjointRoutes,
    VertexMinCut,
    VertexDisjointRoutes,
    DagPathCover,
    Semiconnected,
    MotherVertex,
    StrongConnectCount,
    SccCondensation,
    VertexOrderDag,
    DfsDag,
    MinimumFeedbackArcSet,
    MinimumFeedbackVertexSet,
}

impl AnalysisKind {
    pub const ALL: [AnalysisKind; 27] = [
        Self::Reachability,
        Self::ShortestPath,
        Self::TopologicalOrder,
        Self::EulerTrail,
        Self::DagLongestPath,
        Self::PnPositions,
        Self::SpragueGrundy,
        Self::WinLoseDraw,
        Self::DagKernel,
        Self::Bipartite,
        Self::BipartiteMatching,
        Self::SpanningForest,
        Self::KCore,
        Self::Dominators,
        Self::UnitMinCut,
        Self::EdgeDisjointRoutes,
        Self::VertexMinCut,
        Self::VertexDisjointRoutes,
        Self::DagPathCover,
        Self::Semiconnected,
        Self::MotherVertex,
        Self::StrongConnectCount,
        Self::SccCondensation,
        Self::VertexOrderDag,
        Self::DfsDag,
        Self::MinimumFeedbackArcSet,
        Self::MinimumFeedbackVertexSet,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Reachability => "REACH",
            Self::ShortestPath => "BFS PATH",
            Self::TopologicalOrder => "TOPO",
            Self::FindCycle => "LOOPS",
            Self::EulerTrail => "EULER",
            Self::DagLongestPath => "DAG LONG",
            Self::PnPositions => "P/N",
            Self::SpragueGrundy => "GRUNDY",
            Self::WinLoseDraw => "W/L/D",
            Self::DagKernel => "KERNEL",
            Self::Bipartite => "BIPART",
            Self::BipartiteMatching => "MAX MATCH",
            Self::SpanningForest => "FOREST",
            Self::KCore => "K-CORE",
            Self::Dominators => "DOM",
            Self::UnitMinCut => "MIN CUT",
            Self::EdgeDisjointRoutes => "EDGE DISJ",
            Self::VertexMinCut => "VERT CUT",
            Self::VertexDisjointRoutes => "VERT DISJ",
            Self::DagPathCover => "PATH CVR",
            Self::Semiconnected => "SEMICON",
            Self::MotherVertex => "MOTHER",
            Self::StrongConnectCount => "SC+ EDGES",
            Self::SccCondensation => "COND DAG",
            Self::VertexOrderDag => "ORDER DAG",
            Self::DfsDag => "DFS DAG",
            Self::MinimumFeedbackArcSet => "MIN FAS",
            Self::MinimumFeedbackVertexSet => "MIN FVS",
        }
    }

    pub const fn needs_source(self) -> bool {
        matches!(
            self,
            Self::Reachability
                | Self::ShortestPath
                | Self::Dominators
                | Self::UnitMinCut
                | Self::EdgeDisjointRoutes
                | Self::VertexMinCut
                | Self::VertexDisjointRoutes
        )
    }

    pub const fn needs_target(self) -> bool {
        matches!(
            self,
            Self::ShortestPath
                | Self::UnitMinCut
                | Self::EdgeDisjointRoutes
                | Self::VertexMinCut
                | Self::VertexDisjointRoutes
        )
    }

    pub const fn dag_kind(self) -> Option<DagTransformKind> {
        match self {
            Self::SccCondensation => Some(DagTransformKind::SccCondensation),
            Self::VertexOrderDag => Some(DagTransformKind::VertexOrder),
            Self::DfsDag => Some(DagTransformKind::DfsBackEdgeRemoval),
            Self::MinimumFeedbackArcSet => Some(DagTransformKind::MinimumFeedbackArcSet),
            Self::MinimumFeedbackVertexSet => Some(DagTransformKind::MinimumFeedbackVertexSet),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComponentKind {
    Weak,
    Strong,
    Articulation,
    Bridges,
    TwoEdgeConnected,
    SourcesSinks,
    // Appended to preserve Postcard discriminants of the existing six variants.
    Unilateral,
    // Appended in v0.4.6; loops/cycles live with structural components.
    Loops,
}

impl ComponentKind {
    pub const ALL: [ComponentKind; 8] = [
        Self::Weak,
        Self::Strong,
        Self::Unilateral,
        Self::Loops,
        Self::Articulation,
        Self::Bridges,
        Self::TwoEdgeConnected,
        Self::SourcesSinks,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Weak => "WEAK CC",
            Self::Strong => "SCC",
            Self::Unilateral => "UNILAT CC",
            Self::Loops => "LOOPS",
            Self::Articulation => "ARTIC",
            Self::Bridges => "BRIDGES",
            Self::TwoEdgeConnected => "2-EDGE CC",
            Self::SourcesSinks => "SRC/SINK",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisNoticeLevel {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug)]
pub struct AnalysisNotice {
    pub level: AnalysisNoticeLevel,
    pub line1: &'static str,
    pub line2: &'static str,
}

impl AnalysisNotice {
    pub const fn warning(line1: &'static str, line2: &'static str) -> Self {
        Self {
            level: AnalysisNoticeLevel::Warning,
            line1,
            line2,
        }
    }

    pub const fn error(line1: &'static str, line2: &'static str) -> Self {
        Self {
            level: AnalysisNoticeLevel::Error,
            line1,
            line2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisGroup {
    /// Vertices that belong to this visual object/component.
    pub vertices: Vec<VertexId>,
    /// Edges that belong to this visual object/component.
    pub edges: Vec<EdgeId>,
}

#[derive(Clone, Debug)]
pub struct AnalysisResult {
    pub title: &'static str,
    pub exact: bool,
    pub truncated: bool,
    pub count: u16,
    pub primary_vertices: Vec<VertexId>,
    pub secondary_vertices: Vec<VertexId>,
    pub primary_edges: Vec<EdgeId>,
    pub secondary_edges: Vec<EdgeId>,
    /// Distinct visual groups.  Groups may overlap; renderers use this to
    /// split shared vertices/edges into multiple colors.
    pub groups: Vec<AnalysisGroup>,
    /// Primary numeric value for a vertex. Reachability uses this for FROM.
    pub values: Vec<(VertexId, i16)>,
    /// Secondary numeric value for a vertex. Reachability uses this for TO.
    pub secondary_values: Vec<(VertexId, i16)>,
    pub order: Vec<VertexId>,
    pub dag: Option<DagTransform>,
    pub notice: Option<AnalysisNotice>,
}

impl AnalysisResult {
    fn empty(title: &'static str) -> Self {
        Self {
            title,
            exact: true,
            truncated: false,
            count: 0,
            primary_vertices: Vec::new(),
            secondary_vertices: Vec::new(),
            primary_edges: Vec::new(),
            secondary_edges: Vec::new(),
            groups: Vec::new(),
            values: Vec::new(),
            secondary_values: Vec::new(),
            order: Vec::new(),
            dag: None,
            notice: None,
        }
    }

    fn warn(&mut self, line1: &'static str, line2: &'static str) {
        self.exact = false;
        self.notice = Some(AnalysisNotice::warning(line1, line2));
    }

    fn error(&mut self, line1: &'static str, line2: &'static str) {
        self.exact = false;
        self.notice = Some(AnalysisNotice::error(line1, line2));
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct AnalysisState {
    pub selected: Option<AnalysisKind>,
    pub component: Option<ComponentKind>,
    pub source: Option<VertexId>,
    pub target: Option<VertexId>,
    pub result: Option<AnalysisResult>,
    last_revision: u32,
    last_source: Option<VertexId>,
    last_target: Option<VertexId>,
}

impl AnalysisState {
    pub fn select_algorithm(&mut self, kind: AnalysisKind) {
        self.selected = Some(kind);
        self.component = None;
        self.result = None;
        self.last_revision = u32::MAX;
    }

    pub fn select_component(&mut self, kind: ComponentKind) {
        self.component = Some(kind);
        self.selected = None;
        self.result = None;
        self.last_revision = u32::MAX;
    }

    pub fn clear(&mut self) {
        self.selected = None;
        self.component = None;
        self.result = None;
    }

    /// Recompute immediately.  Menu confirmation uses this so precondition
    /// warnings exist in the same frame the user selects an operation.
    pub fn recompute(&mut self, graph: &Digraph) {
        self.result = if let Some(kind) = self.selected {
            Some(run_algorithm(graph, kind, self.source, self.target))
        } else {
            self.component.map(|kind| run_component(graph, kind))
        };
        self.last_revision = graph.topology_revision;
        self.last_source = self.source;
        self.last_target = self.target;
    }
}

fn dense_index(graph: &Digraph, id: VertexId) -> Option<usize> {
    graph.vertex_index(id)
}

fn arcs(graph: &Digraph) -> Vec<(usize, usize, EdgeId)> {
    let mut out = Vec::with_capacity(graph.edges.len().saturating_mul(2));
    for e in &graph.edges {
        let (Some(a), Some(b)) = (dense_index(graph, e.from), dense_index(graph, e.to)) else {
            continue;
        };
        out.push((a, b, e.id));
        if !e.directed {
            out.push((b, a, e.id));
        }
    }
    out
}

fn directed_adj(graph: &Digraph) -> Vec<Vec<(usize, EdgeId)>> {
    let mut adj = vec![Vec::new(); graph.vertices.len()];
    for (a, b, e) in arcs(graph) {
        adj[a].push((b, e));
    }
    adj
}

fn reverse_adj(graph: &Digraph) -> Vec<Vec<(usize, EdgeId)>> {
    let mut adj = vec![Vec::new(); graph.vertices.len()];
    for (a, b, e) in arcs(graph) {
        adj[b].push((a, e));
    }
    adj
}

fn undirected_adj(graph: &Digraph) -> Vec<Vec<(usize, EdgeId)>> {
    let mut adj = vec![Vec::new(); graph.vertices.len()];
    for e in &graph.edges {
        let (Some(a), Some(b)) = (dense_index(graph, e.from), dense_index(graph, e.to)) else {
            continue;
        };
        adj[a].push((b, e.id));
        adj[b].push((a, e.id));
    }
    adj
}

fn bfs_from(graph: &Digraph, source: VertexId) -> Option<(Vec<i16>, Vec<Option<(usize, EdgeId)>>)> {
    let s = dense_index(graph, source)?;
    let adj = directed_adj(graph);
    let n = graph.vertices.len();
    let mut dist = vec![-1i16; n];
    let mut pred = vec![None; n];
    let mut q = Vec::with_capacity(n);
    let mut head = 0usize;
    dist[s] = 0;
    q.push(s);
    while head < q.len() {
        let v = q[head];
        head += 1;
        for &(w, eid) in &adj[v] {
            if dist[w] < 0 {
                dist[w] = dist[v].saturating_add(1);
                pred[w] = Some((v, eid));
                q.push(w);
            }
        }
    }
    Some((dist, pred))
}

fn bfs_distances_on(adj: &[Vec<(usize, EdgeId)>], source: usize) -> Vec<i16> {
    let n = adj.len();
    let mut dist = vec![-1i16; n];
    let mut q = Vec::with_capacity(n);
    let mut head = 0usize;
    dist[source] = 0;
    q.push(source);
    while head < q.len() {
        let v = q[head];
        head += 1;
        for &(w, _) in &adj[v] {
            if dist[w] < 0 {
                dist[w] = dist[v].saturating_add(1);
                q.push(w);
            }
        }
    }
    dist
}

fn reachability(graph: &Digraph, source: Option<VertexId>) -> AnalysisResult {
    let mut r = AnalysisResult::empty("REACHABILITY");
    let Some(sid) = source else {
        r.warn("SELECT A SOURCE VERTEX", "TO RUN THIS ALGORITHM");
        return r;
    };
    let Some(s) = dense_index(graph, sid) else {
        r.error("SOURCE IS INVALID", "SELECT ANOTHER VERTEX");
        return r;
    };

    // FROM(v) = d(source, v).  TO(v) = d(v, source), which is the same
    // BFS problem on the reversed digraph.  Both use unweighted hop distance.
    let forward = bfs_distances_on(&directed_adj(graph), s);
    let backward = bfs_distances_on(&reverse_adj(graph), s);

    for (i, v) in graph.vertices.iter().enumerate() {
        r.values.push((v.id, forward[i]));
        r.secondary_values.push((v.id, backward[i]));
        if v.id == sid {
            continue;
        }
        if forward[i] >= 0 {
            r.primary_vertices.push(v.id);
        } else {
            r.secondary_vertices.push(v.id);
        }
    }
    r.count = 1u16.saturating_add(r.primary_vertices.len() as u16);
    r
}

fn shortest_path(
    graph: &Digraph,
    source: Option<VertexId>,
    target: Option<VertexId>,
) -> AnalysisResult {
    let mut r = AnalysisResult::empty("SHORTEST PATH");
    let Some(s) = source else {
        r.warn("SELECT A SOURCE VERTEX", "TO RUN THIS ALGORITHM");
        return r;
    };
    let Some(t) = target else {
        r.warn("SELECT A TARGET VERTEX", "TO RUN THIS ALGORITHM");
        return r;
    };
    let Some(ti) = dense_index(graph, t) else {
        r.error("TARGET IS INVALID", "SELECT ANOTHER VERTEX");
        return r;
    };
    let Some((dist, pred)) = bfs_from(graph, s) else {
        r.error("SOURCE IS INVALID", "SELECT ANOTHER VERTEX");
        return r;
    };
    if dist[ti] < 0 {
        r.warn("TARGET IS NOT REACHABLE", "NO DIRECTED PATH EXISTS");
        return r;
    }
    let mut cur = ti;
    r.primary_vertices.push(t);
    while graph.vertices[cur].id != s {
        let Some((p, eid)) = pred[cur] else {
            break;
        };
        r.primary_edges.push(eid);
        cur = p;
        r.primary_vertices.push(graph.vertices[cur].id);
    }
    r.primary_vertices.reverse();
    r.primary_edges.reverse();
    r.count = dist[ti] as u16;
    r
}

fn topo_order(graph: &Digraph) -> Option<Vec<VertexId>> {
    let n = graph.vertices.len();
    let adj = directed_adj(graph);
    let mut indeg = vec![0u16; n];
    for v in 0..n {
        for &(w, _) in &adj[v] {
            indeg[w] = indeg[w].saturating_add(1);
        }
    }
    let mut q = Vec::with_capacity(n);
    for (i, &d) in indeg.iter().enumerate() {
        if d == 0 {
            q.push(i);
        }
    }
    let mut head = 0;
    let mut out = Vec::with_capacity(n);
    while head < q.len() {
        let v = q[head];
        head += 1;
        out.push(graph.vertices[v].id);
        for &(w, _) in &adj[v] {
            indeg[w] -= 1;
            if indeg[w] == 0 {
                q.push(w);
            }
        }
    }
    (out.len() == n).then_some(out)
}

fn topological(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("TOPOLOGICAL ORDER");
    let n = graph.vertices.len();
    let adj = directed_adj(graph);
    let mut indeg = vec![0u16; n];
    for v in 0..n {
        for &(w, _) in &adj[v] {
            indeg[w] = indeg[w].saturating_add(1);
        }
    }

    // Kahn by complete zero-indegree frontiers.  Every frontier is one
    // topological generation, matching the visual language of the original
    // one-screen application.
    let mut frontier: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    let mut generation = 0i16;
    while !frontier.is_empty() {
        frontier.sort_unstable();
        let current = core::mem::take(&mut frontier);
        let mut members = Vec::with_capacity(current.len());
        for &v in &current {
            let id = graph.vertices[v].id;
            members.push(id);
            order.push(id);
            r.values.push((id, generation));
        }
        r.groups.push(AnalysisGroup {
            vertices: members,
            edges: Vec::new(),
        });

        for v in current {
            for &(w, _) in &adj[v] {
                indeg[w] = indeg[w].saturating_sub(1);
                if indeg[w] == 0 {
                    frontier.push(w);
                }
            }
        }
        generation = generation.saturating_add(1);
    }

    if order.len() != n {
        r.groups.clear();
        r.values.clear();
        r.warn("GRAPH IS NOT A DAG", "TOPOLOGICAL SORT CANNOT RUN");
        return r;
    }

    r.count = r.groups.len().min(u16::MAX as usize) as u16;
    r.order = order;
    r
}

fn find_cycle(graph: &Digraph) -> AnalysisResult {
    // Enumerate elementary directed cycles.  Restricting DFS to vertices whose
    // dense index is >= the chosen start makes the start the minimum-index
    // vertex of every reported cycle, eliminating rotational duplicates.
    // The visualization is bounded so a pathological graph cannot freeze a DS.
    let mut r = AnalysisResult::empty("DIRECTED LOOPS");
    let adj = directed_adj(graph);
    let n = graph.vertices.len();
    if n == 0 {
        return r;
    }

    let mut visited = vec![false; n];
    let mut path_vertices: Vec<usize> = Vec::with_capacity(n);
    let mut path_edges: Vec<EdgeId> = Vec::with_capacity(n);
    let mut steps = 0usize;
    let mut truncated = false;

    fn dfs_cycles(
        start: usize,
        v: usize,
        graph: &Digraph,
        adj: &[Vec<(usize, EdgeId)>],
        visited: &mut [bool],
        path_vertices: &mut Vec<usize>,
        path_edges: &mut Vec<EdgeId>,
        groups: &mut Vec<AnalysisGroup>,
        steps: &mut usize,
        truncated: &mut bool,
    ) {
        if *truncated {
            return;
        }
        visited[v] = true;
        path_vertices.push(v);

        for &(w, eid) in &adj[v] {
            *steps = (*steps).saturating_add(1);
            if *steps >= MAX_CYCLE_STEPS || groups.len() >= MAX_CYCLES_SHOWN {
                *truncated = true;
                break;
            }
            if w < start {
                continue;
            }
            if w == start {
                // An undirected stored edge expands to u->v and v->u with the
                // same EdgeId.  Do not report that immediate out-and-back as a
                // graph-theoretic loop; a genuine directed 2-cycle uses two IDs.
                let same_undirected_edge =
                    path_vertices.len() == 2 && path_edges.len() == 1 && path_edges[0] == eid;
                if !same_undirected_edge {
                    let mut vertices = Vec::with_capacity(path_vertices.len());
                    for &idx in path_vertices.iter() {
                        vertices.push(graph.vertices[idx].id);
                    }
                    let mut edges = path_edges.clone();
                    edges.push(eid);
                    groups.push(AnalysisGroup { vertices, edges });
                }
            } else if !visited[w] {
                path_edges.push(eid);
                dfs_cycles(
                    start,
                    w,
                    graph,
                    adj,
                    visited,
                    path_vertices,
                    path_edges,
                    groups,
                    steps,
                    truncated,
                );
                path_edges.pop();
                if *truncated {
                    break;
                }
            }
        }

        path_vertices.pop();
        visited[v] = false;
    }

    for start in 0..n {
        if truncated {
            break;
        }
        dfs_cycles(
            start,
            start,
            graph,
            &adj,
            &mut visited,
            &mut path_vertices,
            &mut path_edges,
            &mut r.groups,
            &mut steps,
            &mut truncated,
        );
    }

    r.count = r.groups.len().min(u16::MAX as usize) as u16;
    // Flatten once for compatibility with older result/highlight code.  The
    // group list remains authoritative for multicolor rendering.
    for g in &r.groups {
        for &id in &g.vertices {
            if !r.primary_vertices.contains(&id) {
                r.primary_vertices.push(id);
            }
        }
        for &id in &g.edges {
            if !r.primary_edges.contains(&id) {
                r.primary_edges.push(id);
            }
        }
    }
    if truncated {
        r.truncated = true;
        r.exact = false;
        r.notice = Some(AnalysisNotice::warning(
            "LOOP ENUMERATION HIT LIMIT",
            "DISPLAY IS TRUNCATED",
        ));
    }
    r
}

fn euler_trail(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("EULER TRAIL");
    let n = graph.vertices.len();
    if n == 0 {
        return r;
    }
    let aa = arcs(graph);
    if aa.is_empty() {
        return r;
    }
    let mut outd = vec![0i16; n];
    let mut ind = vec![0i16; n];
    let mut adj = vec![Vec::<(usize, EdgeId)>::new(); n];
    for &(a, b, e) in &aa {
        outd[a] += 1;
        ind[b] += 1;
        adj[a].push((b, e));
    }
    let mut start = None;
    let mut plus = 0;
    let mut minus = 0;
    for i in 0..n {
        let d = outd[i] - ind[i];
        if d == 1 {
            plus += 1;
            start = Some(i);
        } else if d == -1 {
            minus += 1;
        } else if d != 0 {
            r.warn("NO EULER TRAIL EXISTS", "DEGREE CONDITIONS FAIL");
            return r;
        }
    }
    if !((plus == 1 && minus == 1) || (plus == 0 && minus == 0)) {
        r.warn("NO EULER TRAIL EXISTS", "DEGREE CONDITIONS FAIL");
        return r;
    }
    let s = start.or_else(|| (0..n).find(|&i| outd[i] > 0)).unwrap_or(0);
    // Each stored undirected edge expands to two arcs; Euler here is arc-Eulerian.
    let mut used = vec![false; aa.len()];
    let mut by = vec![Vec::<usize>::new(); n];
    for (i, (a, _, _)) in aa.iter().enumerate() {
        by[*a].push(i);
    }
    let mut cursor = vec![0usize; n];
    let mut stack = vec![s];
    let mut estack: Vec<EdgeId> = Vec::new();
    let mut verts: Vec<usize> = Vec::new();
    let mut edges_rev: Vec<EdgeId> = Vec::new();
    while let Some(&v) = stack.last() {
        while cursor[v] < by[v].len() && used[by[v][cursor[v]]] {
            cursor[v] += 1;
        }
        if cursor[v] == by[v].len() {
            verts.push(v);
            stack.pop();
            if let Some(e) = estack.pop() {
                edges_rev.push(e);
            }
        } else {
            let ai = by[v][cursor[v]];
            cursor[v] += 1;
            used[ai] = true;
            let (_, w, eid) = aa[ai];
            stack.push(w);
            estack.push(eid);
        }
    }
    if used.iter().filter(|&&x| x).count() != aa.len() {
        r.warn("NO EULER TRAIL EXISTS", "ARCS ARE NOT ONE TRAIL");
        return r;
    }
    verts.reverse();
    edges_rev.reverse();
    r.primary_vertices = verts.into_iter().map(|i| graph.vertices[i].id).collect();
    r.primary_edges = edges_rev;
    r.count = aa.len() as u16;
    r
}

fn dag_longest(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("DAG LONGEST PATH");
    let Some(order) = topo_order(graph) else {
        r.warn("GRAPH IS NOT A DAG", "THIS ALGORITHM NEEDS A DAG");
        return r;
    };
    let n = graph.vertices.len();
    let adj = directed_adj(graph);
    let mut dist = vec![0i16; n];
    let mut pred: Vec<Option<(usize, EdgeId)>> = vec![None; n];
    for id in order {
        let v = graph.vertex_index(id).unwrap();
        for &(w, eid) in &adj[v] {
            if dist[v].saturating_add(1) > dist[w] {
                dist[w] = dist[v] + 1;
                pred[w] = Some((v, eid));
            }
        }
    }
    let Some((mut v, &best)) = dist.iter().enumerate().max_by_key(|(_, d)| *d) else {
        return r;
    };
    r.count = best.max(0) as u16;
    r.primary_vertices.push(graph.vertices[v].id);
    while let Some((p, e)) = pred[v] {
        r.primary_edges.push(e);
        v = p;
        r.primary_vertices.push(graph.vertices[v].id);
    }
    r.primary_vertices.reverse();
    r.primary_edges.reverse();
    r
}

fn pn_positions(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("P/N POSITIONS");
    let Some(order) = topo_order(graph) else {
        r.warn("GRAPH IS NOT A DAG", "THIS ALGORITHM NEEDS A DAG");
        return r;
    };
    let adj = directed_adj(graph);
    let mut is_n = vec![false; graph.vertices.len()];
    for id in order.into_iter().rev() {
        let v = graph.vertex_index(id).unwrap();
        is_n[v] = adj[v].iter().any(|&(w, _)| !is_n[w]);
    }
    for (i, v) in graph.vertices.iter().enumerate() {
        if is_n[i] {
            r.secondary_vertices.push(v.id);
            r.values.push((v.id, 1));
        } else {
            r.primary_vertices.push(v.id);
            r.values.push((v.id, 0));
        }
    }
    r.count = r.primary_vertices.len() as u16;
    r
}

fn grundy(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("SPRAGUE-GRUNDY");
    let Some(order) = topo_order(graph) else {
        r.warn("GRAPH IS NOT A DAG", "THIS ALGORITHM NEEDS A DAG");
        return r;
    };
    let adj = directed_adj(graph);
    let n = graph.vertices.len();
    let mut g = vec![0u16; n];
    let mut mark = vec![0u16; n + 1];
    let mut generation = 1u16;
    for id in order.into_iter().rev() {
        let v = graph.vertex_index(id).unwrap();
        generation = generation.wrapping_add(1).max(1);
        for &(w, _) in &adj[v] {
            let x = g[w] as usize;
            if x < mark.len() {
                mark[x] = generation;
            }
        }
        let mut mex = 0usize;
        while mex < mark.len() && mark[mex] == generation {
            mex += 1;
        }
        g[v] = mex as u16;
    }
    for (i, v) in graph.vertices.iter().enumerate() {
        r.values.push((v.id, g[i].min(i16::MAX as u16) as i16));
        if g[i] == 0 {
            r.primary_vertices.push(v.id)
        } else {
            r.secondary_vertices.push(v.id)
        }
    }
    r.count = *g.iter().max().unwrap_or(&0);
    r
}

fn wld(graph: &Digraph) -> AnalysisResult {
    // Retrograde Win/Lose/Draw classification for finite directed play graphs.
    let mut r = AnalysisResult::empty("WIN / LOSE / DRAW");
    let adj = directed_adj(graph);
    let rev = reverse_adj(graph);
    let n = graph.vertices.len();
    let mut state = vec![0u8; n]; // 0 unknown/draw, 1 win, 2 lose
    let mut remaining: Vec<usize> = adj.iter().map(Vec::len).collect();
    let mut q = Vec::new();
    for i in 0..n {
        if remaining[i] == 0 {
            state[i] = 2;
            q.push(i);
        }
    }
    let mut head = 0;
    while head < q.len() {
        let v = q[head];
        head += 1;
        for &(p, _) in &rev[v] {
            if state[p] != 0 {
                continue;
            }
            if state[v] == 2 {
                state[p] = 1;
                q.push(p);
            } else {
                remaining[p] = remaining[p].saturating_sub(1);
                if remaining[p] == 0 {
                    state[p] = 2;
                    q.push(p);
                }
            }
        }
    }
    for (i, v) in graph.vertices.iter().enumerate() {
        match state[i] {
            1 => {
                r.primary_vertices.push(v.id);
                r.values.push((v.id, 1));
            }
            2 => {
                r.secondary_vertices.push(v.id);
                r.values.push((v.id, -1));
            }
            _ => r.values.push((v.id, 0)),
        }
    }
    r.count = state.iter().filter(|&&s| s == 0).count() as u16;
    r
}

fn bipartite(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("BIPARTITE");
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut color = vec![-1i8; n];
    let mut q = Vec::new();
    for s in 0..n {
        if color[s] >= 0 {
            continue;
        }
        color[s] = 0;
        q.clear();
        q.push(s);
        let mut h = 0;
        while h < q.len() {
            let v = q[h];
            h += 1;
            for &(w, _) in &adj[v] {
                if color[w] < 0 {
                    color[w] = 1 - color[v];
                    q.push(w);
                } else if color[w] == color[v] {
                    r.exact = false;
                    return r;
                }
            }
        }
    }
    for (i, v) in graph.vertices.iter().enumerate() {
        if color[i] == 0 {
            r.primary_vertices.push(v.id)
        } else {
            r.secondary_vertices.push(v.id)
        }
    }
    r.count = r.primary_vertices.len() as u16;
    r
}

fn bipartite_matching(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("MAX BIPARTITE MATCHING");
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut color = vec![-1i8; n];
    let mut q = Vec::new();
    for s in 0..n {
        if color[s] >= 0 {
            continue;
        }
        color[s] = 0;
        q.clear();
        q.push(s);
        let mut h = 0;
        while h < q.len() {
            let v = q[h];
            h += 1;
            for &(w, _) in &adj[v] {
                if color[w] < 0 {
                    color[w] = 1 - color[v];
                    q.push(w);
                } else if color[w] == color[v] {
                    r.warn("GRAPH IS NOT BIPARTITE", "THIS ALGORITHM CANNOT RUN");
                    return r;
                }
            }
        }
    }
    let left: Vec<usize> = (0..n).filter(|&i| color[i] == 0).collect();
    let mut pair_l = vec![usize::MAX; n];
    let mut pair_r = vec![usize::MAX; n];
    let mut dist = vec![0i16; n];
    loop {
        let mut qq = Vec::new();
        for &u in &left {
            if pair_l[u] == usize::MAX {
                dist[u] = 0;
                qq.push(u)
            } else {
                dist[u] = -1
            }
        }
        let mut h = 0;
        while h < qq.len() {
            let u = qq[h];
            h += 1;
            for &(v, _) in &adj[u] {
                let pu = pair_r[v];
                if pu != usize::MAX && dist[pu] < 0 {
                    dist[pu] = dist[u] + 1;
                    qq.push(pu)
                }
            }
        }
        fn aug(
            u: usize,
            adj: &[Vec<(usize, EdgeId)>],
            pair_l: &mut [usize],
            pair_r: &mut [usize],
            dist: &mut [i16],
        ) -> bool {
            for &(v, _) in &adj[u] {
                let pu = pair_r[v];
                if pu == usize::MAX
                    || (dist[pu] == dist[u] + 1 && aug(pu, adj, pair_l, pair_r, dist))
                {
                    pair_l[u] = v;
                    pair_r[v] = u;
                    return true;
                }
            }
            dist[u] = -1;
            false
        }
        let mut added = 0;
        for &u in &left {
            if pair_l[u] == usize::MAX && aug(u, &adj, &mut pair_l, &mut pair_r, &mut dist) {
                added += 1
            }
        }
        if added == 0 {
            break;
        }
    }
    for &u in &left {
        let v = pair_l[u];
        if v != usize::MAX {
            r.primary_vertices.push(graph.vertices[u].id);
            r.secondary_vertices.push(graph.vertices[v].id);
            if let Some((_, eid)) = adj[u].iter().find(|(w, _)| *w == v) {
                r.primary_edges.push(*eid);
            }
        }
    }
    r.count = r.primary_edges.len() as u16;
    r
}

fn dag_path_cover(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("DAG MIN PATH COVER");
    if topo_order(graph).is_none() {
        r.warn("GRAPH IS NOT A DAG", "PATH COVER NEEDS A DAG");
        return r;
    }
    let n = graph.vertices.len();
    let adj = directed_adj(graph);
    let mut match_r = vec![usize::MAX; n];
    fn dfs(
        u: usize,
        adj: &[Vec<(usize, EdgeId)>],
        seen: &mut [bool],
        match_r: &mut [usize],
    ) -> bool {
        for &(v, _) in &adj[u] {
            if seen[v] {
                continue;
            }
            seen[v] = true;
            if match_r[v] == usize::MAX || dfs(match_r[v], adj, seen, match_r) {
                match_r[v] = u;
                return true;
            }
        }
        false
    }
    let mut m = 0usize;
    for u in 0..n {
        let mut seen = vec![false; n];
        if dfs(u, &adj, &mut seen, &mut match_r) {
            m += 1
        }
    }

    let mut succ = vec![usize::MAX; n];
    let mut succ_edge = vec![EdgeId::MAX; n];
    let mut pred = vec![usize::MAX; n];
    for (v, &u) in match_r.iter().enumerate() {
        if u != usize::MAX {
            pred[v] = u;
            succ[u] = v;
            if let Some((_, eid)) = adj[u].iter().find(|(w, _)| *w == v) {
                succ_edge[u] = *eid;
                r.primary_edges.push(*eid);
            }
        }
    }
    for start in 0..n {
        if pred[start] != usize::MAX {
            continue;
        }
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let mut v = start;
        loop {
            vertices.push(graph.vertices[v].id);
            if succ[v] == usize::MAX {
                break;
            }
            if succ_edge[v] != EdgeId::MAX {
                edges.push(succ_edge[v]);
            }
            v = succ[v];
        }
        r.groups.push(AnalysisGroup { vertices, edges });
    }
    r.count = n.saturating_sub(m) as u16;
    r
}

fn vertex_flow(
    graph: &Digraph,
    source: Option<VertexId>,
    target: Option<VertexId>,
) -> AnalysisResult {
    let mut r = AnalysisResult::empty("VERTEX MIN-CUT");
    let (Some(sid), Some(tid)) = (source, target) else {
        r.warn("SELECT SOURCE AND TARGET", "TO RUN THIS ALGORITHM");
        return r;
    };
    let (Some(s), Some(t)) = (graph.vertex_index(sid), graph.vertex_index(tid)) else {
        r.error("INVALID SOURCE OR TARGET", "SELECT TWO VERTICES");
        return r;
    };
    if s == t {
        r.warn("SOURCE EQUALS TARGET", "SELECT TWO VERTICES");
        return r;
    }
    let n = graph.vertices.len();
    let nn = n * 2;
    let inf = (n as i16).saturating_add(1);
    let mut cap = vec![vec![0i16; nn]; nn];
    for v in 0..n {
        cap[v * 2][v * 2 + 1] = if v == s || v == t { inf } else { 1 };
    }
    for (a, b, _) in arcs(graph) {
        cap[a * 2 + 1][b * 2] = inf;
    }
    let ss = s * 2 + 1;
    let tt = t * 2;
    let mut flow = 0u16;
    loop {
        let mut pred = vec![usize::MAX; nn];
        let mut q = vec![ss];
        pred[ss] = ss;
        let mut h = 0;
        while h < q.len() && pred[tt] == usize::MAX {
            let v = q[h];
            h += 1;
            for w in 0..nn {
                if pred[w] == usize::MAX && cap[v][w] > 0 {
                    pred[w] = v;
                    q.push(w);
                    if w == tt {
                        break;
                    }
                }
            }
        }
        if pred[tt] == usize::MAX {
            break;
        }
        let mut v = tt;
        while v != ss {
            let p = pred[v];
            cap[p][v] -= 1;
            cap[v][p] += 1;
            v = p;
        }
        flow = flow.saturating_add(1);
        if flow > n as u16 {
            break;
        }
    }
    let mut seen = vec![false; nn];
    let mut q = vec![ss];
    seen[ss] = true;
    let mut h = 0;
    while h < q.len() {
        let v = q[h];
        h += 1;
        for w in 0..nn {
            if !seen[w] && cap[v][w] > 0 {
                seen[w] = true;
                q.push(w)
            }
        }
    }
    for v in 0..n {
        if v != s && v != t && seen[v * 2] && !seen[v * 2 + 1] {
            r.primary_vertices.push(graph.vertices[v].id);
        }
    }
    r.count = flow;
    r
}

fn spanning_forest(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("SPANNING FOREST");
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut seen = vec![false; n];
    let mut q = Vec::new();
    for s in 0..n {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        q.clear();
        q.push(s);
        let mut h = 0;
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        while h < q.len() {
            let v = q[h];
            h += 1;
            let id = graph.vertices[v].id;
            vertices.push(id);
            r.primary_vertices.push(id);
            for &(w, e) in &adj[v] {
                if !seen[w] {
                    seen[w] = true;
                    q.push(w);
                    edges.push(e);
                    r.primary_edges.push(e);
                }
            }
        }
        r.groups.push(AnalysisGroup { vertices, edges });
    }
    r.count = r.groups.len() as u16;
    r
}

fn kcore(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("CORE NUMBERS");
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut deg: Vec<usize> = adj.iter().map(Vec::len).collect();
    let mut removed = vec![false; n];
    let mut core = vec![0u16; n];
    for _ in 0..n {
        let mut best = None;
        for i in 0..n {
            if !removed[i] && best.is_none_or(|b: usize| deg[i] < deg[b]) {
                best = Some(i);
            }
        }
        let Some(v) = best else { break };
        core[v] = deg[v] as u16;
        removed[v] = true;
        for &(w, _) in &adj[v] {
            if !removed[w] && deg[w] > deg[v] {
                deg[w] -= 1;
            }
        }
    }
    let max = *core.iter().max().unwrap_or(&0);
    r.count = max;
    for (i, v) in graph.vertices.iter().enumerate() {
        r.values.push((v.id, core[i] as i16));
    }
    for k in 0..=max {
        let vertices: Vec<VertexId> = graph
            .vertices
            .iter()
            .enumerate()
            .filter_map(|(i, v)| (core[i] == k).then_some(v.id))
            .collect();
        if !vertices.is_empty() {
            r.groups.push(AnalysisGroup {
                vertices,
                edges: Vec::new(),
            });
        }
    }
    add_group_edges(graph, &mut r.groups);
    r
}

fn dominators(graph: &Digraph, source: Option<VertexId>) -> AnalysisResult {
    // Iterative data-flow dominators; O(VE) worst-case but bounded at MAX_VERTICES.
    let mut r = AnalysisResult::empty("DOMINATORS");
    let Some(sid) = source else {
        r.warn("SELECT A SOURCE VERTEX", "TO RUN THIS ALGORITHM");
        return r;
    };
    let Some(s) = graph.vertex_index(sid) else {
        r.error("SOURCE IS INVALID", "SELECT ANOTHER VERTEX");
        return r;
    };
    let adj = directed_adj(graph);
    let rev = reverse_adj(graph);
    let n = graph.vertices.len();
    let mut reachable = vec![false; n];
    let mut q = vec![s];
    reachable[s] = true;
    let mut h = 0;
    while h < q.len() {
        let v = q[h];
        h += 1;
        for &(w, _) in &adj[v] {
            if !reachable[w] {
                reachable[w] = true;
                q.push(w);
            }
        }
    }
    let mut dom = vec![vec![true; n]; n];
    for i in 0..n {
        if !reachable[i] {
            dom[i].fill(false);
        }
    }
    dom[s].fill(false);
    dom[s][s] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for v in 0..n {
            if v == s || !reachable[v] {
                continue;
            }
            let mut next = vec![true; n];
            let mut any = false;
            for &(p, _) in &rev[v] {
                if !reachable[p] {
                    continue;
                }
                if !any {
                    next.clone_from(&dom[p]);
                    any = true
                } else {
                    for j in 0..n {
                        next[j] &= dom[p][j];
                    }
                }
            }
            if !any {
                next.fill(false);
            }
            next[v] = true;
            if next != dom[v] {
                dom[v] = next;
                changed = true;
            }
        }
    }
    // Immediate-dominator tree edges as generic edge-less relation; values hold idom IDs.
    r.primary_vertices.push(sid);
    r.values.push((sid, sid as i16));
    for v in 0..n {
        if v == s || !reachable[v] {
            continue;
        }
        let candidates: Vec<usize> = (0..n).filter(|&d| d != v && dom[v][d]).collect();
        let mut idom = None;
        for &d in &candidates {
            let dominates_other = candidates.iter().any(|&o| o != d && dom[o][d]);
            if !dominates_other {
                idom = Some(d);
                break;
            }
        }
        if let Some(d) = idom {
            r.primary_vertices.push(graph.vertices[v].id);
            r.values
                .push((graph.vertices[v].id, graph.vertices[d].id as i16));
        }
    }
    r.count = r.primary_vertices.len() as u16;
    r
}

fn unit_flow(
    graph: &Digraph,
    source: Option<VertexId>,
    target: Option<VertexId>,
) -> AnalysisResult {
    // Edmonds-Karp on unit arcs; V<=128, E<=1024 expanded arcs. Chosen for compact DS code.
    let mut r = AnalysisResult::empty("UNIT MIN-CUT");
    let (Some(sid), Some(tid)) = (source, target) else {
        r.warn("SELECT SOURCE AND TARGET", "TO RUN THIS ALGORITHM");
        return r;
    };
    let (Some(s), Some(t)) = (graph.vertex_index(sid), graph.vertex_index(tid)) else {
        r.error("INVALID SOURCE OR TARGET", "SELECT TWO VERTICES");
        return r;
    };
    if s == t {
        r.warn("SOURCE EQUALS TARGET", "SELECT TWO VERTICES");
        return r;
    }
    let n = graph.vertices.len();
    let aa = arcs(graph);
    let mut cap = vec![vec![0i16; n]; n];
    for &(a, b, _) in &aa {
        cap[a][b] = cap[a][b].saturating_add(1);
    }
    let mut flow = 0u16;
    loop {
        let mut pred = vec![usize::MAX; n];
        let mut q = vec![s];
        pred[s] = s;
        let mut h = 0;
        while h < q.len() && pred[t] == usize::MAX {
            let v = q[h];
            h += 1;
            for w in 0..n {
                if pred[w] == usize::MAX && cap[v][w] > 0 {
                    pred[w] = v;
                    q.push(w);
                    if w == t {
                        break;
                    }
                }
            }
        }
        if pred[t] == usize::MAX {
            break;
        }
        let mut v = t;
        while v != s {
            let p = pred[v];
            cap[p][v] -= 1;
            cap[v][p] += 1;
            v = p;
        }
        flow = flow.saturating_add(1);
        if flow == u16::MAX {
            break;
        }
    }
    let mut seen = vec![false; n];
    let mut q = vec![s];
    seen[s] = true;
    let mut h = 0;
    while h < q.len() {
        let v = q[h];
        h += 1;
        for w in 0..n {
            if !seen[w] && cap[v][w] > 0 {
                seen[w] = true;
                q.push(w);
            }
        }
    }
    for (i, v) in graph.vertices.iter().enumerate() {
        if seen[i] {
            r.primary_vertices.push(v.id)
        } else {
            r.secondary_vertices.push(v.id)
        }
    }
    for e in &graph.edges {
        let a = graph.vertex_index(e.from).unwrap();
        let b = graph.vertex_index(e.to).unwrap();
        if seen[a] != seen[b] {
            r.primary_edges.push(e.id);
        }
    }
    r.count = flow;
    r
}

fn semiconnected(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("SEMICONNECTED");
    let cond = dag::scc_condensation(graph);
    let c = cond.components.len();
    if c <= 1 {
        r.count = 1;
        return r;
    }
    let mut indeg = vec![0usize; c];
    let mut adj = vec![Vec::new(); c];
    for a in &cond.condensation_arcs {
        adj[a.from_component as usize].push(a.to_component as usize);
        indeg[a.to_component as usize] += 1;
    }
    let mut q = Vec::new();
    for i in 0..c {
        if indeg[i] == 0 {
            q.push(i)
        }
    }
    let mut h = 0;
    let mut order = Vec::new();
    while h < q.len() {
        let v = q[h];
        h += 1;
        order.push(v);
        for &w in &adj[v] {
            indeg[w] -= 1;
            if indeg[w] == 0 {
                q.push(w)
            }
        }
    }
    for pair in order.windows(2) {
        if !adj[pair[0]].contains(&pair[1]) {
            r.exact = false;
            return r;
        }
    }
    r.count = 1;
    r.primary_vertices = graph.vertices.iter().map(|v| v.id).collect();
    r
}

fn mother_vertex(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("MOTHER VERTEX");
    let n = graph.vertices.len();
    if n == 0 {
        return r;
    }
    let adj = directed_adj(graph);
    let mut seen = vec![false; n];
    let mut candidate = 0;
    fn dfs(v: usize, adj: &[Vec<(usize, EdgeId)>], seen: &mut [bool]) {
        seen[v] = true;
        for &(w, _) in &adj[v] {
            if !seen[w] {
                dfs(w, adj, seen)
            }
        }
    }
    for i in 0..n {
        if !seen[i] {
            dfs(i, &adj, &mut seen);
            candidate = i;
        }
    }
    seen.fill(false);
    dfs(candidate, &adj, &mut seen);
    if seen.iter().all(|&x| x) {
        r.primary_vertices.push(graph.vertices[candidate].id);
        r.count = 1;
    }
    r
}

fn strong_connect_count(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("EDGES TO STRONGLY CONNECT");
    let cond = dag::scc_condensation(graph);
    let c = cond.components.len();
    if c <= 1 {
        r.count = 0;
        return r;
    }
    let mut indeg = vec![0u16; c];
    let mut out = vec![0u16; c];
    for a in &cond.condensation_arcs {
        out[a.from_component as usize] += 1;
        indeg[a.to_component as usize] += 1;
    }
    let sources = indeg.iter().filter(|&&d| d == 0).count();
    let sinks = out.iter().filter(|&&d| d == 0).count();
    r.count = sources.max(sinks) as u16;
    for (i, comp) in cond.components.iter().enumerate() {
        if indeg[i] == 0 {
            r.primary_vertices.extend(comp.members.iter().copied());
        }
        if out[i] == 0 {
            r.secondary_vertices.extend(comp.members.iter().copied());
        }
    }
    r
}

fn dag_analysis(graph: &Digraph, kind: AnalysisKind) -> AnalysisResult {
    let dk = kind.dag_kind().unwrap();
    let t = dag::run(graph, dk);
    let mut r = AnalysisResult::empty(kind.code());
    r.exact = t.exact;
    r.truncated = t.truncated;
    r.count = t.resulting_arc_count() as u16;
    if t.truncated {
        r.notice = Some(AnalysisNotice::warning(
            "EXACT SEARCH HIT LIMIT",
            "SHOWING BOUNDED RESULT",
        ));
    }
    r.dag = Some(t);
    r
}

pub fn run_algorithm(
    graph: &Digraph,
    kind: AnalysisKind,
    source: Option<VertexId>,
    target: Option<VertexId>,
) -> AnalysisResult {
    if kind.dag_kind().is_some() {
        return dag_analysis(graph, kind);
    }
    match kind {
        AnalysisKind::Reachability => reachability(graph, source),
        AnalysisKind::ShortestPath => shortest_path(graph, source, target),
        AnalysisKind::TopologicalOrder => topological(graph),
        AnalysisKind::FindCycle => find_cycle(graph),
        AnalysisKind::EulerTrail => euler_trail(graph),
        AnalysisKind::DagLongestPath => dag_longest(graph),
        AnalysisKind::PnPositions | AnalysisKind::DagKernel => pn_positions(graph),
        AnalysisKind::SpragueGrundy => grundy(graph),
        AnalysisKind::WinLoseDraw => wld(graph),
        AnalysisKind::Bipartite => bipartite(graph),
        AnalysisKind::BipartiteMatching => bipartite_matching(graph),
        AnalysisKind::SpanningForest => spanning_forest(graph),
        AnalysisKind::KCore => kcore(graph),
        AnalysisKind::Dominators => dominators(graph, source),
        AnalysisKind::UnitMinCut | AnalysisKind::EdgeDisjointRoutes => {
            unit_flow(graph, source, target)
        }
        AnalysisKind::VertexMinCut | AnalysisKind::VertexDisjointRoutes => {
            vertex_flow(graph, source, target)
        }
        AnalysisKind::DagPathCover => dag_path_cover(graph),
        AnalysisKind::Semiconnected => semiconnected(graph),
        AnalysisKind::MotherVertex => mother_vertex(graph),
        AnalysisKind::StrongConnectCount => strong_connect_count(graph),
        _ => unreachable!(),
    }
}

// ---------- component analyses ----------

fn add_group_edges(graph: &Digraph, groups: &mut [AnalysisGroup]) {
    for group in groups {
        for e in &graph.edges {
            if group.vertices.contains(&e.from) && group.vertices.contains(&e.to) {
                group.edges.push(e.id);
            }
        }
    }
}

fn weak_components(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("WEAK COMPONENTS");
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut seen = vec![false; n];
    for s in 0..n {
        if seen[s] {
            continue;
        }
        let mut members = Vec::new();
        let mut q = vec![s];
        seen[s] = true;
        let mut h = 0;
        while h < q.len() {
            let v = q[h];
            h += 1;
            members.push(graph.vertices[v].id);
            for &(w, _) in &adj[v] {
                if !seen[w] {
                    seen[w] = true;
                    q.push(w)
                }
            }
        }
        let cid = r.groups.len() as i16;
        for &id in &members {
            r.values.push((id, cid));
        }
        r.groups.push(AnalysisGroup {
            vertices: members,
            edges: Vec::new(),
        });
    }
    add_group_edges(graph, &mut r.groups);
    r.count = r.groups.len() as u16;
    r
}

fn strong_components(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("STRONG COMPONENTS");
    let t = dag::scc_condensation(graph);
    for c in &t.components {
        let members = c.members.clone();
        for &id in &members {
            r.values.push((id, c.component as i16));
        }
        r.groups.push(AnalysisGroup {
            vertices: members,
            edges: Vec::new(),
        });
    }
    add_group_edges(graph, &mut r.groups);
    r.count = t.components.len() as u16;
    r
}

/// Maximal unilateral components.  After contracting SCCs, a unilateral
/// component corresponds to a maximal directed path of the condensation DAG;
/// expanding the SCCs yields the original-graph vertex set.  Maximal paths may
/// overlap, so groups are intentionally not a partition.
fn unilateral_components(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("UNILATERAL COMPONENTS");
    let t = dag::scc_condensation(graph);
    let c = t.components.len();
    if c == 0 {
        return r;
    }
    let mut adj = vec![Vec::<usize>::new(); c];
    let mut indeg = vec![0usize; c];
    for a in &t.condensation_arcs {
        let u = a.from_component as usize;
        let v = a.to_component as usize;
        if !adj[u].contains(&v) {
            adj[u].push(v);
            indeg[v] += 1;
        }
    }
    let mut paths: Vec<Vec<usize>> = Vec::new();
    let mut path = Vec::new();
    let mut steps = 0usize;
    let mut truncated = false;
    fn walk(
        v: usize,
        adj: &[Vec<usize>],
        path: &mut Vec<usize>,
        paths: &mut Vec<Vec<usize>>,
        steps: &mut usize,
        truncated: &mut bool,
    ) {
        if *truncated {
            return;
        }
        *steps = (*steps).saturating_add(1);
        if *steps >= MAX_UNILATERAL_STEPS || paths.len() >= MAX_UNILATERAL_COMPONENTS {
            *truncated = true;
            return;
        }
        path.push(v);
        if adj[v].is_empty() {
            paths.push(path.clone());
        } else {
            for &w in &adj[v] {
                walk(w, adj, path, paths, steps, truncated);
                if *truncated {
                    break;
                }
            }
        }
        path.pop();
    }
    for s in 0..c {
        if indeg[s] == 0 {
            walk(s, &adj, &mut path, &mut paths, &mut steps, &mut truncated);
            if truncated {
                break;
            }
        }
    }
    // A shortcut source->sink path can be a strict subset of another path and
    // is therefore not maximal as a unilateral vertex set.
    let mut keep = vec![true; paths.len()];
    for i in 0..paths.len() {
        for j in 0..paths.len() {
            if i == j || paths[i].len() >= paths[j].len() {
                continue;
            }
            if paths[i].iter().all(|x| paths[j].contains(x)) {
                keep[i] = false;
                break;
            }
        }
    }
    for (i, pth) in paths.into_iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let mut members = Vec::new();
        for cid in pth {
            members.extend(t.components[cid].members.iter().copied());
        }
        r.groups.push(AnalysisGroup {
            vertices: members,
            edges: Vec::new(),
        });
    }
    add_group_edges(graph, &mut r.groups);
    for (gi, g) in r.groups.iter().enumerate() {
        for &id in &g.vertices {
            r.values.push((id, gi as i16));
        }
    }
    r.count = r.groups.len() as u16;
    if truncated {
        r.truncated = true;
        r.exact = false;
        r.notice = Some(AnalysisNotice::warning(
            "UNILATERAL SEARCH HIT LIMIT",
            "DISPLAY IS TRUNCATED",
        ));
    }
    r
}

fn articulation_bridges(graph: &Digraph) -> (Vec<VertexId>, Vec<EdgeId>) {
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut disc = vec![-1i16; n];
    let mut low = vec![0i16; n];
    let mut time = 0i16;
    let mut art = vec![false; n];
    let mut bridges = Vec::new();
    fn dfs(
        v: usize,
        parent_edge: Option<EdgeId>,
        adj: &[Vec<(usize, EdgeId)>],
        disc: &mut [i16],
        low: &mut [i16],
        time: &mut i16,
        art: &mut [bool],
        bridges: &mut Vec<EdgeId>,
    ) {
        disc[v] = *time;
        low[v] = *time;
        *time += 1;
        let mut children = 0;
        for &(w, eid) in &adj[v] {
            if Some(eid) == parent_edge {
                continue;
            }
            if disc[w] < 0 {
                children += 1;
                dfs(w, Some(eid), adj, disc, low, time, art, bridges);
                low[v] = low[v].min(low[w]);
                if parent_edge.is_some() && low[w] >= disc[v] {
                    art[v] = true;
                }
                if low[w] > disc[v] {
                    bridges.push(eid);
                }
            } else {
                low[v] = low[v].min(disc[w]);
            }
        }
        if parent_edge.is_none() && children > 1 {
            art[v] = true;
        }
    }
    for s in 0..n {
        if disc[s] < 0 {
            dfs(
                s,
                None,
                &adj,
                &mut disc,
                &mut low,
                &mut time,
                &mut art,
                &mut bridges,
            )
        }
    }
    (
        art.iter()
            .enumerate()
            .filter_map(|(i, &x)| x.then_some(graph.vertices[i].id))
            .collect(),
        bridges,
    )
}

fn two_edge_components(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("2-EDGE COMPONENTS");
    let (_, bridges) = articulation_bridges(graph);
    let adj = undirected_adj(graph);
    let n = graph.vertices.len();
    let mut seen = vec![false; n];
    for s in 0..n {
        if seen[s] {
            continue;
        }
        let mut members = Vec::new();
        let mut q = vec![s];
        seen[s] = true;
        let mut h = 0;
        while h < q.len() {
            let v = q[h];
            h += 1;
            members.push(graph.vertices[v].id);
            for &(w, e) in &adj[v] {
                if bridges.contains(&e) {
                    continue;
                }
                if !seen[w] {
                    seen[w] = true;
                    q.push(w)
                }
            }
        }
        let cid = r.groups.len() as i16;
        for &id in &members {
            r.values.push((id, cid));
        }
        r.groups.push(AnalysisGroup {
            vertices: members,
            edges: Vec::new(),
        });
    }
    add_group_edges(graph, &mut r.groups);
    r.count = r.groups.len() as u16;
    r.secondary_edges = bridges;
    r
}

fn source_sink_components(graph: &Digraph) -> AnalysisResult {
    let mut r = AnalysisResult::empty("SOURCE / SINK SCC");
    let t = dag::scc_condensation(graph);
    let c = t.components.len();
    let mut indeg = vec![0u16; c];
    let mut out = vec![0u16; c];
    for a in &t.condensation_arcs {
        out[a.from_component as usize] += 1;
        indeg[a.to_component as usize] += 1;
    }
    for (i, node) in t.components.iter().enumerate() {
        if indeg[i] == 0 {
            r.primary_vertices.extend(node.members.iter().copied());
        }
        if out[i] == 0 {
            r.secondary_vertices.extend(node.members.iter().copied());
        }
    }
    r.count = c as u16;
    r
}

pub fn run_component(graph: &Digraph, kind: ComponentKind) -> AnalysisResult {
    match kind {
        ComponentKind::Weak => weak_components(graph),
        ComponentKind::Strong => strong_components(graph),
        ComponentKind::Unilateral => unilateral_components(graph),
        ComponentKind::Loops => find_cycle(graph),
        ComponentKind::Articulation => {
            let (mut a, _) = articulation_bridges(graph);
            let mut r = AnalysisResult::empty("ARTICULATION VERTICES");
            r.count = a.len() as u16;
            r.primary_vertices.append(&mut a);
            r
        }
        ComponentKind::Bridges => {
            let (_, mut b) = articulation_bridges(graph);
            let mut r = AnalysisResult::empty("BRIDGES");
            r.count = b.len() as u16;
            r.primary_edges.append(&mut b);
            r
        }
        ComponentKind::TwoEdgeConnected => two_edge_components(graph),
        ComponentKind::SourcesSinks => source_sink_components(graph),
    }
}

pub fn refresh_analysis(graph: Res<Digraph>, mut state: ResMut<AnalysisState>) {
    let needs = state.last_revision != graph.topology_revision
        || state.last_source != state.source
        || state.last_target != state.target
        || state.result.is_none();
    if !needs {
        return;
    }
    state.recompute(&graph);
}
