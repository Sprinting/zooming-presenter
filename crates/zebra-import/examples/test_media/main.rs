//! Make a folder of huge and broken files for trying Zebra by hand (or
//! by an agent through zebractl): see `scripts/make-test-media.sh` and
//! `docs/TEST-MEDIA.md`, which is copied into the folder as its checklist.
//!
//! `cargo run --release -p zebra-import --example test_media -- <folder> [video MB]`
//!
//! Big files are sparse where the format allows (the padding is a hole
//! on APFS, so a "2 GB" video costs a few hundred KB of disk). The
//! PowerPoint file can't be: it really is that big.
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

const SEED_MP4: &[u8] = include_bytes!("seed.mp4");
const SEED_M4A: &[u8] = include_bytes!("seed.m4a");
const PLAN: &str = include_str!("../../../../docs/TEST-MEDIA.md");

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: test_media <folder> [video MB, default 2048]");
        std::process::exit(2);
    };
    let mb: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2048).max(1);
    let dir = Path::new(&dir);
    let big = dir.join("big");
    let bad = dir.join("hostile");
    for d in [&big, &bad] {
        std::fs::create_dir_all(d).expect("make the folder");
    }
    // Each line names the file as it is made (it used to say big/deck.pptx
    // and big/pages.pdf for files named otherwise).
    let step = |p: &Path, what: &str| {
        let name = p.strip_prefix(dir).unwrap_or(p).display().to_string();
        if what.is_empty() { eprintln!("  {name}") } else { eprintln!("  {name} ({what})") }
    };

    let video = big.join(format!("video-{mb}MB.mp4"));
    step(&video, "plays a 6 s clip; the rest is padding");
    padded(SEED_MP4, &video, mb << 20);
    let music = big.join(format!("music-{}MB.m4a", mb / 2));
    step(&music, "");
    padded(SEED_M4A, &music, (mb / 2).max(1) << 20);
    let wav_file = big.join("silence-1GB.wav");
    step(&wav_file, "");
    wav(&wav_file, 1 << 30);
    let picture = big.join("picture-217MB.png");
    step(&picture, "about 217 MB of noise, under Zebra's 256 MB limit");
    noise_png(&picture, 8700, 8700);
    let over = big.join("picture-over-limit-300MB.png");
    step(&over, "");
    sparse_after(&small_png(), &over, 300 << 20);
    let deck = big.join(format!("deck-with-{mb}MB-video.pptx"));
    step(&deck, "the big video inside a PowerPoint file; real size");
    pptx_with(&video, &deck);
    let pages = big.join("600-pages.pdf");
    step(&pages, "600 pages");
    pdf(&pages, 600);

    eprintln!("  hostile/…");
    std::fs::write(bad.join("empty.png"), b"").unwrap();
    std::fs::write(bad.join("empty.pptx"), b"").unwrap();
    std::fs::write(bad.join("empty.zebra"), b"").unwrap();
    std::fs::write(bad.join("garbage.mp4"), noise(1 << 20, 7)).unwrap();
    std::fs::write(bad.join("garbage.zebra"), noise(1 << 20, 8)).unwrap();
    std::fs::write(bad.join("text-named-picture.png"), "just some text\n".repeat(1000)).unwrap();
    std::fs::write(bad.join("picture-named-video.mov"), small_png()).unwrap();
    std::fs::write(bad.join("cut-off-video.mp4"), &SEED_MP4[..SEED_MP4.len() / 3]).unwrap();
    let extras = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extras.pptx")).unwrap();
    std::fs::write(bad.join("cut-off.pptx"), &extras[..extras.len() / 2]).unwrap();
    lying_mp4(&bad.join("lying-sizes.mp4"));
    sparse_after(b"koly-this-is-a-disk-image", &bad.join("disk-image-named.png"), 4 << 30);
    zebra_bomb_json(&bad.join("bomb-document.zebra"));
    zebra_bomb_asset(&bad.join("bomb-picture.zebra"));
    pptx_bomb(&extras, &bad.join("bomb-media.pptx"));

    std::fs::write(dir.join("TEST-MEDIA.md"), PLAN).unwrap();
    eprintln!("Made {}. Checklist: {}", dir.display(), dir.join("TEST-MEDIA.md").display());
}

/// `seed`, then a `free` box (skipped by every MP4 reader) up to `len`
/// bytes, left as a hole.
fn padded(seed: &[u8], out: &Path, len: u64) {
    let len = len.max(seed.len() as u64 + 16);
    let mut f = std::fs::File::create(out).unwrap();
    f.write_all(seed).unwrap();
    f.write_all(&1u32.to_be_bytes()).unwrap();
    f.write_all(b"free").unwrap();
    f.write_all(&(len - seed.len() as u64).to_be_bytes()).unwrap();
    f.set_len(len).unwrap();
}

/// `head`, then a hole up to `len` bytes.
fn sparse_after(head: &[u8], out: &Path, len: u64) {
    let f = std::fs::File::create(out).unwrap();
    (&f).write_all(head).unwrap();
    f.set_len(len).unwrap();
}

/// A real 8-bit mono WAV of `len` bytes (a hole of zeros: a click, then
/// nothing audible).
fn wav(out: &Path, len: u32) {
    let rate = 44_100u32;
    let n = len - 44;
    let mut h = b"RIFF".to_vec();
    h.extend_from_slice(&(36 + n).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&8u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&n.to_le_bytes());
    sparse_after(&h, out, len as u64);
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn small_png() -> Vec<u8> {
    let mut v = vec![];
    image::RgbImage::from_pixel(64, 64, image::Rgb([30, 120, 200])).write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

/// Noise doesn't compress: a PNG about as big as its pixels.
fn noise_png(out: &Path, w: u32, h: u32) {
    let px = noise((w * h * 3) as usize, 3);
    let img = image::RgbImage::from_raw(w, h, px).unwrap();
    let f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    let enc = image::codecs::png::PngEncoder::new_with_quality(f, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::NoFilter);
    img.write_with_encoder(enc).unwrap();
}

/// An MP4 whose boxes claim sizes far past the end of the file.
fn lying_mp4(out: &Path) {
    let mut v = SEED_MP4[..64].to_vec();
    v[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
    v.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xF0]);
    v.extend_from_slice(b"moov");
    v.extend_from_slice(&[0xFF; 64]);
    std::fs::write(out, v).unwrap();
}

fn deflated() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true)
}

fn zeros_into(w: &mut impl Write, n: u64) {
    let chunk = vec![0u8; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len() as u64) as usize;
        w.write_all(&chunk[..k]).unwrap();
        left -= k as u64;
    }
}

fn doc() -> String {
    zebra_core::io::to_json(&zebra_core::Document::default())
}

/// A `.zebra` of about 1 MB whose document.json is 1 GB of spaces.
fn zebra_bomb_json(out: &Path) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    z.start_file("document.json", deflated()).unwrap();
    let spaces = vec![b' '; 1 << 20];
    for _ in 0..1024 {
        z.write_all(&spaces).unwrap();
    }
    z.write_all(doc().as_bytes()).unwrap();
    z.finish().unwrap();
}

/// A `.zebra` of about 2 MB holding a "picture" that inflates to 2 GB:
/// a real PNG header, then zeros.
fn zebra_bomb_asset(out: &Path) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    z.start_file("document.json", deflated()).unwrap();
    z.write_all(doc().as_bytes()).unwrap();
    z.start_file("assets/bomb.png", deflated()).unwrap();
    let png = small_png();
    z.write_all(&png[..png.len() - 12]).unwrap();
    zeros_into(&mut z, 2 << 30);
    z.finish().unwrap();
}

/// The extras deck with its video swapped for one that inflates to 2 GB.
fn pptx_bomb(extras: &[u8], out: &Path) {
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(extras)).unwrap();
    let mut w = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    for i in 0..src.len() {
        let f = src.by_index_raw(i).unwrap();
        if f.name().unwrap() == "ppt/media/media1.mp4" {
            drop(f);
            w.start_file("ppt/media/media1.mp4", deflated()).unwrap();
            w.write_all(&SEED_MP4[..64]).unwrap();
            zeros_into(&mut w, 2 << 30);
        } else {
            w.raw_copy_file(f).unwrap();
        }
    }
    w.finish().unwrap();
}

/// The extras deck with its video replaced by `video`, stored as
/// PowerPoint stores media.
fn pptx_with(video: &Path, out: &Path) {
    let extras = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extras.pptx")).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(extras)).unwrap();
    let mut w = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(out).unwrap()));
    for i in 0..z.len() {
        let f = z.by_index_raw(i).unwrap();
        if f.name().unwrap() == "ppt/media/media1.mp4" {
            drop(f);
            let len = std::fs::metadata(video).unwrap().len();
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(len >= u32::MAX as u64);
            w.start_file("ppt/media/media1.mp4", o).unwrap();
            std::io::copy(&mut std::fs::File::open(video).unwrap(), &mut w).unwrap();
        } else {
            w.raw_copy_file(f).unwrap();
        }
    }
    w.finish().unwrap().flush().unwrap();
}

/// A plain PDF of `n` pages, each with its number in big type.
fn pdf(out: &Path, n: usize) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    let mut offsets = vec![];
    let mut pos = 0u64;
    let mut put = |f: &mut std::io::BufWriter<std::fs::File>, s: String, offsets: &mut Vec<u64>, obj: bool| {
        if obj {
            offsets.push(pos);
        }
        f.write_all(s.as_bytes()).unwrap();
        pos += s.len() as u64;
    };
    put(&mut f, "%PDF-1.4\n".into(), &mut offsets, false);
    // 1 catalog, 2 pages, 3 font, then (page, contents) pairs from 4.
    put(&mut f, "1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n".into(), &mut offsets, true);
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    put(&mut f, format!("2 0 obj << /Type /Pages /Count {n} /Kids [{}] >> endobj\n", kids.join(" ")), &mut offsets, true);
    put(&mut f, "3 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n".into(), &mut offsets, true);
    for i in 0..n {
        let p = 4 + 2 * i;
        put(&mut f, format!("{p} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 842 595] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >> endobj\n", p + 1), &mut offsets, true);
        let text = format!("BT /F1 120 Tf 300 250 Td (Page {}) Tj ET", i + 1);
        put(&mut f, format!("{} 0 obj << /Length {} >> stream\n{text}\nendstream endobj\n", p + 1, text.len() + 1), &mut offsets, true);
    }
    let xref = pos;
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        tail += &format!("{o:010} 00000 n \n");
    }
    tail += &format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1);
    f.write_all(tail.as_bytes()).unwrap();
    f.flush().unwrap();
    let _ = f.seek(SeekFrom::End(0));
}
