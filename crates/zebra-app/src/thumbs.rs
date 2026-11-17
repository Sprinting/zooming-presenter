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
                    let imgs: Vec<Option<ColorImage>> = (0..n)
                        .map(|i| {
                            let target = zebra_render::CaptureTarget::Step { index: i, build: None };
                            let png = renderer.capture(&ed, &target, THUMB_W, THUMB_H).ok()?;
                            let img = image::load_from_memory(&png).ok()?.to_rgba8();
                            Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
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
            self.ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    pub fn get(&self, i: usize) -> Option<&TextureHandle> {
        self.textures.get(i).and_then(|t| t.as_ref())
    }

    pub fn stale(&self, rev: u64) -> bool {
        self.shown_rev != rev
    }
}
