//! Touch/button graph editor state, pan tool, graph generators, and menus.

extern crate alloc;
use alloc::vec::Vec;

use bevy_ecs::prelude::*;
use devy::prelude::*;

use crate::{
    analysis::{AnalysisKind, AnalysisState, ComponentKind},
    dag,
    graph::{COLOR_SELECTED, COLOR_VERTEX, Digraph, EdgeId, VertexId},
    project::{self, ProjectSnapshot},
    top2d::TopViewSettings,
    ui::{
        ActiveTool, EdgeTool, GraphAction, GraphDialog, GraphMenuState, MenuCategory,
        TouchMenuAction, UiState, VertexTool, hit_menu, name_key_at,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorNotice {
    Ready,
    Saved,
    Loaded,
    SaveError,
    LoadError,
    Applied,
    Cleared,
    NeedSource,
    NeedTarget,
}

#[derive(Resource, Clone, Debug)]
pub struct EditorState {
    pub selected: Option<VertexId>,
    pub edge_start: Option<VertexId>,
    pub dragging: bool,
    pub notice: EditorNotice,
    pub edge_cycle: Option<(EdgeId, VertexId, VertexId, u8)>,
    pub pan_last: Option<(i16, i16)>,
    /// Component group currently being dragged in a swapped quotient view.
    pub result_drag_key: Option<u32>,
}
impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected: None,
            edge_start: None,
            dragging: false,
            notice: EditorNotice::Ready,
            edge_cycle: None,
            pan_last: None,
            result_drag_key: None,
        }
    }
}

fn result_group_key(vertices: &[VertexId]) -> u32 {
    let mut sum = 0x811C_9DC5u32;
    let mut xor = 0u32;
    for &id in vertices {
        let mut x = u32::from(id).wrapping_add(0x9E37_79B9);
        x ^= x >> 16;
        x = x.wrapping_mul(0x7FEB_352D);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846C_A68B);
        x ^= x >> 16;
        sum = sum.wrapping_add(x);
        xor ^= x.rotate_left((id as u32) % 31);
    }
    sum ^ xor ^ (vertices.len() as u32).wrapping_mul(0x045D_9F3B)
}
impl EditorState {
    pub fn result_key(vertices: &[VertexId]) -> u32 {
        result_group_key(vertices)
    }
    pub fn clear_result_layout(&mut self) {
        self.result_drag_key = None;
    }
}

fn select_vertex(graph: &mut Digraph, state: &mut EditorState, id: Option<VertexId>) {
    if let Some(old) = state.selected {
        let _ = graph.set_vertex_color(old, COLOR_VERTEX);
    }
    state.selected = id;
    if let Some(id) = id {
        let _ = graph.set_vertex_color(id, COLOR_SELECTED);
    }
}
fn screen_vertex_hit(
    graph: &Digraph,
    ui: &UiState,
    x: i16,
    y: i16,
    radius: i16,
) -> Option<VertexId> {
    let r2 = i32::from(radius) * i32::from(radius);
    graph
        .vertices
        .iter()
        .filter_map(|v| {
            let (sx, sy) = ui.bottom_world_to_screen(v.x, v.y);
            let dx = i32::from(sx - x);
            let dy = i32::from(sy - y);
            let d = dx * dx + dy * dy;
            (d <= r2).then_some((d, v.id))
        })
        .min_by_key(|x| x.0)
        .map(|x| x.1)
}
fn seg_dist2(px: i32, py: i32, ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let vv = vx * vx + vy * vy;
    if vv == 0 {
        return wx * wx + wy * wy;
    }
    let t = ((wx * vx + wy * vy) * 256 / vv).clamp(0, 256);
    let qx = ax + vx * t / 256;
    let qy = ay + vy * t / 256;
    let dx = px - qx;
    let dy = py - qy;
    dx * dx + dy * dy
}
fn screen_edge_hit(graph: &Digraph, ui: &UiState, x: i16, y: i16, radius: i16) -> Option<EdgeId> {
    let r2 = i32::from(radius) * i32::from(radius);
    graph
        .edges
        .iter()
        .filter_map(|e| {
            let (a, b) = (graph.vertex(e.from)?, graph.vertex(e.to)?);
            let (ax, ay) = ui.bottom_world_to_screen(a.x, a.y);
            let (bx, by) = ui.bottom_world_to_screen(b.x, b.y);
            let d = seg_dist2(
                i32::from(x),
                i32::from(y),
                i32::from(ax),
                i32::from(ay),
                i32::from(bx),
                i32::from(by),
            );
            (d <= r2).then_some((d, e.id))
        })
        .min_by_key(|x| x.0)
        .map(|x| x.1)
}

fn component_quotient_active(analysis: &AnalysisState) -> bool {
    matches!(
        analysis.component,
        Some(ComponentKind::Weak | ComponentKind::Strong | ComponentKind::Unilateral)
    ) && analysis
        .result
        .as_ref()
        .is_some_and(|r| !r.groups.is_empty())
}
fn quotient_group_hit(
    graph: &Digraph,
    _editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
    x: i16,
    y: i16,
    radius: i16,
) -> Option<usize> {
    let result = analysis.result.as_ref()?;
    let r2 = i32::from(radius) * i32::from(radius);
    result
        .groups
        .iter()
        .enumerate()
        .filter_map(|(gi, g)| {
            let mut sx = 0i32;
            let mut sy = 0i32;
            let mut n = 0i32;
            for &id in &g.vertices {
                if let Some(v) = graph.vertex(id) {
                    sx += i32::from(v.x);
                    sy += i32::from(v.y);
                    n += 1;
                }
            }
            if n == 0 {
                return None;
            }
            let (wx, wy) = ((sx / n) as i16, (sy / n) as i16);
            let (px, py) = ui.result_world_to_screen(wx, wy);
            let dx = i32::from(px - x);
            let dy = i32::from(py - y);
            let d = dx * dx + dy * dy;
            (d <= r2).then_some((d, gi))
        })
        .min_by_key(|v| v.0)
        .map(|v| v.1)
}

fn snapshot(
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    top: TopViewSettings,
    ui: &UiState,
) -> ProjectSnapshot {
    ProjectSnapshot::capture(
        graph,
        analysis.selected,
        analysis.component,
        top,
        ui,
        editor.selected,
        editor.edge_start,
        analysis.source,
        analysis.target,
    )
}
fn apply_snapshot(
    s: ProjectSnapshot,
    graph: &mut Digraph,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    top: &mut TopViewSettings,
    ui: &mut UiState,
) {
    *graph = s.graph;
    *top = s.top_view.into();
    *ui = s.ui;
    editor.selected = s.selected_vertex.filter(|&id| graph.vertex(id).is_some());
    editor.edge_start = s.edge_start.filter(|&id| graph.vertex(id).is_some());
    editor.dragging = false;
    editor.pan_last = None;
    editor.edge_cycle = None;
    editor.clear_result_layout();
    analysis.source = s.source.filter(|&id| graph.vertex(id).is_some());
    analysis.target = s.target.filter(|&id| graph.vertex(id).is_some());
    analysis.clear();
    if let Some(k) = s.selected_analysis {
        analysis.select_algorithm(k);
    } else if let Some(k) = s.selected_component {
        analysis.select_component(k);
    }
    editor.notice = EditorNotice::Loaded;
}
fn save_project(
    graph: &Digraph,
    editor: &mut EditorState,
    analysis: &AnalysisState,
    top: TopViewSettings,
    ui: &UiState,
) {
    let snap = snapshot(graph, editor, analysis, top, ui);
    editor.notice = if project::save_default(&snap).is_ok() {
        EditorNotice::Saved
    } else {
        EditorNotice::SaveError
    };
}
fn load_project(
    graph: &mut Digraph,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    top: &mut TopViewSettings,
    ui: &mut UiState,
) {
    match project::load_default() {
        Ok(s) => apply_snapshot(s, graph, editor, analysis, top, ui),
        Err(_) => editor.notice = EditorNotice::LoadError,
    }
}
fn save_named_project(
    name: &str,
    graph: &Digraph,
    editor: &mut EditorState,
    analysis: &AnalysisState,
    top: TopViewSettings,
    ui: &UiState,
) -> bool {
    let snap = snapshot(graph, editor, analysis, top, ui);
    let ok = project::save_named(&snap, name).is_ok();
    editor.notice = if ok {
        EditorNotice::Saved
    } else {
        EditorNotice::SaveError
    };
    ok
}
fn load_named_project(
    name: &str,
    graph: &mut Digraph,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    top: &mut TopViewSettings,
    ui: &mut UiState,
) -> bool {
    match project::load_named(name) {
        Ok(s) => {
            apply_snapshot(s, graph, editor, analysis, top, ui);
            true
        }
        Err(_) => {
            editor.notice = EditorNotice::LoadError;
            false
        }
    }
}

fn reset_project_interaction(
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    ui: &mut UiState,
) {
    analysis.clear();
    analysis.source = None;
    analysis.target = None;
    editor.selected = None;
    editor.edge_start = None;
    editor.dragging = false;
    editor.pan_last = None;
    editor.edge_cycle = None;
    editor.clear_result_layout();
    ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
    ui.result_on_bottom = false;
    editor.notice = EditorNotice::Ready;
}

fn grid_positions(n: usize) -> Vec<(i16, i16)> {
    // A virtual-canvas layout rather than packing everything into 256x192.
    // Generated graphs are intentionally spacious; pan/zoom is the navigation layer.
    let mut cols = 1usize;
    while cols * cols < n {
        cols += 1;
    }
    let rows = n.div_ceil(cols);
    let mut out = Vec::with_capacity(n);
    let spacing = 40i32;
    let x0 = 128 - ((cols.saturating_sub(1) as i32) * spacing) / 2;
    let y0 = 88 - ((rows.saturating_sub(1) as i32) * spacing) / 2;
    for i in 0..n {
        let r = i / cols;
        let c = i % cols;
        out.push((
            (x0 + (c as i32) * spacing) as i16,
            (y0 + (r as i32) * spacing) as i16,
        ));
    }
    out
}
fn make_vertices(graph: &mut Digraph, n: usize) -> Vec<VertexId> {
    graph.clear();
    let mut ids = Vec::with_capacity(n);
    for (x, y) in grid_positions(n.min(200)) {
        if let Some(id) = graph.add_vertex(x, y) {
            ids.push(id);
        }
    }
    ids
}
fn make_complete(graph: &mut Digraph, n: usize) {
    let ids = make_vertices(graph, n.min(200));
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let _ = graph.add_generated_edge(ids[i], ids[j], false);
        }
    }
}
fn randomize_edges(graph: &mut Digraph, density: u8, rng: &mut GraphMenuState) {
    graph.clear_edges();
    let ids: Vec<VertexId> = graph.vertices.iter().map(|v| v.id).collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let a = (rng.next_u32() % 100) < u32::from(density);
            let b = (rng.next_u32() % 100) < u32::from(density);
            match (a, b) {
                (true, true) => {
                    let _ = graph.add_generated_edge(ids[i], ids[j], false);
                }
                (true, false) => {
                    let _ = graph.add_generated_edge(ids[i], ids[j], true);
                }
                (false, true) => {
                    let _ = graph.add_generated_edge(ids[j], ids[i], true);
                }
                _ => {}
            }
        }
    }
}
fn randomize_orientations(graph: &mut Digraph, rng: &mut GraphMenuState) {
    let states: Vec<(EdgeId, VertexId, VertexId)> =
        graph.edges.iter().map(|e| (e.id, e.from, e.to)).collect();
    for (id, a, b) in states {
        match rng.next_u32() % 3 {
            0 => {
                let _ = graph.set_edge_state(id, a, b, true);
            }
            1 => {
                let _ = graph.set_edge_state(id, b, a, true);
            }
            _ => {
                let _ = graph.set_edge_state(id, a, b, false);
            }
        }
    }
}
fn make_random(graph: &mut Digraph, n: usize, density: u8, rng: &mut GraphMenuState) {
    let _ = make_vertices(graph, n.min(200));
    randomize_edges(graph, density, rng);
}

fn activate_cursor(
    graph: &mut Digraph,
    ui: &mut UiState,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    library: &mut GraphMenuState,
) {
    match ui.category {
        MenuCategory::Vertex => {
            if let Some(&tool) = VertexTool::ALL.get(ui.cursor) {
                ui.tool = ActiveTool::Vertex(tool);
                editor.notice = EditorNotice::Ready;
                ui.menu_open = false;
            }
        }
        MenuCategory::Edge => {
            if let Some(&tool) = EdgeTool::ALL.get(ui.cursor) {
                ui.tool = ActiveTool::Edge(tool);
                editor.edge_cycle = None;
                editor.notice = EditorNotice::Ready;
                ui.menu_open = false;
            }
        }
        MenuCategory::Algorithms => {
            if let Some(&kind) = AnalysisKind::ALL.get(ui.cursor) {
                analysis.source = None;
                analysis.target = None;
                editor.edge_start = None;
                editor.edge_cycle = None;
                analysis.select_algorithm(kind);
                analysis.recompute(graph);
                ui.menu_open = false;
                if kind.needs_source() {
                    ui.tool = ActiveTool::Vertex(VertexTool::SetSource);
                    editor.notice = EditorNotice::NeedSource;
                } else {
                    ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
                    editor.notice = EditorNotice::Ready;
                }
            }
        }
        MenuCategory::Components => {
            if let Some(&kind) = ComponentKind::ALL.get(ui.cursor) {
                analysis.source = None;
                analysis.target = None;
                editor.edge_start = None;
                editor.edge_cycle = None;
                analysis.select_component(kind);
                analysis.recompute(graph);
                ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
                editor.notice = EditorNotice::Ready;
                ui.menu_open = false;
            }
        }
        MenuCategory::Graphs => {
            if let Some(&action) = GraphAction::ALL.get(ui.cursor) {
                match action {
                    GraphAction::New => {
                        graph.clear();
                        reset_project_interaction(editor, analysis, ui);
                        ui.reset_view();
                        ui.reset_result_view();
                        ui.menu_open = false;
                    }
                    GraphAction::Complete => {
                        library.begin_order_entry();
                        library.dialog = GraphDialog::CompleteOrder;
                    }
                    GraphAction::Random => library.dialog = GraphDialog::RandomSettings,
                    GraphAction::RandomizeEdges => {
                        randomize_edges(graph, library.density, library);
                        editor.notice = EditorNotice::Ready;
                        ui.menu_open = false;
                    }
                    GraphAction::RandomizeOrientations => {
                        randomize_orientations(graph, library);
                        editor.notice = EditorNotice::Ready;
                        ui.menu_open = false;
                    }
                    GraphAction::SwapView => {
                        if analysis.selected.is_some() || analysis.component.is_some() {
                            ui.result_on_bottom = !ui.result_on_bottom;
                        }
                        editor.dragging = false;
                        editor.pan_last = None;
                        ui.menu_open = false;
                    }
                    GraphAction::Stats => {
                        library.dialog = GraphDialog::Stats;
                    }
                    GraphAction::SaveAs => {
                        library.save_name.clear();
                        library.dialog = GraphDialog::SaveName;
                    }
                    GraphAction::Open => {
                        library.saved_names = project::list_saved_names().unwrap_or_default();
                        library.saved_cursor = 0;
                        library.saved_page = 0;
                        library.dialog = GraphDialog::LoadList;
                    }
                }
            }
        }
    }
}

fn apply_analysis(graph: &mut Digraph, editor: &mut EditorState, analysis: &mut AnalysisState) {
    let is_transform = analysis.selected.and_then(|k| k.dag_kind()).is_some();
    if is_transform
        && let Some(result) = analysis.result.as_ref()
        && let Some(t) = result.dag.as_ref()
    {
        *graph = dag::materialize(graph, t);
        editor.selected = None;
        editor.edge_start = None;
        analysis.source = None;
        analysis.target = None;
        analysis.clear();
        editor.notice = EditorNotice::Applied;
        return;
    }
    editor.notice = EditorNotice::Applied;
}
fn cycle_edge_orientation(graph: &mut Digraph, editor: &mut EditorState, id: EdgeId) {
    let Some(edge) = graph.edge(id).copied() else {
        editor.edge_cycle = None;
        return;
    };
    if editor.edge_cycle.map(|x| x.0) != Some(id) {
        editor.edge_cycle = Some((id, edge.from, edge.to, 0));
    }
    let Some((_, origin_from, origin_to, stage)) = editor.edge_cycle else {
        return;
    };
    let next_stage = match stage {
        0 if edge.directed => {
            let _ = graph.set_edge_state(id, origin_to, origin_from, true);
            1
        }
        1 => {
            let _ = graph.make_edge_undirected(id, origin_from, origin_to);
            2
        }
        _ => {
            let _ = graph.set_edge_state(id, origin_from, origin_to, true);
            0
        }
    };
    editor.edge_cycle = Some((id, origin_from, origin_to, next_stage));
}

fn touch_graph(
    x: i16,
    y: i16,
    graph: &mut Digraph,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    ui: &mut UiState,
) {
    let quotient = ui.result_on_bottom && component_quotient_active(analysis);
    match ui.tool {
        ActiveTool::Vertex(tool) => match tool {
            VertexTool::SelectMove => {
                if quotient {
                    if let Some(gi) = quotient_group_hit(graph, editor, analysis, ui, x, y, 12) {
                        if let Some(result) = analysis.result.as_ref()
                            && let Some(g) = result.groups.get(gi)
                        {
                            editor.result_drag_key = Some(EditorState::result_key(&g.vertices));
                            editor.dragging = true;
                        }
                    } else {
                        editor.result_drag_key = None;
                        editor.dragging = false;
                    }
                    select_vertex(graph, editor, None);
                } else if let Some(hit) =
                    screen_vertex_hit(graph, ui, x, y, if ui.result_on_bottom { 12 } else { 9 })
                {
                    // Generic swapped results use the original vertices, so dragging
                    // them updates the authoritative graph geometry and the live result.
                    select_vertex(graph, editor, Some(hit));
                    editor.dragging = true;
                    editor.result_drag_key = None;
                } else {
                    select_vertex(graph, editor, None);
                    editor.result_drag_key = None;
                }
            }
            VertexTool::Pan => {
                editor.pan_last = Some((x, y));
                editor.dragging = false;
            }
            VertexTool::Add => {
                let (wx, wy) = ui.bottom_screen_to_world(x, y);
                if let Some(id) = graph.add_vertex(wx, wy) {
                    select_vertex(graph, editor, Some(id));
                    editor.dragging = true;
                    editor.result_drag_key = None;
                }
            }
            VertexTool::Delete => {
                if quotient {
                    if let Some(gi) = quotient_group_hit(graph, editor, analysis, ui, x, y, 12)
                        && let Some(result) = analysis.result.as_ref()
                    {
                        let ids = result
                            .groups
                            .get(gi)
                            .map(|g| g.vertices.clone())
                            .unwrap_or_default();
                        for id in ids {
                            graph.remove_vertex(id);
                            if editor.selected == Some(id) {
                                editor.selected = None
                            }
                            if editor.edge_start == Some(id) {
                                editor.edge_start = None
                            }
                            if analysis.source == Some(id) {
                                analysis.source = None
                            }
                            if analysis.target == Some(id) {
                                analysis.target = None
                            }
                        }
                    }
                } else if let Some(hit) = screen_vertex_hit(graph, ui, x, y, 9) {
                    graph.remove_vertex(hit);
                    if editor.selected == Some(hit) {
                        editor.selected = None
                    }
                    if editor.edge_start == Some(hit) {
                        editor.edge_start = None
                    }
                    if analysis.source == Some(hit) {
                        analysis.source = None
                    }
                    if analysis.target == Some(hit) {
                        analysis.target = None
                    }
                }
            }
            VertexTool::SetSource => {
                if !quotient && let Some(hit) = screen_vertex_hit(graph, ui, x, y, 9) {
                    analysis.source = Some(hit);
                    select_vertex(graph, editor, Some(hit));
                    if analysis.selected.is_some_and(|kind| kind.needs_target()) {
                        ui.tool = ActiveTool::Vertex(VertexTool::SetTarget);
                        editor.notice = EditorNotice::NeedTarget;
                    } else {
                        ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
                        editor.notice = EditorNotice::Ready;
                    }
                }
            }
            VertexTool::SetTarget => {
                if !quotient && let Some(hit) = screen_vertex_hit(graph, ui, x, y, 9) {
                    analysis.target = Some(hit);
                    select_vertex(graph, editor, Some(hit));
                    ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
                    editor.notice = EditorNotice::Ready;
                }
            }
        },
        ActiveTool::Edge(tool) => {
            // Edge editing maps directly to the original graph when the result uses the
            // original vertices. Quotient super-edges are derived objects, so they are
            // deliberately not mutated independently.
            if !quotient {
                match tool {
                    EdgeTool::Directed | EdgeTool::Undirected => {
                        if let Some(hit) = screen_vertex_hit(graph, ui, x, y, 9) {
                            if let Some(start) = editor.edge_start.take() {
                                if start != hit {
                                    graph.toggle_edge(start, hit, tool == EdgeTool::Directed);
                                    editor.edge_cycle = None;
                                }
                            } else {
                                editor.edge_start = Some(hit);
                            }
                            select_vertex(graph, editor, Some(hit));
                        }
                    }
                    EdgeTool::Delete => {
                        if let Some(e) = screen_edge_hit(graph, ui, x, y, 6) {
                            graph.remove_edge(e);
                            editor.edge_cycle = None;
                        }
                    }
                    EdgeTool::Reverse => {
                        if let Some(e) = screen_edge_hit(graph, ui, x, y, 6) {
                            cycle_edge_orientation(graph, editor, e);
                        }
                    }
                }
            }
        }
    }
}
fn handle_dialog_touch(
    x: i16,
    y: i16,
    graph: &mut Digraph,
    editor: &mut EditorState,
    analysis: &mut AnalysisState,
    top: &mut TopViewSettings,
    ui: &mut UiState,
    library: &mut GraphMenuState,
) {
    match library.dialog {
        GraphDialog::None => {}
        GraphDialog::CompleteOrder => {
            let digit = match (x, y) {
                (24..=86, 112..=126) => Some(1),
                (97..=159, 112..=126) => Some(2),
                (170..=232, 112..=126) => Some(3),
                (24..=86, 128..=142) => Some(4),
                (97..=159, 128..=142) => Some(5),
                (170..=232, 128..=142) => Some(6),
                (24..=86, 144..=158) => Some(7),
                (97..=159, 144..=158) => Some(8),
                (170..=232, 144..=158) => Some(9),
                (97..=159, 160..=178) => Some(0),
                _ => None,
            };
            if let Some(d) = digit {
                library.order_digit(d);
            } else if (24..=86).contains(&x) && (160..=178).contains(&y) {
                library.order_backspace();
            } else if (170..=232).contains(&x) && (160..=178).contains(&y) {
                library.commit_order_entry();
                make_complete(graph, usize::from(library.order));
                reset_project_interaction(editor, analysis, ui);
                ui.reset_view();
                ui.reset_result_view();
                library.dialog = GraphDialog::None;
                ui.menu_open = false;
            }
        }
        GraphDialog::RandomSettings => {
            if (20..=65).contains(&x) && (102..=124).contains(&y) {
                library.order_dec();
            } else if (190..=235).contains(&x) && (102..=124).contains(&y) {
                library.order_inc();
            } else if (20..=65).contains(&x) && (128..=150).contains(&y) {
                library.density_dec();
            } else if (190..=235).contains(&x) && (128..=150).contains(&y) {
                library.density_inc();
            } else if (136..=236).contains(&x) && (155..=178).contains(&y) {
                let n = library.order;
                let p = library.density;
                make_random(graph, usize::from(n), p, library);
                reset_project_interaction(editor, analysis, ui);
                ui.reset_view();
                ui.reset_result_view();
                library.dialog = GraphDialog::None;
                ui.menu_open = false;
            } else if (20..=120).contains(&x) && (155..=178).contains(&y) {
                library.dialog = GraphDialog::None;
            }
        }
        GraphDialog::SaveName => {
            if let Some(ch) = name_key_at(x, y) {
                if library.save_name.len() < project::MAX_GRAPH_NAME {
                    library.save_name.push(ch);
                }
            } else if (0..=83).contains(&x) && (162..=180).contains(&y) {
                let _ = library.save_name.pop();
            } else if (84..=169).contains(&x) && (162..=180).contains(&y) {
                if save_named_project(&library.save_name, graph, editor, analysis, *top, ui) {
                    library.dialog = GraphDialog::None;
                    ui.menu_open = false;
                }
            } else if (170..=255).contains(&x) && (162..=180).contains(&y) {
                library.dialog = GraphDialog::None;
            }
        }
        GraphDialog::LoadList => {
            let start = library.saved_page * 5;
            if (96..=160).contains(&y) {
                let row = ((i32::from(y) - 96) / 13).clamp(0, 4) as usize;
                let idx = start + row;
                if idx < library.saved_names.len() {
                    library.saved_cursor = idx;
                    let name = library.saved_names[idx].clone();
                    if load_named_project(&name, graph, editor, analysis, top, ui) {
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
            } else if (0..=83).contains(&x) && (162..=180).contains(&y) {
                if library.saved_page > 0 {
                    library.saved_page -= 1;
                }
            } else if (84..=169).contains(&x) && (162..=180).contains(&y) {
                let pages = library.saved_names.len().div_ceil(5);
                if library.saved_page + 1 < pages {
                    library.saved_page += 1;
                }
            } else if (170..=255).contains(&x) && (162..=180).contains(&y) {
                library.dialog = GraphDialog::None;
            }
        }
        GraphDialog::Stats => {
            if (170..=255).contains(&x) && (162..=180).contains(&y) {
                library.dialog = GraphDialog::None;
                ui.menu_open = false;
            }
        }
    }
}

pub fn editor_input(
    buttons: Res<ButtonInput<DsButton>>,
    touches: Res<Touches>,
    mut graph: ResMut<Digraph>,
    mut editor: ResMut<EditorState>,
    mut analysis: ResMut<AnalysisState>,
    mut top: ResMut<TopViewSettings>,
    mut ui: ResMut<UiState>,
    mut library: ResMut<GraphMenuState>,
) {
    library.rng_state = library.rng_state.wrapping_add(0x9E37_79B9);
    if buttons.just_pressed(DsButton::Start) {
        ui.zoom_in();
    }
    if buttons.just_pressed(DsButton::Select) {
        ui.zoom_out();
    }

    if library.dialog != GraphDialog::None {
        if buttons.just_pressed(DsButton::B) {
            library.dialog = GraphDialog::None;
        } else {
            match library.dialog {
                GraphDialog::CompleteOrder => {
                    if buttons.just_pressed(DsButton::A) {
                        library.commit_order_entry();
                        make_complete(&mut graph, usize::from(library.order));
                        reset_project_interaction(&mut editor, &mut analysis, &mut ui);
                        ui.reset_view();
                        ui.reset_result_view();
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
                GraphDialog::RandomSettings => {
                    if buttons.just_pressed(DsButton::Left) {
                        library.order_dec();
                    }
                    if buttons.just_pressed(DsButton::Right) {
                        library.order_inc();
                    }
                    if buttons.just_pressed(DsButton::A) {
                        let n = library.order;
                        let p = library.density;
                        make_random(&mut graph, usize::from(n), p, &mut library);
                        reset_project_interaction(&mut editor, &mut analysis, &mut ui);
                        ui.reset_view();
                        ui.reset_result_view();
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
                GraphDialog::SaveName => {
                    if buttons.just_pressed(DsButton::A)
                        && save_named_project(
                            &library.save_name,
                            &graph,
                            &mut editor,
                            &analysis,
                            *top,
                            &ui,
                        )
                    {
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
                GraphDialog::LoadList => {
                    if buttons.just_pressed(DsButton::Left) && library.saved_cursor > 0 {
                        library.saved_cursor -= 1;
                        library.saved_page = library.saved_cursor / 5;
                    }
                    if buttons.just_pressed(DsButton::Right)
                        && library.saved_cursor + 1 < library.saved_names.len()
                    {
                        library.saved_cursor += 1;
                        library.saved_page = library.saved_cursor / 5;
                    }
                    if buttons.just_pressed(DsButton::A)
                        && let Some(name) = library.saved_names.get(library.saved_cursor).cloned()
                        && load_named_project(
                            &name,
                            &mut graph,
                            &mut editor,
                            &mut analysis,
                            &mut top,
                            &mut ui,
                        )
                    {
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
                GraphDialog::Stats => {
                    if buttons.just_pressed(DsButton::A) {
                        library.dialog = GraphDialog::None;
                        ui.menu_open = false;
                    }
                }
                GraphDialog::None => {}
            }
        }
    } else if buttons.just_pressed(DsButton::Up) {
        graph.clear();
        analysis.clear();
        analysis.source = None;
        analysis.target = None;
        editor.selected = None;
        editor.edge_start = None;
        editor.dragging = false;
        editor.pan_last = None;
        editor.edge_cycle = None;
        editor.clear_result_layout();
        ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
        ui.result_on_bottom = false;
        ui.menu_open = false;
        editor.notice = EditorNotice::Cleared;
    } else if ui.menu_open {
        if buttons.just_pressed(DsButton::Down) || buttons.just_pressed(DsButton::B) {
            ui.menu_open = false;
        } else {
            if buttons.just_pressed(DsButton::Left) {
                ui.move_left();
            }
            if buttons.just_pressed(DsButton::Right) {
                ui.move_right();
            }
            if buttons.just_pressed(DsButton::A) {
                activate_cursor(
                    &mut graph,
                    &mut ui,
                    &mut editor,
                    &mut analysis,
                    &mut library,
                );
            }
        }
    } else {
        if buttons.just_pressed(DsButton::Down) {
            ui.menu_open = true;
            ui.clamp_cursor();
        } else if buttons.just_pressed(DsButton::A) {
            ui.open(MenuCategory::Vertex);
        } else if buttons.just_pressed(DsButton::B) {
            ui.open(MenuCategory::Edge);
        } else if buttons.just_pressed(DsButton::Y) {
            ui.open(MenuCategory::Algorithms);
        } else if buttons.just_pressed(DsButton::X) {
            ui.open(MenuCategory::Components);
        }
        if buttons.just_pressed(DsButton::L) {
            apply_analysis(&mut graph, &mut editor, &mut analysis);
        }
    }

    for touch in touches.iter_just_pressed() {
        let p = touch.position();
        let x = p.x.clamp(0.0, 255.0) as i16;
        let y = p.y.clamp(0.0, 191.0) as i16;
        library.bump_entropy((u32::from(x as u16) << 16) | u32::from(y as u16));
        if library.dialog != GraphDialog::None {
            handle_dialog_touch(
                x,
                y,
                &mut graph,
                &mut editor,
                &mut analysis,
                &mut top,
                &mut ui,
                &mut library,
            );
        } else if ui.menu_open {
            match hit_menu(x, y, &ui) {
                TouchMenuAction::Category(category) => {
                    ui.category = category;
                    ui.cursor = 0;
                }
                TouchMenuAction::Item(index) => {
                    ui.cursor = index;
                    activate_cursor(
                        &mut graph,
                        &mut ui,
                        &mut editor,
                        &mut analysis,
                        &mut library,
                    );
                }
                TouchMenuAction::Theme => ui.theme = ui.theme.next(),
                TouchMenuAction::Save => save_project(&graph, &mut editor, &analysis, *top, &ui),
                TouchMenuAction::Load => {
                    load_project(&mut graph, &mut editor, &mut analysis, &mut top, &mut ui)
                }
                TouchMenuAction::None => {}
            }
        } else {
            touch_graph(x, y, &mut graph, &mut editor, &mut analysis, &mut ui);
        }
    }

    if !ui.menu_open && library.dialog == GraphDialog::None {
        if matches!(ui.tool, ActiveTool::Vertex(VertexTool::Pan)) {
            if let Some(touch) = touches.iter().next() {
                let p = touch.position();
                let now = (p.x.clamp(0.0, 255.0) as i16, p.y.clamp(0.0, 191.0) as i16);
                if let Some(last) = editor.pan_last {
                    ui.pan_by(now.0 - last.0, now.1 - last.1);
                }
                editor.pan_last = Some(now);
            }
        } else if editor.dragging
            && matches!(
                ui.tool,
                ActiveTool::Vertex(VertexTool::SelectMove | VertexTool::Add)
            )
        {
            if ui.result_on_bottom && component_quotient_active(&analysis) {
                if let (Some(key), Some(touch), Some(result)) = (
                    editor.result_drag_key,
                    touches.iter().next(),
                    analysis.result.as_ref(),
                ) && let Some(g) = result
                    .groups
                    .iter()
                    .find(|g| EditorState::result_key(&g.vertices) == key)
                {
                    // A quotient super-vertex is the component itself. Dragging it
                    // translates every original member by the same delta so the
                    // original graph on the top LCD follows the bottom result view.
                    let verts = g.vertices.clone();
                    let mut sx = 0i32;
                    let mut sy = 0i32;
                    let mut n = 0i32;
                    for &id in &verts {
                        if let Some(v) = graph.vertex(id) {
                            sx += i32::from(v.x);
                            sy += i32::from(v.y);
                            n += 1;
                        }
                    }
                    if n > 0 {
                        let p = touch.position();
                        let (wx, wy) = ui.result_screen_to_world(
                            p.x.clamp(0.0, 255.0) as i16,
                            p.y.clamp(0.0, 191.0) as i16,
                        );
                        let cx = sx / n;
                        let cy = sy / n;
                        let dx = i32::from(wx) - cx;
                        let dy = i32::from(wy) - cy;
                        let positions: Vec<(VertexId, i16, i16)> = verts
                            .iter()
                            .filter_map(|&id| graph.vertex(id).map(|v| (id, v.x, v.y)))
                            .collect();
                        for (id, x0, y0) in positions {
                            let nx = (i32::from(x0) + dx).clamp(-16_000, 16_000) as i16;
                            let ny = (i32::from(y0) + dy).clamp(-16_000, 16_000) as i16;
                            let _ = graph.move_vertex(id, nx, ny);
                        }
                    }
                }
            } else if let (Some(id), Some(touch)) = (editor.selected, touches.iter().next()) {
                let p = touch.position();
                let (wx, wy) = ui.bottom_screen_to_world(
                    p.x.clamp(0.0, 255.0) as i16,
                    p.y.clamp(0.0, 191.0) as i16,
                );
                graph.move_vertex(id, wx, wy);
            }
        }
    }
    if touches.iter_just_released().next().is_some() {
        editor.dragging = false;
        editor.pan_last = None;
        editor.result_drag_key = None;
    }

    if library.dialog == GraphDialog::None
        && let Some(kind) = analysis.selected
    {
        if kind.needs_source() && analysis.source.is_none() {
            editor.notice = EditorNotice::NeedSource;
            if !ui.menu_open {
                ui.tool = ActiveTool::Vertex(VertexTool::SetSource);
            }
        } else if kind.needs_target() && analysis.target.is_none() {
            editor.notice = EditorNotice::NeedTarget;
            if !ui.menu_open {
                ui.tool = ActiveTool::Vertex(VertexTool::SetTarget);
            }
        } else if matches!(
            editor.notice,
            EditorNotice::NeedSource | EditorNotice::NeedTarget
        ) {
            editor.notice = EditorNotice::Ready;
            if !ui.menu_open {
                ui.tool = ActiveTool::Vertex(VertexTool::SelectMove);
            }
        }
    }
}
