//! Versioned Postcard/Serde project snapshots and DS filesystem I/O.
//!
//! Storage is initialized explicitly with libnds `fatInitDefault()`. On DSi we
//! prefer `sd:/`; on DS / DLDI loaders we fall back to `fat:/`. Named projects
//! maintain a compact Postcard index so the app can browse them without relying
//! on a platform-specific Rust `dirent` layout.

extern crate alloc;

use alloc::{string::String, vec, vec::Vec};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{AnalysisKind, ComponentKind},
    graph::{Digraph, VertexId},
    top2d::TopViewSettings,
    ui::UiState,
};

const MAGIC: [u8; 4] = *b"DGEX";
const VERSION: u16 = 4;
const INDEX_MAGIC: [u8; 4] = *b"DGXI";
const INDEX_VERSION: u16 = 1;
pub const MAX_GRAPH_NAME: usize = 12;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedTopViewSettings {
    pub enabled: bool,
    pub grid: bool,
    pub show_removed: bool,
    pub show_original_when_idle: bool,
}
impl From<TopViewSettings> for SavedTopViewSettings {
    fn from(v: TopViewSettings) -> Self {
        Self {
            enabled: v.enabled,
            grid: v.grid,
            show_removed: v.show_removed,
            show_original_when_idle: v.show_original_when_idle,
        }
    }
}
impl From<SavedTopViewSettings> for TopViewSettings {
    fn from(v: SavedTopViewSettings) -> Self {
        Self {
            enabled: v.enabled,
            grid: v.grid,
            show_removed: v.show_removed,
            show_original_when_idle: v.show_original_when_idle,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    magic: [u8; 4],
    version: u16,
    pub graph: Digraph,
    pub selected_analysis: Option<AnalysisKind>,
    pub selected_component: Option<ComponentKind>,
    pub top_view: SavedTopViewSettings,
    pub ui: UiState,
    pub selected_vertex: Option<VertexId>,
    pub edge_start: Option<VertexId>,
    pub source: Option<VertexId>,
    pub target: Option<VertexId>,
}
impl ProjectSnapshot {
    pub fn capture(
        graph: &Digraph,
        selected_analysis: Option<AnalysisKind>,
        selected_component: Option<ComponentKind>,
        top_view: TopViewSettings,
        ui: &UiState,
        selected_vertex: Option<VertexId>,
        edge_start: Option<VertexId>,
        source: Option<VertexId>,
        target: Option<VertexId>,
    ) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            graph: graph.clone(),
            selected_analysis,
            selected_component,
            top_view: top_view.into(),
            ui: ui.clone(),
            selected_vertex,
            edge_start,
            source,
            target,
        }
    }
    fn validate(mut self) -> Result<Self, ProjectError> {
        if self.magic != MAGIC {
            return Err(ProjectError::BadMagic);
        }
        if self.version != VERSION {
            return Err(ProjectError::UnsupportedVersion(self.version));
        }
        self.graph.normalize_counters();
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectIndex {
    magic: [u8; 4],
    version: u16,
    names: Vec<String>,
}
impl Default for ProjectIndex {
    fn default() -> Self {
        Self {
            magic: INDEX_MAGIC,
            version: INDEX_VERSION,
            names: Vec::new(),
        }
    }
}
impl ProjectIndex {
    fn validate(self) -> Result<Self, ProjectError> {
        if self.magic != INDEX_MAGIC {
            return Err(ProjectError::BadMagic);
        }
        if self.version != INDEX_VERSION {
            return Err(ProjectError::UnsupportedVersion(self.version));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectError {
    Encode,
    Decode,
    BadMagic,
    UnsupportedVersion(u16),
    Io,
    TooLarge,
    FilesystemUnavailable,
    UnsupportedHost,
    InvalidName,
}

pub fn encode(snapshot: &ProjectSnapshot) -> Result<Vec<u8>, ProjectError> {
    postcard::to_allocvec(snapshot).map_err(|_| ProjectError::Encode)
}
pub fn decode(bytes: &[u8]) -> Result<ProjectSnapshot, ProjectError> {
    let snapshot: ProjectSnapshot =
        postcard::from_bytes(bytes).map_err(|_| ProjectError::Decode)?;
    snapshot.validate()
}

pub fn sanitize_name(input: &str) -> Result<String, ProjectError> {
    let mut out = String::new();
    for ch in input.chars() {
        if out.len() >= MAX_GRAPH_NAME {
            break;
        }
        let c = ch.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        }
    }
    if out.is_empty() {
        Err(ProjectError::InvalidName)
    } else {
        Ok(out)
    }
}

#[cfg(target_vendor = "nintendo")]
mod ds_io {
    use super::*;
    use alloc::{format, string::ToString};
    use core::ffi::{c_char, c_int, c_long, c_void};
    #[repr(C)]
    struct CFile {
        _private: [u8; 0],
    }
    unsafe extern "C" {
        fn fatInitDefault() -> bool;
        fn fopen(path: *const c_char, mode: *const c_char) -> *mut CFile;
        fn fclose(file: *mut CFile) -> c_int;
        fn fwrite(ptr: *const c_void, size: usize, count: usize, file: *mut CFile) -> usize;
        fn fread(ptr: *mut c_void, size: usize, count: usize, file: *mut CFile) -> usize;
        fn fseek(file: *mut CFile, offset: c_long, origin: c_int) -> c_int;
        fn ftell(file: *mut CFile) -> c_long;
        fn mkdir(path: *const c_char, mode: u32) -> c_int;
        fn fflush(file: *mut CFile) -> c_int;
    }
    const SEEK_SET: c_int = 0;
    const SEEK_END: c_int = 2;
    const MAX_PROJECT_BYTES: usize = 512 * 1024;
    const SD_GRAPH: &[u8] = b"sd:/graph\0";
    const SD_ANALYSIS: &[u8] = b"sd:/graph/analysis\0";
    const SD_FILE: &[u8] = b"sd:/graph/analysis/current.dgxp\0";
    const SD_INDEX: &[u8] = b"sd:/graph/analysis/graphs.dgxi\0";
    const FAT_GRAPH: &[u8] = b"fat:/graph\0";
    const FAT_ANALYSIS: &[u8] = b"fat:/graph/analysis\0";
    const FAT_FILE: &[u8] = b"fat:/graph/analysis/current.dgxp\0";
    const FAT_INDEX: &[u8] = b"fat:/graph/analysis/graphs.dgxi\0";
    #[derive(Clone, Copy)]
    enum Drive {
        Sd,
        Fat,
    }
    #[inline]
    fn cptr(b: &[u8]) -> *const c_char {
        b.as_ptr().cast()
    }
    fn init() -> Result<(), ProjectError> {
        unsafe {
            if fatInitDefault() {
                Ok(())
            } else {
                Err(ProjectError::FilesystemUnavailable)
            }
        }
    }
    fn ensure_dirs(graph: &[u8], analysis: &[u8]) {
        unsafe {
            let _ = mkdir(cptr(graph), 0o777);
            let _ = mkdir(cptr(analysis), 0o777);
        }
    }
    fn c_path(s: String) -> Vec<u8> {
        let mut v = s.into_bytes();
        v.push(0);
        v
    }
    fn named_path(drive: Drive, name: &str) -> Vec<u8> {
        match drive {
            Drive::Sd => c_path(format!("sd:/graph/analysis/{name}.dgxp")),
            Drive::Fat => c_path(format!("fat:/graph/analysis/{name}.dgxp")),
        }
    }
    fn dirs(drive: Drive) -> (&'static [u8], &'static [u8]) {
        match drive {
            Drive::Sd => (SD_GRAPH, SD_ANALYSIS),
            Drive::Fat => (FAT_GRAPH, FAT_ANALYSIS),
        }
    }
    fn index_path(drive: Drive) -> &'static [u8] {
        match drive {
            Drive::Sd => SD_INDEX,
            Drive::Fat => FAT_INDEX,
        }
    }
    unsafe fn try_write(path: &[u8], graph_dir: &[u8], analysis_dir: &[u8], bytes: &[u8]) -> bool {
        ensure_dirs(graph_dir, analysis_dir);
        let mode = b"wb\0";
        let file = unsafe { fopen(cptr(path), cptr(mode)) };
        if file.is_null() {
            return false;
        }
        let wrote = unsafe { fwrite(bytes.as_ptr().cast(), 1, bytes.len(), file) };
        let _ = unsafe { fflush(file) };
        let close_ok = unsafe { fclose(file) } == 0;
        wrote == bytes.len() && close_ok
    }
    unsafe fn try_read(path: &[u8]) -> Option<Result<Vec<u8>, ProjectError>> {
        let mode = b"rb\0";
        let file = unsafe { fopen(cptr(path), cptr(mode)) };
        if file.is_null() {
            return None;
        }
        if unsafe { fseek(file, 0, SEEK_END) } != 0 {
            let _ = unsafe { fclose(file) };
            return Some(Err(ProjectError::Io));
        }
        let size = unsafe { ftell(file) };
        if size < 0 || size as usize > MAX_PROJECT_BYTES {
            let _ = unsafe { fclose(file) };
            return Some(Err(ProjectError::TooLarge));
        }
        if unsafe { fseek(file, 0, SEEK_SET) } != 0 {
            let _ = unsafe { fclose(file) };
            return Some(Err(ProjectError::Io));
        }
        let mut out = vec![0u8; size as usize];
        let got = unsafe { fread(out.as_mut_ptr().cast(), 1, out.len(), file) };
        let close_ok = unsafe { fclose(file) } == 0;
        Some(if got == out.len() && close_ok {
            Ok(out)
        } else {
            Err(ProjectError::Io)
        })
    }
    pub(super) fn write_default(bytes: &[u8]) -> Result<(), ProjectError> {
        init()?;
        unsafe {
            if try_write(SD_FILE, SD_GRAPH, SD_ANALYSIS, bytes)
                || try_write(FAT_FILE, FAT_GRAPH, FAT_ANALYSIS, bytes)
            {
                Ok(())
            } else {
                Err(ProjectError::Io)
            }
        }
    }
    pub(super) fn read_default() -> Result<Vec<u8>, ProjectError> {
        init()?;
        unsafe {
            if let Some(x) = try_read(SD_FILE) {
                return x;
            }
            if let Some(x) = try_read(FAT_FILE) {
                return x;
            }
        }
        Err(ProjectError::Io)
    }
    fn read_index_drive(drive: Drive) -> ProjectIndex {
        unsafe {
            match try_read(index_path(drive)) {
                Some(Ok(bytes)) => postcard::from_bytes::<ProjectIndex>(&bytes)
                    .ok()
                    .and_then(|x| x.validate().ok())
                    .unwrap_or_default(),
                _ => ProjectIndex::default(),
            }
        }
    }
    fn write_index_drive(drive: Drive, index: &ProjectIndex) -> bool {
        let Ok(bytes) = postcard::to_allocvec(index) else {
            return false;
        };
        let (g, a) = dirs(drive);
        unsafe { try_write(index_path(drive), g, a, &bytes) }
    }
    pub(super) fn write_named(name: &str, bytes: &[u8]) -> Result<(), ProjectError> {
        init()?;
        let mut target = None;
        for drive in [Drive::Sd, Drive::Fat] {
            let (g, a) = dirs(drive);
            let path = named_path(drive, name);
            if unsafe { try_write(&path, g, a, bytes) } {
                target = Some(drive);
                break;
            }
        }
        let Some(drive) = target else {
            return Err(ProjectError::Io);
        };
        let mut idx = read_index_drive(drive);
        if !idx.names.iter().any(|x| x == name) {
            idx.names.push(name.to_string());
            idx.names.sort();
            idx.names.dedup();
        }
        let _ = write_index_drive(drive, &idx);
        Ok(())
    }
    pub(super) fn read_named(name: &str) -> Result<Vec<u8>, ProjectError> {
        init()?;
        for drive in [Drive::Sd, Drive::Fat] {
            let path = named_path(drive, name);
            unsafe {
                if let Some(x) = try_read(&path) {
                    return x;
                }
            }
        }
        Err(ProjectError::Io)
    }
    pub(super) fn list_names() -> Result<Vec<String>, ProjectError> {
        init()?;
        let mut names = read_index_drive(Drive::Sd).names;
        for n in read_index_drive(Drive::Fat).names {
            if !names.iter().any(|x| x == &n) {
                names.push(n);
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }
    pub(super) fn probe() -> bool {
        init().is_ok()
    }
}

pub fn filesystem_available() -> bool {
    #[cfg(target_vendor = "nintendo")]
    {
        ds_io::probe()
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        false
    }
}
pub fn save_default(snapshot: &ProjectSnapshot) -> Result<(), ProjectError> {
    let bytes = encode(snapshot)?;
    #[cfg(target_vendor = "nintendo")]
    {
        ds_io::write_default(&bytes)
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        {
            let _ = bytes;
            Err(ProjectError::UnsupportedHost)
        }
    }
}
pub fn load_default() -> Result<ProjectSnapshot, ProjectError> {
    #[cfg(target_vendor = "nintendo")]
    {
        let bytes = ds_io::read_default()?;
        decode(&bytes)
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        Err(ProjectError::UnsupportedHost)
    }
}
pub fn save_named(snapshot: &ProjectSnapshot, name: &str) -> Result<String, ProjectError> {
    let safe = sanitize_name(name)?;
    let bytes = encode(snapshot)?;
    #[cfg(target_vendor = "nintendo")]
    {
        ds_io::write_named(&safe, &bytes)?;
        Ok(safe)
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        {
            let _ = bytes;
            Err(ProjectError::UnsupportedHost)
        }
    }
}
pub fn load_named(name: &str) -> Result<ProjectSnapshot, ProjectError> {
    let safe = sanitize_name(name)?;
    #[cfg(target_vendor = "nintendo")]
    {
        let bytes = ds_io::read_named(&safe)?;
        decode(&bytes)
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        Err(ProjectError::UnsupportedHost)
    }
}
pub fn list_saved_names() -> Result<Vec<String>, ProjectError> {
    #[cfg(target_vendor = "nintendo")]
    {
        ds_io::list_names()
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        Err(ProjectError::UnsupportedHost)
    }
}
