//! Digraph DS: graph editing, analysis, and two-screen presentation.
#![cfg_attr(target_vendor = "nintendo", no_std)]
// Graph algorithms use shared indices; editor systems expose their resources.
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_range_loop
)]
#![cfg_attr(
    not(target_vendor = "nintendo"),
    allow(dead_code, unused_imports, unused_variables)
)]

extern crate alloc;

pub mod analysis;
pub mod bottom2d;
pub mod dag;
pub mod editor;
pub mod graph;
pub mod project;
pub mod stats;
pub mod top2d;
pub mod ui;

/// Programming guide, with examples checked by `cargo test --doc`.
#[doc = include_str!("../Doc/README.md")]
pub mod guide {}

use bevy_app::prelude::*;
use bevy_ecs::{prelude::*, schedule::IntoScheduleConfigs};
use devy::{HardwareVideoPlugin, InputPlugin};

use analysis::{AnalysisState, refresh_analysis};
use bottom2d::BottomViewPlugin;
use editor::{EditorState, editor_input};
use graph::Digraph;
use stats::StatsPlugin;
use top2d::TopViewPlugin;
use ui::{GraphMenuState, UiState};

/// Assemble the editor. The host backends allow headless execution and tests.
/// On DS, pass this app to `devy::run` for VBlank-paced execution.
pub fn build_app() -> App {
    let mut app = App::new();
    // The graph renderers own their VRAM regions. Storage mounts on save/load.
    app.add_plugins((HardwareVideoPlugin, InputPlugin))
        .init_resource::<Digraph>()
        .init_resource::<EditorState>()
        .init_resource::<AnalysisState>()
        .init_resource::<UiState>()
        .init_resource::<GraphMenuState>()
        .add_plugins((BottomViewPlugin, TopViewPlugin, StatsPlugin))
        .add_systems(Startup, seed_graph)
        .add_systems(Update, (editor_input, refresh_analysis).chain());
    app
}

fn seed_graph(mut graph: ResMut<Digraph>) {
    if !graph.vertices.is_empty() {
        return;
    }
    let a = graph.add_vertex(38, 46).unwrap();
    let b = graph.add_vertex(108, 32).unwrap();
    let c = graph.add_vertex(174, 64).unwrap();
    let d = graph.add_vertex(196, 126).unwrap();
    let e = graph.add_vertex(112, 142).unwrap();
    graph.add_edge(a, b, true);
    graph.add_edge(b, c, true);
    graph.add_edge(c, a, true);
    graph.add_edge(c, d, true);
    graph.add_edge(d, e, true);
    graph.add_edge(e, c, true);
}
