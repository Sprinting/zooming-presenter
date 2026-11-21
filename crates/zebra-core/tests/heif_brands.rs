//! Mac pass 21 Nov, M20: every HEIC was refused with "this file is a
//! video, not an image". HEIF, AVIF and MP4 share one container (an
//! `ftyp` box naming brands), and the movie sniffer took every `ftyp`
//! file for a video unless it named a sound brand. The brands below are
//! as real files carry them.

use base64::Engine;
use serde_json::json;
use zebra_core::media::{fixtures::tiny_mp4, info, is_heic, sniff_kind};
use zebra_core::*;

fn bx(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(typ);
    v.extend_from_slice(body);
    v
}

/// An `ftyp` box: major brand, minor version, compatible brands.
fn ftyp(major: &[u8; 4], minor: u32, compatible: &[&[u8; 4]]) -> Vec<u8> {
    let mut body = major.to_vec();
    body.extend(minor.to_be_bytes());
    for b in compatible {
        body.extend_from_slice(*b);
    }
    bx(b"ftyp", &body)
}

/// A HEIF still: `ftyp`, then a `meta` box whose primary item (1) is
/// `w`x`h` (`ispe`), then an `mdat` of nothing.
fn heif(ftyp: Vec<u8>, w: u32, h: u32) -> Vec<u8> {
    let mut ispe = vec![0, 0, 0, 0];
    ispe.extend(w.to_be_bytes());
    ispe.extend(h.to_be_bytes());
    let ipco = [bx(b"hvcC", &[1, 2, 3]), bx(b"ispe", &ispe)].concat();
    let ipma = [0u8, 0, 0, 0, 0, 0, 0, 1, 0, 1, 2, 1, 2];
    let iprp = bx(b"iprp", &[bx(b"ipco", &ipco), bx(b"ipma", &ipma)].concat());
    let mut hdlr = vec![0u8; 24];
    hdlr[8..12].copy_from_slice(b"pict");
    let meta = bx(b"meta", &[vec![0, 0, 0, 0], bx(b"hdlr", &hdlr), bx(b"pitm", &[0, 0, 0, 0, 0, 1]), iprp].concat());
    [ftyp, meta, bx(b"mdat", &[0; 64])].concat()
}

/// `tiny_mp4`'s movie with its `ftyp` box swapped for `f`.
fn movie(f: Vec<u8>, video: bool) -> Vec<u8> {
    let m = tiny_mp4(640, 360, 6, video);
    let old = u32::from_be_bytes(m[0..4].try_into().unwrap()) as usize;
    [f, m[old..].to_vec()].concat()
}

// The `ftyp` boxes of real files.
/// An iPhone photo (iOS 17).
fn iphone_heic() -> Vec<u8> {
    ftyp(b"heic", 0, &[b"mif1", b"MiHE", b"MiPr", b"miaf", b"MiHB", b"heic"])
}
/// A 10-bit HEIF from a camera (Canon, Sony).
fn heix() -> Vec<u8> {
    ftyp(b"heix", 0, &[b"mif1", b"heix"])
}
/// A HEIF with only the structural brand (libheif, Nokia's samples).
fn mif1() -> Vec<u8> {
    ftyp(b"mif1", 0, &[b"mif1", b"heic"])
}
/// mif1 and nothing else: no codec brand at all.
fn bare_mif1() -> Vec<u8> {
    ftyp(b"mif1", 0, &[b"mif1"])
}
/// A HEIF image sequence (Nokia's "starfield" sample; Apple's HEICS).
fn msf1() -> Vec<u8> {
    ftyp(b"msf1", 0, &[b"msf1", b"iso8", b"hevc"])
}
/// An AVIF (libavif).
fn avif() -> Vec<u8> {
    ftyp(b"avif", 0, &[b"avif", b"mif1", b"miaf", b"MA1B"])
}
/// An MP4 (ffmpeg).
fn mp4() -> Vec<u8> {
    ftyp(b"isom", 0x200, &[b"isom", b"iso2", b"avc1", b"mp41"])
}
/// An M4V (iTunes, HandBrake).
fn m4v() -> Vec<u8> {
    ftyp(b"M4V ", 1, &[b"M4V ", b"M4A ", b"mp42", b"isom"])
}
/// A QuickTime movie.
fn mov() -> Vec<u8> {
    ftyp(b"qt  ", 0x2005_0300, &[b"qt  "])
}
/// An iTunes M4A.
fn m4a() -> Vec<u8> {
    ftyp(b"M4A ", 0, &[b"M4A ", b"mp42", b"isom"])
}

#[test]
fn heif_brands_are_pictures() {
    for (name, f) in [("heic", iphone_heic()), ("heix", heix()), ("mif1", mif1()), ("bare mif1", bare_mif1()), ("msf1", msf1())] {
        let file = heif(f, 4032, 3024);
        assert!(is_heic(&file), "{name}");
        assert_eq!(sniff_kind(&file), Some(MediaKind::Image), "{name}");
        let m = info(&file).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!((m.ext.as_str(), m.kind, m.width, m.height), ("heic", MediaKind::Image, 4032, 3024), "{name}");
        // The head alone (what a drop reads) says the same.
        assert_eq!(sniff_kind(&file[..40]), Some(MediaKind::Image), "{name} head");
    }
}

#[test]
fn movie_and_sound_brands_stay_movies_and_sound() {
    for (name, f, ext) in [("mp4", mp4(), "mp4"), ("m4v", m4v(), "m4v"), ("mov", mov(), "mov")] {
        let file = movie(f, true);
        assert!(!is_heic(&file), "{name}");
        assert_eq!(sniff_kind(&file), Some(MediaKind::Video), "{name}");
        let m = info(&file).unwrap();
        assert_eq!((m.ext.as_str(), m.kind, m.width, m.height), (ext, MediaKind::Video, 640, 360), "{name}");
    }
    let file = movie(m4a(), false);
    assert_eq!(sniff_kind(&file), Some(MediaKind::Audio));
    assert_eq!(info(&file).unwrap().ext, "m4a");
}

/// An AVIF is a picture Zebra doesn't take: it is refused as one, not
/// as "a video".
#[test]
fn avif_is_a_picture_zebra_does_not_take() {
    let file = heif(avif(), 100, 100);
    assert!(!is_heic(&file));
    assert_eq!(sniff_kind(&file), None);
    let e = info(&file).unwrap_err();
    assert!(e.message.contains("AVIF"), "{e:?}");
    assert!(!e.message.contains("video"), "{e:?}");
}

fn add_image_path(path: &std::path::Path) -> ZResult<Outcome> {
    let cmd: Command = serde_json::from_value(json!({"op": "add_image", "path": path.to_string_lossy(), "x": 0, "y": 0})).unwrap();
    Editor::default().apply(cmd, Origin::Human)
}

/// What failed on the Mac: `add_image` by path (and the person's Insert,
/// which runs the same command) of an iPhone photo.
#[test]
fn a_heic_comes_in_by_path_and_as_bytes_and_reopens_as_a_picture() {
    let dir = std::env::temp_dir().join(format!("zebra-heif-brands-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("IMG_0001.HEIC");
    let file = heif(iphone_heic(), 4032, 3024);
    std::fs::write(&path, &file).unwrap();
    let mut ed = Editor::default();
    let cmd: Command = serde_json::from_value(json!({"op": "add_image", "path": path.to_string_lossy(), "x": 0, "y": 0})).unwrap();
    ed.apply(cmd, Origin::Human).expect("a HEIC by path");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&file);
    let cmd: Command = serde_json::from_value(json!({"op": "add_image", "data_base64": b64, "x": 0, "y": 0})).unwrap();
    ed.apply(cmd, Origin::Human).expect("a HEIC as bytes");
    let a = ed.doc.assets.values().next().unwrap();
    assert_eq!((a.ext.as_str(), a.width, a.height), ("heic", 4032, 3024));
    // Saved and opened again, it is still a picture (it was sniffed as
    // a video on open).
    let doc = zebra_core::io::load_bytes(&zebra_core::io::save_bytes(&ed.doc).unwrap()).unwrap();
    let a = doc.assets.values().next().unwrap();
    assert_eq!((a.ext.as_str(), a.media_kind()), ("heic", MediaKind::Image));
    // A video named as a picture is still a video.
    let mp4_path = dir.join("clip.heic");
    std::fs::write(&mp4_path, movie(mp4(), true)).unwrap();
    let e = add_image_path(&mp4_path).unwrap_err();
    assert!(e.message.contains("video"), "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Brands that lie, boxes cut short and sizes of 4 GB are refused or
/// read as far as they go: never a panic, never a big allocation.
#[test]
fn hostile_ftyp_boxes() {
    // A movie whose brands say HEIC has no picture header: refused.
    let lying = movie(ftyp(b"heic", 0, &[b"mif1", b"heic"]), true);
    assert_eq!(sniff_kind(&lying), Some(MediaKind::Image));
    assert!(info(&lying).is_err());
    // A HEIC whose brands say MP4 has no movie in it: not a picture.
    let lying = heif(mp4(), 10, 10);
    assert_ne!(info(&lying).ok().map(|m| m.kind), Some(MediaKind::Image));
    // Cut short anywhere.
    let whole = heif(iphone_heic(), 4032, 3024);
    for n in 0..whole.len() {
        let _ = sniff_kind(&whole[..n]);
        let _ = info(&whole[..n]);
        let _ = is_heic(&whole[..n]);
    }
    // The major brand alone (12 bytes) is enough to know a picture.
    assert_eq!(sniff_kind(b"\0\0\0\x0cftypheic"), Some(MediaKind::Image));
    assert!(info(b"\0\0\0\x0cftypheic").is_err());
    // An `ftyp` claiming 4 GB, and a largesize one claiming 2^64 - 1,
    // over 64 MB of brands: read from the first 256 KB, with nothing
    // collected (the old reader built a Vec of every brand there).
    let mut huge = u32::MAX.to_be_bytes().to_vec();
    huge.extend_from_slice(b"ftypisom\0\0\0\0");
    huge.resize(64 << 20, b'x');
    assert_eq!(sniff_kind(&huge), Some(MediaKind::Video));
    let _ = info(&huge);
    huge[..4].copy_from_slice(&1u32.to_be_bytes());
    huge[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    huge[16..20].copy_from_slice(b"heic");
    assert!(is_heic(&huge));
    let _ = info(&huge);
    // A brand past the first 256 KB doesn't count.
    let mut far = u32::MAX.to_be_bytes().to_vec();
    far.extend_from_slice(b"ftypisom\0\0\0\0");
    far.resize(300 << 10, b'x');
    far.extend_from_slice(b"heic");
    assert!(!is_heic(&far));
    // A size smaller than its own header.
    assert!(!is_heic(b"\0\0\0\x04ftypheic\0\0\0\0mif1"));
    let _ = info(b"\0\0\0\x04ftypheic\0\0\0\0mif1");
}
