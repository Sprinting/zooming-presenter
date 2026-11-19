//! The wasm interface: C ABI functions the page script calls. Strings and
//! byte arrays go through linear memory: the page asks for a buffer with
//! [`zw_alloc`], fills it, and passes (pointer, length); ownership moves to
//! the callee. Results that are strings are left in an out-buffer read
//! with [`zw_out_ptr`] and [`zw_out_len`].
use std::cell::RefCell;
use std::collections::HashSet;

use zebra_core::view::Blank;

use crate::{Click, Player};

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// `console.error` of a UTF-8 string (panics, load errors).
    fn zw_log(ptr: *const u8, len: usize);
}

fn log(s: &str) {
    unsafe { zw_log(s.as_ptr(), s.len()) }
}

fn with<R>(f: impl FnOnce(&mut Player) -> R) -> R {
    PLAYER.with(|p| f(p.borrow_mut().get_or_insert_with(Player::new)))
}

fn set_out(bytes: Vec<u8>) {
    OUT.with(|o| *o.borrow_mut() = bytes);
}

/// Take ownership of a buffer the page filled.
unsafe fn take(ptr: *mut u8, len: usize) -> Vec<u8> {
    if len == 0 {
        return vec![];
    }
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

unsafe fn take_str(ptr: *mut u8, len: usize) -> String {
    String::from_utf8_lossy(&unsafe { take(ptr, len) }).into_owned()
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_init() {
    std::panic::set_hook(Box::new(|info| log(&format!("zebra player: {info}"))));
    with(|_| ());
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` from [`zw_alloc`], filled by the page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zw_add_font(ptr: *mut u8, len: usize) {
    let bytes = unsafe { take(ptr, len) };
    with(|p| p.add_font(bytes));
}

/// # Safety
/// Each (pointer, length) from [`zw_alloc`], filled by the page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zw_add_asset(key: *mut u8, key_len: usize, ext: *mut u8, ext_len: usize, width: u32, height: u32, data: *mut u8, data_len: usize) {
    let (key, ext, bytes) = unsafe { (take_str(key, key_len), take_str(ext, ext_len), take(data, data_len)) };
    with(|p| p.add_asset(&key, &ext, width, height, bytes));
}

/// Load `document.json`. Returns 0, or 1 with the error in the out-buffer.
///
/// # Safety
/// `ptr`/`len` from [`zw_alloc`], filled by the page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zw_load(ptr: *mut u8, len: usize) -> u32 {
    let json = unsafe { take_str(ptr, len) };
    match with(|p| p.load(&json)) {
        Ok(()) => 0,
        Err(e) => {
            log(&e);
            set_out(e.into_bytes());
            1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_resize(w: f64, h: f64, ppp: f64) {
    with(|p| p.resize(w, h, ppp));
}

/// Advance to `now` seconds; draws if needed. Returns seconds until the
/// next frame is wanted, or -1 when idle.
#[unsafe(no_mangle)]
pub extern "C" fn zw_frame(now: f64) -> f64 {
    with(|p| p.frame(now)).unwrap_or(-1.0)
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_pixels() -> *const u8 {
    with(|p| p.pixels.as_ptr())
}
/// Frames drawn so far: the pixels changed when this did.
#[unsafe(no_mangle)]
pub extern "C" fn zw_frames() -> u32 {
    with(|p| p.frames)
}
#[unsafe(no_mangle)]
pub extern "C" fn zw_width() -> u32 {
    with(|p| p.width_px)
}
#[unsafe(no_mangle)]
pub extern "C" fn zw_height() -> u32 {
    with(|p| p.height_px)
}

/// 0 next, 1 previous, 2 overview (Home), 3 last step (End), 4 black
/// screen, 5 white screen, 6 pause autoplay, 7 resume autoplay.
#[unsafe(no_mangle)]
pub extern "C" fn zw_command(op: u32) {
    with(|p| match op {
        0 => p.next(),
        1 => p.prev(),
        2 => p.overview(),
        3 => p.last(),
        4 => p.toggle_blank(Blank::Black),
        5 => p.toggle_blank(Blank::White),
        6 => p.pause_autoplay(true),
        7 => p.pause_autoplay(false),
        _ => {}
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_goto(step: u32) {
    with(|p| p.goto(step as usize));
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_pan(dx: f64, dy: f64) {
    with(|p| p.pan(dx, dy));
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_zoom(factor: f64, x: f64, y: f64) {
    with(|p| p.zoom(factor, x, y));
}

/// A click at CSS pixel (`x`, `y`): 0 next step, 1 zoomed to an object,
/// 2 a link (URL in the out-buffer), 3 a video (its id in the out-buffer).
#[unsafe(no_mangle)]
pub extern "C" fn zw_click(x: f64, y: f64) -> u32 {
    match with(|p| p.click(x, y)) {
        Click::Next => 0,
        Click::Zoom => 1,
        Click::Link(u) => {
            set_out(u.into_bytes());
            2
        }
        Click::Video(id) => {
            set_out(id.into_bytes());
            3
        }
    }
}

/// The videos playing now, as a JSON array of ids.
///
/// # Safety
/// `ptr`/`len` from [`zw_alloc`], filled by the page.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn zw_set_playing(ptr: *mut u8, len: usize) {
    let json = unsafe { take_str(ptr, len) };
    let ids: HashSet<String> = serde_json::from_str(&json).unwrap_or_default();
    with(|p| p.set_playing(ids));
}

/// Put the state JSON (step, count, videos...) in the out-buffer; returns
/// its length.
#[unsafe(no_mangle)]
pub extern "C" fn zw_state() -> usize {
    let s = with(|p| p.state().to_string());
    let n = s.len();
    set_out(s.into_bytes());
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn zw_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn zw_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}
