//! Sub-engine bitmap renderer for the interactive bottom editor + command menu.
//! Rendering happens in main RAM and is uploaded as packed u16 stores because
//! DS VRAM doesn't support byte writes.

extern crate alloc;
use alloc::{format, vec, vec::Vec};
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;

use crate::{
    analysis::{AnalysisKind, AnalysisState},
    editor::{EditorNotice, EditorState},
    graph::{Digraph, Edge},
    stats::StatsState,
    top2d::{Canvas8, TOP_HEIGHT, TOP_WIDTH},
    ui::{
        ACTION_BOTTOM, ACTION_TOP, ActiveTool, EdgeTool, GraphDialog, GraphMenuState, ITEM_H,
        ITEM_TOP, ITEMS_PER_PAGE, MENU_TOP, MenuCategory, NAME_KEYS, TAB_BOTTOM, TAB_TOP, Theme,
        UiState, VertexTool,
    },
};

const C_BG: u8 = 0;
const C_GRID: u8 = 1;
const C_EDGE: u8 = 2;
const C_VERTEX: u8 = 3;
const C_SELECTED: u8 = 4;
const C_SOURCE: u8 = 5;
const C_TEXT: u8 = 6;
const C_MUTED: u8 = 7;
const C_PANEL: u8 = 8;
const C_TAB: u8 = 9;
const C_ACCENT: u8 = 10;
const C_DANGER: u8 = 11;
const C_TARGET: u8 = 12;
const C_RESULT: u8 = 13;
const C_RESULT2: u8 = 14;
const C_GROUP0: u8 = 15;

#[derive(Resource, Debug)]
pub struct BottomViewState {
    initialized: bool,
    last_signature: u32,
    last_base_signature: u32,
    frame: Vec<u8>,
    base_frame: Vec<u8>,
    last_theme: Option<Theme>,
}
impl Default for BottomViewState {
    fn default() -> Self {
        Self {
            initialized: false,
            last_signature: u32::MAX,
            last_base_signature: u32::MAX,
            frame: vec![0; TOP_WIDTH * TOP_HEIGHT],
            base_frame: vec![0; TOP_WIDTH * TOP_HEIGHT],
            last_theme: None,
        }
    }
}
pub struct BottomViewPlugin;
impl Plugin for BottomViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BottomViewState>()
            .add_systems(PostUpdate, render_bottom_view);
    }
}

fn rect(c: &mut Canvas8<'_>, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
    c.hline(x0, x1, y0, color);
    c.hline(x0, x1, y1, color);
    c.vline(x0, y0, y1, color);
    c.vline(x1, y0, y1, color);
}
fn fill_rect(c: &mut Canvas8<'_>, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
    for y in y0..=y1 {
        c.hline(x0, x1, y, color)
    }
}
fn tile_panel(c: &mut Canvas8<'_>, x0: i32, y0: i32, x1: i32, y1: i32) {
    fill_rect(c, x0, y0, x1, y1, C_PANEL);
    for y in (y0..=y1).step_by(8) {
        for x in (x0..=x1).step_by(8) {
            c.pixel(x, y, C_GRID)
        }
    }
    rect(c, x0, y0, x1, y1, C_MUTED);
}

fn draw_grid(c: &mut Canvas8<'_>, ui: &UiState) {
    let (n, d) = if ui.result_on_bottom {
        ui.result_zoom_fraction()
    } else {
        ui.zoom_fraction()
    };
    let step = (16 * n / d).max(4) as usize;
    let sx = step as i32;
    let (px, py) = if ui.result_on_bottom {
        (ui.result_pan_x, ui.result_pan_y)
    } else {
        (ui.pan_x, ui.pan_y)
    };
    let ox = ((i32::from(px) % sx) + sx) % sx;
    let oy = ((i32::from(py) % sx) + sx) % sx;
    let mut x = ox;
    while x < TOP_WIDTH as i32 {
        c.vline(x, 0, TOP_HEIGHT as i32 - 12, C_GRID);
        x += sx;
    }
    let mut y = oy;
    while y < TOP_HEIGHT as i32 - 12 {
        c.hline(0, TOP_WIDTH as i32 - 1, y, C_GRID);
        y += sx;
    }
}
fn clipped_segment(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    start_inset: i32,
    end_inset: i32,
) -> (i32, i32, i32, i32) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let scale = dx.abs().max(dy.abs()).max(1);
    (
        x0 + dx * start_inset / scale,
        y0 + dy * start_inset / scale,
        x1 - dx * end_inset / scale,
        y1 - dy * end_inset / scale,
    )
}
fn current_zoom(ui: &UiState) -> u8 {
    if ui.result_on_bottom {
        ui.result_zoom
    } else {
        ui.zoom
    }
}
fn dense_edge_stride(graph: &Digraph, ui: &UiState) -> usize {
    let e = graph.edges.len();
    let z = current_zoom(ui);
    let budget = if z <= 2 {
        1500
    } else if z <= 5 {
        3000
    } else {
        usize::MAX
    };
    if e <= budget || budget == usize::MAX {
        1
    } else {
        e.div_ceil(budget)
    }
}
fn detailed_arrows(graph: &Digraph, ui: &UiState) -> bool {
    graph.edges.len() <= 4000 || current_zoom(ui) >= 6
}
fn draw_edge_offset(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    e: &Edge,
    ui: &UiState,
    color: u8,
    offset: i32,
) {
    let (Some(a), Some(b)) = (graph.vertex(e.from), graph.vertex(e.to)) else {
        return;
    };
    let (ax, ay) = ui.bottom_world_to_screen(a.x, a.y);
    let (bx, by) = ui.bottom_world_to_screen(b.x, b.y);
    let (x0, y0, x1, y1) = (i32::from(ax), i32::from(ay), i32::from(bx), i32::from(by));
    let dx = x1 - x0;
    let dy = y1 - y0;
    let scale = dx.abs().max(dy.abs()).max(1);
    let ox = (-dy) * offset / scale;
    let oy = dx * offset / scale;
    let (a0, b0, a1, b1) = clipped_segment(x0 + ox, y0 + oy, x1 + ox, y1 + oy, 6, 9);
    if e.directed && detailed_arrows(graph, ui) {
        c.arrow(a0, b0, a1, b1, color)
    } else {
        c.line(a0, b0, a1, b1, color)
    }
}
fn draw_edge_color(c: &mut Canvas8<'_>, graph: &Digraph, e: &Edge, ui: &UiState, color: u8) {
    draw_edge_offset(c, graph, e, ui, color, 0)
}
fn draw_edge(c: &mut Canvas8<'_>, graph: &Digraph, e: &Edge, ui: &UiState) {
    draw_edge_color(c, graph, e, ui, C_EDGE)
}
fn group_color(i: usize) -> u8 {
    C_GROUP0 + (i as u8 % 8)
}
fn group_colors_vertex(
    result: &crate::analysis::AnalysisResult,
    id: crate::graph::VertexId,
    out: &mut [u8; 8],
) -> usize {
    let mut n = 0;
    for (gi, g) in result.groups.iter().enumerate() {
        if g.vertices.contains(&id) {
            let col = group_color(gi);
            if n < 8 && !out[..n].contains(&col) {
                out[n] = col;
                n += 1;
            }
        }
    }
    n
}
fn group_colors_edge(
    result: &crate::analysis::AnalysisResult,
    id: crate::graph::EdgeId,
    out: &mut [u8; 8],
) -> usize {
    let mut n = 0;
    for (gi, g) in result.groups.iter().enumerate() {
        if g.edges.contains(&id) {
            let col = group_color(gi);
            if n < 8 && !out[..n].contains(&col) {
                out[n] = col;
                n += 1;
            }
        }
    }
    n
}
fn draw_grouped_edge(c: &mut Canvas8<'_>, graph: &Digraph, e: &Edge, ui: &UiState, colors: &[u8]) {
    if colors.len() <= 1 {
        draw_edge_color(c, graph, e, ui, colors.first().copied().unwrap_or(C_EDGE));
        return;
    }
    let offsets = [-3, -2, -1, 0, 1, 2, 3, 4];
    for (i, &col) in colors.iter().take(8).enumerate() {
        draw_edge_offset(c, graph, e, ui, col, offsets[i]);
    }
}
fn draw_multicolor_vertex(c: &mut Canvas8<'_>, x: i32, y: i32, colors: &[u8]) {
    let r = 4;
    if colors.is_empty() {
        c.fill_circle(x, y, r, C_VERTEX);
    } else if colors.len() == 1 {
        c.fill_circle(x, y, r, colors[0]);
    } else {
        let rr = r * r;
        for py in -r..=r {
            for px in -r..=r {
                if px * px + py * py <= rr {
                    let band = (((px + r) as usize) * colors.len() / (2 * r + 1) as usize)
                        .min(colors.len() - 1);
                    c.pixel(x + px, y + py, colors[band]);
                }
            }
        }
    }
    c.circle(x, y, 5, C_TEXT);
}
fn draw_vertex_label(c: &mut Canvas8<'_>, x: i32, y: i32, id: crate::graph::VertexId) {
    c.text(x + 7, y - 3, "B", C_MUTED);
    let mut buf = [0u8; 3];
    let n = if id >= 100 {
        buf[0] = b'0' + (id / 100) as u8;
        buf[1] = b'0' + ((id / 10) % 10) as u8;
        buf[2] = b'0' + (id % 10) as u8;
        3
    } else if id >= 10 {
        buf[0] = b'0' + (id / 10) as u8;
        buf[1] = b'0' + (id % 10) as u8;
        2
    } else {
        buf[0] = b'0' + id as u8;
        1
    };
    if let Ok(t) = core::str::from_utf8(&buf[..n]) {
        c.text(x + 13, y - 3, t, C_MUTED);
    }
}

fn notice_text(n: EditorNotice) -> &'static str {
    match n {
        EditorNotice::Ready => "READY",
        EditorNotice::Saved => "SAVED",
        EditorNotice::Loaded => "LOADED",
        EditorNotice::SaveError => "SAVE ERR",
        EditorNotice::LoadError => "LOAD ERR",
        EditorNotice::Applied => "APPLIED",
        EditorNotice::Cleared => "CLEARED",
        EditorNotice::NeedSource => "SET SOURCE",
        EditorNotice::NeedTarget => "SET TARGET",
    }
}
fn tool_text(ui: &UiState) -> &'static str {
    match ui.tool {
        ActiveTool::Vertex(VertexTool::SelectMove) => "SELECT",
        ActiveTool::Vertex(VertexTool::Pan) => "PAN",
        ActiveTool::Vertex(VertexTool::Add) => "ADD VTX",
        ActiveTool::Vertex(VertexTool::Delete) => "DEL VTX",
        ActiveTool::Vertex(VertexTool::SetSource) => "SOURCE",
        ActiveTool::Vertex(VertexTool::SetTarget) => "TARGET",
        ActiveTool::Edge(EdgeTool::Directed) => "EDGE DIR",
        ActiveTool::Edge(EdgeTool::Undirected) => "EDGE UND",
        ActiveTool::Edge(EdgeTool::Delete) => "DEL EDGE",
        ActiveTool::Edge(EdgeTool::Reverse) => "REV EDGE",
    }
}
fn zoom_text(z: u8) -> &'static str {
    match z {
        0 => "67%",
        1 => "80%",
        2 => "100%",
        3 => "125%",
        4 => "150%",
        5 => "200%",
        6 => "300%",
        7 => "400%",
        8 => "600%",
        9 => "800%",
        10 => "1200%",
        11 => "1600%",
        12 => "2400%",
        13 => "3200%",
        14 => "6400%",
        _ => "12800%",
    }
}

fn draw_dag_transform_bottom(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    analysis: &AnalysisState,
    ui: &UiState,
) -> bool {
    let Some(result) = analysis.result.as_ref() else {
        return false;
    };
    let Some(t) = result.dag.as_ref() else {
        return false;
    };
    draw_grid(c, ui);
    if t.kind == crate::dag::DagTransformKind::SccCondensation {
        let mut centers: Vec<(i16, i16)> = Vec::with_capacity(t.components.len());
        for comp in &t.components {
            let mut sx = 0i32;
            let mut sy = 0i32;
            let mut n = 0i32;
            for &id in &comp.members {
                if let Some(v) = graph.vertex(id) {
                    sx += i32::from(v.x);
                    sy += i32::from(v.y);
                    n += 1;
                }
            }
            centers.push(if n == 0 {
                (128, 88)
            } else {
                ((sx / n) as i16, (sy / n) as i16)
            });
        }
        for arc in &t.condensation_arcs {
            let a = arc.from_component as usize;
            let b = arc.to_component as usize;
            if a >= centers.len() || b >= centers.len() {
                continue;
            }
            let (x0, y0) = ui.result_world_to_screen(centers[a].0, centers[a].1);
            let (x1, y1) = ui.result_world_to_screen(centers[b].0, centers[b].1);
            let (a0, b0, a1, b1) = clipped_segment(
                i32::from(x0),
                i32::from(y0),
                i32::from(x1),
                i32::from(y1),
                10,
                11,
            );
            c.arrow(a0, b0, a1, b1, C_EDGE);
        }
        for (i, _comp) in t.components.iter().enumerate() {
            if i >= centers.len() {
                break;
            }
            let (x, y) = ui.result_world_to_screen(centers[i].0, centers[i].1);
            let (x, y) = (i32::from(x), i32::from(y));
            c.fill_circle(x, y, 8, group_color(i));
            c.circle(x, y, 9, C_TEXT);
            c.text(x - 5, y - 3, "C", C_TEXT);
        }
        return true;
    }
    for arc in &t.removed_arcs {
        let (Some(a), Some(b)) = (graph.vertex(arc.from), graph.vertex(arc.to)) else {
            continue;
        };
        let (x0, y0) = ui.result_world_to_screen(a.x, a.y);
        let (x1, y1) = ui.result_world_to_screen(b.x, b.y);
        c.dashed_line(
            i32::from(x0),
            i32::from(y0),
            i32::from(x1),
            i32::from(y1),
            C_DANGER,
        );
    }
    for arc in &t.kept_arcs {
        let (Some(a), Some(b)) = (graph.vertex(arc.from), graph.vertex(arc.to)) else {
            continue;
        };
        let (x0, y0) = ui.result_world_to_screen(a.x, a.y);
        let (x1, y1) = ui.result_world_to_screen(b.x, b.y);
        let (a0, b0, a1, b1) = clipped_segment(
            i32::from(x0),
            i32::from(y0),
            i32::from(x1),
            i32::from(y1),
            6,
            9,
        );
        c.arrow(a0, b0, a1, b1, C_RESULT);
    }
    for v in &graph.vertices {
        let (x, y) = ui.result_world_to_screen(v.x, v.y);
        let (x, y) = (i32::from(x), i32::from(y));
        if t.removed_vertices.contains(&v.id) {
            c.line(x - 5, y - 5, x + 5, y + 5, C_DANGER);
            c.line(x - 5, y + 5, x + 5, y - 5, C_DANGER);
            c.circle(x, y, 6, C_DANGER);
        } else {
            draw_multicolor_vertex(c, x, y, &[C_RESULT]);
        }
        draw_vertex_label(c, x, y, v.id);
    }
    true
}

fn is_component_quotient(analysis: &AnalysisState) -> bool {
    matches!(
        analysis.component,
        Some(
            crate::analysis::ComponentKind::Weak
                | crate::analysis::ComponentKind::Strong
                | crate::analysis::ComponentKind::Unilateral
        )
    ) && analysis
        .result
        .as_ref()
        .is_some_and(|r| !r.groups.is_empty())
}
fn quotient_centroids(graph: &Digraph, analysis: &AnalysisState) -> Vec<(i16, i16)> {
    let Some(result) = analysis.result.as_ref() else {
        return Vec::new();
    };
    result
        .groups
        .iter()
        .map(|g| {
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
                (128, 88)
            } else {
                ((sx / n) as i16, (sy / n) as i16)
            }
        })
        .collect()
}
fn group_memberships(
    result: &crate::analysis::AnalysisResult,
    id: crate::graph::VertexId,
    out: &mut [usize; 8],
) -> usize {
    let mut n = 0usize;
    for (gi, g) in result.groups.iter().enumerate() {
        if g.vertices.contains(&id) && n < out.len() {
            out[n] = gi;
            n += 1;
        }
    }
    n
}
fn draw_component_quotient_bottom(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    draw_grid(c, ui);
    let Some(result) = analysis.result.as_ref() else {
        return;
    };
    let k = result.groups.len();
    if k == 0 {
        return;
    }
    let world = quotient_centroids(graph, analysis);
    let mut arcs = vec![vec![false; k]; k];
    for e in &graph.edges {
        let mut aa = [0usize; 8];
        let mut bb = [0usize; 8];
        let na = group_memberships(result, e.from, &mut aa);
        let nb = group_memberships(result, e.to, &mut bb);
        for &a in &aa[..na] {
            for &b in &bb[..nb] {
                if a == b {
                    continue;
                }
                arcs[a][b] = true;
                if !e.directed {
                    arcs[b][a] = true;
                }
            }
        }
    }
    for a in 0..k {
        for b in (a + 1)..k {
            let ab = arcs[a][b];
            let ba = arcs[b][a];
            if !ab && !ba {
                continue;
            }
            let (x0, y0) = ui.result_world_to_screen(world[a].0, world[a].1);
            let (x1, y1) = ui.result_world_to_screen(world[b].0, world[b].1);
            let (a0, b0, a1, b1) = clipped_segment(
                i32::from(x0),
                i32::from(y0),
                i32::from(x1),
                i32::from(y1),
                10,
                11,
            );
            if ab && ba {
                c.line(a0, b0, a1, b1, C_EDGE)
            } else if ab {
                c.arrow(a0, b0, a1, b1, C_EDGE)
            } else {
                c.arrow(a1, b1, a0, b0, C_EDGE)
            }
        }
    }
    for (i, g) in result.groups.iter().enumerate() {
        let (x, y) = ui.result_world_to_screen(world[i].0, world[i].1);
        let x = i32::from(x);
        let y = i32::from(y);
        let col = group_color(i);
        c.fill_circle(x, y, 8, col);
        c.circle(x, y, 9, C_TEXT);
        if editor.result_drag_key == Some(EditorState::result_key(&g.vertices)) {
            c.circle(x, y, 10, C_SELECTED);
        }
        c.text(x - 5, y - 3, "C", C_TEXT);
        let mut b = [0u8; 3];
        let n = i as u16;
        let len = if n >= 100 {
            b[0] = b'0' + (n / 100) as u8;
            b[1] = b'0' + ((n / 10) % 10) as u8;
            b[2] = b'0' + (n % 10) as u8;
            3
        } else if n >= 10 {
            b[0] = b'0' + (n / 10) as u8;
            b[1] = b'0' + (n % 10) as u8;
            2
        } else {
            b[0] = b'0' + n as u8;
            1
        };
        if let Ok(t) = core::str::from_utf8(&b[..len]) {
            c.text(x + 1, y - 3, t, C_TEXT);
        }
        let m = g.vertices.len() as u16;
        let mut mb = [0u8; 3];
        let ml = if m >= 100 {
            mb[0] = b'0' + (m / 100) as u8;
            mb[1] = b'0' + ((m / 10) % 10) as u8;
            mb[2] = b'0' + (m % 10) as u8;
            3
        } else if m >= 10 {
            mb[0] = b'0' + (m / 10) as u8;
            mb[1] = b'0' + (m % 10) as u8;
            2
        } else {
            mb[0] = b'0' + m as u8;
            1
        };
        c.text(x - 7, y + 12, "N", C_MUTED);
        if let Ok(t) = core::str::from_utf8(&mb[..ml]) {
            c.text(x - 1, y + 12, t, C_MUTED);
        }
    }
}
fn draw_graph(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    draw_grid(c, ui);
    let fast = graph.edges.len() > 2500 && (editor.dragging || editor.pan_last.is_some());
    let stride = dense_edge_stride(graph, ui);
    if let Some(result) = analysis.result.as_ref() {
        let base_edge = if ui.result_on_bottom { C_MUTED } else { C_EDGE };
        for (ei, e) in graph.edges.iter().enumerate() {
            let mut colors = [0u8; 8];
            let n = group_colors_edge(result, e.id, &mut colors);
            if n > 0 {
                draw_grouped_edge(c, graph, e, ui, &colors[..n]);
            } else if !fast && ei % stride == 0 {
                draw_edge_color(c, graph, e, ui, base_edge);
            }
        }
        // Always draw semantically important result edges, even when the dense
        // context layer is sampled or hidden during interaction.
        if result.groups.is_empty() {
            for e in &graph.edges {
                let primary = result.primary_edges.contains(&e.id);
                let secondary = result.secondary_edges.contains(&e.id);
                if primary && secondary {
                    draw_grouped_edge(c, graph, e, ui, &[C_RESULT, C_DANGER]);
                } else if primary {
                    draw_edge_color(c, graph, e, ui, C_RESULT);
                } else if secondary {
                    draw_edge_color(c, graph, e, ui, C_DANGER);
                }
            }
        } else {
            for &eid in &result.secondary_edges {
                if let Some(e) = graph.edge(eid) {
                    draw_edge_color(c, graph, e, ui, C_DANGER)
                }
            }
        }
    } else if !fast {
        for (ei, e) in graph.edges.iter().enumerate() {
            if ei % stride == 0 {
                draw_edge(c, graph, e, ui);
            }
        }
    }
    let reach = analysis.selected == Some(AnalysisKind::Reachability);
    let show_labels = !fast && (graph.vertices.len() <= 80 || current_zoom(ui) >= 5);
    for v in &graph.vertices {
        let (sx, sy) = ui.bottom_world_to_screen(v.x, v.y);
        let x = i32::from(sx);
        let y = i32::from(sy);
        if x < -10 || x > TOP_WIDTH as i32 + 10 || !(-10..=181).contains(&y) {
            continue;
        }
        let mut colors = [0u8; 8];
        let gn = analysis
            .result
            .as_ref()
            .map(|r| group_colors_vertex(r, v.id, &mut colors))
            .unwrap_or(0);
        if analysis.source == Some(v.id) {
            draw_multicolor_vertex(c, x, y, &[C_SOURCE]);
        } else if analysis.target == Some(v.id) {
            draw_multicolor_vertex(c, x, y, &[C_TARGET]);
        } else if gn > 0 {
            draw_multicolor_vertex(c, x, y, &colors[..gn]);
        } else if analysis.result.as_ref().is_some_and(|r| {
            r.primary_vertices.contains(&v.id) && r.secondary_vertices.contains(&v.id)
        }) {
            draw_multicolor_vertex(c, x, y, &[C_RESULT, C_RESULT2]);
        } else {
            let color = if reach
                && analysis
                    .result
                    .as_ref()
                    .is_some_and(|r| r.secondary_vertices.contains(&v.id))
            {
                C_DANGER
            } else if analysis
                .result
                .as_ref()
                .is_some_and(|r| r.primary_vertices.contains(&v.id))
            {
                C_RESULT
            } else if analysis
                .result
                .as_ref()
                .is_some_and(|r| r.secondary_vertices.contains(&v.id))
            {
                C_RESULT2
            } else {
                C_VERTEX
            };
            draw_multicolor_vertex(c, x, y, &[color]);
        }
        if editor.edge_start == Some(v.id) {
            c.circle(x, y, 7, C_SOURCE);
        }
        if editor.selected == Some(v.id) {
            c.circle(x, y, 6, C_SELECTED);
        }
        if show_labels {
            draw_vertex_label(c, x, y, v.id);
        }
    }
}
fn draw_tab(c: &mut Canvas8<'_>, x0: i32, x1: i32, label: &str, active: bool) {
    if active {
        fill_rect(c, x0, TAB_TOP, x1, TAB_BOTTOM, C_TAB)
    }
    rect(
        c,
        x0,
        TAB_TOP,
        x1,
        TAB_BOTTOM,
        if active { C_ACCENT } else { C_MUTED },
    );
    c.text(
        x0 + 2,
        TAB_TOP + 4,
        label,
        if active { C_TEXT } else { C_MUTED },
    );
}
fn draw_menu(c: &mut Canvas8<'_>, ui: &UiState) {
    tile_panel(c, 0, MENU_TOP, 255, 180);
    draw_tab(c, 0, 50, "VERT", ui.category == MenuCategory::Vertex);
    draw_tab(c, 51, 101, "EDGE", ui.category == MenuCategory::Edge);
    draw_tab(c, 102, 152, "ALGO", ui.category == MenuCategory::Algorithms);
    draw_tab(c, 153, 203, "COMP", ui.category == MenuCategory::Components);
    draw_tab(c, 204, 255, "GRAPH", ui.category == MenuCategory::Graphs);
    let start = ui.page_start();
    for local in 0..ITEMS_PER_PAGE {
        let idx = start + local;
        if idx >= ui.category_len() {
            break;
        }
        let col = local % 3;
        let row = local / 3;
        let x0 = (col as i32) * 85 + 2;
        let x1 = x0 + 80;
        let y0 = ITEM_TOP + (row as i32) * ITEM_H;
        let y1 = y0 + 19;
        let active = idx == ui.cursor;
        if active {
            fill_rect(c, x0, y0, x1, y1, C_TAB)
        }
        rect(c, x0, y0, x1, y1, if active { C_ACCENT } else { C_GRID });
        if let Some(label) = ui.item_label(idx) {
            c.text(x0 + 3, y0 + 6, label, if active { C_TEXT } else { C_MUTED });
        }
    }
    let pages = ui.category_len().div_ceil(ITEMS_PER_PAGE);
    let page = ui.page() + 1;
    c.text(3, 153, "<", C_MUTED);
    c.text(246, 153, ">", C_MUTED);
    if pages > 1 {
        let mut b = *b"0/0";
        b[0] = b'0' + page.min(9) as u8;
        b[2] = b'0' + pages.min(9) as u8;
        if let Ok(t) = core::str::from_utf8(&b) {
            c.text(116, 153, t, C_MUTED)
        }
    }
    for (i, (a, b, label)) in [(0, 83, "THEME"), (84, 169, "SAVE"), (170, 255, "LOAD")]
        .into_iter()
        .enumerate()
    {
        let color = if i == 0 { C_ACCENT } else { C_MUTED };
        rect(c, a, ACTION_TOP, b, ACTION_BOTTOM, color);
        c.text(a + 10, ACTION_TOP + 6, label, C_TEXT)
    }
}
fn draw_center_value(c: &mut Canvas8<'_>, y: i32, label: &str, value: u8) {
    c.text(76, y, label, C_MUTED);
    let mut b = [0u8; 3];
    let n = if value >= 100 {
        b[0] = b'0' + value / 100;
        b[1] = b'0' + (value / 10) % 10;
        b[2] = b'0' + value % 10;
        3
    } else if value >= 10 {
        b[0] = b'0' + value / 10;
        b[1] = b'0' + value % 10;
        2
    } else {
        b[0] = b'0' + value;
        1
    };
    if let Ok(t) = core::str::from_utf8(&b[..n]) {
        c.text(144, y, t, C_TEXT)
    }
}
fn draw_meter(c: &mut Canvas8<'_>, x0: i32, y0: i32, w: i32, value_tenths: u16, color: u8) {
    rect(c, x0, y0, x0 + w, y0 + 5, C_GRID);
    let inner = (i32::from(value_tenths.min(1000)) * w.saturating_sub(2) / 1000).max(0);
    if inner > 0 {
        fill_rect(c, x0 + 1, y0 + 1, x0 + inner, y0 + 4, color);
    }
}
fn analysis_label(analysis: &AnalysisState) -> &'static str {
    if let Some(k) = analysis.selected {
        k.code()
    } else if let Some(k) = analysis.component {
        k.code()
    } else {
        "NONE"
    }
}
fn draw_stats_dialog(
    c: &mut Canvas8<'_>,
    stats: &StatsState,
    graph: &Digraph,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    c.text(105, 87, "STATS", C_TEXT);
    let cpu = format!(
        "ARM9 LOAD~ {:>3}.{}%",
        stats.cpu_load_tenths / 10,
        stats.cpu_load_tenths % 10
    );
    c.text(15, 99, &cpu, C_TEXT);
    draw_meter(c, 15, 109, 226, stats.cpu_load_tenths, C_ACCENT);
    let fps = format!(
        "FPS {:>3}.{}   WORK {}.{:03} ms",
        stats.fps_tenths / 10,
        stats.fps_tenths % 10,
        stats.frame_work_us / 1000,
        stats.frame_work_us % 1000
    );
    c.text(15, 118, &fps, C_MUTED);
    let used_k = stats.ram_used_bytes / 1024;
    let free_k = stats.ram_free_bytes / 1024;
    let total = stats.ram_used_bytes.saturating_add(stats.ram_free_bytes);
    let ram_pct = if total == 0 {
        0
    } else {
        ((u64::from(stats.ram_used_bytes) * 1000) / u64::from(total)).min(1000) as u16
    };
    let ram = format!("RAM {}K LIVE  {}K FREE", used_k, free_k);
    c.text(15, 129, &ram, C_TEXT);
    draw_meter(c, 15, 139, 226, ram_pct, C_RESULT);
    let heap = format!(
        "HEAP {}K LIVE  {}K SPAN",
        stats.heap_used_bytes / 1024,
        stats.heap_span_bytes / 1024
    );
    c.text(15, 148, &heap, C_MUTED);
    let graph_line = format!(
        "V{} E{}  {}",
        graph.vertices.len(),
        graph.edges.len(),
        analysis_label(analysis)
    );
    c.text(15, 158, &graph_line, C_TEXT);
    let mode = format!(
        "{}  {}  Z{}",
        if stats.dsi_mode { "DSI" } else { "NTR" },
        if ui.result_on_bottom {
            "RESULT"
        } else {
            "EDIT"
        },
        zoom_text(if ui.result_on_bottom {
            ui.result_zoom
        } else {
            ui.zoom
        })
    );
    c.text(15, 168, &mode, C_MUTED);
    rect(c, 170, 162, 255, 180, C_ACCENT);
    c.text(194, 168, "BACK", C_TEXT);
}
fn draw_graph_dialog(
    c: &mut Canvas8<'_>,
    library: &GraphMenuState,
    stats: &StatsState,
    graph: &Digraph,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    tile_panel(c, 0, MENU_TOP, 255, 180);
    match library.dialog {
        GraphDialog::None => {}
        GraphDialog::CompleteOrder => {
            c.text(74, 86, "COMPLETE GRAPH", C_TEXT);
            rect(c, 24, 95, 232, 109, C_ACCENT);
            let value = format!("ORDER {}", library.order_edit);
            c.text(88, 100, &value, C_TEXT);
            let keys = [
                ("1", 24, 112),
                ("2", 97, 112),
                ("3", 170, 112),
                ("4", 24, 128),
                ("5", 97, 128),
                ("6", 170, 128),
                ("7", 24, 144),
                ("8", 97, 144),
                ("9", 170, 144),
            ];
            for (label, x0, y0) in keys {
                rect(c, x0, y0, x0 + 62, y0 + 14, C_GRID);
                c.text(x0 + 28, y0 + 5, label, C_TEXT);
            }
            rect(c, 24, 160, 86, 178, C_MUTED);
            c.text(40, 166, "DEL", C_TEXT);
            rect(c, 97, 160, 159, 178, C_GRID);
            c.text(125, 166, "0", C_TEXT);
            rect(c, 170, 160, 232, 178, C_ACCENT);
            c.text(190, 166, "CREATE", C_TEXT);
        }
        GraphDialog::RandomSettings => {
            c.text(72, 87, "RANDOM GRAPH", C_TEXT);
            rect(c, 20, 102, 65, 124, C_MUTED);
            c.text(39, 110, "-", C_TEXT);
            rect(c, 190, 102, 235, 124, C_MUTED);
            c.text(209, 110, "+", C_TEXT);
            draw_center_value(c, 110, "ORDER", library.order);
            rect(c, 20, 128, 65, 150, C_MUTED);
            c.text(39, 136, "-", C_TEXT);
            rect(c, 190, 128, 235, 150, C_MUTED);
            c.text(209, 136, "+", C_TEXT);
            draw_center_value(c, 136, "DENS %", library.density);
            rect(c, 20, 155, 120, 178, C_MUTED);
            c.text(49, 163, "CANCEL", C_TEXT);
            rect(c, 136, 155, 236, 178, C_ACCENT);
            c.text(165, 163, "CREATE", C_TEXT);
        }
        GraphDialog::SaveName => {
            c.text(82, 87, "SAVE AS", C_TEXT);
            rect(c, 12, 88, 243, 101, C_GRID);
            if library.save_name.is_empty() {
                c.text(18, 92, "TOUCH NAME...", C_MUTED)
            } else {
                c.text(18, 92, &library.save_name, C_TEXT)
            }
            for (i, &ch) in NAME_KEYS.iter().enumerate() {
                let col = i % 10;
                let row = i / 10;
                let x0 = (col as i32) * 25 + 2;
                let y0 = 104 + (row as i32) * 14;
                rect(c, x0, y0, x0 + 21, y0 + 11, C_GRID);
                let b = [ch];
                if let Ok(t) = core::str::from_utf8(&b) {
                    c.text(x0 + 8, y0 + 3, t, C_TEXT)
                }
            }
            rect(c, 0, 162, 83, 180, C_MUTED);
            c.text(27, 168, "DEL", C_TEXT);
            rect(c, 84, 162, 169, 180, C_ACCENT);
            c.text(109, 168, "SAVE", C_TEXT);
            rect(c, 170, 162, 255, 180, C_MUTED);
            c.text(190, 168, "CANCEL", C_TEXT);
        }
        GraphDialog::LoadList => {
            c.text(78, 87, "SAVED GRAPHS", C_TEXT);
            let start = library.saved_page * 5;
            if library.saved_names.is_empty() {
                c.text(77, 120, "NO SAVES", C_MUTED)
            } else {
                for row in 0..5 {
                    let idx = start + row;
                    if let Some(name) = library.saved_names.get(idx) {
                        let y = 96 + (row as i32) * 13;
                        if idx == library.saved_cursor {
                            fill_rect(c, 16, y, 239, y + 11, C_TAB)
                        }
                        rect(
                            c,
                            16,
                            y,
                            239,
                            y + 11,
                            if idx == library.saved_cursor {
                                C_ACCENT
                            } else {
                                C_GRID
                            },
                        );
                        c.text(
                            22,
                            y + 3,
                            name,
                            if idx == library.saved_cursor {
                                C_TEXT
                            } else {
                                C_MUTED
                            },
                        );
                    }
                }
            }
            rect(c, 0, 162, 83, 180, C_MUTED);
            c.text(28, 168, "PREV", C_TEXT);
            rect(c, 84, 162, 169, 180, C_MUTED);
            c.text(112, 168, "NEXT", C_TEXT);
            rect(c, 170, 162, 255, 180, C_ACCENT);
            c.text(194, 168, "BACK", C_TEXT);
        }
        GraphDialog::Stats => draw_stats_dialog(c, stats, graph, analysis, ui),
    }
}
fn render_base_into(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    c.clear(C_BG);
    if ui.result_on_bottom && draw_dag_transform_bottom(c, graph, analysis, ui) {
    } else if ui.result_on_bottom && is_component_quotient(analysis) {
        draw_component_quotient_bottom(c, graph, editor, analysis, ui);
    } else {
        draw_graph(c, graph, editor, analysis, ui);
    }
}
fn render_overlay_into(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
    library: &GraphMenuState,
    stats: &StatsState,
) {
    if library.dialog != GraphDialog::None {
        draw_graph_dialog(c, library, stats, graph, analysis, ui);
    } else if ui.menu_open {
        draw_menu(c, ui);
    }
    c.hline(0, 255, 181, C_MUTED);
    if library.dialog != GraphDialog::None {
        c.text(2, 183, "A OK", C_TEXT);
        c.text(43, 183, "B BACK", C_TEXT);
        c.text(104, 183, "TOUCH", C_MUTED);
    } else if ui.menu_open {
        c.text(2, 183, "A OK", C_TEXT);
        c.text(43, 183, "B BACK", C_TEXT);
        c.text(105, 183, "< > MOVE", C_MUTED);
        c.text(190, 183, "D CLOSE", C_TEXT);
    } else {
        c.text(2, 183, tool_text(ui), C_TEXT);
        c.text(70, 183, notice_text(editor.notice), C_MUTED);
        let z = if ui.result_on_bottom {
            ui.result_zoom
        } else {
            ui.zoom
        };
        c.text(
            142,
            183,
            if ui.result_on_bottom {
                "RESULT"
            } else {
                "EDIT"
            },
            C_MUTED,
        );
        c.text(178, 183, zoom_text(z), C_TEXT);
        c.text(226, 183, "D", C_TEXT);
    }
}
pub fn render_into(
    c: &mut Canvas8<'_>,
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
    library: &GraphMenuState,
    stats: &StatsState,
) {
    render_base_into(c, graph, editor, analysis, ui);
    render_overlay_into(c, graph, editor, analysis, ui, library, stats);
}

fn mix(mut h: u32, x: u32) -> u32 {
    h ^= x;
    h.wrapping_mul(16_777_619)
}
/// O(1) cache key for the expensive graph layer. `visual_revision` changes on
/// vertex motion while `topology_revision` changes on graph edits. Analysis
/// results are deterministic from topology/source/target, so we do not hash
/// all 20k edges or every result group on every frame.
fn base_signature(
    graph: &Digraph,
    editor: &EditorState,
    analysis: &AnalysisState,
    ui: &UiState,
) -> u32 {
    let mut h = 2_166_136_261;
    h = mix(h, graph.topology_revision);
    h = mix(h, graph.visual_revision);
    h = mix(h, editor.selected.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, editor.edge_start.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, editor.result_drag_key.unwrap_or(u32::MAX));
    h = mix(h, editor.dragging as u32);
    h = mix(h, editor.pan_last.is_some() as u32);
    h = mix(h, analysis.selected.map(|k| k as u32 + 1).unwrap_or(0));
    h = mix(h, analysis.component.map(|k| k as u32 + 101).unwrap_or(0));
    h = mix(h, analysis.source.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, analysis.target.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, analysis.result.is_some() as u32);
    h = mix(h, ui.zoom as u32);
    h = mix(h, ui.result_zoom as u32);
    h = mix(h, ui.result_on_bottom as u32);
    h = mix(h, ui.pan_x as u16 as u32);
    h = mix(h, ui.pan_y as u16 as u32);
    h = mix(h, ui.result_pan_x as u16 as u32);
    h = mix(h, ui.result_pan_y as u16 as u32);
    h
}
fn overlay_signature(
    base: u32,
    editor: &EditorState,
    ui: &UiState,
    library: &GraphMenuState,
    stats: &StatsState,
) -> u32 {
    let mut h = base;
    h = mix(h, editor.notice as u32);
    h = mix(h, ui.menu_open as u32);
    h = mix(h, ui.category as u32);
    h = mix(h, ui.cursor as u32);
    h = mix(h, ui.theme as u32);
    h = mix(h, library.dialog as u32);
    h = mix(h, u32::from(library.order));
    h = mix(h, u32::from(library.order_edit));
    h = mix(h, library.order_edit_fresh as u32);
    h = mix(h, u32::from(library.density));
    h = mix(h, library.save_name.len() as u32);
    h = mix(h, library.saved_cursor as u32);
    h = mix(h, library.saved_page as u32);
    for b in tool_text(ui).bytes() {
        h = mix(h, u32::from(b));
    }
    for b in library.save_name.bytes() {
        h = mix(h, u32::from(b));
    }
    for name in &library.saved_names {
        for b in name.bytes() {
            h = mix(h, u32::from(b));
        }
    }
    if library.dialog == GraphDialog::Stats {
        h = mix(h, stats.sample_seq);
    }
    h
}

fn render_bottom_view(
    graph: Res<Digraph>,
    editor: Res<EditorState>,
    analysis: Res<AnalysisState>,
    ui: Res<UiState>,
    library: Res<GraphMenuState>,
    stats: Res<StatsState>,
    mut state: ResMut<BottomViewState>,
) {
    #[cfg(target_vendor = "nintendo")]
    {
        if !state.initialized {
            init_sub_bitmap_bottom(ui.theme);
            clear_8bpp_vram(SUB_BG_VRAM as *mut u16);
            state.initialized = true;
            state.last_signature = u32::MAX;
            state.last_base_signature = u32::MAX;
            state.last_theme = Some(ui.theme);
        }
        if state.last_theme != Some(ui.theme) {
            upload_palette(ui.theme);
            state.last_theme = Some(ui.theme);
            state.last_signature = u32::MAX;
        }
        let base = base_signature(&graph, &editor, &analysis, &ui);
        let sig = overlay_signature(base, &editor, &ui, &library, &stats);
        if sig == state.last_signature {
            return;
        }
        if base != state.last_base_signature {
            {
                let mut canvas = Canvas8::new(&mut state.base_frame);
                render_base_into(&mut canvas, &graph, &editor, &analysis, &ui);
            }
            state.last_base_signature = base;
        }
        {
            let state_ref: &mut BottomViewState = &mut state;
            let BottomViewState {
                frame, base_frame, ..
            } = state_ref;
            frame.copy_from_slice(base_frame);
        }
        {
            let mut canvas = Canvas8::new(&mut state.frame);
            render_overlay_into(
                &mut canvas,
                &graph,
                &editor,
                &analysis,
                &ui,
                &library,
                &stats,
            );
        }
        upload_8bpp_vram(SUB_BG_VRAM as *mut u16, &state.frame);
        state.last_signature = sig;
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        let _ = (
            &graph, &editor, &analysis, &ui, &library, &stats, &mut state,
        );
    }
}

#[cfg(target_vendor = "nintendo")]
fn clear_8bpp_vram(dst: *mut u16) {
    unsafe {
        for i in 0..(256 * 256 / 2) {
            core::ptr::write_volatile(dst.add(i), 0)
        }
    }
}
#[cfg(target_vendor = "nintendo")]
fn upload_8bpp_vram(dst: *mut u16, src: &[u8]) {
    unsafe {
        for i in 0..(src.len() / 2) {
            let lo = src[i * 2] as u16;
            let hi = (src[i * 2 + 1] as u16) << 8;
            core::ptr::write_volatile(dst.add(i), lo | hi)
        }
    }
}

#[cfg(target_vendor = "nintendo")]
const REG_DISPCNT_SUB: *mut u32 = 0x0400_1000 as *mut u32;
#[cfg(target_vendor = "nintendo")]
const REG_VRAMCNT_C: *mut u8 = 0x0400_0242 as *mut u8;
#[cfg(target_vendor = "nintendo")]
const SUB_BG_VRAM: usize = 0x0620_0000;
#[cfg(target_vendor = "nintendo")]
const SUB_BG_PALETTE: *mut u16 = 0x0500_0400 as *mut u16;
#[cfg(target_vendor = "nintendo")]
const MODE_5_2D: u32 = 0x0001_0005;
#[cfg(target_vendor = "nintendo")]
const VRAM_ENABLE: u8 = 0x80;
#[cfg(target_vendor = "nintendo")]
const VRAM_C_SUB_BG: u8 = 4;
#[cfg(target_vendor = "nintendo")]
const REG_BG3CNT_SUB: *mut u16 = 0x0400_100E as *mut u16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PA_SUB: *mut i16 = 0x0400_1030 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PB_SUB: *mut i16 = 0x0400_1032 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PC_SUB: *mut i16 = 0x0400_1034 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PD_SUB: *mut i16 = 0x0400_1036 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3X_SUB: *mut i32 = 0x0400_1038 as *mut i32;
#[cfg(target_vendor = "nintendo")]
const REG_BG3Y_SUB: *mut i32 = 0x0400_103C as *mut i32;
#[cfg(target_vendor = "nintendo")]
const DISPLAY_BG3_ACTIVE: u32 = 1 << 11;
#[cfg(target_vendor = "nintendo")]
const BG_BMP8_256X256: u16 = (1 << 14) | (1 << 7);
#[cfg(target_vendor = "nintendo")]
const fn rgb5(r: u16, g: u16, b: u16) -> u16 {
    (r & 31) | ((g & 31) << 5) | ((b & 31) << 10)
}
#[cfg(target_vendor = "nintendo")]
fn palette(theme: Theme) -> [u16; 23] {
    match theme {
        Theme::Midnight => [
            rgb5(2, 3, 5),
            rgb5(4, 6, 8),
            rgb5(13, 21, 29),
            rgb5(7, 20, 31),
            rgb5(31, 25, 4),
            rgb5(8, 31, 15),
            rgb5(31, 31, 31),
            rgb5(13, 13, 15),
            rgb5(3, 5, 8),
            rgb5(5, 9, 14),
            rgb5(5, 24, 31),
            rgb5(31, 7, 7),
            rgb5(5, 18, 31),
            rgb5(15, 28, 31),
            rgb5(22, 11, 31),
            rgb5(31, 18, 5),
            rgb5(25, 8, 31),
            rgb5(4, 26, 28),
            rgb5(31, 10, 22),
            rgb5(19, 31, 7),
            rgb5(8, 14, 31),
            rgb5(31, 28, 7),
            rgb5(18, 10, 31),
        ],
        Theme::Graphite => [
            rgb5(3, 3, 4),
            rgb5(7, 7, 8),
            rgb5(20, 20, 22),
            rgb5(18, 18, 20),
            rgb5(31, 26, 8),
            rgb5(13, 31, 19),
            rgb5(31, 31, 31),
            rgb5(15, 15, 17),
            rgb5(5, 5, 6),
            rgb5(9, 9, 11),
            rgb5(25, 25, 28),
            rgb5(31, 8, 8),
            rgb5(8, 16, 31),
            rgb5(24, 24, 31),
            rgb5(19, 19, 23),
            rgb5(31, 19, 7),
            rgb5(25, 12, 31),
            rgb5(8, 25, 27),
            rgb5(31, 12, 23),
            rgb5(20, 29, 10),
            rgb5(10, 17, 31),
            rgb5(31, 28, 10),
            rgb5(20, 13, 31),
        ],
        Theme::Terminal => [
            rgb5(0, 2, 0),
            rgb5(1, 6, 2),
            rgb5(5, 20, 8),
            rgb5(5, 25, 9),
            rgb5(31, 26, 3),
            rgb5(4, 31, 8),
            rgb5(16, 31, 17),
            rgb5(5, 15, 6),
            rgb5(0, 4, 1),
            rgb5(2, 10, 3),
            rgb5(8, 31, 12),
            rgb5(31, 8, 6),
            rgb5(5, 22, 31),
            rgb5(15, 31, 15),
            rgb5(8, 24, 10),
            rgb5(31, 20, 4),
            rgb5(23, 10, 31),
            rgb5(6, 28, 28),
            rgb5(31, 9, 20),
            rgb5(18, 31, 6),
            rgb5(5, 17, 31),
            rgb5(31, 27, 5),
            rgb5(17, 9, 31),
        ],
        Theme::Paper => [
            rgb5(27, 27, 25),
            rgb5(22, 23, 22),
            rgb5(5, 8, 12),
            rgb5(7, 14, 21),
            rgb5(25, 16, 2),
            rgb5(3, 18, 7),
            rgb5(2, 3, 4),
            rgb5(10, 11, 11),
            rgb5(25, 25, 23),
            rgb5(21, 22, 22),
            rgb5(2, 15, 25),
            rgb5(25, 4, 4),
            rgb5(4, 10, 25),
            rgb5(3, 17, 20),
            rgb5(14, 5, 22),
            rgb5(25, 14, 3),
            rgb5(18, 6, 24),
            rgb5(2, 17, 20),
            rgb5(24, 6, 16),
            rgb5(12, 20, 3),
            rgb5(4, 9, 24),
            rgb5(24, 19, 3),
            rgb5(12, 5, 23),
        ],
    }
}
#[cfg(target_vendor = "nintendo")]
fn upload_palette(theme: Theme) {
    use core::ptr::write_volatile;
    let p = palette(theme);
    unsafe {
        for (i, c) in p.into_iter().enumerate() {
            write_volatile(SUB_BG_PALETTE.add(i), c)
        }
    }
}
#[cfg(target_vendor = "nintendo")]
fn init_sub_bitmap_bottom(theme: Theme) {
    use core::ptr::write_volatile;
    unsafe {
        write_volatile(REG_VRAMCNT_C, VRAM_ENABLE | VRAM_C_SUB_BG);
        write_volatile(REG_DISPCNT_SUB, MODE_5_2D | DISPLAY_BG3_ACTIVE);
        write_volatile(REG_BG3CNT_SUB, BG_BMP8_256X256);
        write_volatile(REG_BG3PA_SUB, 1 << 8);
        write_volatile(REG_BG3PB_SUB, 0);
        write_volatile(REG_BG3PC_SUB, 0);
        write_volatile(REG_BG3PD_SUB, 1 << 8);
        write_volatile(REG_BG3X_SUB, 0);
        write_volatile(REG_BG3Y_SUB, 0);
    }
    upload_palette(theme);
}
