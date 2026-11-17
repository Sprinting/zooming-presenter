//! Window-level macOS integration through small objc2 calls. Nothing here
//! puts an AppKit control inside the egui UI (Kartik's rule); these are the
//! pieces AppKit owns anyway:
//! - the system accent colour (`controlAccentColor`), read at launch and
//!   when the app becomes active, so Zebra follows Blue, Graphite, Pink...;
//! - the window's document state: title, the edited dot in the close
//!   button / "— Edited" (`setDocumentEdited`) and the proxy icon
//!   (`setRepresentedURL`);
//! - System Settings › Accessibility › Reduce motion;
//! - App Nap: held off only while presenting.
//!
//! Off macOS every function is a cheap no-op so callers need no cfg.

/// System "Reduce motion" (Accessibility › Display). `false` off macOS.
pub fn system_reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Read the system accent colour into `style::accent()`. Returns true if
/// it changed.
pub fn refresh_accent() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Some(c) = imp::accent() {
            if c != crate::style::accent() {
                crate::style::set_accent(c);
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "macos")]
pub use imp::sync_window;

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_app_kit::{NSColor, NSColorSpace, NSView, NSWindow, NSWorkspace};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::app::ZebraApp;

    pub fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    pub fn accent() -> Option<egui::Color32> {
        let c = NSColor::controlAccentColor();
        let c = c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(egui::Color32::from_rgb(to8(c.redComponent()), to8(c.greenComponent()), to8(c.blueComponent())))
    }

    /// What was last pushed to the window, so AppKit is only called on change.
    #[derive(Default)]
    pub struct WindowSync {
        title: String,
        edited: bool,
        url: Option<std::path::PathBuf>,
        focused: Option<bool>,
        activity: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    }

    fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
        let h = frame.window_handle().ok()?;
        let RawWindowHandle::AppKit(a) = h.as_raw() else { return None };
        // SAFETY: winit hands out a valid NSView pointer for the lifetime of
        // the window; we only use it on the main thread during `ui`.
        let view: &NSView = unsafe { a.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    /// Push document state to the window; refresh the accent when the app
    /// becomes active; hold an App Nap activity while presenting.
    pub fn sync_window(app: &mut ZebraApp, frame: &eframe::Frame) {
        let focused = app.ctx.input(|i| i.viewport().focused);
        let presenting = app.editor.view.mode == zebra_core::Mode::Present;
        let title = app.window_title();
        let edited = app.editor.dirty();
        let url = app.editor.file_path.clone();
        let s = &mut app.mac_sync;
        if s.focused != focused {
            s.focused = focused;
            if focused == Some(true) && super::refresh_accent() {
                crate::style::apply_visuals(&app.ctx);
            }
        }
        match (presenting, s.activity.is_some()) {
            (true, false) => {
                let reason = NSString::from_str(&crate::t!("mac-activity-presenting"));
                s.activity = Some(NSProcessInfo::processInfo().beginActivityWithOptions_reason(NSActivityOptions::UserInitiated, &reason));
            }
            (false, true) => {
                if let Some(a) = s.activity.take() {
                    // SAFETY: `a` came from beginActivity on this process.
                    unsafe { NSProcessInfo::processInfo().endActivity(&a) };
                }
            }
            _ => {}
        }
        if s.title == title && s.edited == edited && s.url == url {
            return;
        }
        let Some(win) = ns_window(frame) else { return };
        if s.title != title {
            win.setTitle(&NSString::from_str(&title));
            s.title = title;
        }
        if s.edited != edited {
            win.setDocumentEdited(edited);
            s.edited = edited;
        }
        if s.url != url {
            let u = url.as_ref().map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())));
            win.setRepresentedURL(u.as_deref());
            s.url = url;
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::WindowSync;

/// Off macOS there is nothing to sync.
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct WindowSync;
