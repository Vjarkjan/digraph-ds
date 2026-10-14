//! Independent top-screen 2D analysis renderer.
//!
//! Rendering uses an 8-bit main-RAM shadow buffer. Nintendo DS VRAM only
//! supports 16/32-bit writes, so the completed frame is uploaded as packed u16.
//!
//! Hardware ownership on Nintendo DS:
//! - VideoPlugin keeps its BG0 text console in VRAM A.
//! - This renderer never writes VRAM A. It maps VRAM B to main-BG slot 1.
//! - Main BG3 is a 16-bit 256x256 direct-colour bitmap at base 8 (0x06020000).
//! - BG3 priority is 3, so the BG0 console stays visible above it for panic/debug text.
//! - Only the visible 256x192 region is uploaded.
//!
//! The renderer is non-destructive: it reads `Digraph` and `AnalysisState`
//! and draws either the original graph or the selected DAG transform.

extern crate alloc;

use alloc::{vec, vec::Vec};
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;

use crate::{
    analysis::{AnalysisKind, AnalysisNoticeLevel, AnalysisState, ComponentKind},
    dag::{DagTransform, DagTransformKind},
    graph::{Digraph, Edge, Vertex, VertexId},
    ui::{Theme, UiState},
};

pub const TOP_WIDTH: usize = 256;
pub const TOP_HEIGHT: usize = 192;
const HEADER_H: i32 = 15;
const GRAPH_Y0: i32 = HEADER_H + 2;
const GRAPH_Y1: i32 = 132;
const PANEL_Y0: i32 = 136;
const PANEL_Y1: i32 = 190;

// Palette indexes used by the 8-bit background.
const C_BG: u8 = 0;
const C_GRID: u8 = 1;
const C_EDGE: u8 = 2;
const C_REMOVED: u8 = 3;
const C_VERTEX: u8 = 4;
const C_ACCENT: u8 = 5;
const C_TEXT: u8 = 6;
const C_MUTED: u8 = 7;
const C_SOURCE: u8 = 8;
const C_TARGET: u8 = 9;
const C_REACHABLE: u8 = 10;
const C_UNREACHABLE: u8 = 11;
const C_WARNING: u8 = 12;
const C_PANEL: u8 = 13;
const C_COMPONENT0: u8 = 16;

#[derive(Resource, Clone, Copy, Debug)]
pub struct TopViewSettings {
    /// Master switch for the second renderer.
    pub enabled: bool,
    /// Draw a faint 16-pixel grid behind the graph.
    pub grid: bool,
    /// For feedback/order transforms, draw removed arcs/vertices as dashed/X marks.
    pub show_removed: bool,
    /// If no DAG analysis is selected, show the original graph.
    pub show_original_when_idle: bool,
}

impl Default for TopViewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            grid: true,
            show_removed: true,
            show_original_when_idle: true,
        }
    }
}

#[derive(Resource, Debug)]
pub struct TopViewState {
    initialized: bool,
    last_signature: u32,
    frame: Vec<u8>,
}

impl Default for TopViewState {
    fn default() -> Self {
        Self {
            initialized: false,
            last_signature: u32::MAX,
            frame: vec![0; TOP_WIDTH * TOP_HEIGHT],
        }
    }
}

/// Add this after the graph/DAG resources are initialized.  The first render
/// happens in PostUpdate, after normal Startup video initialization has run.
pub struct TopViewPlugin;

impl Plugin for TopViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TopViewSettings>()
            .init_resource::<TopViewState>()
            .add_systems(PostUpdate, render_top_view);
    }
}

/// Small software canvas shared by the hardware renderer and host-side tests.
pub struct Canvas8<'a> {
    pixels: &'a mut [u8],
}

impl<'a> Canvas8<'a> {
    pub fn new(pixels: &'a mut [u8]) -> Self {
        Self { pixels }
    }

    #[inline]
    pub fn pixel(&mut self, x: i32, y: i32, color: u8) {
        if x < 0 || y < 0 || x >= TOP_WIDTH as i32 || y >= TOP_HEIGHT as i32 {
            return;
        }
        self.pixels[y as usize * TOP_WIDTH + x as usize] = color;
    }

    pub fn clear(&mut self, color: u8) {
        self.pixels[..TOP_WIDTH * TOP_HEIGHT].fill(color);
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, color: u8) {
        let a = x0.min(x1).max(0);
        let b = x0.max(x1).min(TOP_WIDTH as i32 - 1);
        if y < 0 || y >= TOP_HEIGHT as i32 || a > b {
            return;
        }
        for x in a..=b {
            self.pixel(x, y, color);
        }
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, color: u8) {
        let a = y0.min(y1).max(0);
        let b = y0.max(y1).min(TOP_HEIGHT as i32 - 1);
        if x < 0 || x >= TOP_WIDTH as i32 || a > b {
            return;
        }
        for y in a..=b {
            self.pixel(x, y, color);
        }
    }

    /// Cohen-Sutherland clipping before Bresenham. Dense, zoomed graphs often
    /// have endpoints thousands of pixels off-screen; clipping prevents the
    /// rasterizer from walking all of those invisible pixels.
    fn clip_line(
        mut x0: i32,
        mut y0: i32,
        mut x1: i32,
        mut y1: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        const LEFT: u8 = 1;
        const RIGHT: u8 = 2;
        const TOP: u8 = 4;
        const BOTTOM: u8 = 8;
        let xmax = TOP_WIDTH as i32 - 1;
        let ymax = TOP_HEIGHT as i32 - 1;
        let code = |x: i32, y: i32| -> u8 {
            let mut c = 0;
            if x < 0 {
                c |= LEFT
            } else if x > xmax {
                c |= RIGHT
            };
            if y < 0 {
                c |= TOP
            } else if y > ymax {
                c |= BOTTOM
            };
            c
        };
        loop {
            let c0 = code(x0, y0);
            let c1 = code(x1, y1);
            if (c0 | c1) == 0 {
                return Some((x0, y0, x1, y1));
            }
            if (c0 & c1) != 0 {
                return None;
            }
            let out = if c0 != 0 { c0 } else { c1 };
            let (x, y) = if (out & TOP) != 0 {
                if y1 == y0 {
                    return None;
                }
                (
                    x0 + (((x1 - x0) as i64 * (-y0) as i64) / (y1 - y0) as i64) as i32,
                    0,
                )
            } else if (out & BOTTOM) != 0 {
                if y1 == y0 {
                    return None;
                }
                (
                    x0 + (((x1 - x0) as i64 * (ymax - y0) as i64) / (y1 - y0) as i64) as i32,
                    ymax,
                )
            } else if (out & RIGHT) != 0 {
                if x1 == x0 {
                    return None;
                }
                (
                    xmax,
                    y0 + (((y1 - y0) as i64 * (xmax - x0) as i64) / (x1 - x0) as i64) as i32,
                )
            } else {
                if x1 == x0 {
                    return None;
                }
                (
                    0,
                    y0 + (((y1 - y0) as i64 * (-x0) as i64) / (x1 - x0) as i64) as i32,
                )
            };
            if out == c0 {
                x0 = x;
                y0 = y;
            } else {
                x1 = x;
                y1 = y;
            }
        }
    }

    /// Integer Bresenham line over the visible part of the segment only.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        let Some((mut x0, mut y0, x1, y1)) = Self::clip_line(x0, y0, x1, y1) else {
            return;
        };
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err * 2;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn dashed_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        let Some((mut x0, mut y0, x1, y1)) = Self::clip_line(x0, y0, x1, y1) else {
            return;
        };
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut step = 0u8;
        loop {
            if (step / 3) & 1 == 0 {
                self.pixel(x0, y0, color);
            }
            step = step.wrapping_add(1);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err * 2;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: u8) {
        let rr = r * r;
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= rr {
                    self.pixel(cx + x, cy + y, color);
                }
            }
        }
    }

    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, color: u8) {
        let mut x = r;
        let mut y = 0;
        let mut err = 1 - r;
        while x >= y {
            for (dx, dy) in [
                (x, y),
                (y, x),
                (-y, x),
                (-x, y),
                (-x, -y),
                (-y, -x),
                (y, -x),
                (x, -y),
            ] {
                self.pixel(cx + dx, cy + dy, color);
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    pub fn arrow(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        self.line(x0, y0, x1, y1, color);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let scale = dx.abs().max(dy.abs()).max(1);
        let ux = dx * 6 / scale;
        let uy = dy * 6 / scale;
        let px = -uy / 2;
        let py = ux / 2;
        self.line(x1, y1, x1 - ux + px, y1 - uy + py, color);
        self.line(x1, y1, x1 - ux - px, y1 - uy - py, color);
    }

    pub fn text(&mut self, mut x: i32, y: i32, text: &str, color: u8) {
        for ch in text.bytes() {
            if ch == b'\n' {
                break;
            }
            self.glyph(x, y, ch.to_ascii_uppercase(), color);
            x += 6;
            if x > TOP_WIDTH as i32 - 5 {
                break;
            }
        }
    }

    fn glyph(&mut self, x: i32, y: i32, ch: u8, color: u8) {
        let rows = glyph_rows(ch);
        for (ry, bits) in rows.iter().enumerate() {
            for rx in 0..5 {
                if bits & (1 << (4 - rx)) != 0 {
                    self.pixel(x + rx, y + ry as i32, color);
                }
            }
        }
    }
}

fn glyph_rows(c: u8) -> [u8; 7] {
    match c {
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1F],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x14, 0x04, 0x04, 0x04, 0x1F],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x10, 0x1E, 0x01, 0x01, 0x1E],
        b'6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E],
        b':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        b'+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        b'=' => [0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
        b'<' => [0x01, 0x02, 0x04, 0x08, 0x04, 0x02, 0x01],
        _ => [0; 7],
    }
}

#[inline]
fn map_x(x: i16) -> i32 {
    4 + (i32::from(x.clamp(0, 255)) * 247 / 255)
}

#[inline]
fn map_y(y: i16) -> i32 {
    GRAPH_Y0 + (i32::from(y.clamp(0, 191)) * (GRAPH_Y1 - GRAPH_Y0) / 191)
}

#[inline]
fn map_y_full(y: i16) -> i32 {
    GRAPH_Y0 + (i32::from(y.clamp(0, 191)) * (190 - GRAPH_Y0) / 191)
}

fn draw_grid(canvas: &mut Canvas8<'_>, y1: i32) {
    for x in (0..TOP_WIDTH as i32).step_by(16) {
        canvas.vline(x, GRAPH_Y0, y1, C_GRID);
    }
    for y in (GRAPH_Y0..=y1).step_by(16) {
        canvas.hline(0, TOP_WIDTH as i32 - 1, y, C_GRID);
    }
}

fn fill_rect(canvas: &mut Canvas8<'_>, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
    for y in y0..=y1 {
        canvas.hline(x0, x1, y, color);
    }
}

fn rect(canvas: &mut Canvas8<'_>, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
    canvas.hline(x0, x1, y0, color);
    canvas.hline(x0, x1, y1, color);
    canvas.vline(x0, y0, y1, color);
    canvas.vline(x1, y0, y1, color);
}

fn text_width(text: &str) -> i32 {
    (text.len() as i32).saturating_mul(6).saturating_sub(1)
}

fn text_centered(canvas: &mut Canvas8<'_>, y: i32, text: &str, color: u8) {
    let x = ((TOP_WIDTH as i32 - text_width(text)) / 2).max(1);
    canvas.text(x, y, text, color);
}

fn endpoints(graph: &Digraph, from: VertexId, to: VertexId) -> Option<(i32, i32, i32, i32)> {
    let a = graph.vertex(from)?;
    let b = graph.vertex(to)?;
    Some((map_x(a.x), map_y(a.y), map_x(b.x), map_y(b.y)))
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

fn draw_edge_offset(
    canvas: &mut Canvas8<'_>,
    graph: &Digraph,
    edge: &Edge,
    color: u8,
    offset: i32,
) {
    let Some((x0, y0, x1, y1)) = endpoints(graph, edge.from, edge.to) else {
        return;
    };
    let dx = x1 - x0;
    let dy = y1 - y0;
    let scale = dx.abs().max(dy.abs()).max(1);
    let ox = (-dy) * offset / scale;
    let oy = dx * offset / scale;
    // Tip stops outside the vertex outline.  This keeps the arrowhead visible
    // instead of burying it under the target circle.
    let (ax, ay, bx, by) = clipped_segment(x0 + ox, y0 + oy, x1 + ox, y1 + oy, 6, 9);
    if edge.directed {
        canvas.arrow(ax, ay, bx, by, color)
    } else {
        canvas.line(ax, ay, bx, by, color)
    }
}

fn draw_edge(canvas: &mut Canvas8<'_>, graph: &Digraph, edge: &Edge, color: u8) {
    draw_edge_offset(canvas, graph, edge, color, 0);
}

fn group_color(index: usize) -> u8 {
    C_COMPONENT0 + (index as u8 % 8)
}

fn group_colors_for_vertex(
    result: &crate::analysis::AnalysisResult,
    id: VertexId,
    out: &mut [u8; 8],
) -> usize {
    let mut n = 0usize;
    for (gi, g) in result.groups.iter().enumerate() {
        if g.vertices.contains(&id) {
            let c = group_color(gi);
            if !out[..n].contains(&c) && n < out.len() {
                out[n] = c;
                n += 1;
            }
        }
    }
    n
}
fn group_colors_for_edge(
    result: &crate::analysis::AnalysisResult,
    id: crate::graph::EdgeId,
    out: &mut [u8; 8],
) -> usize {
    let mut n = 0usize;
    for (gi, g) in result.groups.iter().enumerate() {
        if g.edges.contains(&id) {
            let c = group_color(gi);
            if !out[..n].contains(&c) && n < out.len() {
                out[n] = c;
                n += 1;
            }
        }
    }
    n
}

fn draw_multicolor_vertex(canvas: &mut Canvas8<'_>, v: &Vertex, colors: &[u8]) {
    let x = map_x(v.x);
    let y = map_y(v.y);
    let r = 4;
    let count = colors.len().max(1);
    if colors.is_empty() {
        canvas.fill_circle(x, y, r, C_VERTEX);
    } else if colors.len() == 1 {
        canvas.fill_circle(x, y, r, colors[0]);
    } else {
        let rr = r * r;
        for py in -r..=r {
            for px in -r..=r {
                if px * px + py * py <= rr {
                    let band = (((px + r) as usize) * count / (2 * r + 1) as usize).min(count - 1);
                    canvas.pixel(x + px, y + py, colors[band]);
                }
            }
        }
    }
    canvas.circle(x, y, 5, C_TEXT);
}

fn draw_vertex(canvas: &mut Canvas8<'_>, v: &Vertex, fill: u8) {
    draw_multicolor_vertex(canvas, v, &[fill]);
}

fn draw_grouped_edge(canvas: &mut Canvas8<'_>, graph: &Digraph, edge: &Edge, colors: &[u8]) {
    if colors.is_empty() {
        draw_edge(canvas, graph, edge, C_MUTED);
        return;
    }
    if colors.len() == 1 {
        draw_edge(canvas, graph, edge, colors[0]);
        return;
    }
    let offsets = [-3, -2, -1, 0, 1, 2, 3, 4];
    for (i, &color) in colors.iter().take(offsets.len()).enumerate() {
        draw_edge_offset(canvas, graph, edge, color, offsets[i]);
    }
}

fn top_edge_stride(graph: &Digraph) -> usize {
    let e = graph.edges.len();
    let budget = 1500usize;
    if e <= budget { 1 } else { e.div_ceil(budget) }
}
fn top_detailed_arrows(graph: &Digraph) -> bool {
    graph.edges.len() <= 4000
}
fn draw_original(canvas: &mut Canvas8<'_>, graph: &Digraph) {
    // Idle/original mode keeps the full vertical canvas. Analysis modes use the
    // compressed map_y() region so the contextual panel has dedicated space.
    let stride = top_edge_stride(graph);
    let arrows = top_detailed_arrows(graph);
    for (ei, edge) in graph.edges.iter().enumerate() {
        if ei % stride != 0 {
            continue;
        }
        let (Some(a), Some(b)) = (graph.vertex(edge.from), graph.vertex(edge.to)) else {
            continue;
        };
        let (x0, y0, x1, y1) = (map_x(a.x), map_y_full(a.y), map_x(b.x), map_y_full(b.y));
        let (ax, ay, bx, by) = clipped_segment(x0, y0, x1, y1, 6, 9);
        if edge.directed && arrows {
            canvas.arrow(ax, ay, bx, by, C_EDGE);
        } else {
            canvas.line(ax, ay, bx, by, C_EDGE);
        }
    }
    for v in &graph.vertices {
        let x = map_x(v.x);
        let y = map_y_full(v.y);
        canvas.fill_circle(x, y, 4, C_VERTEX);
        canvas.circle(x, y, 5, C_TEXT);
    }
}

fn draw_original_editor_view(
    canvas: &mut Canvas8<'_>,
    graph: &Digraph,
    analysis: &AnalysisState,
    ui: &UiState,
) {
    let map_point = |x: i16, y: i16| -> (i32, i32) {
        let (sx, sy) = ui.world_to_screen(x, y);
        let xx = i32::from(sx);
        let yy = GRAPH_Y0 + i32::from(sy) * (GRAPH_Y1 - GRAPH_Y0) / 181;
        (xx, yy)
    };
    let result = analysis.result.as_ref();
    let stride = top_edge_stride(graph);
    let arrows = top_detailed_arrows(graph);
    for (ei, edge) in graph.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (graph.vertex(edge.from), graph.vertex(edge.to)) else {
            continue;
        };
        let (x0, y0) = map_point(a.x, a.y);
        let (x1, y1) = map_point(b.x, b.y);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let scale = dx.abs().max(dy.abs()).max(1);
        let mut colors = [0u8; 8];
        let gn = result
            .map(|r| group_colors_for_edge(r, edge.id, &mut colors))
            .unwrap_or(0);
        let mut draw_one = |color: u8, offset: i32| {
            let ox = (-dy) * offset / scale;
            let oy = dx * offset / scale;
            let (ax, ay, bx, by) = clipped_segment(x0 + ox, y0 + oy, x1 + ox, y1 + oy, 6, 9);
            if edge.directed && arrows {
                canvas.arrow(ax, ay, bx, by, color)
            } else {
                canvas.line(ax, ay, bx, by, color)
            }
        };
        if gn > 0 {
            let offsets = [-3, -2, -1, 0, 1, 2, 3, 4];
            for (i, &color) in colors[..gn].iter().enumerate() {
                draw_one(color, offsets[i]);
            }
        } else if let Some(r) = result {
            let primary = r.primary_edges.contains(&edge.id);
            let secondary = r.secondary_edges.contains(&edge.id);
            if primary && secondary {
                draw_one(C_ACCENT, -1);
                draw_one(C_REMOVED, 1);
            } else if primary {
                draw_one(C_ACCENT, 0);
            } else if secondary {
                draw_one(C_REMOVED, 0);
            } else if ei % stride == 0 {
                draw_one(C_EDGE, 0);
            }
        } else if ei % stride == 0 {
            draw_one(C_EDGE, 0);
        }
    }
    let is_reach = analysis.selected == Some(AnalysisKind::Reachability);
    for v in &graph.vertices {
        let (x, y) = map_point(v.x, v.y);
        let mut colors = [0u8; 8];
        let gn = result
            .map(|r| group_colors_for_vertex(r, v.id, &mut colors))
            .unwrap_or(0);
        let draw_at = |canvas: &mut Canvas8<'_>, cols: &[u8]| {
            let r = 4;
            if cols.is_empty() {
                canvas.fill_circle(x, y, r, C_VERTEX);
            } else if cols.len() == 1 {
                canvas.fill_circle(x, y, r, cols[0]);
            } else {
                let rr = r * r;
                for py in -r..=r {
                    for px in -r..=r {
                        if px * px + py * py <= rr {
                            let band = (((px + r) as usize) * cols.len() / (2 * r + 1) as usize)
                                .min(cols.len() - 1);
                            canvas.pixel(x + px, y + py, cols[band]);
                        }
                    }
                }
            }
            canvas.circle(x, y, 5, C_TEXT);
        };
        if analysis.source == Some(v.id) {
            draw_at(canvas, &[C_SOURCE]);
        } else if analysis.target == Some(v.id) {
            draw_at(canvas, &[C_TARGET]);
        } else if gn > 0 {
            draw_at(canvas, &colors[..gn]);
        } else if let Some(r) = result {
            if r.primary_vertices.contains(&v.id) && r.secondary_vertices.contains(&v.id) {
                draw_at(canvas, &[C_ACCENT, C_REMOVED]);
            } else if is_reach && r.secondary_vertices.contains(&v.id) {
                draw_at(canvas, &[C_UNREACHABLE]);
            } else if is_reach && r.primary_vertices.contains(&v.id) {
                draw_at(canvas, &[C_REACHABLE]);
            } else if r.primary_vertices.contains(&v.id) {
                draw_at(canvas, &[C_ACCENT]);
            } else if r.secondary_vertices.contains(&v.id) {
                draw_at(canvas, &[C_REMOVED]);
            } else {
                draw_at(canvas, &[C_VERTEX]);
            }
        } else {
            draw_at(canvas, &[C_VERTEX]);
        }
    }
}

fn draw_transform(canvas: &mut Canvas8<'_>, graph: &Digraph, t: &DagTransform, show_removed: bool) {
    if t.kind == DagTransformKind::SccCondensation {
        for arc in &t.condensation_arcs {
            let Some(a) = t.components.get(arc.from_component as usize) else {
                continue;
            };
            let Some(b) = t.components.get(arc.to_component as usize) else {
                continue;
            };
            {
                let (x0, y0, x1, y1) =
                    clipped_segment(map_x(a.x), map_y(a.y), map_x(b.x), map_y(b.y), 7, 10);
                canvas.arrow(x0, y0, x1, y1, C_EDGE);
            }
        }
        for c in &t.components {
            let color = C_COMPONENT0 + (c.component as u8 % 8);
            let x = map_x(c.x);
            let y = map_y(c.y);
            canvas.fill_circle(x, y, 6, color);
            canvas.circle(x, y, 7, C_TEXT);
            let mut buf = [0u8; 3];
            let label = small_u16(c.members.len() as u16, &mut buf);
            if let Ok(s) = core::str::from_utf8(label) {
                canvas.text(x + 8, y - 3, s, C_TEXT);
            }
        }
        return;
    }

    if show_removed {
        for arc in &t.removed_arcs {
            if let Some((x0, y0, x1, y1)) = endpoints(graph, arc.from, arc.to) {
                canvas.dashed_line(x0, y0, x1, y1, C_REMOVED);
            }
        }
    }

    for arc in &t.kept_arcs {
        if let Some((x0, y0, x1, y1)) = endpoints(graph, arc.from, arc.to) {
            {
                let (a, b, c, d) = clipped_segment(x0, y0, x1, y1, 6, 9);
                canvas.arrow(a, b, c, d, C_EDGE);
            }
        }
    }

    for v in &graph.vertices {
        if t.removed_vertices.contains(&v.id) {
            if show_removed {
                let x = map_x(v.x);
                let y = map_y(v.y);
                canvas.line(x - 5, y - 5, x + 5, y + 5, C_REMOVED);
                canvas.line(x - 5, y + 5, x + 5, y - 5, C_REMOVED);
                canvas.circle(x, y, 6, C_REMOVED);
            }
        } else {
            draw_vertex(canvas, v, C_ACCENT);
        }
    }
}

fn value_for(values: &[(VertexId, i16)], id: VertexId) -> Option<i16> {
    values
        .iter()
        .find_map(|(vid, value)| (*vid == id).then_some(*value))
}

fn draw_distance(canvas: &mut Canvas8<'_>, x: i32, y: i32, value: i16, color: u8) {
    if value < 0 {
        canvas.text(x, y, "--", color);
        return;
    }
    let mut buf = [0u8; 3];
    let bytes = small_u16(value as u16, &mut buf);
    if let Ok(text) = core::str::from_utf8(bytes) {
        canvas.text(x, y, text, color);
    }
}

fn draw_reachability_distances(canvas: &mut Canvas8<'_>, v: &Vertex, from: i16, to: i16) {
    let x = map_x(v.x);
    let y = map_y(v.y);
    // Keep both labels on-screen.  If the vertex is close to the header, put
    // the labels below it; otherwise put them above it.
    let (label_y, value_y) = if y < GRAPH_Y0 + 20 {
        (y + 7, y + 15)
    } else {
        (y - 16, y - 8)
    };
    let left_x = (x - 18).max(1);
    let right_x = (x + 7).min(226);
    canvas.text(left_x, label_y, "TO", C_MUTED);
    draw_distance(canvas, left_x + 3, value_y, to, C_TEXT);
    canvas.text(right_x, label_y, "FROM", C_MUTED);
    draw_distance(canvas, right_x + 6, value_y, from, C_TEXT);
}

fn is_component_quotient(analysis: &AnalysisState) -> bool {
    matches!(
        analysis.component,
        Some(ComponentKind::Weak | ComponentKind::Strong | ComponentKind::Unilateral)
    ) && analysis
        .result
        .as_ref()
        .is_some_and(|r| !r.groups.is_empty())
}

/// Place quotient super-vertices as close as possible to the geometry of the
/// original editable graph.  A component inherits the centroid of its member
/// vertices, then we apply only the minimum collision separation needed to keep
/// two super-vertices legible.  Singleton components therefore stay almost
/// exactly above their original vertex.
fn quotient_positions(
    graph: &Digraph,
    result: &crate::analysis::AnalysisResult,
) -> Vec<(i32, i32)> {
    const X_MIN: i32 = 12;
    const X_MAX: i32 = 243;
    const Y_MIN: i32 = GRAPH_Y0 + 11;
    // Leave room for the N# label drawn below a super-vertex.
    const Y_MAX: i32 = 171;
    const MIN_SEP: i32 = 21;
    const MIN_SEP2: i32 = MIN_SEP * MIN_SEP;

    let mut out: Vec<(i32, i32)> = Vec::with_capacity(result.groups.len());

    for (gi, group) in result.groups.iter().enumerate() {
        let mut sx = 0i32;
        let mut sy = 0i32;
        let mut n = 0i32;
        for &id in &group.vertices {
            if let Some(v) = graph.vertex(id) {
                sx += i32::from(v.x);
                sy += i32::from(v.y);
                n += 1;
            }
        }

        // Empty groups should not normally exist, but keep a deterministic
        // fallback so malformed/legacy analysis data cannot break rendering.
        let (cx, cy) = if n > 0 { (sx / n, sy / n) } else { (128, 96) };

        let mut x = map_x(cx as i16).clamp(X_MIN, X_MAX);
        let mut y = map_y_full(cy as i16).clamp(Y_MIN, Y_MAX);

        // Preserve the centroid unless circles would overlap.  Resolve only
        // the current node and only by one pixel per iteration, which keeps the
        // displacement minimal and deterministic on the DS.
        for &(px, py) in &out {
            let mut steps = 0usize;
            loop {
                let dx = x - px;
                let dy = y - py;
                if dx * dx + dy * dy >= MIN_SEP2 || steps >= 48 {
                    break;
                }

                if dx == 0 && dy == 0 {
                    // Deterministic direction for identical centroids.
                    if gi & 1 == 0 {
                        x += 1;
                    } else {
                        x -= 1;
                    }
                } else if dx.abs() >= dy.abs() {
                    x += if dx >= 0 { 1 } else { -1 };
                } else {
                    y += if dy >= 0 { 1 } else { -1 };
                }
                x = x.clamp(X_MIN, X_MAX);
                y = y.clamp(Y_MIN, Y_MAX);
                steps += 1;
            }
        }

        out.push((x, y));
    }

    out
}

fn quotient_memberships(
    result: &crate::analysis::AnalysisResult,
    id: VertexId,
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

fn draw_supervertex(canvas: &mut Canvas8<'_>, x: i32, y: i32, group: usize, members: usize) {
    let color = group_color(group);
    canvas.fill_circle(x, y, 8, color);
    canvas.circle(x, y, 9, C_TEXT);
    let mut ibuf = [0u8; 3];
    let ibytes = small_u16(group as u16, &mut ibuf);
    canvas.text(x - 5, y - 3, "C", C_TEXT);
    if let Ok(t) = core::str::from_utf8(ibytes) {
        canvas.text(x + 1, y - 3, t, C_TEXT);
    }
    let mut nbuf = [0u8; 3];
    let nbytes = small_u16(members as u16, &mut nbuf);
    canvas.text(x - 8, y + 12, "N", C_MUTED);
    if let Ok(t) = core::str::from_utf8(nbytes) {
        canvas.text(x - 2, y + 12, t, C_MUTED);
    }
}

/// Top-screen component quotient/component graph.
///
/// Strong and weak components are partitions, so this is an ordinary quotient.
/// Maximal unilateral components may overlap; for that mode we intentionally
/// keep one super-vertex per reported component and draw boundary arcs between
/// component sets.  The bottom screen retains the multicolor overlap language.
fn draw_component_quotient(canvas: &mut Canvas8<'_>, graph: &Digraph, analysis: &AnalysisState) {
    let Some(result) = analysis.result.as_ref() else {
        return;
    };
    let k = result.groups.len();
    if k == 0 {
        return;
    }
    let pos = quotient_positions(graph, result);
    let mut arcs = vec![vec![false; k]; k];

    for e in &graph.edges {
        let mut from_groups = [0usize; 8];
        let mut to_groups = [0usize; 8];
        let nf = quotient_memberships(result, e.from, &mut from_groups);
        let nt = quotient_memberships(result, e.to, &mut to_groups);
        for &a in &from_groups[..nf] {
            for &b in &to_groups[..nt] {
                if a == b {
                    continue;
                }
                // With overlapping unilateral components, suppress an arc that
                // is internal to both sets; otherwise it would be a visual
                // artifact of the overlap rather than a true boundary arc.
                let internal_to_a = result.groups[a].vertices.contains(&e.to);
                let internal_to_b = result.groups[b].vertices.contains(&e.from);
                if internal_to_a && internal_to_b {
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
            let (x0, y0) = pos[a];
            let (x1, y1) = pos[b];
            let (ax, ay, bx, by) = clipped_segment(x0, y0, x1, y1, 10, 11);
            if ab && ba {
                canvas.line(ax, ay, bx, by, C_EDGE);
            } else if ab {
                canvas.arrow(ax, ay, bx, by, C_EDGE);
            } else {
                canvas.arrow(bx, by, ax, ay, C_EDGE);
            }
        }
    }

    for (i, group) in result.groups.iter().enumerate() {
        let (x, y) = pos[i];
        draw_supervertex(canvas, x, y, i, group.vertices.len());
    }

    if analysis.component == Some(ComponentKind::Unilateral) {
        canvas.text(4, 181, "OVERLAP COMPONENT GRAPH", C_MUTED);
    }
}

fn draw_generic(
    canvas: &mut Canvas8<'_>,
    graph: &Digraph,
    analysis: &AnalysisState,
    show_removed: bool,
) {
    let Some(result) = analysis.result.as_ref() else {
        draw_original(canvas, graph);
        return;
    };
    if let Some(t) = result.dag.as_ref() {
        draw_transform(canvas, graph, t, show_removed);
        return;
    }

    // Keep the graph context visible and layer the algorithm result over it.
    // Group memberships are drawn first; an edge shared by two or more groups
    // gets parallel colored strokes instead of losing one of the memberships.
    let stride = top_edge_stride(graph);
    for (ei, e) in graph.edges.iter().enumerate() {
        let mut colors = [0u8; 8];
        let n = group_colors_for_edge(result, e.id, &mut colors);
        if n > 0 {
            draw_grouped_edge(canvas, graph, e, &colors[..n]);
        } else if ei % stride == 0 {
            draw_edge(canvas, graph, e, C_MUTED);
        }
    }
    if result.groups.is_empty() {
        for e in &graph.edges {
            let primary = result.primary_edges.contains(&e.id);
            let secondary = result.secondary_edges.contains(&e.id);
            if primary && secondary {
                draw_grouped_edge(canvas, graph, e, &[C_ACCENT, C_REMOVED]);
            } else if primary {
                draw_edge(canvas, graph, e, C_ACCENT);
            } else if secondary {
                draw_edge(canvas, graph, e, C_REMOVED);
            }
        }
    } else {
        for &eid in &result.secondary_edges {
            if let Some(e) = graph.edge(eid) {
                draw_edge(canvas, graph, e, C_REMOVED);
            }
        }
    }

    let is_reach = analysis.selected == Some(AnalysisKind::Reachability);
    for v in &graph.vertices {
        let mut fill = C_VERTEX;
        let mut group_colors = [0u8; 8];
        let group_n = group_colors_for_vertex(result, v.id, &mut group_colors);

        if group_n == 0 {
            if let Some((_, value)) = result.values.iter().find(|(id, _)| *id == v.id)
                && analysis.component.is_some()
            {
                fill = C_COMPONENT0 + ((*value as u8) & 7);
            }

            if is_reach {
                if result.secondary_vertices.contains(&v.id) {
                    fill = C_UNREACHABLE;
                }
                if result.primary_vertices.contains(&v.id) {
                    fill = C_REACHABLE;
                }
            } else {
                if result.secondary_vertices.contains(&v.id) {
                    fill = C_REMOVED;
                }
                if result.primary_vertices.contains(&v.id) {
                    fill = C_ACCENT;
                }
            }
        }

        // Source/target colors always win over result/group colors so the user
        // can see the actual arguments passed to an algorithm.
        if analysis.source == Some(v.id) {
            draw_vertex(canvas, v, C_SOURCE);
        } else if analysis.target == Some(v.id) {
            draw_vertex(canvas, v, C_TARGET);
        } else if group_n > 0 {
            draw_multicolor_vertex(canvas, v, &group_colors[..group_n]);
        } else if result.primary_vertices.contains(&v.id)
            && result.secondary_vertices.contains(&v.id)
        {
            draw_multicolor_vertex(canvas, v, &[C_ACCENT, C_REMOVED]);
        } else {
            draw_vertex(canvas, v, fill);
        }

        if is_reach {
            let from = value_for(&result.values, v.id).unwrap_or(-1);
            let to = value_for(&result.secondary_values, v.id).unwrap_or(-1);
            draw_reachability_distances(canvas, v, from, to);
        } else if let Some((_, value)) = result.values.iter().find(|(id, _)| *id == v.id)
            && analysis.selected.is_some()
        {
            let x = map_x(v.x) + 7;
            let y = map_y(v.y) - 3;
            let mut buf = [0u8; 4];
            let n = if *value < 0 {
                buf[0] = b'-';
                let z = (-*value).min(99) as u8;
                if z >= 10 {
                    buf[1] = b'0' + z / 10;
                    buf[2] = b'0' + z % 10;
                    3
                } else {
                    buf[1] = b'0' + z;
                    2
                }
            } else {
                let z = (*value).min(999) as u16;
                if z >= 100 {
                    buf[0] = b'0' + (z / 100) as u8;
                    buf[1] = b'0' + ((z / 10) % 10) as u8;
                    buf[2] = b'0' + (z % 10) as u8;
                    3
                } else if z >= 10 {
                    buf[0] = b'0' + (z / 10) as u8;
                    buf[1] = b'0' + (z % 10) as u8;
                    2
                } else {
                    buf[0] = b'0' + z as u8;
                    1
                }
            };
            if let Ok(t) = core::str::from_utf8(&buf[..n]) {
                canvas.text(x, y, t, C_TEXT);
            }
        }
    }
}

fn u16_wide(value: u16, buf: &mut [u8; 5]) -> &[u8] {
    if value >= 10000 {
        buf[0] = b'0' + (value / 10000) as u8;
        buf[1] = b'0' + ((value / 1000) % 10) as u8;
        buf[2] = b'0' + ((value / 100) % 10) as u8;
        buf[3] = b'0' + ((value / 10) % 10) as u8;
        buf[4] = b'0' + (value % 10) as u8;
        &buf[..5]
    } else if value >= 1000 {
        buf[0] = b'0' + (value / 1000) as u8;
        buf[1] = b'0' + ((value / 100) % 10) as u8;
        buf[2] = b'0' + ((value / 10) % 10) as u8;
        buf[3] = b'0' + (value % 10) as u8;
        &buf[..4]
    } else if value >= 100 {
        buf[0] = b'0' + (value / 100) as u8;
        buf[1] = b'0' + ((value / 10) % 10) as u8;
        buf[2] = b'0' + (value % 10) as u8;
        &buf[..3]
    } else if value >= 10 {
        buf[0] = b'0' + (value / 10) as u8;
        buf[1] = b'0' + (value % 10) as u8;
        &buf[..2]
    } else {
        buf[0] = b'0' + value as u8;
        &buf[..1]
    }
}

fn small_u16(mut value: u16, buf: &mut [u8; 3]) -> &[u8] {
    if value > 999 {
        value = 999;
    }
    if value >= 100 {
        buf[0] = b'0' + (value / 100) as u8;
        buf[1] = b'0' + ((value / 10) % 10) as u8;
        buf[2] = b'0' + (value % 10) as u8;
        &buf[..3]
    } else if value >= 10 {
        buf[0] = b'0' + (value / 10) as u8;
        buf[1] = b'0' + (value % 10) as u8;
        &buf[..2]
    } else {
        buf[0] = b'0' + value as u8;
        &buf[..1]
    }
}

fn legend_item(canvas: &mut Canvas8<'_>, x: i32, y: i32, color: u8, label: &str) {
    canvas.fill_circle(x + 5, y + 5, 4, color);
    canvas.circle(x + 5, y + 5, 5, C_TEXT);
    canvas.text(x + 14, y + 2, label, C_TEXT);
}

fn legend_shared(canvas: &mut Canvas8<'_>, x: i32, y: i32, label: &str) {
    let cx = x + 5;
    let cy = y + 5;
    let colors = [C_COMPONENT0, C_COMPONENT0 + 1, C_COMPONENT0 + 2];
    let r = 4;
    let rr = r * r;
    for py in -r..=r {
        for px in -r..=r {
            if px * px + py * py <= rr {
                let band = (((px + r) as usize) * colors.len() / (2 * r + 1) as usize)
                    .min(colors.len() - 1);
                canvas.pixel(cx + px, cy + py, colors[band]);
            }
        }
    }
    canvas.circle(cx, cy, 5, C_TEXT);
    canvas.text(x + 14, y + 2, label, C_TEXT);
}

fn draw_vertex_token(canvas: &mut Canvas8<'_>, mut x: i32, y: i32, id: VertexId, color: u8) -> i32 {
    canvas.text(x, y, "B", color);
    x += 6;
    let mut buf = [0u8; 3];
    let bytes = small_u16(id, &mut buf);
    if let Ok(t) = core::str::from_utf8(bytes) {
        canvas.text(x, y, t, color);
        x += text_width(t) + 1;
    }
    x
}

fn draw_sequence(canvas: &mut Canvas8<'_>, label: &str, ids: &[VertexId], y: i32, color: u8) {
    canvas.text(8, y, label, C_MUTED);
    let mut x = 8 + text_width(label) + 7;
    let mut row_y = y;
    let mut rows = 0usize;
    for (i, &id) in ids.iter().enumerate() {
        let need = 12
            + if id >= 100 {
                18
            } else if id >= 10 {
                12
            } else {
                6
            };
        if x + need > 248 {
            rows += 1;
            if rows >= 2 {
                canvas.text(232, row_y, "...", C_MUTED);
                break;
            }
            row_y += 10;
            x = 8;
        }
        x = draw_vertex_token(canvas, x, row_y, id, color);
        if i + 1 < ids.len() {
            canvas.text(x, row_y, ">", C_MUTED);
            x += 7;
        }
    }
}

fn draw_count(canvas: &mut Canvas8<'_>, x: i32, y: i32, label: &str, value: u16, color: u8) {
    canvas.text(x, y, label, C_MUTED);
    let mut buf = [0u8; 3];
    let b = small_u16(value, &mut buf);
    if let Ok(t) = core::str::from_utf8(b) {
        canvas.text(x + text_width(label) + 7, y, t, color);
    }
}

fn draw_topological_panel(canvas: &mut Canvas8<'_>, result: &crate::analysis::AnalysisResult) {
    canvas.text(8, PANEL_Y0 + 5, "GENERATIONS", C_MUTED);
    let shown = result.groups.len().min(5);
    for i in 0..shown {
        let x = 84 + (i as i32) * 33;
        let color = group_color(i);
        canvas.fill_circle(x, PANEL_Y0 + 9, 4, color);
        canvas.circle(x, PANEL_Y0 + 9, 5, C_TEXT);
        canvas.text(x + 7, PANEL_Y0 + 6, "G", C_MUTED);
        let mut buf = [0u8; 3];
        let b = small_u16(i as u16, &mut buf);
        if let Ok(t) = core::str::from_utf8(b) {
            canvas.text(x + 13, PANEL_Y0 + 6, t, C_TEXT);
        }
    }
    if result.groups.len() > shown {
        canvas.text(243, PANEL_Y0 + 6, "+", C_MUTED);
    }
    draw_sequence(canvas, "ORDER", &result.order, PANEL_Y0 + 23, C_TEXT);
}

fn draw_info_panel(canvas: &mut Canvas8<'_>, analysis: &AnalysisState) {
    if analysis.selected.is_none() && analysis.component.is_none() {
        return;
    }

    let Some(result) = analysis.result.as_ref() else {
        fill_rect(canvas, 2, PANEL_Y0, TOP_WIDTH as i32 - 3, PANEL_Y1, C_PANEL);
        rect(canvas, 2, PANEL_Y0, TOP_WIDTH as i32 - 3, PANEL_Y1, C_MUTED);
        text_centered(canvas, PANEL_Y0 + 20, "WAITING FOR RESULT", C_MUTED);
        return;
    };

    fill_rect(canvas, 2, PANEL_Y0, TOP_WIDTH as i32 - 3, PANEL_Y1, C_PANEL);
    rect(canvas, 2, PANEL_Y0, TOP_WIDTH as i32 - 3, PANEL_Y1, C_MUTED);

    if let Some(notice) = result.notice {
        let (title, color) = match notice.level {
            AnalysisNoticeLevel::Warning => ("WARNING", C_WARNING),
            AnalysisNoticeLevel::Error => ("ERROR", C_REMOVED),
        };
        rect(
            canvas,
            12,
            PANEL_Y0 + 3,
            TOP_WIDTH as i32 - 13,
            PANEL_Y1 - 3,
            color,
        );
        text_centered(canvas, PANEL_Y0 + 7, title, color);
        canvas.hline(20, TOP_WIDTH as i32 - 21, PANEL_Y0 + 17, color);
        text_centered(canvas, PANEL_Y0 + 23, notice.line1, C_TEXT);
        if !notice.line2.is_empty() {
            text_centered(canvas, PANEL_Y0 + 36, notice.line2, C_TEXT);
        }
        return;
    }

    if let Some(component) = analysis.component {
        match component {
            ComponentKind::Loops => {
                if result.count == 0 {
                    text_centered(canvas, PANEL_Y0 + 12, "NO DIRECTED LOOPS", C_MUTED);
                    text_centered(canvas, PANEL_Y0 + 28, "GRAPH IS ACYCLIC", C_TEXT);
                } else {
                    legend_item(canvas, 8, PANEL_Y0 + 6, C_COMPONENT0, "LOOP");
                    legend_shared(canvas, 132, PANEL_Y0 + 6, "SHARED");
                    draw_count(canvas, 8, PANEL_Y0 + 31, "FOUND", result.count, C_TEXT);
                    canvas.text(82, PANEL_Y0 + 31, "LOOPS", C_MUTED);
                }
            }
            ComponentKind::SourcesSinks => {
                legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "SOURCE SCC");
                legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "SINK SCC");
                draw_count(canvas, 8, PANEL_Y0 + 30, "SCCS", result.count, C_TEXT);
            }
            ComponentKind::Unilateral => {
                legend_item(canvas, 8, PANEL_Y0 + 6, C_COMPONENT0, "UNILATERAL");
                legend_shared(canvas, 132, PANEL_Y0 + 6, "SHARED");
                draw_count(canvas, 8, PANEL_Y0 + 31, "COMPONENTS", result.count, C_TEXT);
            }
            ComponentKind::Weak => {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_COMPONENT0, "WEAK CC");
                draw_count(canvas, 8, PANEL_Y0 + 31, "COMPONENTS", result.count, C_TEXT);
            }
            ComponentKind::Strong => {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_COMPONENT0, "SCC");
                draw_count(canvas, 8, PANEL_Y0 + 31, "COMPONENTS", result.count, C_TEXT);
            }
            ComponentKind::TwoEdgeConnected => {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_COMPONENT0, "2-EDGE CC");
                draw_count(canvas, 8, PANEL_Y0 + 31, "COMPONENTS", result.count, C_TEXT);
            }
            ComponentKind::Articulation => {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_ACCENT, "ARTICULATION");
                draw_count(canvas, 8, PANEL_Y0 + 31, "FOUND", result.count, C_TEXT);
            }
            ComponentKind::Bridges => {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_ACCENT, "BRIDGE");
                draw_count(canvas, 8, PANEL_Y0 + 31, "FOUND", result.count, C_TEXT);
            }
        }
        return;
    }

    if analysis.selected == Some(AnalysisKind::Bipartite) && !result.exact {
        text_centered(canvas, PANEL_Y0 + 15, "NOT BIPARTITE", C_REMOVED);
        return;
    }
    if analysis.selected == Some(AnalysisKind::Semiconnected) && !result.exact {
        text_centered(canvas, PANEL_Y0 + 15, "NOT SEMICONNECTED", C_REMOVED);
        return;
    }
    if analysis.selected == Some(AnalysisKind::FindCycle) {
        // Backward compatibility for projects saved before LOOPS moved to
        // Components in v0.4.6.
        if result.count == 0 {
            text_centered(canvas, PANEL_Y0 + 12, "NO DIRECTED LOOPS", C_MUTED);
            text_centered(canvas, PANEL_Y0 + 28, "GRAPH IS ACYCLIC", C_TEXT);
        } else {
            legend_item(canvas, 8, PANEL_Y0 + 6, C_COMPONENT0, "LOOP");
            legend_shared(canvas, 132, PANEL_Y0 + 6, "SHARED");
            draw_count(canvas, 8, PANEL_Y0 + 31, "FOUND", result.count, C_TEXT);
        }
        return;
    }

    match analysis.selected {
        Some(AnalysisKind::Reachability) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REACHABLE, "REACHABLE");
            legend_item(canvas, 8, PANEL_Y0 + 24, C_UNREACHABLE, "NO REACH");
            canvas.text(132, PANEL_Y0 + 26, "TO/FROM = HOPS", C_MUTED);
        }
        Some(AnalysisKind::ShortestPath) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_TARGET, "TARGET");
            legend_item(canvas, 8, PANEL_Y0 + 24, C_ACCENT, "PATH");
            draw_count(canvas, 132, PANEL_Y0 + 26, "DIST", result.count, C_TEXT);
        }
        Some(AnalysisKind::TopologicalOrder) => draw_topological_panel(canvas, result),
        Some(AnalysisKind::EulerTrail) => {
            draw_count(canvas, 8, PANEL_Y0 + 5, "ARCS", result.count, C_TEXT);
            draw_sequence(
                canvas,
                "TRAIL",
                &result.primary_vertices,
                PANEL_Y0 + 22,
                C_TEXT,
            );
        }
        Some(AnalysisKind::DagLongestPath) => {
            draw_count(canvas, 8, PANEL_Y0 + 5, "LENGTH", result.count, C_TEXT);
            draw_sequence(
                canvas,
                "PATH",
                &result.primary_vertices,
                PANEL_Y0 + 22,
                C_TEXT,
            );
        }
        Some(AnalysisKind::PnPositions) | Some(AnalysisKind::DagKernel) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "P POSITION");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "N POSITION");
            draw_count(
                canvas,
                8,
                PANEL_Y0 + 31,
                "P COUNT",
                result.primary_vertices.len() as u16,
                C_TEXT,
            );
        }
        Some(AnalysisKind::SpragueGrundy) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "SG = 0");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "SG > 0");
            canvas.text(8, PANEL_Y0 + 27, "NUMBER = GRUNDY VALUE", C_MUTED);
        }
        Some(AnalysisKind::WinLoseDraw) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "WIN");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "LOSE");
            legend_item(canvas, 8, PANEL_Y0 + 24, C_VERTEX, "DRAW");
        }
        Some(AnalysisKind::Bipartite) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "SIDE A");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "SIDE B");
            text_centered(canvas, PANEL_Y0 + 31, "GRAPH IS BIPARTITE", C_TEXT);
        }
        Some(AnalysisKind::BipartiteMatching) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "SIDE A");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "SIDE B");
            draw_count(canvas, 8, PANEL_Y0 + 31, "MATCHING", result.count, C_TEXT);
        }
        Some(AnalysisKind::SpanningForest) => {
            legend_item(canvas, 8, PANEL_Y0 + 8, C_ACCENT, "FOREST EDGE");
            draw_count(canvas, 8, PANEL_Y0 + 31, "TREES", result.count, C_TEXT);
        }
        Some(AnalysisKind::KCore) => {
            draw_count(canvas, 8, PANEL_Y0 + 8, "MAX CORE", result.count, C_TEXT);
            canvas.text(8, PANEL_Y0 + 30, "NUMBER BESIDE VERTEX = CORE", C_MUTED);
        }
        Some(AnalysisKind::Dominators) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            canvas.text(8, PANEL_Y0 + 28, "NUMBER BESIDE VERTEX = IDOM", C_MUTED);
        }
        Some(AnalysisKind::UnitMinCut) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_TARGET, "TARGET");
            draw_count(
                canvas,
                8,
                PANEL_Y0 + 31,
                "MIN EDGE CUT",
                result.count,
                C_TEXT,
            );
        }
        Some(AnalysisKind::EdgeDisjointRoutes) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_TARGET, "TARGET");
            draw_count(canvas, 8, PANEL_Y0 + 31, "MAX ROUTES", result.count, C_TEXT);
        }
        Some(AnalysisKind::VertexMinCut) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_TARGET, "TARGET");
            draw_count(
                canvas,
                8,
                PANEL_Y0 + 31,
                "MIN VERT CUT",
                result.count,
                C_TEXT,
            );
        }
        Some(AnalysisKind::VertexDisjointRoutes) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_SOURCE, "SOURCE");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_TARGET, "TARGET");
            draw_count(canvas, 8, PANEL_Y0 + 31, "MAX ROUTES", result.count, C_TEXT);
        }
        Some(AnalysisKind::DagPathCover) => {
            legend_item(canvas, 8, PANEL_Y0 + 8, C_ACCENT, "COVER EDGE");
            draw_count(canvas, 8, PANEL_Y0 + 31, "PATHS", result.count, C_TEXT);
        }
        Some(AnalysisKind::Semiconnected) => {
            text_centered(canvas, PANEL_Y0 + 17, "GRAPH IS SEMICONNECTED", C_TEXT);
        }
        Some(AnalysisKind::MotherVertex) => {
            if result.count == 0 {
                text_centered(canvas, PANEL_Y0 + 17, "NO MOTHER VERTEX", C_MUTED);
            } else {
                legend_item(canvas, 8, PANEL_Y0 + 8, C_ACCENT, "MOTHER VERTEX");
            }
        }
        Some(AnalysisKind::StrongConnectCount) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "SOURCE SCC");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "SINK SCC");
            draw_count(canvas, 8, PANEL_Y0 + 31, "ADD EDGES", result.count, C_TEXT);
        }
        Some(AnalysisKind::SccCondensation)
        | Some(AnalysisKind::VertexOrderDag)
        | Some(AnalysisKind::DfsDag)
        | Some(AnalysisKind::MinimumFeedbackArcSet)
        | Some(AnalysisKind::MinimumFeedbackVertexSet) => {
            legend_item(canvas, 8, PANEL_Y0 + 5, C_ACCENT, "KEPT");
            legend_item(canvas, 132, PANEL_Y0 + 5, C_REMOVED, "REMOVED");
            draw_count(
                canvas,
                8,
                PANEL_Y0 + 31,
                "RESULT ARCS",
                result.count,
                C_TEXT,
            );
        }
        Some(AnalysisKind::FindCycle) | None => {}
    }
}

fn header(canvas: &mut Canvas8<'_>, graph: &Digraph, analysis: &AnalysisState) {
    canvas.hline(0, TOP_WIDTH as i32 - 1, HEADER_H - 1, C_MUTED);
    let label = analysis
        .selected
        .map(|k| k.code())
        .or_else(|| analysis.component.map(|k| k.code()))
        .unwrap_or("ORIGINAL");
    canvas.text(3, 3, label, C_TEXT);
    let mut vbuf = [0u8; 5];
    let mut ebuf = [0u8; 5];
    let v = u16_wide(graph.vertices.len() as u16, &mut vbuf);
    let e = u16_wide(graph.edges.len() as u16, &mut ebuf);
    canvas.text(146, 3, "V", C_MUTED);
    if let Ok(s) = core::str::from_utf8(v) {
        canvas.text(154, 3, s, C_TEXT);
    }
    canvas.text(184, 3, "E", C_MUTED);
    if let Ok(s) = core::str::from_utf8(e) {
        canvas.text(192, 3, s, C_TEXT);
    }
    if let Some(r) = analysis.result.as_ref() {
        let (tag, color) = if let Some(n) = r.notice {
            match n.level {
                AnalysisNoticeLevel::Warning => ("WRN", C_WARNING),
                AnalysisNoticeLevel::Error => ("ERR", C_REMOVED),
            }
        } else if r.truncated {
            ("BD", C_WARNING)
        } else if r.exact {
            ("EX", C_ACCENT)
        } else {
            ("NO", C_REMOVED)
        };
        canvas.text(238, 3, tag, color);
    }
}

pub fn render_into(
    canvas: &mut Canvas8<'_>,
    graph: &Digraph,
    analysis: &AnalysisState,
    settings: &TopViewSettings,
    ui: &UiState,
) {
    canvas.clear(C_BG);
    let active = analysis.selected.is_some() || analysis.component.is_some();

    // Logical view swap: keep the touch/result canvas on the bottom while the
    // top remains the stable information surface. The original editor camera is
    // shown here, and warnings/legends/order text never leave the top LCD.
    if ui.result_on_bottom {
        if settings.grid {
            draw_grid(canvas, if active { GRAPH_Y1 } else { 190 });
        }
        if settings.show_original_when_idle || active {
            draw_original_editor_view(canvas, graph, analysis, ui);
        }
        draw_info_panel(canvas, analysis);
        header(canvas, graph, analysis);
        return;
    }

    let quotient = is_component_quotient(analysis);
    if settings.grid {
        draw_grid(canvas, if quotient || !active { 190 } else { GRAPH_Y1 });
    }
    if quotient {
        draw_component_quotient(canvas, graph, analysis);
    } else if active {
        draw_generic(canvas, graph, analysis, settings.show_removed);
    } else if settings.show_original_when_idle {
        draw_original(canvas, graph);
    }
    if !quotient {
        draw_info_panel(canvas, analysis);
    }
    header(canvas, graph, analysis);
}

fn mix(mut h: u32, x: u32) -> u32 {
    h ^= x;
    h = h.wrapping_mul(16_777_619);
    h
}

fn signature(
    graph: &Digraph,
    analysis: &AnalysisState,
    settings: &TopViewSettings,
    ui: &UiState,
) -> u32 {
    let mut h = 2_166_136_261u32;
    h = mix(h, graph.topology_revision);
    h = mix(h, graph.visual_revision);
    h = mix(h, analysis.selected.map(|x| x as u32 + 1).unwrap_or(0));
    h = mix(h, analysis.component.map(|x| x as u32 + 100).unwrap_or(0));
    h = mix(h, analysis.source.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, analysis.target.map(u32::from).unwrap_or(u32::MAX));
    h = mix(h, analysis.result.is_some() as u32);
    h = mix(h, settings.grid as u32);
    h = mix(h, settings.show_removed as u32);
    h = mix(h, settings.show_original_when_idle as u32);
    h = mix(h, ui.theme as u32);
    h = mix(h, ui.result_on_bottom as u32);
    // The top screen only follows the editor camera while the result has been
    // swapped to the bottom. Normal bottom-screen pan/zoom must not invalidate
    // a dense, otherwise unchanged top overview.
    if ui.result_on_bottom {
        h = mix(h, ui.zoom as u32);
        h = mix(h, ui.pan_x as u16 as u32);
        h = mix(h, ui.pan_y as u16 as u32);
    }
    h
}

fn render_top_view(
    graph: Res<Digraph>,
    analysis: Res<AnalysisState>,
    settings: Res<TopViewSettings>,
    ui: Res<UiState>,
    mut state: ResMut<TopViewState>,
) {
    if !settings.enabled {
        return;
    }
    #[cfg(target_vendor = "nintendo")]
    {
        if !state.initialized {
            init_main_bitmap_top();
            clear_top_vram(MAIN_BG_VRAM as *mut u16, ui.theme);
            state.initialized = true;
            state.last_signature = u32::MAX;
        }
        let sig = signature(&graph, &analysis, &settings, &ui);
        if sig == state.last_signature {
            return;
        }
        {
            let mut canvas = Canvas8::new(&mut state.frame);
            render_into(&mut canvas, &graph, &analysis, &settings, &ui);
        }
        upload_top_vram(MAIN_BG_VRAM as *mut u16, &state.frame, ui.theme);
        state.last_signature = sig;
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        let _ = (&graph, &analysis, &settings, &ui, &mut state);
    }
}

#[cfg(target_vendor = "nintendo")]
fn clear_top_vram(dst: *mut u16, theme: Theme) {
    // Only the visible 256x192 area needs clearing. VRAM B is dedicated to this
    // bitmap; all writes are 16-bit, which is valid for DS VRAM.
    unsafe {
        for i in 0..(TOP_WIDTH * TOP_HEIGHT) {
            core::ptr::write_volatile(dst.add(i), top_color(C_BG, theme));
        }
    }
}

#[cfg(target_vendor = "nintendo")]
fn upload_top_vram(dst: *mut u16, src: &[u8], theme: Theme) {
    debug_assert_eq!(src.len(), TOP_WIDTH * TOP_HEIGHT);
    unsafe {
        for (i, &idx) in src.iter().enumerate() {
            core::ptr::write_volatile(dst.add(i), top_color(idx, theme));
        }
    }
}

#[cfg(target_vendor = "nintendo")]
const fn top_color(idx: u8, theme: Theme) -> u16 {
    let (r, g, b) = match theme {
        Theme::Midnight => match idx {
            0 => (2, 3, 5),
            1 => (4, 6, 8),
            2 => (13, 21, 29),
            3 => (31, 8, 8),
            4 => (7, 20, 31),
            5 => (8, 31, 15),
            6 => (31, 31, 31),
            7 => (13, 13, 15),
            8 => (8, 31, 15),
            9 => (5, 18, 31),
            10 => (4, 26, 28),
            11 => (31, 10, 6),
            12 => (31, 12, 3),
            13 => (3, 5, 8),
            16 => (31, 18, 5),
            17 => (25, 8, 31),
            18 => (4, 26, 28),
            19 => (31, 10, 22),
            20 => (19, 31, 7),
            21 => (8, 14, 31),
            22 => (31, 28, 7),
            23 => (18, 10, 31),
            _ => (31, 31, 31),
        },
        Theme::Graphite => match idx {
            0 => (3, 3, 4),
            1 => (7, 7, 8),
            2 => (21, 21, 23),
            3 => (31, 8, 8),
            4 => (18, 18, 20),
            5 => (26, 26, 30),
            6 => (31, 31, 31),
            7 => (14, 14, 16),
            8 => (8, 28, 13),
            9 => (8, 16, 31),
            10 => (8, 25, 27),
            11 => (31, 8, 8),
            12 => (31, 14, 4),
            13 => (6, 6, 8),
            16 => (31, 19, 7),
            17 => (25, 12, 31),
            18 => (8, 25, 27),
            19 => (31, 12, 23),
            20 => (20, 29, 10),
            21 => (10, 17, 31),
            22 => (31, 28, 10),
            23 => (20, 13, 31),
            _ => (20, 20, 23),
        },
        Theme::Terminal => match idx {
            0 => (0, 2, 0),
            1 => (1, 6, 2),
            2 => (5, 20, 8),
            3 => (31, 7, 5),
            4 => (4, 25, 8),
            5 => (8, 31, 12),
            6 => (17, 31, 18),
            7 => (4, 14, 5),
            8 => (7, 31, 10),
            9 => (5, 22, 31),
            10 => (12, 31, 16),
            11 => (31, 7, 5),
            12 => (31, 16, 3),
            13 => (0, 5, 1),
            16 => (31, 20, 4),
            17 => (23, 10, 31),
            18 => (6, 28, 28),
            19 => (31, 9, 20),
            20 => (18, 31, 6),
            21 => (5, 17, 31),
            22 => (31, 27, 5),
            23 => (17, 9, 31),
            _ => (9, 27, 11),
        },
        Theme::Paper => match idx {
            0 => (28, 28, 26),
            1 => (22, 23, 22),
            2 => (5, 8, 12),
            3 => (26, 5, 5),
            4 => (7, 14, 21),
            5 => (3, 18, 21),
            6 => (2, 3, 4),
            7 => (12, 12, 12),
            8 => (4, 20, 8),
            9 => (4, 10, 25),
            10 => (3, 17, 20),
            11 => (26, 5, 5),
            12 => (28, 9, 2),
            13 => (25, 25, 23),
            16 => (25, 14, 3),
            17 => (18, 6, 24),
            18 => (2, 17, 20),
            19 => (24, 6, 16),
            20 => (12, 20, 3),
            21 => (4, 9, 24),
            22 => (24, 19, 3),
            23 => (12, 5, 23),
            _ => (12, 7, 20),
        },
    };
    0x8000 | rgb5(r, g, b)
}

// ---------------------------------------------------------------------------
// Nintendo DS hardware setup. VideoPlugin has already performed the safe
// libnds/calico bootstrap. The top physical LCD is the Main engine in that
// mapping, so this module owns only Main BG3 + VRAM B; VRAM A stays with BG0.
// ---------------------------------------------------------------------------

#[cfg(target_vendor = "nintendo")]
const REG_DISPCNT: *mut u32 = 0x0400_0000 as *mut u32;
#[cfg(target_vendor = "nintendo")]
const REG_VRAMCNT_B: *mut u8 = 0x0400_0241 as *mut u8;
#[cfg(target_vendor = "nintendo")]
const MAIN_BG_VRAM: usize = 0x0602_0000;
#[cfg(target_vendor = "nintendo")]
const VRAM_ENABLE: u8 = 0x80;
#[cfg(target_vendor = "nintendo")]
const VRAM_B_MAIN_BG_SLOT1: u8 = 1 | (1 << 3);
#[cfg(target_vendor = "nintendo")]
const REG_BG3CNT: *mut u16 = 0x0400_000E as *mut u16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PA: *mut i16 = 0x0400_0030 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PB: *mut i16 = 0x0400_0032 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PC: *mut i16 = 0x0400_0034 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3PD: *mut i16 = 0x0400_0036 as *mut i16;
#[cfg(target_vendor = "nintendo")]
const REG_BG3X: *mut i32 = 0x0400_0038 as *mut i32;
#[cfg(target_vendor = "nintendo")]
const REG_BG3Y: *mut i32 = 0x0400_003C as *mut i32;
#[cfg(target_vendor = "nintendo")]
const DISPLAY_BG3_ACTIVE: u32 = 1 << 11;
#[cfg(target_vendor = "nintendo")]
const BG_BMP16_256X256: u16 = (1 << 14) | (1 << 7) | (1 << 2);
#[cfg(target_vendor = "nintendo")]
const BG_BMP_BASE_8: u16 = 8 << 8;
#[cfg(target_vendor = "nintendo")]
const BG_PRIORITY_3: u16 = 3;
#[cfg(target_vendor = "nintendo")]
const fn rgb5(r: u16, g: u16, b: u16) -> u16 {
    (r & 31) | ((g & 31) << 5) | ((b & 31) << 10)
}

#[cfg(target_vendor = "nintendo")]
fn init_main_bitmap_top() {
    use core::ptr::{read_volatile, write_volatile};

    // VideoPlugin owns VRAM A + BG0 for the top console. Keep that allocation
    // intact. VRAM B is a separate 128 KiB bank and is mapped to main-BG slot 1
    // (0x06020000), matching libnds/bevy_nds_bg's bitmap layout.
    unsafe {
        write_volatile(REG_VRAMCNT_B, VRAM_ENABLE | VRAM_B_MAIN_BG_SLOT1);

        // Preserve display-mode/output bits and enabled BG0; change only the
        // graphics mode to 5 and enable BG3.
        let disp = read_volatile(REG_DISPCNT);
        write_volatile(REG_DISPCNT, (disp & !0x7) | 5 | DISPLAY_BG3_ACTIVE);

        // 256x256 16bpp bitmap, base 8 (= 8 * 16 KiB = 0x20000), priority 3.
        // BG0 therefore stays above the analysis layer for panic/debug output.
        write_volatile(REG_BG3CNT, BG_BMP16_256X256 | BG_BMP_BASE_8 | BG_PRIORITY_3);

        write_volatile(REG_BG3PA, 1 << 8);
        write_volatile(REG_BG3PB, 0);
        write_volatile(REG_BG3PC, 0);
        write_volatile(REG_BG3PD, 1 << 8);
        write_volatile(REG_BG3X, 0);
        write_volatile(REG_BG3Y, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnalysisState;
    #[test]
    fn draws_without_oob() {
        let mut pixels = [0u8; TOP_WIDTH * TOP_HEIGHT];
        let mut c = Canvas8::new(&mut pixels);
        c.line(-100, -100, 400, 300, C_TEXT);
        assert!(pixels.iter().any(|&p| p != 0));
    }
    #[test]
    fn original_renders() {
        let mut g = Digraph::new();
        let a = g.add_vertex(20, 30).unwrap();
        let b = g.add_vertex(60, 70).unwrap();
        g.add_edge(a, b, true);
        let mut pixels = [0u8; TOP_WIDTH * TOP_HEIGHT];
        let mut c = Canvas8::new(&mut pixels);
        render_into(
            &mut c,
            &g,
            &AnalysisState::default(),
            &TopViewSettings::default(),
            &UiState::default(),
        );
        assert!(pixels.iter().any(|&p| p != 0));
    }
}
