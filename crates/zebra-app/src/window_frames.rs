//! Where document windows were, like Keynote: each document reopens where
//! its window was last, and Zebra opens at the size and place it was last
//! used. Kept in `~/Library/Application Support/Zebra/Window Frames.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Documents remembered at most (the least recently used go first).
const KEEP: usize = 200;

/// A window's place: its top-left corner (outer, in points), the size of
/// its content (points), and the size of the screen it was on, so a frame
/// from another screen doesn't put the window off the edge.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub screen: Option<[f32; 2]>,
}

impl Frame {
    /// The frame of a window as egui last saw it. None while it is full
    /// screen or minimised (Keynote keeps the frame from before).
    pub fn of(info: &egui::ViewportInfo) -> Option<Frame> {
        if info.fullscreen == Some(true) || info.minimized == Some(true) {
            return None;
        }
        let outer = info.outer_rect?;
        let inner = info.inner_rect?;
        if inner.width() < 1.0 || inner.height() < 1.0 {
            return None;
        }
        Some(Frame { pos: [outer.min.x, outer.min.y], size: [inner.width(), inner.height()], screen: info.monitor_size.map(|s| [s.x, s.y]) })
    }

    /// Where to put the window now: the size, no bigger than the screen,
    /// and the corner only if it is the same screen as before (else macOS
    /// places it).
    pub fn fit(&self, screen: Option<egui::Vec2>) -> (Option<egui::Pos2>, egui::Vec2) {
        let mut size = egui::vec2(self.size[0], self.size[1]);
        let mut pos = Some(egui::pos2(self.pos[0], self.pos[1]));
        if let Some(s) = screen {
            size = size.min(s);
            let same = self.screen.is_some_and(|f| (f[0] - s.x).abs() < 1.0 && (f[1] - s.y).abs() < 1.0);
            if !same {
                pos = None;
            }
        }
        (pos, size)
    }

    /// Close enough to `other` to count as the same place (macOS rounds).
    pub fn near(&self, other: &Frame) -> bool {
        let d = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 2.0 && (a[1] - b[1]).abs() < 2.0;
        d(self.pos, other.pos) && d(self.size, other.size)
    }
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Frames {
    /// The window used last, for Zebra's first window at launch.
    pub last: Option<Frame>,
    /// Saved documents, most recently used first.
    pub docs: Vec<(PathBuf, Frame)>,
}

impl Frames {
    pub fn get(&self, path: &Path) -> Option<Frame> {
        self.docs.iter().find(|(p, _)| p == path).map(|(_, f)| *f)
    }

    /// Note where a window is now. True if anything changed.
    pub fn remember(&mut self, path: Option<&Path>, f: Frame) -> bool {
        let mut changed = self.last != Some(f);
        self.last = Some(f);
        if let Some(p) = path {
            match self.docs.iter().position(|(q, _)| q == p) {
                Some(0) if self.docs[0].1 == f => {}
                Some(i) => {
                    self.docs.remove(i);
                    self.docs.insert(0, (p.to_path_buf(), f));
                    changed = true;
                }
                None => {
                    self.docs.insert(0, (p.to_path_buf(), f));
                    self.docs.truncate(KEEP);
                    changed = true;
                }
            }
        }
        changed
    }

    fn file() -> Option<PathBuf> {
        let d = dirs::data_dir()?.join("Zebra");
        std::fs::create_dir_all(&d).ok()?;
        Some(d.join("Window Frames.json"))
    }

    pub fn load() -> Frames {
        Self::file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn store(&self) {
        let Some(f) = Self::file() else { return };
        let Ok(b) = serde_json::to_vec(self) else { return };
        let tmp = f.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &f);
        }
    }
}

/// What the shell keeps per window: the document whose frame the window
/// was last put in or noted for, and a frame it was just sent to (noting
/// waits until the window gets there, or it would note the old place
/// under the new document).
#[derive(Default, Clone, Debug)]
pub struct Placement {
    pub path: Option<PathBuf>,
    pub settled: bool,
    pub going_to: Option<(Frame, std::time::Instant)>,
}

/// What to do with a window this frame.
#[derive(PartialEq, Debug)]
pub enum Step {
    Nothing,
    /// Move it here (a document with a remembered frame just opened in it).
    Place(Frame),
    /// Note this frame.
    Note(Frame),
}

/// Decide for one window. `path` is its document's file (None if
/// untitled), `now` its frame (None while full screen, minimised or not
/// yet shown), `may_move` false while it shares a tab group (moving one
/// tab moves them all).
pub fn step(p: &mut Placement, frames: &Frames, path: Option<&Path>, now: Option<Frame>, may_move: bool) -> Step {
    if p.path.as_deref() != path || !p.settled {
        p.path = path.map(Path::to_path_buf);
        p.settled = true;
        p.going_to = None;
        if let (Some(path), true) = (path, may_move) {
            if let Some(f) = frames.get(path) {
                if now.is_some_and(|n| n.near(&f)) {
                    return Step::Nothing;
                }
                p.going_to = Some((f, std::time::Instant::now()));
                return Step::Place(f);
            }
        }
    }
    let Some(now) = now else { return Step::Nothing };
    if let Some((f, at)) = p.going_to {
        // Wait for the window to get there; give up after a second (macOS
        // may have kept it on screen elsewhere).
        if !now.near(&f) && at.elapsed().as_secs_f32() < 1.0 {
            return Step::Nothing;
        }
        p.going_to = None;
    }
    Step::Note(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(x: f32, w: f32) -> Frame {
        Frame { pos: [x, 40.0], size: [w, 700.0], screen: Some([1512.0, 982.0]) }
    }

    #[test]
    fn a_document_reopens_where_it_was() {
        let mut frames = Frames::default();
        frames.remember(Some(Path::new("/a.zebra")), fr(100.0, 900.0));
        let mut p = Placement::default();
        let s = step(&mut p, &frames, Some(Path::new("/a.zebra")), Some(fr(0.0, 1440.0)), true);
        assert_eq!(s, Step::Place(fr(100.0, 900.0)));
        // Still at the old place next frame: not noted under a.zebra.
        assert_eq!(step(&mut p, &frames, Some(Path::new("/a.zebra")), Some(fr(0.0, 1440.0)), true), Step::Nothing);
        // Arrived: noted from then on.
        assert_eq!(step(&mut p, &frames, Some(Path::new("/a.zebra")), Some(fr(100.0, 900.0)), true), Step::Note(fr(100.0, 900.0)));
    }

    #[test]
    fn untitled_and_unknown_documents_stay_put_and_are_noted() {
        let frames = Frames::default();
        let mut p = Placement::default();
        assert_eq!(step(&mut p, &frames, None, Some(fr(0.0, 1440.0)), true), Step::Note(fr(0.0, 1440.0)));
        assert_eq!(step(&mut p, &frames, Some(Path::new("/new.zebra")), Some(fr(0.0, 1440.0)), true), Step::Note(fr(0.0, 1440.0)));
    }

    #[test]
    fn a_tab_is_not_moved() {
        let mut frames = Frames::default();
        frames.remember(Some(Path::new("/a.zebra")), fr(100.0, 900.0));
        let mut p = Placement::default();
        assert_eq!(step(&mut p, &frames, Some(Path::new("/a.zebra")), Some(fr(0.0, 1440.0)), false), Step::Note(fr(0.0, 1440.0)));
    }

    #[test]
    fn full_screen_is_not_noted() {
        let info = egui::ViewportInfo { fullscreen: Some(true), outer_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0))), inner_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0))), ..Default::default() };
        assert_eq!(Frame::of(&info), None);
    }

    #[test]
    fn another_screen_keeps_the_size_but_not_the_place() {
        let f = Frame { pos: [2000.0, 40.0], size: [1800.0, 1000.0], screen: Some([2560.0, 1440.0]) };
        let (pos, size) = f.fit(Some(egui::vec2(1512.0, 982.0)));
        assert_eq!(pos, None);
        assert_eq!(size, egui::vec2(1512.0, 982.0));
        let (pos, _) = f.fit(Some(egui::vec2(2560.0, 1440.0)));
        assert_eq!(pos, Some(egui::pos2(2000.0, 40.0)));
    }

    #[test]
    fn most_recent_documents_are_kept() {
        let mut frames = Frames::default();
        for i in 0..(KEEP + 5) {
            frames.remember(Some(&PathBuf::from(format!("/{i}.zebra"))), fr(i as f32, 900.0));
        }
        assert_eq!(frames.docs.len(), KEEP);
        assert!(frames.get(Path::new("/0.zebra")).is_none());
        assert!(frames.get(&PathBuf::from(format!("/{}.zebra", KEEP + 4))).is_some());
    }
}
