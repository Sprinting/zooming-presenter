//! Memory and speed measurements for the `perf` query.
//!
//! * Heap: [`CountingAlloc`] wraps the system allocator and keeps a running
//!   total in an atomic. The app binary installs it as its global
//!   allocator; elsewhere (tests, libraries) `heap` reads as unavailable.
//! * Frame times and repaint rate: the host calls
//!   [`PerfStats::record_frame`] once per drawn frame.
//! * Cache sizes: the host fills [`PerfStats::caches`] (renderer image and
//!   text caches, thumbnails...).
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

use serde_json::{json, Value};

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// A global allocator that counts live heap bytes. Costs two relaxed
/// atomic operations per allocation.
pub struct CountingAlloc;

impl CountingAlloc {
    fn add(n: usize) {
        let now = ALLOCATED.fetch_add(n, Relaxed) + n;
        if now > PEAK.load(Relaxed) {
            PEAK.store(now, Relaxed);
        }
        if !INSTALLED.load(Relaxed) {
            INSTALLED.store(true, Relaxed);
        }
    }
}

// SAFETY: forwards every call to `System` unchanged; only counts sizes.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            Self::add(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            Self::add(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        ALLOCATED.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            ALLOCATED.fetch_sub(l.size(), Relaxed);
            Self::add(new);
        }
        q
    }
}

/// Live heap bytes, if [`CountingAlloc`] is the global allocator.
pub fn heap_bytes() -> Option<usize> {
    INSTALLED.load(Relaxed).then(|| ALLOCATED.load(Relaxed))
}

/// Highest heap seen since launch.
pub fn heap_peak() -> Option<usize> {
    INSTALLED.load(Relaxed).then(|| PEAK.load(Relaxed))
}

const FRAMES_KEPT: usize = 600;

/// Frame and cache statistics, kept by the host.
#[derive(Debug, Default, Clone)]
pub struct PerfStats {
    /// (time drawn, CPU milliseconds spent building the frame).
    frames: VecDeque<(f64, f32)>,
    /// Bytes or counts per cache, filled by the host each frame.
    pub caches: BTreeMap<String, u64>,
}

impl PerfStats {
    pub fn record_frame(&mut self, now: f64, ms: f32) {
        self.frames.push_back((now, ms));
        while self.frames.len() > FRAMES_KEPT {
            self.frames.pop_front();
        }
    }

    /// Frames drawn in the last `window` seconds before `now`.
    pub fn repaints_in(&self, now: f64, window: f64) -> usize {
        self.frames.iter().rev().take_while(|(t, _)| now - *t <= window).count()
    }

    /// Frame time percentile (0..=100) over the kept frames, ms.
    pub fn frame_ms_percentile(&self, pct: f64) -> Option<f32> {
        if self.frames.is_empty() {
            return None;
        }
        let mut v: Vec<f32> = self.frames.iter().map(|f| f.1).collect();
        v.sort_by(f32::total_cmp);
        let i = ((pct / 100.0) * (v.len() - 1) as f64).round() as usize;
        Some(v[i.min(v.len() - 1)])
    }

    pub fn report(&self, now: f64) -> Value {
        let mb = |b: usize| (b as f64 / 1_048_576.0 * 10.0).round() / 10.0;
        json!({
            "heap_mb": heap_bytes().map(mb),
            "heap_peak_mb": heap_peak().map(mb),
            "frame_ms_p50": self.frame_ms_percentile(50.0),
            "frame_ms_p95": self.frame_ms_percentile(95.0),
            "frames_sampled": self.frames.len(),
            "repaints_last_second": self.repaints_in(now, 1.0),
            "repaints_last_10s": self.repaints_in(now, 10.0),
            "caches": self.caches,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_repaint_window() {
        let mut p = PerfStats::default();
        for i in 0..100 {
            p.record_frame(i as f64 * 0.1, i as f32);
        }
        assert_eq!(p.frame_ms_percentile(50.0), Some(50.0));
        assert_eq!(p.frame_ms_percentile(95.0), Some(94.0));
        // Frames at 9.0..=9.9: 10 in the last second (plus the boundary one).
        assert!((10..=11).contains(&p.repaints_in(9.9, 1.0)));
        assert_eq!(p.repaints_in(100.0, 1.0), 0);
    }
}
