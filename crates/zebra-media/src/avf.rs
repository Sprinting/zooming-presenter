//! AVFoundation backend (macOS).
//!
//! * Video: `AVPlayer` decodes (hardware where it can); frames are pulled
//!   from an `AVPlayerItemVideoOutput` as BGRA pixel buffers and converted
//!   to RGBA, scaled down to what the caller shows, in one pass.
//! * Audio: `AVAudioPlayer`.
//! * Posters: `AVAssetImageGenerator` drawn into a bitmap context.
//!
//! Threading: `AVPlayer` and `AVPlayerItem` are main-thread objects. The
//! app creates, drives and drops players from the egui update loop, which
//! runs on the main thread on macOS; `open_video` refuses to run anywhere
//! else. Posters and audio have no such requirement.
//!
//! Memory: a player holds no decoded frame of its own. `next_frame` hands
//! the one converted frame to the caller; AVFoundation's decode queue is a
//! few frames deep. Files are streamed from disk, never read whole.

use std::cell::Cell;
use std::ffi::c_void;
use std::path::Path;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2_av_foundation::{
    AVAssetImageGenerator, AVMediaTypeVideo, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem,
    AVPlayerItemVideoOutput, AVURLAsset,
};
use objc2_avf_audio::AVAudioPlayer;
use objc2_core_foundation::{CFRetained, CFString, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGInterpolationQuality, kCGColorSpaceSRGB,
};
use objc2_core_media::{CMClock, CMTime, kCMTimePositiveInfinity, kCMTimeZero};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString, NSURL};

use crate::{AudioPlayer, Backend, Frame, VideoPlayer, encode_png, fit};

/// CMTime timescale for times made from seconds (600 divides evenly by
/// 24, 25, 30 and 60 fps).
const TIMESCALE: i32 = 600;

/// How close to the end counts as ended (one frame at 60 fps).
const VIDEO_END_SLACK: f64 = 1.0 / 60.0;
const AUDIO_END_SLACK: f64 = 0.05;

pub struct AvFoundation;

impl Backend for AvFoundation {
    fn name(&self) -> &'static str {
        "avfoundation"
    }
    fn can_play(&self) -> bool {
        true
    }
    fn open_video(&self, path: &Path) -> Result<Box<dyn VideoPlayer>, String> {
        Ok(Box::new(Video::open(path)?))
    }
    fn open_audio(&self, path: &Path) -> Result<Box<dyn AudioPlayer>, String> {
        Ok(Box::new(Audio::open(path)?))
    }
    fn poster_png(&self, path: &Path, at: f64, max_dim: u32) -> Result<Vec<u8>, String> {
        poster(path, at, max_dim).map(|f| encode_png(&f))
    }
}

// ---------------------------------------------------------------- helpers

fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
    if !path.is_file() {
        return Err(format!("{}: no such file", path.display()));
    }
    NSURL::from_file_path(path).ok_or_else(|| format!("{}: not a usable file path", path.display()))
}

fn describe(e: &NSError) -> String {
    e.localizedDescription().to_string()
}

fn cmtime(secs: f64) -> CMTime {
    // SAFETY: CMTimeMakeWithSeconds is a pure function of its arguments.
    unsafe { CMTime::with_seconds(secs.max(0.0), TIMESCALE) }
}

/// Seconds, or None when the time is invalid, indefinite or infinite
/// (CMTimeGetSeconds returns NaN or an infinity for those).
fn seconds(t: CMTime) -> Option<f64> {
    // SAFETY: CMTimeGetSeconds is a pure function of its argument.
    let s = unsafe { t.seconds() };
    s.is_finite().then_some(s)
}

/// Open `path` as an asset and check it is playable.
fn open_asset(path: &Path) -> Result<Retained<AVURLAsset>, String> {
    let url = file_url(path)?;
    // SAFETY: plain constructor and property read on a freshly made asset.
    // `isPlayable` loads the file header synchronously, which is quick for
    // the local files this app opens.
    unsafe {
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        if !asset.isPlayable() {
            return Err(format!(
                "{}: not a media file this Mac can play",
                path.display()
            ));
        }
        Ok(asset)
    }
}

/// Quarter turns clockwise needed to show the first video track upright
/// (phone footage is stored sideways with a rotation in the track header).
/// Err when the asset has no video track.
fn video_quarter_turns(asset: &AVURLAsset) -> Result<u8, &'static str> {
    // SAFETY: AVMediaTypeVideo is a constant NSString exported by
    // AVFoundation; the track lookup and transform read are synchronous
    // property reads (the lookup is deprecated in favour of an async one,
    // but still supported and fine for a local file).
    #[allow(deprecated)]
    let t = unsafe {
        let Some(kind) = AVMediaTypeVideo else {
            return Err("no video track");
        };
        let tracks = asset.tracksWithMediaType(kind);
        let Some(track) = tracks.firstObject() else {
            return Err("no video track");
        };
        track.preferredTransform()
    };
    // The transform maps stored pixels to display pixels in y-down
    // coordinates: (x, y) -> (a*x + c*y, b*x + d*y) + translation.
    Ok(if t.a.abs() >= t.b.abs() {
        if t.a >= 0.0 { 0 } else { 2 }
    } else if t.b > 0.0 {
        1
    } else {
        3
    })
}

// ------------------------------------------------------------------ video

struct Video {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    /// The clock `CACurrentMediaTime` reads; maps "now" to an item time.
    host_clock: CFRetained<CMClock>,
    /// From the asset at open, for when the item does not know yet.
    asset_duration: Option<f64>,
    /// Quarter turns clockwise to apply to decoded frames.
    turns: u8,
    /// Target and start of a seek that may still be in flight. AVPlayer
    /// seeks asynchronously and reports the old time until it lands, so
    /// `position` answers with the target meanwhile; otherwise a caller
    /// looping on `ended()` would seek again every frame.
    seeking: Cell<Option<(f64, Instant)>>,
}

// SAFETY: AVPlayer and friends are main-thread objects and not Send in
// objc2. The trait needs `Send` so players can live in app state, but this
// app only creates, calls and drops players from the main thread (the egui
// update loop runs there on macOS), and `Video::open` checks that it is on
// the main thread. Nothing here moves a player to another thread.
unsafe impl Send for Video {}

impl Video {
    fn open(path: &Path) -> Result<Video, String> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err("video players must be opened on the main thread".into());
        };
        let asset = open_asset(path)?;
        let turns = video_quarter_turns(&asset).map_err(|e| format!("{}: {e}", path.display()))?;

        // Ask the output for BGRA buffers (the cheapest format to turn
        // into RGBA).
        let format = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_32BGRA);
        let format: &AnyObject = &format;
        // SAFETY: the key is a constant CFString exported by CoreVideo;
        // CFString is toll-free bridged to NSString, so the pointer cast is
        // the documented way to use it as a dictionary key.
        let key: &NSString = unsafe {
            let key: &'static CFString = kCVPixelBufferPixelFormatTypeKey;
            &*(key as *const CFString).cast::<NSString>()
        };
        let attrs = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[format]);

        // SAFETY: plain AVFoundation constructors and setters, called on
        // the main thread (`mtm`), with live, correctly typed arguments.
        // The attributes dictionary has the NSString -> NSNumber shape the
        // output expects.
        unsafe {
            let asset_duration = seconds(asset.duration()).filter(|d| *d > 0.0);
            let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(&attrs),
            );
            let item = AVPlayerItem::playerItemWithAsset(&asset, mtm);
            item.addOutput(&output);
            let player = AVPlayer::playerWithPlayerItem(Some(&item), mtm);
            // Stop at the end: `ended()` reports it and the caller loops by
            // seeking.
            player.setActionAtItemEnd(AVPlayerActionAtItemEnd::Pause);
            Ok(Video {
                player,
                item,
                output,
                host_clock: CMClock::host_time_clock(),
                asset_duration,
                turns,
                seeking: Cell::new(None),
            })
        }
    }

    /// The player's own idea of the position.
    fn raw_position(&self) -> f64 {
        // SAFETY: property read on the main thread.
        seconds(unsafe { self.player.currentTime() })
            .unwrap_or(0.0)
            .max(0.0)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        // SAFETY: plain call on the main thread (see `unsafe impl Send`).
        // Stops the sound at once instead of whenever the player is freed.
        unsafe { self.player.pause() }
    }
}

impl VideoPlayer for Video {
    fn play(&mut self) {
        // Like the other backends, play at the end starts over.
        if self.ended() {
            self.seek(0.0);
        }
        // SAFETY: plain call on the main thread.
        unsafe { self.player.play() }
    }
    fn pause(&mut self) {
        // SAFETY: plain call on the main thread.
        unsafe { self.player.pause() }
    }
    fn is_playing(&self) -> bool {
        // SAFETY: property read on the main thread.
        unsafe { self.player.rate() != 0.0 }
    }
    fn seek(&mut self, secs: f64) {
        let secs = match self.duration() {
            Some(d) => secs.clamp(0.0, d),
            None => secs.max(0.0),
        };
        self.seeking.set(Some((secs, Instant::now())));
        // SAFETY: plain call on the main thread; kCMTimeZero is a constant
        // exported by CoreMedia. Zero tolerance asks for the exact frame.
        unsafe {
            self.player.seekToTime_toleranceBefore_toleranceAfter(
                cmtime(secs),
                kCMTimeZero,
                kCMTimeZero,
            )
        }
    }
    fn position(&self) -> f64 {
        let now = self.raw_position();
        if let Some((target, since)) = self.seeking.get() {
            // The seek has landed once the player reports a time at (or,
            // when playing, just past) the target. Give up waiting after a
            // second so a failed seek cannot pin the position.
            let landed = now >= target - 0.05 && now <= target + 0.5;
            if !landed && since.elapsed().as_secs_f64() < 1.0 {
                return target;
            }
            self.seeking.set(None);
        }
        now
    }
    fn duration(&self) -> Option<f64> {
        // SAFETY: property read on the main thread.
        let item = seconds(unsafe { self.item.duration() }).filter(|d| *d > 0.0);
        item.or(self.asset_duration)
    }
    fn set_muted(&mut self, muted: bool) {
        // SAFETY: plain setter on the main thread.
        unsafe { self.player.setMuted(muted) }
    }
    fn set_volume(&mut self, volume: f32) {
        // SAFETY: plain setter on the main thread.
        unsafe { self.player.setVolume(volume.clamp(0.0, 1.0)) }
    }
    fn next_frame(&mut self, max_dim: u32) -> Option<Frame> {
        // SAFETY: calls on the main thread with valid arguments. A null
        // `itemTimeForDisplay` out-pointer is allowed by the API. The
        // returned buffer is owned (+1, "copy") and released on drop.
        let buffer = unsafe {
            // The item time that should be on screen now, like
            // `itemTimeForHostTime(CACurrentMediaTime())`; the host time
            // clock is the clock CACurrentMediaTime reads.
            let host = seconds(self.host_clock.time());
            let mut t = match host {
                Some(h) => self.output.itemTimeForHostTime(h),
                None => self.item.currentTime(),
            };
            if seconds(t).is_none() {
                t = self.item.currentTime();
            }
            if !self.output.hasNewPixelBufferForItemTime(t) {
                return None;
            }
            self.output
                .copyPixelBufferForItemTime_itemTimeForDisplay(t, std::ptr::null_mut())?
        };
        convert(&buffer, max_dim, self.turns)
    }
    fn ended(&self) -> bool {
        match self.duration() {
            Some(d) => self.position() >= d - VIDEO_END_SLACK,
            None => false,
        }
    }
}

/// Unlocks the pixel buffer when dropped.
struct ReadLock<'a>(&'a CVPixelBuffer);

impl Drop for ReadLock<'_> {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful read-only lock taken in
        // `convert`, with the same flags, on the same live buffer.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags::ReadOnly) };
    }
}

/// Copy a BGRA pixel buffer into an RGBA frame no larger than `max_dim`,
/// turned upright by `turns` quarter turns clockwise.
fn convert(buffer: &CVPixelBuffer, max_dim: u32, turns: u8) -> Option<Frame> {
    if CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_32BGRA {
        return None; // we asked for BGRA; anything else we cannot read
    }
    // SAFETY: `buffer` is a live pixel buffer. The lock is released by
    // `ReadLock` on every path out of this function.
    if unsafe { CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) }
        != kCVReturnSuccess
    {
        return None;
    }
    let _lock = ReadLock(buffer);

    let (w, h) = (
        CVPixelBufferGetWidth(buffer),
        CVPixelBufferGetHeight(buffer),
    );
    let stride = CVPixelBufferGetBytesPerRow(buffer);
    let base = CVPixelBufferGetBaseAddress(buffer) as *const u8;
    if base.is_null() || w == 0 || h == 0 || stride < w * 4 || w > 1 << 15 || h > 1 << 15 {
        return None;
    }
    // SAFETY: while locked, a non-planar buffer's base address points at
    // `h` rows of `stride` bytes, each holding at least `w` 4-byte pixels
    // (checked above); the slice stops at the last pixel of the last row
    // and is not used after `_lock` drops.
    let src = unsafe { std::slice::from_raw_parts(base, stride * (h - 1) + w * 4) };
    Some(bgra_to_rgba(src, w, h, stride, max_dim, turns))
}

/// The conversion proper: BGRA rows of `stride` bytes in, tightly packed
/// RGBA out, fitted to `max_dim` and rotated by `turns` quarter turns
/// clockwise. Video is opaque, so alpha is set to 255 (premultiplied and
/// straight alpha are then the same thing).
fn bgra_to_rgba(src: &[u8], w: usize, h: usize, stride: usize, max_dim: u32, turns: u8) -> Frame {
    // Size before rotation (sw x sh), then the size the caller sees.
    let (sw, sh) = fit(w as u32, h as u32, max_dim);
    let (sw, sh) = (sw as usize, sh as usize);
    let sideways = turns % 2 == 1;
    let (out_w, out_h) = if sideways { (sh, sw) } else { (sw, sh) };
    let mut rgba = vec![0u8; out_w * out_h * 4];

    if (sw, sh) == (w, h) && turns == 0 {
        // Same size, upright: swap red and blue row by row.
        for (dst, row) in rgba.chunks_exact_mut(w * 4).zip(src.chunks(stride)) {
            for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], 255]);
            }
        }
    } else {
        // Source byte offset of each scaled column and row (nearest
        // neighbour). When shrinking to half or less, average a 2x2 block
        // around the sample instead, which removes most of the shimmer.
        let average = w >= sw * 2 && h >= sh * 2;
        let nudge = |full: usize, scaled: usize| if average { (full / scaled - 2) / 2 } else { 0 };
        let (nx, ny) = (nudge(w, sw), nudge(h, sh));
        let xs: Vec<usize> = (0..sw).map(|x| (x * w / sw + nx) * 4).collect();
        let ys: Vec<usize> = (0..sh).map(|y| (y * h / sh + ny) * stride).collect();
        let mut out = rgba.chunks_exact_mut(4);
        for oy in 0..out_h {
            for ox in 0..out_w {
                // Where this output pixel sits in the unrotated image.
                let (x, y) = match turns % 4 {
                    0 => (ox, oy),
                    1 => (oy, sh - 1 - ox),
                    2 => (sw - 1 - ox, sh - 1 - oy),
                    _ => (sw - 1 - oy, ox),
                };
                let i = ys[y] + xs[x];
                let px = if average {
                    // The block's far corner is in range: the sample is at
                    // least one pixel and one row short of the edge when
                    // the scale is 2 or more.
                    let j = i + stride;
                    let avg = |c: usize| {
                        ((src[i + c] as u32
                            + src[i + 4 + c] as u32
                            + src[j + c] as u32
                            + src[j + 4 + c] as u32
                            + 2)
                            / 4) as u8
                    };
                    [avg(2), avg(1), avg(0), 255]
                } else {
                    [src[i + 2], src[i + 1], src[i], 255]
                };
                if let Some(d) = out.next() {
                    d.copy_from_slice(&px);
                }
            }
        }
    }
    Frame {
        width: out_w as u32,
        height: out_h as u32,
        rgba,
    }
}

// ------------------------------------------------------------------ audio

struct Audio {
    player: Retained<AVAudioPlayer>,
    /// We asked it to play and have not paused since. If it is then not
    /// playing, it stopped by itself: it reached the end.
    running: bool,
    looping: bool,
}

// SAFETY: Retained<AVAudioPlayer> is not Send in objc2 because the class
// makes no thread-safety promise. This app only creates, calls and drops
// players from the main thread (the egui update loop), so a player is
// never used from two threads.
unsafe impl Send for Audio {}

impl Audio {
    fn open(path: &Path) -> Result<Audio, String> {
        let url = file_url(path)?;
        // SAFETY: plain initialiser with a live file URL; errors come back
        // as NSError. The file is streamed, not loaded whole.
        let player =
            unsafe { AVAudioPlayer::initWithContentsOfURL_error(AVAudioPlayer::alloc(), &url) }
                .map_err(|e| format!("{}: {}", path.display(), describe(&e)))?;
        // SAFETY: plain call; preloads buffers so play() starts promptly.
        // A false return only means play() will do the preparing.
        unsafe { player.prepareToPlay() };
        Ok(Audio {
            player,
            running: false,
            looping: false,
        })
    }
}

impl AudioPlayer for Audio {
    fn play(&mut self) {
        // SAFETY: plain call on a live player.
        self.running = unsafe { self.player.play() };
    }
    fn pause(&mut self) {
        self.running = false;
        // SAFETY: plain call on a live player.
        unsafe { self.player.pause() }
    }
    fn is_playing(&self) -> bool {
        // SAFETY: property read.
        unsafe { self.player.isPlaying() }
    }
    fn seek(&mut self, secs: f64) {
        let secs = match self.duration() {
            Some(d) => secs.clamp(0.0, d),
            None => secs.max(0.0),
        };
        // SAFETY: plain setter; the value is within the sound's length.
        unsafe { self.player.setCurrentTime(secs) }
    }
    fn position(&self) -> f64 {
        // SAFETY: property read.
        unsafe { self.player.currentTime() }.max(0.0)
    }
    fn duration(&self) -> Option<f64> {
        // SAFETY: property read.
        let d = unsafe { self.player.duration() };
        (d.is_finite() && d > 0.0).then_some(d)
    }
    fn set_volume(&mut self, volume: f32) {
        // SAFETY: plain setter.
        unsafe { self.player.setVolume(volume.clamp(0.0, 1.0)) }
    }
    fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
        // SAFETY: plain setter; -1 loops forever, 0 plays once.
        unsafe { self.player.setNumberOfLoops(if looping { -1 } else { 0 }) }
    }
    fn ended(&self) -> bool {
        if self.looping {
            return false;
        }
        // AVAudioPlayer stops and rewinds to 0 when it finishes, so "we
        // started it and it is no longer playing" is the reliable sign.
        if self.running && !self.is_playing() {
            return true;
        }
        self.duration()
            .is_some_and(|d| self.position() >= d - AUDIO_END_SLACK)
    }
}

// ----------------------------------------------------------------- poster

fn poster(path: &Path, at: f64, max_dim: u32) -> Result<Frame, String> {
    let asset = open_asset(path)?;
    let fail = |what: &str| format!("{}: {what}", path.display());

    // SAFETY: plain AVFoundation calls with live arguments; the generator
    // may be used from any thread. `copyCGImageAtTime` is the synchronous
    // call (deprecated in favour of an async one, still supported); a null
    // `actualTime` out-pointer is allowed. The CMTime constants are
    // exported by CoreMedia.
    #[allow(deprecated)]
    let image = unsafe {
        let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
        generator.setAppliesPreferredTrackTransform(true);
        if max_dim > 0 {
            // The generator keeps the aspect ratio inside this box.
            generator.setMaximumSize(CGSize {
                width: max_dim as f64,
                height: max_dim as f64,
            });
        }
        // Stay a little short of the end: there is no frame at `duration`.
        let at = match seconds(asset.duration()) {
            Some(d) => at.clamp(0.0, (d - 0.05).max(0.0)),
            None => at.max(0.0),
        };
        generator.setRequestedTimeToleranceBefore(kCMTimeZero);
        generator.setRequestedTimeToleranceAfter(kCMTimeZero);
        match generator.copyCGImageAtTime_actualTime_error(cmtime(at), std::ptr::null_mut()) {
            Ok(image) => image,
            Err(_) => {
                // Some files cannot give the exact frame; take the nearest
                // one it can decode.
                generator.setRequestedTimeToleranceBefore(kCMTimePositiveInfinity);
                generator.setRequestedTimeToleranceAfter(kCMTimePositiveInfinity);
                generator
                    .copyCGImageAtTime_actualTime_error(cmtime(at), std::ptr::null_mut())
                    .map_err(|e| fail(&describe(&e)))?
            }
        }
    };

    let (iw, ih) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    if iw == 0 || ih == 0 || iw > 1 << 15 || ih > 1 << 15 {
        return Err(fail("the poster frame has no usable size"));
    }
    let (w, h) = fit(iw as u32, ih as u32, max_dim);
    let mut rgba = vec![0u8; w as usize * h as usize * 4];

    // sRGB when available, device RGB otherwise.
    // SAFETY: kCGColorSpaceSRGB is a constant CFString exported by
    // CoreGraphics.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .or_else(CGColorSpace::new_device_rgb)
        .ok_or_else(|| fail("no RGB colour space"))?;
    // SAFETY: the context draws into `rgba`, which is exactly
    // `h` rows of `w * 4` bytes and outlives the context (dropped at the
    // end of this block, before `rgba` is read or moved). 8 bits per
    // component with premultiplied alpha last and the default byte order
    // is R, G, B, A in memory.
    {
        let context = unsafe {
            CGBitmapContextCreate(
                rgba.as_mut_ptr().cast::<c_void>(),
                w as usize,
                h as usize,
                8,
                w as usize * 4,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .ok_or_else(|| fail("could not make a bitmap for the poster"))?;
        CGContext::set_interpolation_quality(Some(&context), CGInterpolationQuality::High);
        let rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: w as f64,
                height: h as f64,
            },
        };
        // Bitmap memory starts at the top row, so the image lands upright.
        CGContext::draw_image(Some(&context), rect, Some(&image));
    }
    // `encode_png` writes an opaque image; video frames are opaque, but
    // make sure of it rather than store stray alpha.
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Frame {
        width: w,
        height: h,
        rgba,
    })
}
