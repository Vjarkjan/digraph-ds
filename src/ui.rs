//! Bottom-screen command palette, graph-library dialogs, and viewport state.

extern crate alloc;

use alloc::{string::String, vec::Vec};
use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::analysis::{AnalysisKind, ComponentKind};

pub const MENU_TOP: i32 = 82;
pub const TAB_TOP: i32 = 86;
pub const TAB_BOTTOM: i32 = 101;
pub const ITEM_TOP: i32 = 106;
pub const ITEM_H: i32 = 23;
pub const ITEMS_PER_PAGE: usize = 6;
pub const ACTION_TOP: i32 = 158;
pub const ACTION_BOTTOM: i32 = 178;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Theme {
    Midnight,
    Graphite,
    Terminal,
    Paper,
}
impl Theme {
    pub const fn next(self) -> Self {
        match self {
            Self::Midnight => Self::Graphite,
            Self::Graphite => Self::Terminal,
            Self::Terminal => Self::Paper,
            Self::Paper => Self::Midnight,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Midnight => "MIDNIGHT",
            Self::Graphite => "GRAPHITE",
            Self::Terminal => "TERMINAL",
            Self::Paper => "PAPER",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MenuCategory {
    Vertex,
    Edge,
    Algorithms,
    Components,
    Graphs,
}
impl MenuCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertex => "VERT",
            Self::Edge => "EDGE",
            Self::Algorithms => "ALGO",
            Self::Components => "COMP",
            Self::Graphs => "GRAPH",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VertexTool {
    SelectMove,
    Add,
    Delete,
    SetSource,
    SetTarget,
    Pan,
}
impl VertexTool {
    pub const ALL: [Self; 6] = [
        Self::SelectMove,
        Self::Pan,
        Self::Add,
        Self::Delete,
        Self::SetSource,
        Self::SetTarget,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::SelectMove => "SELECT/MOVE",
            Self::Pan => "PAN VIEW",
            Self::Add => "ADD VERTEX",
            Self::Delete => "DELETE VERT",
            Self::SetSource => "SET SOURCE",
            Self::SetTarget => "SET TARGET",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeTool {
    Directed,
    Undirected,
    Delete,
    Reverse,
}
impl EdgeTool {
    pub const ALL: [Self; 4] = [
        Self::Directed,
        Self::Undirected,
        Self::Delete,
        Self::Reverse,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Directed => "DIRECTED",
            Self::Undirected => "UNDIRECTED",
            Self::Delete => "DELETE EDGE",
            Self::Reverse => "REVERSE EDGE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphAction {
    New,
    Complete,
    Random,
    RandomizeEdges,
    RandomizeOrientations,
    SwapView,
    Stats,
    SaveAs,
    Open,
}
impl GraphAction {
    pub const ALL: [Self; 9] = [
        Self::New,
        Self::Complete,
        Self::Random,
        Self::RandomizeEdges,
        Self::RandomizeOrientations,
        Self::SwapView,
        Self::Stats,
        Self::SaveAs,
        Self::Open,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "NEW EMPTY",
            Self::Complete => "COMPLETE K_n",
            Self::Random => "RANDOM G(n,p)",
            Self::RandomizeEdges => "RAND EDGES",
            Self::RandomizeOrientations => "RAND ORIENT",
            Self::SwapView => "SWAP VIEW",
            Self::Stats => "STATS",
            Self::SaveAs => "SAVE AS...",
            Self::Open => "OPEN SAVED...",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActiveTool {
    Vertex(VertexTool),
    Edge(EdgeTool),
}

fn default_pan() -> i16 {
    0
}
fn default_zoom() -> u8 {
    2
}
fn default_result_zoom() -> u8 {
    2
}
fn default_false() -> bool {
    false
}
#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub menu_open: bool,
    pub category: MenuCategory,
    pub cursor: usize,
    pub tool: ActiveTool,
    pub theme: Theme,
    /// Editor-camera zoom. Values 0..=5 preserve the previous scale table;
    /// higher values extend the range up to 128x for large graphs.
    #[serde(default = "default_zoom")]
    pub zoom: u8,
    /// Camera translation for the editable/original graph.
    #[serde(default = "default_pan")]
    pub pan_x: i16,
    #[serde(default = "default_pan")]
    pub pan_y: i16,
    /// Logical view swap. The physical LCD ownership does not change.
    #[serde(default = "default_false")]
    pub result_on_bottom: bool,
    /// Independent camera for the analysis/result canvas.
    #[serde(default = "default_result_zoom")]
    pub result_zoom: u8,
    #[serde(default = "default_pan")]
    pub result_pan_x: i16,
    #[serde(default = "default_pan")]
    pub result_pan_y: i16,
}
impl Default for UiState {
    fn default() -> Self {
        Self {
            menu_open: false,
            category: MenuCategory::Vertex,
            cursor: 0,
            tool: ActiveTool::Vertex(VertexTool::SelectMove),
            theme: Theme::Midnight,
            zoom: 2,
            pan_x: 0,
            pan_y: 0,
            result_on_bottom: false,
            result_zoom: 2,
            result_pan_x: 0,
            result_pan_y: 0,
        }
    }
}

impl UiState {
    pub fn category_len(&self) -> usize {
        match self.category {
            MenuCategory::Vertex => VertexTool::ALL.len(),
            MenuCategory::Edge => EdgeTool::ALL.len(),
            MenuCategory::Algorithms => AnalysisKind::ALL.len(),
            MenuCategory::Components => ComponentKind::ALL.len(),
            MenuCategory::Graphs => GraphAction::ALL.len(),
        }
    }
    pub fn clamp_cursor(&mut self) {
        let len = self.category_len();
        if len == 0 {
            self.cursor = 0
        } else if self.cursor >= len {
            self.cursor = len - 1
        }
    }
    pub fn open(&mut self, cat: MenuCategory) {
        self.menu_open = true;
        self.category = cat;
        self.cursor = 0;
    }
    pub fn move_left(&mut self) {
        let len = self.category_len();
        if len > 0 {
            self.cursor = if self.cursor == 0 {
                len - 1
            } else {
                self.cursor - 1
            };
        }
    }
    pub fn move_right(&mut self) {
        let len = self.category_len();
        if len > 0 {
            self.cursor = (self.cursor + 1) % len;
        }
    }
    pub fn page(&self) -> usize {
        self.cursor / ITEMS_PER_PAGE
    }
    pub fn page_start(&self) -> usize {
        self.page() * ITEMS_PER_PAGE
    }
    pub const MAX_ZOOM_INDEX: u8 = 15;
    pub const fn zoom_fraction_for(z: u8) -> (i32, i32) {
        match z {
            0 => (2, 3),
            1 => (4, 5),
            2 => (1, 1),
            3 => (5, 4),
            4 => (3, 2),
            5 => (2, 1),
            6 => (3, 1),
            7 => (4, 1),
            8 => (6, 1),
            9 => (8, 1),
            10 => (12, 1),
            11 => (16, 1),
            12 => (24, 1),
            13 => (32, 1),
            14 => (64, 1),
            _ => (128, 1),
        }
    }
    pub fn zoom_in(&mut self) {
        if self.result_on_bottom {
            self.result_zoom = self.result_zoom.saturating_add(1).min(Self::MAX_ZOOM_INDEX);
        } else {
            self.zoom = self.zoom.saturating_add(1).min(Self::MAX_ZOOM_INDEX);
        }
    }
    pub fn zoom_out(&mut self) {
        if self.result_on_bottom {
            self.result_zoom = self.result_zoom.saturating_sub(1);
        } else {
            self.zoom = self.zoom.saturating_sub(1);
        }
    }
    pub const fn zoom_fraction(&self) -> (i32, i32) {
        Self::zoom_fraction_for(self.zoom)
    }
    pub const fn result_zoom_fraction(&self) -> (i32, i32) {
        Self::zoom_fraction_for(self.result_zoom)
    }
    fn transform(x: i16, y: i16, zoom: u8, pan_x: i16, pan_y: i16) -> (i16, i16) {
        let (n, d) = Self::zoom_fraction_for(zoom);
        let cx = 128i32;
        let cy = 88i32;
        (
            ((cx + (i32::from(x) - cx) * n / d + i32::from(pan_x)).clamp(-32760, 32760)) as i16,
            ((cy + (i32::from(y) - cy) * n / d + i32::from(pan_y)).clamp(-32760, 32760)) as i16,
        )
    }
    fn inverse_transform(x: i16, y: i16, zoom: u8, pan_x: i16, pan_y: i16) -> (i16, i16) {
        let (n, d) = Self::zoom_fraction_for(zoom);
        let cx = 128i32;
        let cy = 88i32;
        (
            ((cx + (i32::from(x) - i32::from(pan_x) - cx) * d / n)
                .clamp(i16::MIN as i32 + 1, i16::MAX as i32 - 1)) as i16,
            ((cy + (i32::from(y) - i32::from(pan_y) - cy) * d / n)
                .clamp(i16::MIN as i32 + 1, i16::MAX as i32 - 1)) as i16,
        )
    }
    pub fn world_to_screen(&self, x: i16, y: i16) -> (i16, i16) {
        Self::transform(x, y, self.zoom, self.pan_x, self.pan_y)
    }
    pub fn screen_to_world(&self, x: i16, y: i16) -> (i16, i16) {
        Self::inverse_transform(x, y, self.zoom, self.pan_x, self.pan_y)
    }
    pub fn result_world_to_screen(&self, x: i16, y: i16) -> (i16, i16) {
        Self::transform(x, y, self.result_zoom, self.result_pan_x, self.result_pan_y)
    }
    pub fn result_screen_to_world(&self, x: i16, y: i16) -> (i16, i16) {
        Self::inverse_transform(x, y, self.result_zoom, self.result_pan_x, self.result_pan_y)
    }
    pub fn bottom_world_to_screen(&self, x: i16, y: i16) -> (i16, i16) {
        if self.result_on_bottom {
            self.result_world_to_screen(x, y)
        } else {
            self.world_to_screen(x, y)
        }
    }
    pub fn bottom_screen_to_world(&self, x: i16, y: i16) -> (i16, i16) {
        if self.result_on_bottom {
            self.result_screen_to_world(x, y)
        } else {
            self.screen_to_world(x, y)
        }
    }
    pub fn pan_by(&mut self, dx: i16, dy: i16) {
        if self.result_on_bottom {
            self.result_pan_x =
                (i32::from(self.result_pan_x) + i32::from(dx)).clamp(-30000, 30000) as i16;
            self.result_pan_y =
                (i32::from(self.result_pan_y) + i32::from(dy)).clamp(-30000, 30000) as i16;
        } else {
            self.pan_x = (i32::from(self.pan_x) + i32::from(dx)).clamp(-30000, 30000) as i16;
            self.pan_y = (i32::from(self.pan_y) + i32::from(dy)).clamp(-30000, 30000) as i16;
        }
    }
    pub fn reset_view(&mut self) {
        self.pan_x = 0;
        self.pan_y = 0;
        self.zoom = 2;
    }
    pub fn reset_result_view(&mut self) {
        self.result_pan_x = 0;
        self.result_pan_y = 0;
        self.result_zoom = 2;
    }
    pub fn item_label(&self, index: usize) -> Option<&'static str> {
        match self.category {
            MenuCategory::Vertex => VertexTool::ALL.get(index).map(|x| x.label()),
            MenuCategory::Edge => EdgeTool::ALL.get(index).map(|x| x.label()),
            MenuCategory::Algorithms => AnalysisKind::ALL.get(index).map(|x| x.code()),
            MenuCategory::Components => ComponentKind::ALL.get(index).map(|x| x.code()),
            MenuCategory::Graphs => GraphAction::ALL.get(index).map(|x| x.label()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphDialog {
    None,
    CompleteOrder,
    RandomSettings,
    SaveName,
    LoadList,
    Stats,
}

#[derive(Resource, Clone, Debug)]
pub struct GraphMenuState {
    pub dialog: GraphDialog,
    pub order: u8,
    pub density: u8,
    pub save_name: String,
    pub saved_names: Vec<String>,
    pub saved_cursor: usize,
    pub saved_page: usize,
    pub rng_state: u32,
    /// Scratch value used by the touch numeric keypad in graph dialogs.
    pub order_edit: u16,
    /// True until the first digit is entered; the first digit replaces the old value.
    pub order_edit_fresh: bool,
}
impl Default for GraphMenuState {
    fn default() -> Self {
        Self {
            dialog: GraphDialog::None,
            order: 8,
            density: 35,
            save_name: String::new(),
            saved_names: Vec::new(),
            saved_cursor: 0,
            saved_page: 0,
            rng_state: 0x9E37_79B9,
            order_edit: 8,
            order_edit_fresh: true,
        }
    }
}
impl GraphMenuState {
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if x == 0 {
            x = 0xA341_316C;
        }
        self.rng_state = x;
        x
    }
    pub fn bump_entropy(&mut self, x: u32) {
        self.rng_state ^= x.wrapping_mul(0x045D_9F3B);
        let _ = self.next_u32();
    }
    pub fn order_inc(&mut self) {
        self.order = self.order.saturating_add(1).min(200);
    }
    pub fn order_dec(&mut self) {
        self.order = self.order.saturating_sub(1).max(1);
    }
    pub fn density_inc(&mut self) {
        self.density = self.density.saturating_add(5).min(95);
    }
    pub fn density_dec(&mut self) {
        self.density = self.density.saturating_sub(5).max(5);
    }
    pub fn begin_order_entry(&mut self) {
        self.order_edit = u16::from(self.order);
        self.order_edit_fresh = true;
    }
    pub fn order_digit(&mut self, digit: u8) {
        let digit = u16::from(digit.min(9));
        let next = if self.order_edit_fresh {
            digit
        } else {
            self.order_edit.saturating_mul(10).saturating_add(digit)
        };
        // Keep the editor bounded to the application's graph-size limit.
        if next <= 200 {
            self.order_edit = next;
            self.order_edit_fresh = false;
        }
    }
    pub fn order_backspace(&mut self) {
        self.order_edit /= 10;
        self.order_edit_fresh = false;
    }
    pub fn commit_order_entry(&mut self) {
        self.order = self.order_edit.clamp(1, 200) as u8;
        self.order_edit = u16::from(self.order);
        self.order_edit_fresh = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchMenuAction {
    Category(MenuCategory),
    Item(usize),
    Theme,
    Save,
    Load,
    None,
}

pub fn hit_menu(x: i16, y: i16, ui: &UiState) -> TouchMenuAction {
    let x = i32::from(x);
    let y = i32::from(y);
    if (TAB_TOP..=TAB_BOTTOM).contains(&y) {
        let cat = match x {
            0..=50 => MenuCategory::Vertex,
            51..=101 => MenuCategory::Edge,
            102..=152 => MenuCategory::Algorithms,
            153..=203 => MenuCategory::Components,
            _ => MenuCategory::Graphs,
        };
        return TouchMenuAction::Category(cat);
    }
    if (ITEM_TOP..ACTION_TOP).contains(&y) {
        let col = (x / 85).clamp(0, 2) as usize;
        let row = ((y - ITEM_TOP) / ITEM_H).clamp(0, 1) as usize;
        let local = row * 3 + col;
        let idx = ui.page_start() + local;
        if idx < ui.category_len() {
            return TouchMenuAction::Item(idx);
        }
    }
    if (ACTION_TOP..=ACTION_BOTTOM).contains(&y) {
        return match x {
            0..=83 => TouchMenuAction::Theme,
            84..=169 => TouchMenuAction::Save,
            _ => TouchMenuAction::Load,
        };
    }
    TouchMenuAction::None
}

pub const NAME_KEYS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
pub fn name_key_at(x: i16, y: i16) -> Option<char> {
    let x = i32::from(x);
    let y = i32::from(y);
    if !(104..160).contains(&y) {
        return None;
    }
    let col = (x / 25).clamp(0, 9) as usize;
    let row = ((y - 104) / 14).clamp(0, 3) as usize;
    let idx = row * 10 + col;
    NAME_KEYS.get(idx).copied().map(char::from)
}
