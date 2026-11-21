//! TIFF, BMP and HEIC pictures come in as Keynote takes them, kept as
//! they are; photos show upright whatever orientation their file gives
//! (EXIF in a JPEG, PNG, WebP or TIFF; a HEIC's rotation and mirror).

mod common;
use common::*;
use zebra_core::media::{oriented_from, picture_header, PictureFormat};
use zebra_core::*;
use zebra_render::images::{for_web, orient_pixels, picture_region};
use zebra_render::{CaptureTarget, Renderer};

/// A `w`x`h` picture whose left half is red and right half blue.
fn halves(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgb([220, 20, 20]) } else { image::Rgb([20, 20, 220]) })
}

fn encode(img: &image::RgbImage, fmt: image::ImageFormat) -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, fmt).unwrap();
    out.into_inner()
}

/// An EXIF block (big-endian TIFF layout) holding only `orientation`.
fn exif(orientation: u16) -> Vec<u8> {
    let mut t = b"MM\0\x2a\0\0\0\x08\0\x01".to_vec();
    t.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1]);
    t.extend_from_slice(&orientation.to_be_bytes());
    t.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    t
}

/// `jpeg` with an APP1 EXIF block saying `orientation`.
fn jpeg_turned(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(exif(orientation));
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend(app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn crc32(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in b {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// `png` with an eXIf chunk saying `orientation`, after its IHDR.
fn png_turned(png: &[u8], orientation: u16) -> Vec<u8> {
    let data = exif(orientation);
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = b"eXIf".to_vec();
    body.extend(&data);
    chunk.extend(&body);
    chunk.extend(crc32(&body).to_be_bytes());
    let at = 8 + 25;
    let mut out = png[..at].to_vec();
    out.extend(chunk);
    out.extend_from_slice(&png[at..]);
    out
}

/// An ISO media box.
fn iso(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(ty);
    b.extend_from_slice(body);
    b
}

/// The boxes of a HEIC that says its primary picture is `w`x`h` with
/// these transforms (`irot` quarters anticlockwise, then `imir` axis),
/// and no picture data: enough for its header.
fn heic(w: u32, h: u32, irot: Option<u8>, imir: Option<u8>) -> Vec<u8> {
    let mut out = iso(b"ftyp", b"heic\0\0\0\0mif1heic");
    let mut ispe = vec![0, 0, 0, 0];
    ispe.extend(w.to_be_bytes());
    ispe.extend(h.to_be_bytes());
    let mut ipco = iso(b"hvcC", &[1, 2, 3]);
    ipco.extend(iso(b"ispe", &ispe));
    let mut assoc = vec![1u8, 2];
    if let Some(r) = irot {
        ipco.extend(iso(b"irot", &[r]));
        assoc.push(3);
    }
    if let Some(m) = imir {
        ipco.extend(iso(b"imir", &[m]));
        assoc.push(if irot.is_some() { 4 } else { 3 });
    }
    // ipma: version 0, flags 0, one entry: item 1.
    let mut ipma = vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 1, assoc.len() as u8];
    ipma.extend(assoc);
    let mut iprp = iso(b"ipco", &ipco);
    iprp.extend(iso(b"ipma", &ipma));
    let mut meta = vec![0, 0, 0, 0];
    meta.extend(iso(b"hdlr", &[0; 24]));
    meta.extend(iso(b"pitm", &[0, 0, 0, 0, 0, 1]));
    meta.extend(iso(b"iprp", &iprp));
    out.extend(iso(b"meta", &meta));
    out.extend(iso(b"mdat", &[0; 64]));
    out
}

fn deck(bytes: Vec<u8>) -> (Renderer, Editor, String) {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(200.0, 200.0);
    let key = ed.add_asset(bytes).unwrap();
    let a = &ed.doc.assets[&key];
    let (w, h) = (a.width as f64, a.height as f64);
    add(&mut ed, "p", 0.0, 0.0, w, h, 1.0, 0.0, ElementKind::Image(ImageData { asset: key.clone(), alt: String::new(), crop: None, mask: ImageMask::Rectangle, symbol: None, page: None }));
    (r, ed, key)
}

/// The colour at (x, y) of a 200x200 capture fitting the picture.
fn colour_at(r: &Renderer, ed: &Editor, (x, y): (u32, u32)) -> [u8; 3] {
    let png = r.capture(ed, &CaptureTarget::Element { id: "p".into() }, 200, 200).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgb8();
    img.get_pixel(x, y).0
}

fn reddish(c: [u8; 3]) -> bool {
    c[0] > 150 && c[2] < 100
}
fn bluish(c: [u8; 3]) -> bool {
    c[2] > 150 && c[0] < 100
}

/// TIFF and BMP come in as they are and draw as the file says.
#[test]
fn tiff_and_bmp_pictures_come_in_and_draw() {
    for (fmt, ext) in [(image::ImageFormat::Tiff, "tiff"), (image::ImageFormat::Bmp, "bmp")] {
        let file = encode(&halves(120, 60), fmt);
        let (r, ed, key) = deck(file.clone());
        let a = &ed.doc.assets[&key];
        assert_eq!((a.ext.as_str(), a.width, a.height), (ext, 120, 60));
        assert_eq!(a.bytes.as_ref(), &file[..], "kept as it is");
        assert!(reddish(colour_at(&r, &ed, (40, 100))), "{ext} left");
        assert!(bluish(colour_at(&r, &ed, (160, 100))), "{ext} right");
    }
}

/// A TIFF whose first directory is at the end of a big file: its size is
/// found from the offset, not from the first 256 KB.
#[test]
fn a_tiff_header_is_found_wherever_it_is() {
    let mut f = b"II\x2a\0".to_vec();
    let body = 600_000usize;
    f.extend(((8 + body) as u32).to_le_bytes());
    f.extend(vec![0u8; body]);
    f.extend(3u16.to_le_bytes());
    for (tag, ty, v) in [(256u16, 4u16, 70_000u32), (257, 3, 500), (274, 3, 8)] {
        f.extend(tag.to_le_bytes());
        f.extend(ty.to_le_bytes());
        f.extend(1u32.to_le_bytes());
        f.extend(v.to_le_bytes());
    }
    f.extend([0; 4]);
    let h = picture_header(&f).unwrap();
    assert_eq!((h.format, h.width, h.height, h.orientation, h.shown()), (PictureFormat::Tiff, 70_000, 500, 8, (500, 70_000)));
    // BigTIFF too.
    let mut b = b"MM\0\x2b\0\x08\0\0".to_vec();
    b.extend(16u64.to_be_bytes());
    b.extend(2u64.to_be_bytes());
    for (tag, v) in [(256u16, 9u8), (257, 7)] {
        b.extend(tag.to_be_bytes());
        b.extend(4u16.to_be_bytes());
        b.extend(1u64.to_be_bytes());
        b.extend([0, 0, 0, v, 0, 0, 0, 0]);
    }
    let h = picture_header(&b).unwrap();
    assert_eq!((h.width, h.height), (9, 7));
    // Broken: a directory past the end, an entry count of 65,535.
    let mut broken = b"II\x2a\0\xff\xff\xff\x7f".to_vec();
    broken.extend([0; 32]);
    assert!(picture_header(&broken).is_none(), "no size: not a picture");
    let mut many = b"II\x2a\0\x08\0\0\0\xff\xff".to_vec();
    many.extend([0; 40]);
    assert!(picture_header(&many).is_none());
}

/// A HEIC's size comes from its primary picture, turned by its `irot`
/// and `imir`. Only the Mac decodes HEIC; here it comes in and draws the
/// "can't be shown" placeholder, without failing.
#[test]
fn heic_pictures_come_in_with_their_turn() {
    let cases = [(None, None, 1, (4032, 3024)), (Some(1), None, 8, (3024, 4032)), (Some(2), None, 3, (4032, 3024)), (Some(3), None, 6, (3024, 4032)), (None, Some(0), 2, (4032, 3024)), (None, Some(1), 4, (4032, 3024))];
    for (irot, imir, o, shown) in cases {
        let h = picture_header(&heic(4032, 3024, irot, imir)).unwrap();
        assert_eq!((h.format, h.orientation, h.shown()), (PictureFormat::Heic, o, shown), "{irot:?} {imir:?}");
    }
    // A turn then a mirror is one of the eight, worked out, not looked up:
    // check it against applying the two in turn.
    let h = picture_header(&heic(3, 2, Some(1), Some(0))).unwrap();
    let grid: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0, 255]).collect();
    let (once, s1) = orient_pixels(grid.clone(), (3, 2), 8);
    let (twice, _) = orient_pixels(once, s1, 2);
    assert_eq!(orient_pixels(grid, (3, 2), h.orientation).0, twice);

    let (r, ed, key) = deck(heic(400, 300, Some(3), None));
    let a = &ed.doc.assets[&key];
    assert_eq!((a.ext.as_str(), a.width, a.height), ("heic", 300, 400));
    let _ = colour_at(&r, &ed, (100, 100));
    // An AVIF shares the container but isn't one.
    let mut avif = heic(10, 10, None, None);
    avif[8..12].copy_from_slice(b"avif");
    avif[16..24].copy_from_slice(b"mif1avif");
    assert!(Editor::default().add_asset(avif).is_err());
}

/// A photo turned by its EXIF shows upright, as Keynote shows it: its
/// size is turned and so are its pixels.
#[test]
fn photos_show_upright() {
    let jpeg = encode(&halves(80, 40), image::ImageFormat::Jpeg);
    let png = encode(&halves(80, 40), image::ImageFormat::Png);
    for file in [jpeg_turned(&jpeg, 6), png_turned(&png, 6)] {
        let (r, ed, key) = deck(file);
        let a = &ed.doc.assets[&key];
        assert_eq!((a.width, a.height), (40, 80), "{}", a.ext);
        // Turned a quarter clockwise: the left (red) half is on top.
        assert!(reddish(colour_at(&r, &ed, (100, 40))), "{} top", a.ext);
        assert!(bluish(colour_at(&r, &ed, (100, 160))), "{} bottom", a.ext);
    }
    // Upside down (3): red on the right.
    let (r, ed, _) = deck(jpeg_turned(&jpeg, 3));
    assert!(bluish(colour_at(&r, &ed, (40, 100))) && reddish(colour_at(&r, &ed, (160, 100))));
}

/// All eight orientations turn pixels as the image library does.
#[test]
fn the_eight_orientations_match_the_image_library() {
    let (w, h) = (5u32, 3u32);
    let src = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8 * 40, y as u8 * 80, (x * h + y) as u8, 255]));
    for o in 1..=8u8 {
        let mut want = image::DynamicImage::ImageRgba8(src.clone());
        want.apply_orientation(image::metadata::Orientation::from_exif(o).unwrap());
        let (got, size) = orient_pixels(src.as_raw().clone(), (w, h), o);
        assert_eq!(size, (want.width(), want.height()), "{o}");
        assert_eq!(got, want.to_rgba8().into_raw(), "orientation {o}");
        // And `oriented_from` is the map behind it.
        let (x, y) = (0, size.1 - 1);
        let (sx, sy) = oriented_from(o, (w, h), (x, y));
        assert_eq!(&got[((y * size.0 + x) * 4) as usize..][..4], &src.get_pixel(sx, sy).0, "{o}");
    }
}

/// A part of a turned picture (P6 detail tiles) is that part of it as
/// shown, in every orientation.
#[test]
fn a_part_of_a_turned_picture_is_the_part_shown() {
    let (w, h) = (50u32, 30u32);
    let src = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8 * 5, y as u8 * 8, ((x / 7 + y / 5) % 2) as u8 * 255]));
    let png = encode(&src, image::ImageFormat::Png);
    for o in 1..=8u8 {
        let file = png_turned(&png, o as u16);
        let head = picture_header(&file).unwrap();
        let shown = head.shown();
        let rgba: Vec<u8> = src.pixels().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let (whole, _) = orient_pixels(rgba, (w, h), o);
        let (x, y, rw, rh) = (3, 4, 11, 7);
        let part = picture_region(&file, shown, shown, (x, y, rw, rh)).unwrap_or_else(|| panic!("orientation {o}"));
        let mut want = vec![];
        for row in y..y + rh {
            let at = ((row * shown.0 + x) * 4) as usize;
            want.extend_from_slice(&whole[at..at + rw as usize * 4]);
        }
        assert_eq!(part, want, "orientation {o}");
    }
}

/// The web player gets a TIFF, BMP or HEIC as a JPEG (or a PNG with
/// transparency), upright.
#[test]
fn the_web_gets_a_jpeg_or_png() {
    let tiff = encode(&halves(64, 32), image::ImageFormat::Tiff);
    let (ext, file) = for_web(&tiff).unwrap();
    assert_eq!(ext, "jpg");
    let img = image::load_from_memory(&file).unwrap();
    assert_eq!((img.width(), img.height()), (64, 32));
    let clear = image::RgbaImage::from_fn(20, 10, |x, _| image::Rgba([0, 0, 0, if x < 10 { 0 } else { 255 }]));
    let mut bmp = std::io::Cursor::new(vec![]);
    clear.write_to(&mut bmp, image::ImageFormat::Bmp).unwrap();
    assert_eq!(for_web(bmp.get_ref()).unwrap().0, "png");
    let turned = for_web(&png_turned(&encode(&halves(64, 32), image::ImageFormat::Png), 6)).unwrap();
    let img = image::load_from_memory(&turned.1).unwrap();
    assert_eq!((img.width(), img.height()), (32, 64));
}

/// Huge or broken files are refused from their header, never decoded.
#[test]
fn huge_or_broken_new_formats_are_turned_away() {
    // A BMP claiming 100,000 x 100,000 pixels.
    let mut bmp = encode(&halves(4, 4), image::ImageFormat::Bmp);
    bmp[18..22].copy_from_slice(&100_000i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&100_000i32.to_le_bytes());
    // (On the Mac ImageIO draws it scaled, so it comes in.)
    let h = picture_header(&bmp).unwrap();
    assert_eq!((h.width, h.height, h.fits()), (100_000, 100_000, cfg!(target_os = "macos")));
    // A HEIC with no size, and one whose boxes run past the end.
    assert!(Editor::default().add_asset(heic(0, 0, None, None)).is_err());
    let mut cut = heic(10, 10, Some(1), None);
    let n = cut.len();
    cut.truncate(n - 72 - 10);
    assert!(Editor::default().add_asset(cut).is_err());
}

/// Grey of row `y` of a 256x64 capture centred on world (cx, 0).
fn grey_row(r: &Renderer, ed: &Editor, cx: f64, zoom: f64, y: u32) -> Vec<u8> {
    let cam = Camera { cx, cy: 0.0, zoom, rotation: 0.0 };
    let png = r.capture(ed, &CaptureTarget::Camera { camera: cam }, 256, 64).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    (0..img.width()).map(|x| img.get_pixel(x, y)[0]).collect()
}

/// Stripes `period` screen pixels apart, deep black to white.
fn striped(row: &[u8], period: usize) -> bool {
    let n = row.len() - period;
    let same = (0..n).filter(|&i| row[i].abs_diff(row[i + period]) < 40).count();
    let deep = (0..n).filter(|&i| {
        let w = &row[i..i + period];
        w.iter().max().unwrap() - w.iter().min().unwrap() > 150
    });
    same * 10 >= n * 9 && deep.count() * 10 >= n * 9
}

fn big_deck(file: Vec<u8>) -> (Renderer, Editor) {
    let (r, mut ed, key) = deck(file);
    ed.view.viewport = Vec2::new(256.0, 64.0);
    let a = &ed.doc.assets[&key];
    assert_eq!((a.width, a.height), (20_000, 600));
    (r, ed)
}

/// Large files: a 20,000 x 600 TIFF, and a PNG stored 600 x 20,000 that
/// its EXIF turns a quarter, both stay sharp to their own pixels when
/// zoomed in (the detail tiles, read from the stored pixels and turned).
#[test]
fn big_tiffs_and_turned_pictures_are_sharp_close_up() {
    let wide = image::GrayImage::from_fn(20_000, 600, |x, _| image::Luma([if x % 2 == 0 { 0 } else { 255 }]));
    let mut tiff = std::io::Cursor::new(vec![]);
    wide.write_to(&mut tiff, image::ImageFormat::Tiff).unwrap();
    let tiff = tiff.into_inner();
    // Inserted from a file, as a person drops it: its tags sit after
    // 12 MB of pixels, past the first 256 KB Zebra reads to sniff it.
    let at = out_dir().join("wide.tiff");
    std::fs::write(&at, &tiff).unwrap();
    let mut by_path = Editor::default();
    let key = by_path.add_asset_file(&at).unwrap();
    let a = &by_path.doc.assets[&key];
    assert_eq!((a.ext.as_str(), a.width, a.height), ("tiff", 20_000, 600));
    let _ = std::fs::remove_file(&at);
    let (r, ed) = big_deck(tiff);
    assert!(!striped(&grey_row(&r, &ed, 3000.0, 0.25, 32), 8), "zoomed out: the 8,192 level");
    for cx in [-9000.0, 0.5, 9_900.25] {
        assert!(striped(&grey_row(&r, &ed, cx, 4.0, 32), 8), "tiff at {cx}");
    }
    let tall = image::GrayImage::from_fn(600, 20_000, |_, y| image::Luma([if y % 2 == 0 { 0 } else { 255 }]));
    let (r, ed) = big_deck(png_turned(&encode_grey(&tall), 6));
    for cx in [-9000.0, 0.5, 9_900.25] {
        assert!(striped(&grey_row(&r, &ed, cx, 4.0, 32), 8), "turned png at {cx}");
    }
}

fn encode_grey(img: &image::GrayImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
