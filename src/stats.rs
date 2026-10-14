//! Lightweight runtime statistics for the on-device diagnostics panel.
//!
//! `cpu_load_tenths` is an estimate of ARM9 application load: time spent
//! inside one Bevy `app.update()` divided by the start-to-start frame period.
//! The runtime waits for VBlank between `app.update()` calls, so this is a
//! useful measure of how much of the available frame budget the app consumes.

use bevy_app::{App, First, Last, Plugin, Startup};
use bevy_ecs::prelude::*;

const BUS_CLOCK_HZ: u64 = 33_513_982;
const SYSTEM_COUNTER_HZ: u64 = BUS_CLOCK_HZ / 64;
const SAMPLE_EVERY_FRAMES: u8 = 15;
const MAIN_RAM_START: usize = 0x0200_0000;

#[derive(Resource, Clone, Debug, Default)]
pub struct StatsState {
    frame_start_ticks: u64,
    previous_frame_start_ticks: u64,
    sample_divider: u8,
    pub sample_seq: u32,
    /// Approximate ARM9 application load, in tenths of one percent.
    pub cpu_load_tenths: u16,
    /// Smoothed frames per second, in tenths of one FPS.
    pub fps_tenths: u16,
    /// Time spent inside the most recently sampled app frame, in microseconds.
    pub frame_work_us: u32,
    /// Start-to-start frame period, in microseconds.
    pub frame_period_us: u32,
    /// Approximate main-RAM bytes committed from 0x02000000 through heap end.
    pub ram_used_bytes: u32,
    /// Main-RAM headroom between heap end and heap limit.
    pub ram_free_bytes: u32,
    /// Bytes currently held by live malloc/Rust allocations (`mallinfo.uordblks`).
    pub heap_used_bytes: u32,
    /// Current heap address-space span (`getHeapEnd - getHeapStart`). This may
    /// stay high after frees because the allocator can keep free blocks for reuse.
    pub heap_span_bytes: u32,
    /// Free bytes already inside the allocator arena (`mallinfo.fordblks`).
    pub heap_free_blocks_bytes: u32,
    /// Maximum heap extent (`getHeapLimit - getHeapStart`).
    pub heap_capacity_bytes: u32,
    /// True when the available main-RAM range is larger than NTR-mode RAM.
    pub dsi_mode: bool,
}

pub struct StatsPlugin;
impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StatsState>()
            .add_systems(Startup, stats_setup)
            .add_systems(First, stats_frame_begin)
            .add_systems(Last, stats_frame_end);
    }
}

#[inline]
fn ticks_to_us(ticks: u64) -> u32 {
    ((ticks.saturating_mul(1_000_000)) / SYSTEM_COUNTER_HZ).min(u64::from(u32::MAX)) as u32
}

#[inline]
fn smooth(old: u16, new: u16) -> u16 {
    if old == 0 {
        new
    } else {
        ((u32::from(old) * 7 + u32::from(new) * 3) / 10) as u16
    }
}

#[cfg(target_vendor = "nintendo")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MallInfo {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

#[cfg(target_vendor = "nintendo")]
unsafe extern "C" {
    fn systemCounterSetup();
    fn systemCounterGetTicks() -> u64;
    fn getHeapStart() -> *mut u8;
    fn getHeapEnd() -> *mut u8;
    fn getHeapLimit() -> *mut u8;
    fn mallinfo() -> MallInfo;
}

#[cfg(target_vendor = "nintendo")]
#[inline]
fn now_ticks() -> u64 {
    unsafe { systemCounterGetTicks() }
}
#[cfg(not(target_vendor = "nintendo"))]
#[inline]
fn now_ticks() -> u64 {
    0
}

fn stats_setup(mut stats: ResMut<StatsState>) {
    #[cfg(target_vendor = "nintendo")]
    unsafe {
        systemCounterSetup();
        let now = systemCounterGetTicks();
        stats.frame_start_ticks = now;
        stats.previous_frame_start_ticks = now;
    }
    refresh_memory(&mut stats);
}

fn stats_frame_begin(mut stats: ResMut<StatsState>) {
    let now = now_ticks();
    if now == 0 {
        return;
    }
    if stats.previous_frame_start_ticks != 0 {
        let period_ticks = now.saturating_sub(stats.previous_frame_start_ticks);
        let period_us = ticks_to_us(period_ticks).max(1);
        stats.frame_period_us = period_us;
        let fps_tenths = (10_000_000u64 / u64::from(period_us)).min(9_999) as u16;
        stats.fps_tenths = smooth(stats.fps_tenths, fps_tenths);
    }
    stats.previous_frame_start_ticks = now;
    stats.frame_start_ticks = now;
}

fn stats_frame_end(mut stats: ResMut<StatsState>) {
    let now = now_ticks();
    if now != 0 && stats.frame_start_ticks != 0 {
        let work_us = ticks_to_us(now.saturating_sub(stats.frame_start_ticks));
        stats.frame_work_us = work_us;
        if stats.frame_period_us > 0 {
            let load =
                ((u64::from(work_us) * 1_000) / u64::from(stats.frame_period_us)).min(1_000) as u16;
            stats.cpu_load_tenths = smooth(stats.cpu_load_tenths, load);
        }
    }
    stats.sample_divider = stats.sample_divider.saturating_add(1);
    if stats.sample_divider >= SAMPLE_EVERY_FRAMES {
        stats.sample_divider = 0;
        refresh_memory(&mut stats);
        stats.sample_seq = stats.sample_seq.wrapping_add(1);
    }
}

fn refresh_memory(stats: &mut StatsState) {
    #[cfg(target_vendor = "nintendo")]
    unsafe {
        let start = getHeapStart() as usize;
        let end = getHeapEnd() as usize;
        let limit = getHeapLimit() as usize;
        if start >= MAIN_RAM_START && end >= start && limit >= end {
            let mi = mallinfo();
            let heap_capacity = limit.saturating_sub(start);
            let live_heap = mi.uordblks.min(heap_capacity);
            let static_before_heap = start.saturating_sub(MAIN_RAM_START);

            // getHeapEnd() reports the current arena span, not the amount of
            // live data inside that arena. mallinfo().uordblks is the useful
            // live-allocation number; fordblks is memory already freed and
            // reusable without growing the heap again.
            stats.heap_used_bytes = live_heap.min(u32::MAX as usize) as u32;
            stats.heap_span_bytes = end.saturating_sub(start).min(u32::MAX as usize) as u32;
            stats.heap_free_blocks_bytes = mi.fordblks.min(u32::MAX as usize) as u32;
            stats.heap_capacity_bytes = heap_capacity.min(u32::MAX as usize) as u32;

            // Approximate live main-RAM footprint: static/data before the heap
            // plus currently live allocator blocks. This intentionally does not
            // count free allocator arena as "used" RAM because it is reusable.
            stats.ram_used_bytes = static_before_heap
                .saturating_add(live_heap)
                .min(u32::MAX as usize) as u32;
            stats.ram_free_bytes = heap_capacity
                .saturating_sub(live_heap)
                .min(u32::MAX as usize) as u32;
            // NTR mode exposes roughly 4 MiB. A larger heap limit means TWL/DSi RAM is available.
            stats.dsi_mode = limit > 0x0240_0000;
        }
    }
    #[cfg(not(target_vendor = "nintendo"))]
    {
        let _ = stats;
    }
}
