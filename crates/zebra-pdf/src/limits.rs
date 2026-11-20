//! Every bound on the PDF helper, in one place.
use std::time::Duration;

/// Memory one helper may use (its footprint, see [`crate::sys::footprint`])
/// before the app kills it.
pub const BUDGET: u64 = 1536 * 1024 * 1024;
/// The helper ends itself this far past the budget, in case the app
/// can't read its memory (the sandbox) or is too slow to.
pub const SELF_MARGIN: u64 = 256 * 1024 * 1024;
/// Opening a PDF and counting its pages (a damaged cross-reference table
/// makes PDFium read the whole file).
pub const OPEN_DEADLINE: Duration = Duration::from_secs(20);
/// One page's preview at import (and the page sizes).
pub const PAGE_DEADLINE: Duration = Duration::from_secs(10);
/// One part of a page drawn for the canvas, the presenter or an export.
pub const RECT_DEADLINE: Duration = Duration::from_secs(10);
/// The helper ends itself this long after a deadline the app missed.
pub const SELF_GRACE: Duration = Duration::from_secs(2);
/// How often the app looks at a busy helper's memory and the clock.
pub const POLL: Duration = Duration::from_millis(25);
/// A helper unused this long is replaced on the next request (so the
/// app never talks to one about to leave)...
pub const IDLE: Duration = Duration::from_secs(60);
/// ...and leaves by itself after this long with nothing to do.
pub const SELF_IDLE: Duration = Duration::from_secs(90);
/// Wait before starting a helper again after `deaths` in a row:
/// 0, 0.5, 1, 2, 4 … 30 s.
pub fn backoff(deaths: u32) -> Duration {
    match deaths {
        0 | 1 => Duration::ZERO,
        n => Duration::from_millis(500u64.saturating_mul(1 << (n - 2).min(10))).min(MAX_BACKOFF),
    }
}
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Biggest part of a page drawn in one go, pixels per side.
pub const MAX_RECT_PX: u32 = 4096;
/// Widest and tallest whole page drawn.
pub const MAX_PAGE_PX: (u32, u32) = (1 << 17, 1 << 19);
/// Characters of a page's first line kept.
pub const TEXT_CHARS: usize = 120;

/// The bounds a pool works to: [`Limits::default`] in the app; tests
/// make them smaller.
#[derive(Debug, Clone)]
pub struct Limits {
    pub budget: u64,
    pub open: Duration,
    pub page: Duration,
    pub rect: Duration,
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { budget: BUDGET, open: OPEN_DEADLINE, page: PAGE_DEADLINE, rect: RECT_DEADLINE, idle: IDLE }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backoff_grows_and_stops_at_30s() {
        let v: Vec<u64> = (0..12).map(|n| super::backoff(n).as_millis() as u64).collect();
        assert_eq!(&v[..7], &[0, 0, 500, 1000, 2000, 4000, 8000]);
        assert_eq!(v[11], 30_000);
        assert_eq!(super::backoff(u32::MAX), super::MAX_BACKOFF);
    }
}
