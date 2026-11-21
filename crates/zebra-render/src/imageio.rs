//! Pictures Zebra's own decoders can't scale down within memory, drawn
//! by the Mac's ImageIO, as Keynote draws every picture: a big JPEG
//! (decoded straight at a smaller size), a huge progressive JPEG, a huge
//! WebP. ImageIO makes a thumbnail no bigger than the level asked for, so
//! the picture is never held at full size by Zebra.
//!
//! The bytes are handed over without a copy (`CFDataCreateWithBytesNoCopy`
//! over the mapped file); the call returns before they go away.
#![cfg(target_os = "macos")]
#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;

type CFTypeRef = *const c_void;
type CFIndex = isize;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorNull: CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDataCreateWithBytesNoCopy(alloc: CFTypeRef, bytes: *const u8, len: CFIndex, dealloc: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: CFIndex, kcb: *const c_void, vcb: *const c_void) -> CFTypeRef;
    fn CFNumberCreate(alloc: CFTypeRef, kind: CFIndex, value: *const c_void) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImageSourceCreateThumbnailFromImageAlways: CFTypeRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFTypeRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFTypeRef;
    static kCGImageSourceShouldCache: CFTypeRef;
    fn CGImageSourceCreateWithData(data: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceGetCount(src: CFTypeRef) -> usize;
    fn CGImageSourceCreateThumbnailAtIndex(src: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceCreateImageAtIndex(src: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CFTypeRef;
    fn CGColorSpaceCreateWithName(name: CFTypeRef) -> CFTypeRef;
    fn CGImageGetWidth(img: CFTypeRef) -> usize;
    fn CGImageGetHeight(img: CFTypeRef) -> usize;
    fn CGBitmapContextCreate(data: *mut c_void, w: usize, h: usize, bits: usize, row: usize, cs: CFTypeRef, info: u32) -> CFTypeRef;
    fn CGContextDrawImage(ctx: CFTypeRef, rect: CGRect, img: CFTypeRef);
    fn CGImageCreateWithImageInRect(img: CFTypeRef, rect: CGRect) -> CFTypeRef;
    fn CGContextSetInterpolationQuality(ctx: CFTypeRef, q: i32);
}

/// kCGImageAlphaPremultipliedLast: RGBA, alpha premultiplied.
const PREMULTIPLIED_LAST: u32 = 1;
/// kCFNumberSInt64Type.
const SINT64: CFIndex = 4;

/// Releases a Core Foundation object when dropped (null is skipped).
struct Owned(CFTypeRef);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a +1 object this module created and owns.
            unsafe { CFRelease(self.0) }
        }
    }
}

/// The picture in `bytes` drawn by ImageIO at most `max_side` pixels on
/// its long side, as straight (not premultiplied) RGBA rows: (width,
/// height, pixels). None if ImageIO can't read it, or the result would
/// pass `max_bytes`.
pub fn thumbnail(bytes: &[u8], max_side: u32, max_bytes: u64) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.is_empty() || max_side == 0 {
        return None;
    }
    // SAFETY: plain Core Foundation, ImageIO and Core Graphics calls on
    // objects created here; each is checked for null and released by
    // `Owned`. The CFData borrows `bytes`, which outlives every object
    // made from it (all are released before this returns, and the
    // thumbnail is drawn into memory Zebra owns).
    unsafe {
        let data = Owned(CFDataCreateWithBytesNoCopy(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex, kCFAllocatorNull));
        if data.0.is_null() {
            return None;
        }
        // Don't keep the decoded picture around: only the thumbnail.
        let no_cache_keys = [kCGImageSourceShouldCache];
        let no_cache_vals = [kCFBooleanFalse];
        let src_opts = Owned(CFDictionaryCreate(std::ptr::null(), no_cache_keys.as_ptr(), no_cache_vals.as_ptr(), 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks));
        let src = Owned(CGImageSourceCreateWithData(data.0, src_opts.0));
        if src.0.is_null() || CGImageSourceGetCount(src.0) == 0 {
            return None;
        }
        let side = max_side as i64;
        let n = Owned(CFNumberCreate(std::ptr::null(), SINT64, &side as *const i64 as *const c_void));
        let keys = [kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceThumbnailMaxPixelSize, kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCache];
        // No EXIF rotation: Zebra's other decoders don't apply it either.
        let vals = [kCFBooleanTrue, n.0, kCFBooleanFalse, kCFBooleanFalse];
        let opts = Owned(CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), vals.as_ptr(), keys.len() as CFIndex, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks));
        let img = Owned(CGImageSourceCreateThumbnailAtIndex(src.0, 0, opts.0));
        if img.0.is_null() {
            return None;
        }
        let (w, h) = (CGImageGetWidth(img.0), CGImageGetHeight(img.0));
        if w == 0 || h == 0 || w > max_side as usize * 2 || h > max_side as usize * 2 || (w as u64 * h as u64 * 4) > max_bytes {
            return None;
        }
        let mut px = vec![0u8; w * h * 4];
        let cs = Owned(CGColorSpaceCreateWithName(kCGColorSpaceSRGB));
        let ctx = Owned(CGBitmapContextCreate(px.as_mut_ptr() as *mut c_void, w, h, 8, w * 4, cs.0, PREMULTIPLIED_LAST));
        if ctx.0.is_null() {
            return None;
        }
        CGContextDrawImage(ctx.0, CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: w as f64, height: h as f64 } }, img.0);
        drop(ctx);
        // Back to straight alpha, as the other decoders give.
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some((w as u32, h as u32, px))
    }
}

/// The part `(x, y, w, h)` of the picture in `bytes`, at its own pixels,
/// drawn by ImageIO at `out_w` x `out_h`, as premultiplied RGBA rows: the
/// detail of a big JPEG or WebP seen close (P6). ImageIO may decode the
/// whole picture to draw it, so None unless that fits in `max_bytes`
/// (or ImageIO can't read it).
pub fn region(bytes: &[u8], (x, y, w, h): (u32, u32, u32, u32), (out_w, out_h): (u32, u32), max_bytes: u64) -> Option<Vec<u8>> {
    if bytes.is_empty() || w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return None;
    }
    // SAFETY: as in `thumbnail`: Core Foundation, ImageIO and Core
    // Graphics objects made here, each checked for null and released by
    // `Owned` before this returns; the CFData borrows `bytes`, which
    // outlives them; the context draws into memory Zebra owns.
    unsafe {
        let data = Owned(CFDataCreateWithBytesNoCopy(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex, kCFAllocatorNull));
        if data.0.is_null() {
            return None;
        }
        let no_cache_keys = [kCGImageSourceShouldCache];
        let no_cache_vals = [kCFBooleanFalse];
        let opts = Owned(CFDictionaryCreate(std::ptr::null(), no_cache_keys.as_ptr(), no_cache_vals.as_ptr(), 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks));
        let src = Owned(CGImageSourceCreateWithData(data.0, opts.0));
        if src.0.is_null() || CGImageSourceGetCount(src.0) == 0 {
            return None;
        }
        let img = Owned(CGImageSourceCreateImageAtIndex(src.0, 0, opts.0));
        if img.0.is_null() {
            return None;
        }
        let (iw, ih) = (CGImageGetWidth(img.0) as u64, CGImageGetHeight(img.0) as u64);
        if iw * ih * 4 > max_bytes || (x + w) as u64 > iw || (y + h) as u64 > ih {
            return None;
        }
        let part = Owned(CGImageCreateWithImageInRect(img.0, CGRect { origin: CGPoint { x: x as f64, y: y as f64 }, size: CGSize { width: w as f64, height: h as f64 } }));
        if part.0.is_null() {
            return None;
        }
        let (ow, oh) = (out_w as usize, out_h as usize);
        let mut px = vec![0u8; ow * oh * 4];
        let cs = Owned(CGColorSpaceCreateWithName(kCGColorSpaceSRGB));
        let ctx = Owned(CGBitmapContextCreate(px.as_mut_ptr() as *mut c_void, ow, oh, 8, ow * 4, cs.0, PREMULTIPLIED_LAST));
        if ctx.0.is_null() {
            return None;
        }
        // kCGInterpolationHigh.
        CGContextSetInterpolationQuality(ctx.0, 3);
        CGContextDrawImage(ctx.0, CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: ow as f64, height: oh as f64 } }, part.0);
        drop(ctx);
        Some(px)
    }
}
