//! A video card's play window (YouTube, Vimeo): the site's own embed
//! player, large and centred over a dimmed canvas, like Quick Look.
//!
//! * The only place Zebra shows web content (WKWebView on the Mac): a
//!   private, non-persistent data store; no script message handlers (no
//!   bridge to Zebra); no file access (the page is a string with an https
//!   base); no pop-ups. Navigation stays on the embed hosts; any other
//!   link (such as "Watch on YouTube") opens in the default browser.
//! * Every key goes to the player, Left, Right and Space too. Only Esc
//!   stays with Zebra and closes it. A click outside, or the × that shows
//!   on hover, closes it too; closing frees the web view.
//! * The page reports how far the video got by setting its title to a
//!   number (seconds; a negative number is the length), read strictly on
//!   close: the resume point, kept in memory while the deck is open.
//! * Elsewhere there is no play window: the card stays a card, and the
//!   same state machine runs so the presenter view and keys behave alike.
use std::collections::HashMap;

use zebra_core::weblink;
use zebra_core::{ElementId, ElementKind, LinkKind, VideoLink};

use crate::app::ZebraApp;

/// The page's origin: the embed players answer it, and the page is loaded
/// as a string under it (never a file).
pub const ORIGIN: &str = "https://app.zebra.Zebra";
/// Hosts a card's player frame may load from.
pub const EMBED_HOSTS: &[&str] = &["www.youtube-nocookie.com", "youtube-nocookie.com", "www.youtube.com", "player.vimeo.com"];
/// Seconds with no word from the player before the page says it couldn't
/// reach the site.
const SILENT_LIMIT_MS: u32 = 12_000;

/// A card's open play window.
pub struct CardPlayer {
    pub id: ElementId,
    pub site: String,
    /// Latest position the page reported, seconds.
    pub at: Option<f64>,
    /// The video's length, when the page said.
    pub length: Option<f64>,
    #[cfg(target_os = "macos")]
    mac: Option<mac::Window>,
}

/// Where a card was left: the resume point and how long it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resume {
    pub at: f64,
    pub length: Option<f64>,
}

/// What the page's title says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Title {
    /// Seconds into the video.
    At(f64),
    /// The video's length.
    Length(f64),
    Close,
    /// Anything else: ignored.
    Nothing,
}

/// Read the page's title strictly: a finite number of at most 24
/// characters (digits, one point, a leading minus), or the word "close".
pub fn parse_title(t: &str) -> Title {
    let t = t.trim();
    if t == "close" {
        return Title::Close;
    }
    let ok = !t.is_empty()
        && t.len() <= 24
        && t.char_indices().all(|(i, c)| c.is_ascii_digit() || c == '.' || (c == '-' && i == 0))
        && t.matches('.').count() <= 1;
    match t.parse::<f64>() {
        Ok(v) if ok && v.is_finite() && v >= 0.0 => Title::At(v),
        Ok(v) if ok && v.is_finite() && v < 0.0 => Title::Length(-v),
        _ => Title::Nothing,
    }
}

/// What to do with a navigation in the play window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Allow,
    /// Not here: in the default browser (a link the person clicked).
    Browser,
    Block,
}

/// The play window's navigation rule. The page itself (the main frame)
/// is only ever our own string; its player frame only the embed hosts.
/// Any https or http link elsewhere opens in the browser when clicked.
pub fn navigation(url: &str, main_frame: bool, clicked: bool) -> Nav {
    let u = url.trim();
    if u == "about:blank" || u == "about:srcdoc" {
        return Nav::Allow;
    }
    let Some(p) = weblink::parse(u) else { return Nav::Block };
    let web = p.scheme == "https" || p.scheme == "http";
    if main_frame {
        if p.scheme == "https" && format!("https://{}", p.host) == ORIGIN.to_ascii_lowercase() {
            return Nav::Allow;
        }
        return if web { Nav::Browser } else { Nav::Block };
    }
    if p.scheme == "https" && EMBED_HOSTS.contains(&p.host.as_str()) {
        return Nav::Allow;
    }
    if web && clicked {
        Nav::Browser
    } else {
        Nav::Block
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The play window's page: the site's embed player filling it, and a
/// small script that tells Zebra how far it got through the page title
/// and shows `offline` when the player can't be reached. None for a link
/// that has no embed player.
pub fn player_page(link: &VideoLink, start: f64, end: Option<f64>, offline: &str) -> Option<String> {
    let src = weblink::embed_url(link, start, end, true, Some(ORIGIN))?;
    let youtube = link.kind == LinkKind::Youtube;
    let target = if youtube { "https://www.youtube-nocookie.com" } else { "https://player.vimeo.com" };
    Some(format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src {frames}; script-src 'unsafe-inline'; style-src 'unsafe-inline'">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe{{border:0;width:100%;height:100%;display:block}}#off{{display:none;position:absolute;inset:0;align-items:center;justify-content:center;color:#ddd;font:15px -apple-system,sans-serif;text-align:center;padding:24px}}</style>
</head><body><iframe id="p" src="{src}" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowfullscreen referrerpolicy="strict-origin-when-cross-origin"></iframe><div id="off">{offline}</div>
<script>(function(){{
var f=document.getElementById('p'),youtube={youtube},target='{target}',heard=false,quiet=0,stop={stop};
function say(v){{if(typeof v==='number'&&isFinite(v)&&Date.now()>=quiet)document.title=String(v);}}
function length(v){{if(typeof v==='number'&&isFinite(v)&&v>0&&!say.l){{say.l=1;document.title=String(-v);quiet=Date.now()+400;}}}}
function off(){{f.style.display='none';document.getElementById('off').style.display='flex';}}
window.addEventListener('message',function(e){{
 if(e.source!==f.contentWindow||e.origin!==target)return;
 var d=e.data;if(typeof d==='string'){{try{{d=JSON.parse(d);}}catch(x){{return;}}}}
 if(!d||typeof d!=='object')return;heard=true;
 if(youtube){{if(d.event==='infoDelivery'&&d.info&&typeof d.info==='object'){{length(d.info.duration);say(d.info.currentTime);}}}}
 else{{if(d.event==='ready')listen();if(d.event==='timeupdate'&&d.data&&typeof d.data==='object'){{length(d.data.duration);say(d.data.seconds);if(stop>0&&d.data.seconds>=stop)f.contentWindow.postMessage(JSON.stringify({{method:'pause'}}),target);}}}}
}});
function listen(){{try{{f.contentWindow.postMessage(JSON.stringify(youtube?{{event:'listening',id:1,channel:'widget'}}:{{method:'addEventListener',value:'timeupdate'}}),target);}}catch(x){{}}}}
f.addEventListener('load',listen);
if(!navigator.onLine)off();
window.addEventListener('offline',function(){{if(!heard)off();}});
setTimeout(function(){{if(!heard)off();}},{silent});
}})();</script></body></html>"#,
        frames = EMBED_HOSTS.iter().map(|h| format!("https://{h}")).collect::<Vec<_>>().join(" "),
        src = html_escape(&src),
        offline = html_escape(offline),
        youtube = youtube,
        target = target,
        silent = SILENT_LIMIT_MS,
        // Vimeo's player has no end in its link: the page pauses it there.
        stop = end.filter(|e| e.is_finite() && *e > 0.0).unwrap_or(0.0),
    ))
}

/// The site of the card whose window is open.
pub fn playing_site(app: &ZebraApp) -> Option<String> {
    app.media.card.as_ref().map(|c| c.site.clone())
}

/// Open card `id`'s play window (a click while presenting, a double-click
/// while editing). It starts where it was left, or at the card's start.
pub fn open(app: &mut ZebraApp, id: &str) {
    if app.media.card.as_ref().is_some_and(|c| c.id == id) {
        return;
    }
    close(app);
    let Some(e) = app.editor.doc.get(id) else { return };
    let ElementKind::Video(v) = &e.kind else { return };
    let Some(link) = v.link.clone().filter(|l| l.kind.is_card()) else { return };
    let resume = app.media.card_resume.get(id).map(|r| r.at).filter(|t| *t > v.start && v.end.is_none_or(|e| *t < e - 1.0));
    let start = resume.unwrap_or(v.start);
    let site = weblink::site_name(&link);
    let aspect = if e.size.h > 0.0 { e.size.w / e.size.h } else { 16.0 / 9.0 };
    #[cfg(target_os = "macos")]
    let mac = {
        let page = player_page(&link, start, v.end, &crate::t!("web-video-error-unreachable"));
        let parent = app.mac_sync.window.clone();
        match (page, parent) {
            (Some(page), Some(parent)) => mac::Window::open(&parent, &page, aspect, &crate::t!("web-video-close"), &crate::t!("media-clear-button")),
            _ => None,
        }
    };
    #[cfg(not(target_os = "macos"))]
    let _ = (aspect, start);
    app.media.card = Some(CardPlayer {
        id: id.to_string(),
        site,
        at: None,
        length: None,
        #[cfg(target_os = "macos")]
        mac,
    });
    app.ctx.request_repaint();
}

/// Close the play window, keeping where the video got to.
pub fn close(app: &mut ZebraApp) {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let Some(mut c) = app.media.card.take() else { return };
    #[cfg(target_os = "macos")]
    if let Some(m) = c.mac.take() {
        for t in m.titles() {
            note(&mut c, parse_title(&t));
        }
        m.close();
    }
    if let Some(at) = c.at {
        app.media.card_resume.insert(c.id.clone(), Resume { at, length: c.length });
    }
    app.ctx.request_repaint();
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn note(c: &mut CardPlayer, t: Title) {
    match t {
        Title::At(v) => c.at = Some(v),
        Title::Length(v) => c.length = Some(v),
        _ => {}
    }
}

/// Per frame while a window is open: what the page said, Esc, a click
/// outside or ×, and the card being deleted or the deck closed.
pub fn tick(app: &mut ZebraApp) {
    let Some(id) = app.media.card.as_ref().map(|c| c.id.clone()) else { return };
    let gone = !matches!(app.editor.doc.get(&id).map(|e| &e.kind), Some(ElementKind::Video(v)) if v.link.as_ref().is_some_and(|l| l.kind.is_card()));
    let mut shut = gone;
    #[cfg(target_os = "macos")]
    if let Some(c) = app.media.card.as_mut() {
        let seen = c.mac.as_ref().map(|m| {
            m.hover();
            (m.titles(), m.wants_close())
        });
        match seen {
            Some((titles, close)) => {
                shut |= close;
                for t in titles {
                    let t = parse_title(&t);
                    shut |= t == Title::Close;
                    note(c, t);
                }
            }
            // No window could be made: nothing to show it in.
            None => shut = true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // No play window here: Esc closes the card's state all the same.
        shut |= app.ctx.input(|i| i.key_pressed(egui::Key::Escape));
    }
    if shut {
        close(app);
    } else {
        app.ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// Resume points as 0..1 of the way through, for the cards' progress
/// lines.
pub fn progress(app: &ZebraApp) -> HashMap<ElementId, f32> {
    let mut out = HashMap::new();
    for (id, r) in &app.media.card_resume {
        let Some(ElementKind::Video(v)) = app.editor.doc.get(id).map(|e| &e.kind) else { continue };
        let length = v.end.or(r.length).or((v.duration > 0.0).then_some(v.duration));
        if let Some(l) = length.filter(|l| *l > 0.0) {
            let f = (r.at / l).clamp(0.0, 1.0) as f32;
            if f > 0.0 {
                out.insert(id.clone(), f);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
mod mac {
    //! The window itself: a borderless child window over the deck's
    //! window, dimmed, holding the web view, with the Esc monitor.
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSBackingStoreType, NSBezelStyle, NSButton, NSColor, NSEvent, NSEventMask, NSView, NSWindow,
        NSWindowOrderingMode, NSWindowStyleMask, NSWorkspace,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL};
    use objc2_web_kit::{
        WKAudiovisualMediaTypes, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
        WKNavigationType, WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
        WKWindowFeatures,
    };

    use super::{Nav, navigation};

    /// Esc's key code.
    const ESC: u16 = 53;

    #[derive(Default)]
    struct Flags {
        close: AtomicBool,
    }

    define_class!(
        // SAFETY: NSWindow allows subclassing; this only answers that a
        // borderless window may be key (so the player gets the keys).
        #[unsafe(super(NSWindow))]
        #[thread_kind = MainThreadOnly]
        #[name = "ZebraCardWindow"]
        struct CardWindow;

        impl CardWindow {
            #[unsafe(method(canBecomeKeyWindow))]
            fn can_become_key(&self) -> bool {
                true
            }
            #[unsafe(method(canBecomeMainWindow))]
            fn can_become_main(&self) -> bool {
                false
            }
        }
    );

    struct DimIvars {
        flags: Arc<Flags>,
    }

    define_class!(
        // SAFETY: a plain NSView; a click on it (outside the player) or
        // its × asks to close.
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "ZebraCardDim"]
        #[ivars = DimIvars]
        struct Dim;

        impl Dim {
            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _event: &NSEvent) {
                self.ivars().flags.close.store(true, Ordering::Relaxed);
            }
            #[unsafe(method(zebraClose:))]
            fn close_clicked(&self, _sender: Option<&AnyObject>) {
                self.ivars().flags.close.store(true, Ordering::Relaxed);
            }
            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                true
            }
        }
    );

    fn open_in_browser(url: &NSURL) {
        let s = url.absoluteString().map(|s| s.to_string()).unwrap_or_default();
        if s.starts_with("https://") || s.starts_with("http://") {
            NSWorkspace::sharedWorkspace().openURL(url);
        }
    }

    define_class!(
        // SAFETY: NSObject; the delegate keeps no state and touches only
        // the navigation it is asked about.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ZebraCardNav"]
        struct NavDelegate;

        unsafe impl NSObjectProtocol for NavDelegate {}

        unsafe impl WKNavigationDelegate for NavDelegate {
            #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
            fn decide(&self, _web: &WKWebView, action: &WKNavigationAction, decide: &DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
                // SAFETY: property reads on the action WebKit hands us.
                let (url, main, clicked) = unsafe {
                    let url = action.request().URL();
                    let main = action.targetFrame().is_none_or(|f| f.isMainFrame());
                    (url, main, action.navigationType() == WKNavigationType::LinkActivated)
                };
                let text = url.as_ref().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
                match navigation(&text, main, clicked) {
                    Nav::Allow => decide.call((WKNavigationActionPolicy::Allow,)),
                    Nav::Browser => {
                        decide.call((WKNavigationActionPolicy::Cancel,));
                        if let Some(u) = url {
                            open_in_browser(&u);
                        }
                    }
                    Nav::Block => decide.call((WKNavigationActionPolicy::Cancel,)),
                }
            }
        }

        unsafe impl WKUIDelegate for NavDelegate {
            /// No pop-ups or new windows: a link meant for one opens in
            /// the browser instead.
            #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
            fn create(&self, _web: &WKWebView, _cfg: &WKWebViewConfiguration, action: &WKNavigationAction, _features: &WKWindowFeatures) -> *mut WKWebView {
                // SAFETY: property reads on the action WebKit hands us.
                let clicked = unsafe { action.navigationType() == WKNavigationType::LinkActivated };
                if clicked {
                    if let Some(u) = unsafe { action.request().URL() } {
                        open_in_browser(&u);
                    }
                }
                // Never a new web view.
                std::ptr::null_mut()
            }
        }
    );

    pub struct Window {
        window: Retained<CardWindow>,
        parent: Retained<NSWindow>,
        web: Retained<WKWebView>,
        close_button: Retained<NSButton>,
        _nav: Retained<NavDelegate>,
        monitor: Option<Retained<AnyObject>>,
        flags: Arc<Flags>,
        /// Titles seen since the last look (the page may change it more
        /// than once a frame; only the newest is kept here).
        last: RefCell<String>,
        /// The player's rectangle in screen coordinates (for hover).
        video: NSRect,
    }

    impl Window {
        /// A window over `parent`, `aspect` wide, with `page` in it.
        pub fn open(parent: &NSWindow, page: &str, aspect: f64, close_label: &str, close_glyph: &str) -> Option<Window> {
            let mtm = MainThreadMarker::new()?;
            let frame = parent.frame();
            let flags = Arc::new(Flags::default());
            // SAFETY: AppKit and WebKit constructors and setters on the
            // main thread (`mtm`), with live arguments.
            unsafe {
                let window: Retained<CardWindow> = msg_send![
                    CardWindow::alloc(mtm),
                    initWithContentRect: frame,
                    styleMask: NSWindowStyleMask::Borderless,
                    backing: NSBackingStoreType::Buffered,
                    defer: false
                ];
                window.setReleasedWhenClosed(false);
                window.setOpaque(false);
                window.setHasShadow(false);
                // The dimmed canvas, like Quick Look's.
                window.setBackgroundColor(Some(&NSColor::colorWithCalibratedWhite_alpha(0.0, 0.62)));

                let dim: Retained<Dim> = msg_send![super(Dim::alloc(mtm).set_ivars(DimIvars { flags: flags.clone() })), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), frame.size)];
                window.setContentView(Some(&dim));

                // Large and centred: 86% of the window, at the video's shape.
                let (fw, fh) = (frame.size.width, frame.size.height);
                let aspect = if aspect.is_finite() && aspect > 0.1 && aspect < 10.0 { aspect } else { 16.0 / 9.0 };
                let (mut w, mut h) = (fw * 0.86, fw * 0.86 / aspect);
                if h > fh * 0.86 {
                    h = fh * 0.86;
                    w = h * aspect;
                }
                let rect = NSRect::new(NSPoint::new((fw - w) / 2.0, (fh - h) / 2.0), NSSize::new(w, h));

                let config = WKWebViewConfiguration::new(mtm);
                // Private: nothing kept after the window closes, nothing
                // shared with Safari or other windows.
                config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
                config.preferences().setJavaScriptCanOpenWindowsAutomatically(false);
                // It was clicked to play: no second click for sound.
                config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
                let web = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config);
                let nav: Retained<NavDelegate> = msg_send![NavDelegate::alloc(mtm), init];
                web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*nav)));
                web.setUIDelegate(Some(ProtocolObject::from_ref(&*nav)));
                dim.addSubview(&web);

                let close_button = NSButton::buttonWithTitle_target_action(&NSString::from_str(close_glyph), Some(&dim), Some(sel!(zebraClose:)), mtm);
                close_button.setBezelStyle(NSBezelStyle::Circular);
                close_button.setFrame(NSRect::new(NSPoint::new(rect.origin.x + rect.size.width - 14.0, rect.origin.y + rect.size.height - 14.0), NSSize::new(28.0, 28.0)));
                close_button.setAlphaValue(0.0);
                close_button.setToolTip(Some(&NSString::from_str(close_label)));
                dim.addSubview(&close_button);

                let base = NSURL::URLWithString(&NSString::from_str(super::ORIGIN));
                web.loadHTMLString_baseURL(&NSString::from_str(page), base.as_deref());

                parent.addChildWindow_ordered(&window, NSWindowOrderingMode::Above);
                window.makeKeyAndOrderFront(None);
                window.makeFirstResponder(Some(&web));

                // Esc, and only Esc, stays with Zebra.
                let f2 = flags.clone();
                let win_ptr = Retained::as_ptr(&window) as usize;
                let handler = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                    let e = event.as_ref();
                    let ours = e.window(MainThreadMarker::new_unchecked()).is_some_and(|w| Retained::as_ptr(&w) as usize == win_ptr);
                    if ours && e.keyCode() == ESC {
                        f2.close.store(true, Ordering::Relaxed);
                        return std::ptr::null_mut();
                    }
                    event.as_ptr()
                });
                let monitor = NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler);

                let origin = frame.origin;
                Some(Window {
                    window,
                    parent: parent.retain(),
                    web,
                    close_button,
                    _nav: nav,
                    monitor,
                    flags,
                    last: RefCell::new(String::new()),
                    video: NSRect::new(NSPoint::new(origin.x + rect.origin.x, origin.y + rect.origin.y), rect.size),
                })
            }
        }

        /// The page's title, when it changed.
        pub fn titles(&self) -> Vec<String> {
            // SAFETY: property read on the main thread.
            let t = unsafe { self.web.title() }.map(|t| t.to_string()).unwrap_or_default();
            if t.len() > 64 || *self.last.borrow() == t {
                return vec![];
            }
            *self.last.borrow_mut() = t.clone();
            vec![t]
        }

        pub fn wants_close(&self) -> bool {
            self.flags.close.load(Ordering::Relaxed)
        }

        /// The × shows while the pointer is over the player.
        pub fn hover(&self) {
            let m = NSEvent::mouseLocation();
            let r = self.video;
            let over = m.x >= r.origin.x - 20.0 && m.x <= r.origin.x + r.size.width + 20.0 && m.y >= r.origin.y - 20.0 && m.y <= r.origin.y + r.size.height + 20.0;
            self.close_button.setAlphaValue(if over { 1.0 } else { 0.0 });
        }

        /// Close and free everything; keys go back to the deck.
        pub fn close(self) {
            drop(self);
        }
    }

    impl Drop for Window {
        /// However the window goes (closed, the deck closed, the show
        /// ended), the web view and the Esc monitor go with it.
        fn drop(&mut self) {
            // SAFETY: AppKit and WebKit calls on the main thread (the
            // window lives in app state, used only there) with the objects
            // this window made.
            unsafe {
                if let Some(m) = self.monitor.take() {
                    NSEvent::removeMonitor(&m);
                }
                self.web.stopLoading();
                self.web.setNavigationDelegate(None);
                self.web.setUIDelegate(None);
                self.web.removeFromSuperview();
                self.parent.removeChildWindow(&self.window);
                self.window.orderOut(None);
                self.parent.makeKeyAndOrderFront(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_read_strictly() {
        assert_eq!(parse_title("12.5"), Title::At(12.5));
        assert_eq!(parse_title(" 0 "), Title::At(0.0));
        assert_eq!(parse_title("-300"), Title::Length(300.0));
        assert_eq!(parse_title("close"), Title::Close);
        for junk in ["", "Close", "NaN", "inf", "1e9999", "12.5.1", "1-2", "--1", "12s", "<b>1</b>", "javascript:1", &"9".repeat(40), "YouTube", "0x10"] {
            assert_eq!(parse_title(junk), Title::Nothing, "{junk:?}");
        }
    }

    #[test]
    fn the_play_window_stays_on_the_players() {
        assert_eq!(navigation("https://app.zebra.Zebra/", true, false), Nav::Allow);
        assert_eq!(navigation("about:blank", false, false), Nav::Allow);
        assert_eq!(navigation("https://www.youtube-nocookie.com/embed/abc", false, false), Nav::Allow);
        assert_eq!(navigation("https://player.vimeo.com/video/1", false, false), Nav::Allow);
        // "Watch on YouTube" and other links: the browser, never here.
        assert_eq!(navigation("https://www.youtube.com/watch?v=abc", true, true), Nav::Browser);
        assert_eq!(navigation("https://example.com/", false, true), Nav::Browser);
        // Not clicked (a script or redirect): nowhere.
        assert_eq!(navigation("https://tracker.example.com/", false, false), Nav::Block);
        assert_eq!(navigation("http://player.vimeo.com/video/1", false, false), Nav::Block);
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,hi", "zebra://x", ""] {
            assert_eq!(navigation(bad, true, true), Nav::Block, "{bad}");
            assert_eq!(navigation(bad, false, true), Nav::Block, "{bad}");
        }
    }

    #[test]
    fn the_page_holds_only_the_embed_player() {
        let yt = VideoLink { url: "https://youtu.be/dQw4w9WgXcQ".into(), kind: LinkKind::Youtube };
        let page = player_page(&yt, 65.0, Some(90.0), "Couldn't reach <this> link").unwrap();
        assert!(page.contains("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?"), "{page}");
        assert!(page.contains("start=65") && page.contains("end=90"), "{page}");
        assert!(page.contains("Couldn&#39;t reach &lt;this&gt; link"), "the message is text, never markup");
        assert!(page.contains("default-src 'none'"));
        assert!(!page.contains("<script src"), "no remote script in the page itself");
        let vm = VideoLink { url: "https://vimeo.com/76979871".into(), kind: LinkKind::Vimeo };
        assert!(player_page(&vm, 0.0, None, "x").unwrap().contains("https://player.vimeo.com/video/76979871"));
        let file = VideoLink { url: "https://cdn.example.com/a.mp4".into(), kind: LinkKind::File };
        assert!(player_page(&file, 0.0, None, "x").is_none());
    }
}
