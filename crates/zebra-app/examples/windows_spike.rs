//! Spike: can a second egui (immediate) viewport be a real Mac document
//! window that joins the root window in a native tab group, and can we find
//! its NSWindow? Prints findings, quits by itself after ~8 s.
//!
//!   cargo run --release -p zebra-app --example windows_spike

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Spike A").with_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Spike", opts, Box::new(|_| Ok(Box::new(Spike::default()))))
}

#[derive(Default)]
struct Spike {
    t0: Option<std::time::Instant>,
    root_frames: u64,
    child_frames: u64,
    step: u32,
    child_open: bool,
}

const TOKEN: &str = "zebra-window-d2";

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t0 = *self.t0.get_or_insert_with(std::time::Instant::now);
        let t = t0.elapsed().as_secs_f64();
        self.root_frames += 1;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("A (root) frames {}  child frames {}", self.root_frames, self.child_frames));
        });
        if self.step == 0 {
            self.child_open = true;
            self.step = 1;
        }
        if self.child_open {
            let child_frames = &mut self.child_frames;
            let closed = ui.ctx().show_viewport_immediate(
                egui::ViewportId::from_hash_of("doc-d2"),
                egui::ViewportBuilder::default().with_title(TOKEN).with_inner_size([600.0, 380.0]),
                |ui, class| {
                    *child_frames += 1;
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.heading(format!("B (child, immediate={}) frames {child_frames}", matches!(class, egui::ViewportClass::Immediate)));
                    });
                    ui.ctx().input(|i| i.viewport().close_requested())
                },
            );
            if closed {
                println!("spike: child close requested at {t:.1}s");
                self.child_open = false;
            }
        }
        #[cfg(target_os = "macos")]
        mac::steps(self, frame, t);
        #[cfg(not(target_os = "macos"))]
        let _ = frame;
        if t > 8.0 {
            println!("spike: root frames {}, child frames {}", self.root_frames, self.child_frames);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSView, NSWindow};
    use objc2_foundation::NSString;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    fn root(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
        let h = frame.window_handle().ok()?;
        let RawWindowHandle::AppKit(a) = h.as_raw() else { return None };
        let view: &NSView = unsafe { a.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    fn by_title(title: &str) -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(mtm);
        app.windows().iter().find(|w| w.title().to_string() == title)
    }

    pub fn steps(s: &mut super::Spike, frame: &eframe::Frame, t: f64) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if s.step == 1 && t > 1.5 {
            s.step = 2;
            println!("spike: allowsAutomaticWindowTabbing = {}", NSWindow::allowsAutomaticWindowTabbing(mtm));
            let app = NSApplication::sharedApplication(mtm);
            let titles: Vec<String> = app.windows().iter().map(|w| w.title().to_string()).collect();
            println!("spike: NSApp windows = {titles:?}");
            match by_title(super::TOKEN) {
                Some(w) => {
                    w.setTitle(&NSString::from_str("Spike B"));
                    w.setDocumentEdited(true);
                    println!("spike: found child NSWindow by title token; renamed to 'Spike B', edited dot on; tabbingMode {:?}", w.tabbingMode());
                }
                None => println!("spike: child NSWindow NOT found by title"),
            }
            if let Some(r) = root(frame) {
                println!("spike: root tabbingMode {:?}, tabbingIdentifier {}", r.tabbingMode(), r.tabbingIdentifier());
            }
        }
        if s.step == 2 && t > 2.5 {
            s.step = 3;
            if let Some(r) = root(frame) {
                r.mergeAllWindows(None);
                let n = r.tabbedWindows().map(|v| v.len()).unwrap_or(0);
                println!("spike: after mergeAllWindows, root has {n} tabbed windows (2 = one tab group)");
            }
        }
        if s.step == 3 && t > 4.5 {
            s.step = 4;
            println!("spike: frames so far: root {}, child {}", s.root_frames, s.child_frames);
            if let Some(w) = by_title("Spike B") {
                w.makeKeyAndOrderFront(None);
                println!("spike: selected tab B; is it key? {}", w.isKeyWindow());
            }
        }
        if s.step == 4 && t > 6.0 {
            s.step = 5;
            println!("spike: frames after switching to tab B: root {}, child {}", s.root_frames, s.child_frames);
        }
    }
}
