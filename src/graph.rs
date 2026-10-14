extern crate alloc;

use alloc::vec::Vec;
use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

pub type VertexId = u16;
pub type EdgeId = u16;

pub const MAX_VERTICES: usize = 200;
pub const MAX_EDGES: usize = 20_000;

pub const COLOR_VERTEX: u16 = rgb15(8, 19, 31);
pub const COLOR_EDGE: u16 = rgb15(12, 18, 24);
pub const COLOR_SELECTED: u16 = rgb15(31, 25, 4);
pub const COLOR_SOURCE: u16 = rgb15(4, 31, 10);
pub const COLOR_TARGET: u16 = rgb15(31, 7, 7);
pub const COLOR_MARK: u16 = rgb15(30, 8, 25);

pub const fn rgb15(r: u16, g: u16, b: u16) -> u16 {
    0x8000 | (r & 31) | ((g & 31) << 5) | ((b & 31) << 10)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vertex {
    pub id: VertexId,
    /// Bottom-screen position in full touch coordinates (0..255, 0..191).
    pub x: i16,
    pub y: i16,
    /// RGB15 + opaque bit.
    pub color: u16,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edge {
    pub id: EdgeId,
    pub from: VertexId,
    pub to: VertexId,
    pub directed: bool,
    pub color: u16,
}

#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
pub struct Digraph {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    /// Incremented only when vertices/edges are added, removed, or changed.
    /// Live analyses key off this value; moving or recoloring a vertex does not
    /// force an algorithm recomputation.
    #[serde(default)]
    pub topology_revision: u32,
    /// Rendering-only revision. This is intentionally not serialized: it exists
    /// only to make framebuffer cache invalidation O(1) when vertices move.
    #[serde(skip)]
    pub visual_revision: u32,
    next_vertex_id: VertexId,
    next_edge_id: EdgeId,
}

impl Default for Digraph {
    fn default() -> Self {
        Self::new()
    }
}

impl Digraph {
    /// Build a graph from serialized/materialized parts while restoring the
    /// internal ID counters.  Used by save/load and non-destructive analysis
    /// materialization.
    pub fn from_parts(mut vertices: Vec<Vertex>, mut edges: Vec<Edge>) -> Self {
        vertices.sort_unstable_by_key(|v| v.id);
        edges.sort_unstable_by_key(|e| e.id);
        let mut graph = Self {
            vertices,
            edges,
            topology_revision: 0,
            visual_revision: 0,
            next_vertex_id: 0,
            next_edge_id: 0,
        };
        graph.normalize_counters();
        graph
    }

    pub const fn new() -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
            topology_revision: 0,
            visual_revision: 0,
            next_vertex_id: 0,
            next_edge_id: 0,
        }
    }

    #[inline]
    fn topology_changed(&mut self) {
        self.topology_revision = self.topology_revision.wrapping_add(1);
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }

    #[inline]
    fn visual_changed(&mut self) {
        self.visual_revision = self.visual_revision.wrapping_add(1);
    }

    fn simplify_reciprocal_edges(&mut self) {
        let mut simple: Vec<Edge> = Vec::with_capacity(self.edges.len());
        for edge in self.edges.iter().copied() {
            if let Some(existing) = simple.iter_mut().find(|e| {
                (e.from == edge.from && e.to == edge.to) || (e.from == edge.to && e.to == edge.from)
            }) {
                let opposite_directed = existing.directed
                    && edge.directed
                    && existing.from == edge.to
                    && existing.to == edge.from;
                if !existing.directed || !edge.directed || opposite_directed {
                    existing.directed = false;
                }
                continue;
            }
            simple.push(edge);
        }
        self.edges = simple;
    }

    pub fn normalize_counters(&mut self) {
        self.vertices.sort_unstable_by_key(|v| v.id);
        self.edges.sort_unstable_by_key(|e| e.id);
        self.simplify_reciprocal_edges();
        self.edges.sort_unstable_by_key(|e| e.id);
        self.next_vertex_id = self
            .vertices
            .iter()
            .map(|v| v.id)
            .max()
            .map(|v| v.saturating_add(1))
            .unwrap_or(0);
        self.next_edge_id = self
            .edges
            .iter()
            .map(|e| e.id)
            .max()
            .map(|v| v.saturating_add(1))
            .unwrap_or(0);
    }

    #[inline]
    pub fn vertex_index(&self, id: VertexId) -> Option<usize> {
        self.vertices.binary_search_by_key(&id, |v| v.id).ok()
    }

    #[inline]
    pub fn edge_index(&self, id: EdgeId) -> Option<usize> {
        self.edges.binary_search_by_key(&id, |e| e.id).ok()
    }

    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertex_index(id).map(|i| &self.vertices[i])
    }

    pub fn vertex_mut(&mut self, id: VertexId) -> Option<&mut Vertex> {
        self.vertex_index(id).map(|i| &mut self.vertices[i])
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edge_index(id).map(|i| &self.edges[i])
    }

    pub fn edge_mut(&mut self, id: EdgeId) -> Option<&mut Edge> {
        self.edge_index(id).map(|i| &mut self.edges[i])
    }

    pub fn set_edge_state(
        &mut self,
        id: EdgeId,
        from: VertexId,
        to: VertexId,
        directed: bool,
    ) -> bool {
        if from == to || self.vertex(from).is_none() || self.vertex(to).is_none() {
            return false;
        }
        let Some(i) = self.edge_index(id) else {
            return false;
        };
        self.edges[i].from = from;
        self.edges[i].to = to;
        self.edges[i].directed = directed;
        self.topology_changed();
        true
    }

    pub fn reverse_edge(&mut self, id: EdgeId) -> bool {
        let Some(i) = self.edge_index(id) else {
            return false;
        };
        if !self.edges[i].directed {
            return false;
        }
        let (from, to) = (self.edges[i].from, self.edges[i].to);
        self.edges[i].from = to;
        self.edges[i].to = from;
        self.topology_changed();
        true
    }

    pub fn make_edge_undirected(&mut self, id: EdgeId, from: VertexId, to: VertexId) -> bool {
        self.set_edge_state(id, from, to, false)
    }

    pub fn add_vertex(&mut self, x: i16, y: i16) -> Option<VertexId> {
        if self.vertices.len() >= MAX_VERTICES || self.next_vertex_id == VertexId::MAX {
            return None;
        }
        let id = self.next_vertex_id;
        self.next_vertex_id = self.next_vertex_id.saturating_add(1);
        self.vertices.push(Vertex {
            id,
            x: x.clamp(-16_000, 16_000),
            y: y.clamp(-16_000, 16_000),
            color: COLOR_VERTEX,
        });
        self.topology_changed();
        Some(id)
    }

    pub fn remove_vertex(&mut self, id: VertexId) -> bool {
        let Some(i) = self.vertex_index(id) else {
            return false;
        };
        self.vertices.remove(i);
        self.edges.retain(|e| e.from != id && e.to != id);
        self.topology_changed();
        true
    }

    pub fn move_vertex(&mut self, id: VertexId, x: i16, y: i16) -> bool {
        let nx = x.clamp(-16_000, 16_000);
        let ny = y.clamp(-16_000, 16_000);
        let Some(i) = self.vertex_index(id) else {
            return false;
        };
        if self.vertices[i].x == nx && self.vertices[i].y == ny {
            return true;
        }
        self.vertices[i].x = nx;
        self.vertices[i].y = ny;
        self.visual_changed();
        true
    }

    pub fn set_vertex_color(&mut self, id: VertexId, color: u16) -> bool {
        let Some(i) = self.vertex_index(id) else {
            return false;
        };
        if self.vertices[i].color == color {
            return true;
        }
        self.vertices[i].color = color;
        self.visual_changed();
        true
    }

    /// O(1) insertion for graph generators that already guarantee one edge per
    /// unordered vertex pair. This avoids the duplicate scan in `add_edge`,
    /// which would make K_200 generation quadratic in the number of edges.
    pub fn add_generated_edge(
        &mut self,
        from: VertexId,
        to: VertexId,
        directed: bool,
    ) -> Option<EdgeId> {
        if from == to || self.vertex(from).is_none() || self.vertex(to).is_none() {
            return None;
        }
        if self.edges.len() >= MAX_EDGES || self.next_edge_id == VertexId::MAX {
            return None;
        }
        let id = self.next_edge_id;
        self.next_edge_id = self.next_edge_id.saturating_add(1);
        self.edges.push(Edge {
            id,
            from,
            to,
            directed,
            color: COLOR_EDGE,
        });
        self.topology_changed();
        Some(id)
    }

    pub fn add_edge(&mut self, from: VertexId, to: VertexId, directed: bool) -> Option<EdgeId> {
        if from == to || self.vertex(from).is_none() || self.vertex(to).is_none() {
            return None;
        }

        // The editor intentionally represents every unordered pair by at most
        // one stored edge.  Two reciprocal directed insertions collapse to one
        // undirected edge, which is the visual/semantic "both directions" state
        // used by the original Digraph Explorer.
        if let Some(i) = self.edges.iter().position(|e| {
            !e.directed && ((e.from == from && e.to == to) || (e.from == to && e.to == from))
        }) {
            return Some(self.edges[i].id);
        }

        if directed {
            if let Some(i) = self
                .edges
                .iter()
                .position(|e| e.directed && e.from == from && e.to == to)
            {
                return Some(self.edges[i].id);
            }
            if let Some(i) = self
                .edges
                .iter()
                .position(|e| e.directed && e.from == to && e.to == from)
            {
                let id = self.edges[i].id;
                self.edges[i].directed = false;
                self.topology_changed();
                return Some(id);
            }
        } else if let Some(i) = self
            .edges
            .iter()
            .position(|e| (e.from == from && e.to == to) || (e.from == to && e.to == from))
        {
            let id = self.edges[i].id;
            self.edges[i].directed = false;
            self.topology_changed();
            return Some(id);
        }

        if self.edges.len() >= MAX_EDGES || self.next_edge_id == EdgeId::MAX {
            return None;
        }
        let id = self.next_edge_id;
        self.next_edge_id = self.next_edge_id.saturating_add(1);
        self.edges.push(Edge {
            id,
            from,
            to,
            directed,
            color: COLOR_EDGE,
        });
        self.topology_changed();
        Some(id)
    }

    pub fn remove_edge(&mut self, id: EdgeId) -> bool {
        let Some(i) = self.edge_index(id) else {
            return false;
        };
        self.edges.remove(i);
        self.topology_changed();
        true
    }

    pub fn find_edge(&self, from: VertexId, to: VertexId, directed: bool) -> Option<EdgeId> {
        self.edges
            .iter()
            .find(|e| {
                if e.directed != directed {
                    return false;
                }
                if directed {
                    e.from == from && e.to == to
                } else {
                    (e.from == from && e.to == to) || (e.from == to && e.to == from)
                }
            })
            .map(|e| e.id)
    }

    pub fn toggle_edge(&mut self, from: VertexId, to: VertexId, directed: bool) -> bool {
        if let Some(id) = self.find_edge(from, to, directed) {
            self.remove_edge(id);
            false
        } else {
            self.add_edge(from, to, directed).is_some()
        }
    }

    /// Every traversable arc represented by an edge. Directed edges contribute
    /// one arc; undirected edges contribute both orientations.
    pub fn arcs_of(edge: &Edge) -> [(Option<VertexId>, Option<VertexId>); 2] {
        if edge.directed {
            [(Some(edge.from), Some(edge.to)), (None, None)]
        } else {
            [
                (Some(edge.from), Some(edge.to)),
                (Some(edge.to), Some(edge.from)),
            ]
        }
    }

    pub fn nearest_vertex(&self, x: i16, y: i16, radius: i16) -> Option<VertexId> {
        let r2 = i32::from(radius) * i32::from(radius);
        self.vertices
            .iter()
            .filter_map(|v| {
                let dx = i32::from(v.x - x);
                let dy = i32::from(v.y - y);
                let d2 = dx * dx + dy * dy;
                (d2 <= r2).then_some((d2, v.id))
            })
            .min_by_key(|&(d2, _)| d2)
            .map(|(_, id)| id)
    }

    pub fn clear_edges(&mut self) {
        if self.edges.is_empty() && self.edges.capacity() == 0 {
            return;
        }
        // `Vec::clear()` keeps its allocation. For very dense graphs that can
        // retain hundreds of KiB after the graph is emptied. Replacing the
        // vector drops the old backing allocation immediately so the allocator
        // can reuse that memory.
        self.edges = Vec::new();
        self.next_edge_id = 0;
        self.topology_changed();
    }

    pub fn clear(&mut self) {
        let changed = !self.vertices.is_empty() || !self.edges.is_empty();
        let had_capacity = self.vertices.capacity() != 0 || self.edges.capacity() != 0;
        // A true project reset should release graph storage, not merely set
        // len=0 while retaining the old K_n-sized capacity.
        self.vertices = Vec::new();
        self.edges = Vec::new();
        self.next_vertex_id = 0;
        self.next_edge_id = 0;
        if changed || had_capacity {
            self.topology_changed();
        }
    }
}
