//! Path sidebar thumbnails, rendered off the UI thread with the CPU renderer
//! and cached by document revision (debounced so typing never stalls).
use std::sync::mpsc::{channel, Receiver, Sender};

use egui::{ColorImage, TextureHandle, TextureOptions};
use zebra_core::{Document, Editor};

pub const THUMB_W: u32 = 288;
pub const THUMB_H: u32 = 180;

struct Job {
    rev: u64,
    doc: Document,
}

/// The step whose drawing fails, in tests.
#[cfg(test)]
static FAIL_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Draw step `i`.
fn render(i: usize, ed: &Editor, renderer: &zebra_render::Renderer) -> Option<ColorImage> {
    #[cfg(test)]
    if FAIL_STEP.load(std::sync::atomic::Ordering::Relaxed) == i {
        panic!("test: thumbnail {i}");
    }
    let target = zebra_render::CaptureTarget::Step { index: i, build: None };
    let png = renderer.capture(ed, &target, THUMB_W, THUMB_H).ok()?;
    let img = image::load_from_memory(&png).ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

pub struct Thumbs {
    tx: Sender<Job>,
    rx: Receiver<(u64, Vec<Option<ColorImage>>)>,
    pub textures: Vec<Option<TextureHandle>>,
    /// Revision the textures show.
    pub shown_rev: u64,
    sent_rev: u64,
    seen_rev: u64,
    seen_at: f64,
    ctx: egui::Context,
}

impl Thumbs {
    pub fn new(ctx: egui::Context, renderer: std::sync::Arc<zebra_render::Renderer>) -> Self {
        let (tx, jrx) = channel::<Job>();
        let (rtx, rx) = channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("zebra-thumbs".into())
            .spawn(move || {
                while let Ok(mut job) = jrx.recv() {
                    // Coalesce: only render the newest request.
                    while let Ok(j) = jrx.try_recv() {
                        job = j;
                    }
                    let mut ed = Editor::new(Document::default());
                    ed.measure = renderer.measure();
                    ed.load(job.doc, None);
                    ed.view.viewport = zebra_core::Vec2::new(THUMB_W as f64, THUMB_H as f64);
                    let n = ed.doc.path.len();
                    // One step that can't be drawn (a panic in the renderer)
                    // leaves that thumbnail empty; it used to end this thread,
                    // and no thumbnail was ever drawn again.
                    let imgs: Vec<Option<ColorImage>> = (0..n)
                        .map(|i| {
                            let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(i, &ed, &renderer)));
                            match drawn {
                                Ok(img) => img,
                                Err(_) => {
                                    eprintln!("zebra: the thumbnail for step {} could not be drawn", i + 1);
                                    None
                                }
                            }
                        })
                        .collect();
                    if rtx.send((job.rev, imgs)).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            })
            .expect("spawn thumbnail thread");
        Thumbs { tx, rx, textures: vec![], shown_rev: u64::MAX, sent_rev: u64::MAX, seen_rev: u64::MAX, seen_at: 0.0, ctx }
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        self.shown_rev = u64::MAX;
        self.sent_rev = u64::MAX;
    }

    /// Call every frame.
    pub fn update(&mut self, ed: &Editor, now: f64) {
        while let Ok((rev, imgs)) = self.rx.try_recv() {
            self.textures = imgs
                .into_iter()
                .enumerate()
                .map(|(i, img)| img.map(|im| self.ctx.load_texture(format!("thumb-{i}"), im, TextureOptions::LINEAR)))
                .collect();
            self.shown_rev = rev;
        }
        if ed.revision != self.seen_rev {
            self.seen_rev = ed.revision;
            self.seen_at = now;
        }
        // Debounce: wait until edits pause (or immediately for the first set).
        let quiet = now - self.seen_at > 0.6 || self.sent_rev == u64::MAX;
        if self.sent_rev != ed.revision && quiet && !ed.human_gesture_active() {
            self.sent_rev = ed.revision;
            if !ed.doc.path.is_empty() {
                let _ = self.tx.send(Job { rev: ed.revision, doc: ed.doc.clone() });
            } else {
                self.textures.clear();
                self.shown_rev = ed.revision;
            }
        } else if self.sent_rev != ed.revision {
            // Wake once the edit burst has gone quiet, not on a poll.
            self.ctx.request_repaint_after(std::time::Duration::from_secs_f64((0.6 - (now - self.seen_at)).max(0.05)));
        }
    }

    pub fn get(&self, i: usize) -> Option<&TextureHandle> {
        self.textures.get(i).and_then(|t| t.as_ref())
    }

    pub fn stale(&self, rev: u64) -> bool {
        self.shown_rev != rev
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zebra_core::{Command, Origin};

    fn wait(t: &mut Thumbs, ed: &Editor) {
        for i in 0..600 {
            t.update(ed, 100.0 + i as f64);
            if !t.stale(ed.revision) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("no thumbnails arrived: the thread is gone");
    }

    #[test]
    fn a_step_that_cannot_be_drawn_does_not_end_the_thumbnails() {
        let renderer = std::sync::Arc::new(zebra_render::Renderer::new());
        let mut ed = Editor::new(Document::default());
        ed.measure = renderer.measure();
        for x in [0.0, 500.0] {
            let add: Command = serde_json::from_value(serde_json::json!({ "op": "add_frame", "x": x, "y": 0.0, "w": 300.0, "h": 200.0, "add_to_path": true })).unwrap();
            ed.apply(add, Origin::Human).unwrap();
        }
        assert_eq!(ed.doc.path.len(), 2);
        let mut t = Thumbs::new(egui::Context::default(), renderer);
        FAIL_STEP.store(1, std::sync::atomic::Ordering::Relaxed);
        wait(&mut t, &ed);
        assert!(t.get(0).is_some() && t.get(1).is_none(), "the step that failed is empty, the other is drawn");
        FAIL_STEP.store(usize::MAX, std::sync::atomic::Ordering::Relaxed);
        ed.apply(Command::SetTitle { title: "again".into() }, Origin::Human).unwrap();
        wait(&mut t, &ed);
        assert!(t.get(0).is_some() && t.get(1).is_some(), "the next round draws both");
    }
}
