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

/// Open a file in the default web browser (the app that opens https
/// links), whatever opens .html files: an exported web page.
#[cfg(target_os = "macos")]
pub fn open_in_browser(page: &std::path::Path) {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
    use objc2_foundation::{NSArray, NSString, NSURL};
    let ws = NSWorkspace::sharedWorkspace();
    let file = NSURL::fileURLWithPath(&NSString::from_str(&page.to_string_lossy()));
    let browser = NSURL::URLWithString(&NSString::from_str("https://example.com")).and_then(|u| ws.URLForApplicationToOpenURL(&u));
    match browser {
        Some(app) => {
            let urls = NSArray::from_retained_slice(&[file]);
            ws.openURLs_withApplicationAtURL_configuration_completionHandler(&urls, &app, &NSWorkspaceOpenConfiguration::configuration(), None);
        }
        None => {
            ws.openURL(&file);
        }
    }
}

/// Height of a document window's title row: the part at the top that
/// AppKit draws over our content. That is the title bar (28 pt) and, when
/// the window is one of several tabs, the tab bar under it. With a fixed
/// 28 the tab bar sat on top of the toolbar (Mac round, 18 Nov).
pub fn title_row(app: &crate::app::ZebraApp) -> f32 {
    #[cfg(target_os = "macos")]
    {
        title_row_from(imp::top_inset(app), app.ctx.zoom_factor())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        crate::panels::TITLE_ROW
    }
}

/// `inset` is what AppKit covers at the top, in its points; ours are scaled
/// by the zoom factor. Never less than the plain title row (in full screen
/// AppKit reports none while the title bar is tucked away; the row stays).
pub fn title_row_from(inset: Option<f64>, zoom: f32) -> f32 {
    let row = crate::panels::TITLE_ROW;
    match inset {
        Some(i) if i.is_finite() && zoom > 0.0 => (i as f32 / zoom).clamp(row, row + 60.0),
        _ => row,
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
pub use imp::{NsParent, adopt_root, adopt_window, sync_window};

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_app_kit::{NSColor, NSColorSpace, NSView, NSWindow, NSWorkspace};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::app::ZebraApp;

    /// What AppKit covers at the top of the window's content (title bar,
    /// tab bar), in points.
    pub fn top_inset(app: &ZebraApp) -> Option<f64> {
        let win = app.mac_sync.window.as_ref()?;
        let view = win.contentView()?;
        let free = win.contentLayoutRect();
        Some(view.frame().size.height - (free.origin.y + free.size.height))
    }

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
        /// The NSWindow of a window that has no eframe `Frame` (every
        /// document window but the first).
        pub window: Option<Retained<NSWindow>>,
    }

    /// A window as a parent for rfd sheets.
    pub struct NsParent(pub Retained<NSWindow>);

    impl raw_window_handle::HasWindowHandle for NsParent {
        fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            let view = self.0.contentView().ok_or(raw_window_handle::HandleError::Unavailable)?;
            let ptr = std::ptr::NonNull::from(&*view).cast::<std::ffi::c_void>();
            let raw = raw_window_handle::RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(ptr));
            // SAFETY: the view lives as long as the window we hold.
            Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
        }
    }

    impl raw_window_handle::HasDisplayHandle for NsParent {
        fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            Ok(raw_window_handle::DisplayHandle::appkit())
        }
    }

    /// Find a new window by the placeholder title it was created with, and
    /// keep it (call before anything retitles it).
    pub fn adopt_window(app: &mut ZebraApp, title: &str) {
        if app.mac_sync.window.is_some() {
            return;
        }
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.mac_sync.window = ns_app.windows().iter().find(|w| w.title().to_string() == title);
        if let Some(w) = &app.mac_sync.window {
            name_tab_group(w);
        }
    }

    /// Give a document window its tab group's name. Our title bar is
    /// see-through (the content runs under it), and for such a window AppKit
    /// offers none of its tab commands (Show Tab Bar, Show Next Tab, Move Tab
    /// to New Window, Merge All Windows) unless the window names its group
    /// itself. The mode stays Automatic, so the person's "Prefer tabs"
    /// setting still decides when a new window becomes a tab. (winit's own
    /// tabbing_identifier option also forces the mode to Preferred.)
    fn name_tab_group(win: &NSWindow) {
        win.setTabbingIdentifier(&NSString::from_str("app.zebra.Zebra.document"));
    }

    fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
        let h = frame.window_handle().ok()?;
        let RawWindowHandle::AppKit(a) = h.as_raw() else { return None };
        // SAFETY: winit hands out a valid NSView pointer for the lifetime of
        // the window; we only use it on the main thread during `ui`.
        let view: &NSView = unsafe { a.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    /// Get hold of the root window and name its tab group before anything
    /// else happens in its first frame, so the tab commands AppKit put in
    /// the menu bar at launch (see `menu_mac::install_early`) apply to it
    /// from the start.
    pub fn adopt_root(app: &mut ZebraApp, frame: &eframe::Frame) {
        if app.mac_sync.window.is_some() {
            return;
        }
        app.mac_sync.window = ns_window(frame);
        if let Some(w) = &app.mac_sync.window {
            name_tab_group(w);
        }
    }

    /// Push document state to the window; refresh the accent when the app
    /// becomes active; hold an App Nap activity while presenting.
    pub fn sync_window(app: &mut ZebraApp, frame: Option<&eframe::Frame>) {
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
        if s.window.is_none() {
            s.window = frame.and_then(ns_window);
            if let Some(w) = &s.window {
                name_tab_group(w);
            }
        }
        let Some(win) = s.window.clone() else { return };
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

/// Clicking the Dock icon while no window is open asks for one (like
/// Keynote). Call once; [`take_reopen`] then says when it happened.
pub fn install_reopen_hook(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    reopen::install(ctx);
    #[cfg(not(target_os = "macos"))]
    let _ = ctx;
}

/// The person's "Prefer tabs when opening documents" setting (System
/// Settings › Desktop & Dock).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabPref {
    Never,
    Always,
    FullScreen,
}

pub fn tab_pref() -> TabPref {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWindowUserTabbingPreference as P;
        let Some(mtm) = objc2::MainThreadMarker::new() else { return TabPref::Never };
        let p = objc2_app_kit::NSWindow::userTabbingPreference(mtm);
        if p == P::Always {
            TabPref::Always
        } else if p == P::InFullScreen {
            TabPref::FullScreen
        } else {
            TabPref::Never
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        TabPref::Never
    }
}

/// Whether a new document window joins the window it was made from as a
/// tab. AppKit does this by itself only for windows whose tab group is
/// named when they first show; ours are named a moment later, so a new
/// document made from a full-screen window floated over it instead.
pub fn new_window_is_tab(pref: TabPref, from_full_screen: bool) -> bool {
    match pref {
        TabPref::Always => true,
        TabPref::FullScreen => from_full_screen,
        TabPref::Never => false,
    }
}

/// True once after the + button in a window's tab bar was clicked.
pub fn take_new_tab() -> bool {
    #[cfg(target_os = "macos")]
    {
        reopen::NEW_TAB.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Put `child` into `parent`'s tab group, after its current tab.
pub fn add_tab(app_parent: &crate::app::ZebraApp, child: &crate::app::ZebraApp) -> bool {
    #[cfg(target_os = "macos")]
    {
        let (Some(p), Some(c)) = (&app_parent.mac_sync.window, &child.mac_sync.window) else { return false };
        p.addTabbedWindow_ordered(c, objc2_app_kit::NSWindowOrderingMode::Above);
        c.makeKeyAndOrderFront(None);
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_parent, child);
        true
    }
}

/// True once after the Dock icon was clicked with no window open.
pub fn take_reopen() -> bool {
    #[cfg(target_os = "macos")]
    {
        reopen::REOPEN.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod reopen {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{Encode, sel};

    pub static REOPEN: AtomicBool = AtomicBool::new(false);
    pub static NEW_TAB: AtomicBool = AtomicBool::new(false);

    /// The + in the tab bar: AppKit shows it when someone answers this.
    unsafe extern "C-unwind" fn new_window_for_tab(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
        NEW_TAB.store(true, Ordering::SeqCst);
        if let Some(c) = WAKE.get() {
            c.request_repaint();
        }
    }
    static WAKE: OnceLock<egui::Context> = OnceLock::new();

    unsafe extern "C-unwind" fn should_reopen(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, has_visible: Bool) -> Bool {
        if !has_visible.as_bool() {
            REOPEN.store(true, Ordering::SeqCst);
            if let Some(c) = WAKE.get() {
                c.request_repaint();
            }
        }
        Bool::YES
    }

    /// winit's app delegate doesn't answer
    /// `applicationShouldHandleReopen:hasVisibleWindows:`; add it.
    pub fn install(ctx: &egui::Context) {
        if WAKE.set(ctx.clone()).is_err() {
            return;
        }
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else { return };
        let obj: &AnyObject = delegate.as_ref();
        let cls: &AnyClass = obj.class();
        let types = std::ffi::CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING)).unwrap_or_default();
        // SAFETY: the function matches the selector's signature and the
        // type string; adding a method to a live class is what the
        // runtime API is for. A class that already has it keeps its own.
        unsafe {
            let imp: Imp = std::mem::transmute(should_reopen as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool);
            objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel!(applicationShouldHandleReopen:hasVisibleWindows:), imp, types.as_ptr());
            let types = std::ffi::CString::new("v@:@").unwrap_or_default();
            let imp: Imp = std::mem::transmute(new_window_for_tab as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject));
            objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel!(newWindowForTab:), imp, types.as_ptr());
        }
    }
}

/// Let Finder, the Dock, Spotlight and Open With hand Zebra files to open.
/// Call before `eframe::run_native`: a double-click that launches Zebra
/// delivers its file while launch finishes, before the first frame. The
/// files go to `open_events`. No-op off macOS.
pub fn install_open_hook() {
    #[cfg(target_os = "macos")]
    open_hook::install();
}

/// winit's app delegate has no `application:openURLs:`, so AppKit handed
/// opened files to NSDocumentController, which knows no document class for
/// our type: "Zebra cannot open files in the 'Zebra Presentation' format",
/// then an Untitled window (Kartik, 18 Nov). We add the method to the
/// delegate. winit only sets its delegate inside `run_native`, and AppKit
/// sends launch files during `finishLaunching`, so the method goes on when
/// launch is about to finish (NSApplicationWillFinishLaunchingNotification):
/// the delegate exists by then, and no open event has been sent yet.
#[cfg(target_os = "macos")]
mod open_hook {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Imp, NSObject, Sel};
    use objc2::{AnyThread, define_class, msg_send, sel};
    use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSURL};

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "ZebraLaunchObserver"]
        struct LaunchObserver;

        impl LaunchObserver {
            #[unsafe(method(zebraWillFinishLaunching:))]
            fn will_finish_launching(&self, _note: &NSNotification) {
                add_open_urls();
            }
        }
    );

    unsafe extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *mut NSArray<NSURL>) {
        // SAFETY: AppKit passes a valid NSArray of NSURL for this selector.
        let Some(urls) = (unsafe { urls.as_ref() }) else { return };
        let paths: Vec<std::path::PathBuf> = urls.iter().filter_map(|u| u.to_file_path()).collect();
        crate::open_events::push(paths);
    }

    fn add_open_urls() {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return };
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else {
            eprintln!("zebra: no app delegate at launch; files from Finder will not open");
            return;
        };
        let obj: &AnyObject = delegate.as_ref();
        let cls: &AnyClass = obj.class();
        let types = c"v@:@@";
        // SAFETY: the function matches the selector's signature and the type
        // string. A class that already has the method keeps its own.
        unsafe {
            let imp: Imp = std::mem::transmute(open_urls as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut NSArray<NSURL>));
            objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel!(application:openURLs:), imp, types.as_ptr());
        }
    }

    pub fn install() {
        if objc2::MainThreadMarker::new().is_none() {
            return;
        }
        // SAFETY: plain init of an NSObject subclass with no ivars.
        let observer: Retained<LaunchObserver> = unsafe { msg_send![LaunchObserver::alloc(), init] };
        // SAFETY: the observer answers the selector and lives for the rest
        // of the process (leaked below); the notification name is AppKit's.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &observer,
                sel!(zebraWillFinishLaunching:),
                Some(objc2_app_kit::NSApplicationWillFinishLaunchingNotification),
                None,
            );
        }
        // The notification centre keeps no reference of its own.
        std::mem::forget(observer);
    }
}

/// Off macOS there is nothing to sync.
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct WindowSync;

#[cfg(test)]
mod title_row_tests {
    use super::title_row_from;

    /// Mac round, 18 Nov: two windows merged into tabs, and the tab bar was
    /// drawn over the toolbar. The title row grows by the tab bar's height.
    #[test]
    fn the_title_row_makes_room_for_a_tab_bar() {
        let row = crate::panels::TITLE_ROW;
        assert_eq!(title_row_from(None, 1.0), row, "no window yet");
        assert_eq!(title_row_from(Some(28.0), 1.0).max(28.0), row.max(28.0), "title bar alone");
        if cfg!(target_os = "macos") {
            assert_eq!(title_row_from(Some(56.0), 1.0), 56.0, "title bar and tab bar");
            assert_eq!(title_row_from(Some(56.0), 2.0), 28.0, "our points are zoomed");
            assert_eq!(title_row_from(Some(0.0), 1.0), 28.0, "full screen keeps the row");
        }
        assert_eq!(title_row_from(Some(f64::NAN), 1.0), row);
    }

    /// Mac round, 18 Nov: File › New from a full-screen window opened a
    /// small window floating over it. With the Mac's default setting
    /// ("Prefer tabs: in full screen") it is a tab.
    #[test]
    fn a_new_document_from_full_screen_is_a_tab() {
        use super::{TabPref, new_window_is_tab};
        assert!(new_window_is_tab(TabPref::FullScreen, true));
        assert!(!new_window_is_tab(TabPref::FullScreen, false));
        assert!(new_window_is_tab(TabPref::Always, false));
        assert!(!new_window_is_tab(TabPref::Never, true));
    }
}
