//! Drawing SVG symbols (Insert › Symbols & Shapes) to pixels, with the
//! symbol's colours and the sticker look applied.
use resvg::{tiny_skia, usvg};

/// How to draw one symbol. Colours are straight RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SvgStyle {
    /// One-colour symbols: replaces `currentColor`. Two-colour symbols:
    /// the outline colour.
    pub color: Option<[u8; 4]>,
    /// Two-colour symbols (IconPark): the fill colour.
    pub color2: Option<[u8; 4]>,
    /// White outline and soft shadow.
    pub sticker: bool,
}

impl SvgStyle {
    /// Part of the image cache key.
    pub fn key(&self) -> String {
        let hex = |c: Option<[u8; 4]>| c.map_or(String::from("-"), |c| format!("{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3]));
        format!("{}{}{}", hex(self.color), hex(self.color2), if self.sticker { "s" } else { "" })
    }

    pub fn is_plain(&self) -> bool {
        *self == SvgStyle::default()
    }
}

/// Margin round a sticker for its outline and shadow, as a fraction of the
/// long side.
pub const STICKER_MARGIN: f64 = 0.07;

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The SVG text with the style's colours put in.
fn recolour(svg: &str, style: &SvgStyle) -> String {
    let mut s = svg.to_string();
    if let Some(c) = style.color {
        let h = hex(c);
        s = s.replace("currentColor", &h);
        if style.color2.is_some() {
            for k in ["\"#000\"", "\"#000000\""] {
                s = s.replace(k, &format!("\"{h}\""));
            }
        }
    }
    if let Some(c) = style.color2 {
        let h = format!("\"{}\"", hex(c));
        for k in ["\"#2F88FF\"", "\"#2f88ff\"", "\"#43CCF8\"", "\"#43ccf8\""] {
            s = s.replace(k, &h);
        }
    }
    // Colour alpha becomes the symbol's opacity.
    match style.color.map(|c| c[3]) {
        Some(a) if a < 255 => s.replacen("<svg ", &format!("<svg opacity=\"{:.3}\" ", a as f64 / 255.0), 1),
        _ => s,
    }
}

/// An SVG may hold pictures of its own, but only inside itself (`data:`
/// links). The default also opens any file an `<image href>` names, so a
/// picture someone sent could pull a file from this Mac into an export.
fn options() -> usvg::Options<'static> {
    let mut opt = usvg::Options::default();
    opt.image_href_resolver.resolve_string = Box::new(|_, _| None);
    opt
}

/// Draw `svg` into a `w` x `h` picture (premultiplied RGBA8), fitted and
/// centred. None when the SVG can't be read.
pub fn rasterize(svg: &[u8], style: &SvgStyle, w: u32, h: u32) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(svg).ok()?;
    let text = if style.color.is_some() || style.color2.is_some() { recolour(text, style) } else { text.to_string() };
    let opt = options();
    let tree = usvg::Tree::from_str(&text, &opt).ok()?;
    let mut pm = tiny_skia::Pixmap::new(w.max(1), h.max(1))?;
    let margin = if style.sticker { STICKER_MARGIN * w.max(h) as f64 } else { 0.0 };
    let size = tree.size();
    let (bw, bh) = (w as f64 - 2.0 * margin, h as f64 - 2.0 * margin);
    let k = (bw / size.width() as f64).min(bh / size.height() as f64);
    if !(k.is_finite() && k > 0.0) {
        return None;
    }
    let (dx, dy) = ((w as f64 - size.width() as f64 * k) / 2.0, (h as f64 - size.height() as f64 * k) / 2.0);
    let t = tiny_skia::Transform::from_row(k as f32, 0.0, 0.0, k as f32, dx as f32, dy as f32);
    if !affordable(&tree, t, w, h) {
        return None;
    }
    resvg::render(&tree, t, &mut pm.as_mut());
    let mut px = pm.take();
    if style.sticker {
        sticker(&mut px, w as usize, h as usize, (w.max(h) as f64 * 0.028).max(1.5));
    }
    Some(px)
}

/// Largest side an SVG is drawn at; a bigger level is drawn at this and
/// scaled.
pub const MAX_RASTER_SIDE: u32 = 4096;
/// Most work (pixels times kernel taps) one filter primitive may cost.
/// resvg's morphology looks at every pixel of its (2r+1)^2 window, so
/// `radius=100000` was hours of work; blurs are box blurs, whose cost
/// doesn't grow with the deviation.
const MAX_FILTER_WORK: f64 = (1u64 << 28) as f64;
/// Largest filter region, in pixels at the size drawn.
const MAX_FILTER_AREA: f32 = 16384.0 * 16384.0;
/// Largest pattern tile resvg may draw, in pixels: it allocates the whole
/// tile (a 5,000,000-unit tile asked for 655 TB and aborted, audit T-C2).
/// Masks and clip paths are drawn canvas-sized, so the raster cap bounds
/// them.
const MAX_TILE_AREA: f32 = 8192.0 * 8192.0;
/// Most nodes in one SVG (clip paths, masks and patterns included).
const MAX_NODES: usize = 100_000;

/// Whether `tree` drawn with `t` into `w`x`h` pixels stays cheap enough to
/// draw at all. A 277-byte SVG with `feMorphology radius=100000` pinned a
/// core for minutes (hostile-input audit H5); such pictures are drawn as
/// the "unreadable picture" placeholder instead.
fn affordable(tree: &usvg::Tree, t: tiny_skia::Transform, w: u32, h: u32) -> bool {
    let mut c = Cost { nodes: 0, canvas: w as f64 * h as f64, w: w as f64, h: h as f64 };
    c.group(tree.root(), t) && c.nodes <= MAX_NODES
}

struct Cost {
    nodes: usize,
    canvas: f64,
    w: f64,
    h: f64,
}

impl Cost {
    fn group(&mut self, g: &usvg::Group, t: tiny_skia::Transform) -> bool {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return false;
        }
        let (sx, sy) = t.pre_concat(g.abs_transform()).get_scale();
        for f in g.filters() {
            let r = f.rect();
            let area = r.width() * sx * r.height() * sy;
            if !(area <= MAX_FILTER_AREA) {
                return false;
            }
            // resvg clips the region to the canvas.
            let region = (area as f64).min(self.canvas);
            for p in f.primitives() {
                use usvg::filter::Kind;
                let taps = match p.kind() {
                    Kind::Morphology(m) => {
                        let rx = (m.radius_x().get() as f64 * sx as f64).min(self.w);
                        let ry = (m.radius_y().get() as f64 * sy as f64).min(self.h);
                        (2.0 * rx + 1.0) * (2.0 * ry + 1.0)
                    }
                    Kind::ConvolveMatrix(m) => m.matrix().columns() as f64 * m.matrix().rows() as f64,
                    Kind::Turbulence(tb) => tb.num_octaves() as f64,
                    _ => 1.0,
                };
                if !(region * taps <= MAX_FILTER_WORK) {
                    return false;
                }
            }
        }
        for n in g.children() {
            let ok = match n {
                usvg::Node::Group(c) => self.group(c, t),
                usvg::Node::Path(p) => {
                    self.nodes += 1;
                    let at = t.pre_concat(p.abs_transform());
                    let paints = p.fill().map(|f| f.paint()).into_iter().chain(p.stroke().map(|s| s.paint()));
                    self.nodes <= MAX_NODES && paints.into_iter().all(|paint| tile_ok(paint, at))
                }
                _ => {
                    self.nodes += 1;
                    self.nodes <= MAX_NODES
                }
            };
            // Clip paths, masks, patterns and filter images hold trees too.
            let mut sub_ok = true;
            n.subroots(|sub| sub_ok = sub_ok && self.group(sub, t));
            if !(ok && sub_ok) {
                return false;
            }
        }
        true
    }
}

/// A pattern's tile, drawn with `t`, is within [`MAX_TILE_AREA`].
fn tile_ok(paint: &usvg::Paint, t: tiny_skia::Transform) -> bool {
    let usvg::Paint::Pattern(p) = paint else { return true };
    let (sx, sy) = t.pre_concat(p.transform()).get_scale();
    let r = p.rect();
    r.width() * sx * r.height() * sy <= MAX_TILE_AREA
}

/// Squared distance from each pixel to the nearest pixel where `inside`
/// is true (Felzenszwalb and Huttenlocher, separable).
fn distance_sq(inside: &[bool], w: usize, h: usize) -> Vec<f64> {
    const INF: f64 = 1e20;
    let n = w.max(h);
    let mut d: Vec<f64> = inside.iter().map(|&b| if b { 0.0 } else { INF }).collect();
    let (mut f, mut out, mut v, mut z) = (vec![0f64; n], vec![0f64; n], vec![0usize; n], vec![0f64; n + 1]);
    let mut pass = |f: &[f64], n: usize, out: &mut [f64]| {
        let sect = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
        let mut k = 0usize;
        v[0] = 0;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            let mut s = sect(q, v[k]);
            while s <= z[k] && k > 0 {
                k -= 1;
                s = sect(q, v[k]);
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = INF;
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate().take(n) {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f64 - p as f64).powi(2) + f[p];
        }
    };
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        pass(&f, h, &mut out);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&d[y * w..y * w + w]);
        pass(&f, w, &mut out);
        d[y * w..y * w + w].copy_from_slice(&out[..w]);
    }
    d
}

/// Three box blurs of an alpha plane (about a Gaussian).
fn blur(a: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut tmp = vec![0f32; a.len()];
    for _ in 0..3 {
        for y in 0..h {
            let row = &a[y * w..y * w + w];
            let mut s: f32 = row[..r.min(w)].iter().sum();
            for x in 0..w {
                if x + r < w {
                    s += row[x + r];
                }
                if x > r {
                    s -= row[x - r - 1];
                }
                tmp[y * w + x] = s / (2 * r + 1) as f32;
            }
        }
        for x in 0..w {
            let mut s: f32 = (0..r.min(h)).map(|y| tmp[y * w + x]).sum();
            for y in 0..h {
                if y + r < h {
                    s += tmp[(y + r) * w + x];
                }
                if y > r {
                    s -= tmp[(y - r - 1) * w + x];
                }
                a[y * w + x] = s / (2 * r + 1) as f32;
            }
        }
    }
}

/// Put a white outline `r` pixels wide and a soft shadow under the
/// picture (premultiplied RGBA8, in place).
fn sticker(px: &mut [u8], w: usize, h: usize, r: f64) {
    let inside: Vec<bool> = px.chunks_exact(4).map(|p| p[3] > 24).collect();
    let d = distance_sq(&inside, w, h);
    let outline: Vec<f32> = d.iter().map(|&dd| (r - dd.sqrt() + 0.5).clamp(0.0, 1.0) as f32).collect();
    let r = r as f32;
    let mut shadow = outline.clone();
    blur(&mut shadow, w, h, (r * 0.6).round().max(1.0) as usize);
    let off = (r * 0.5).round() as usize;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let sh = if y >= off { shadow[(y - off) * w + x] * 0.35 } else { 0.0 };
            let ol = outline[i];
            // Back to front: shadow (black), outline (white), symbol.
            let mut c = [0f32; 4];
            c[3] = sh;
            let over = |c: &mut [f32; 4], src: [f32; 4]| {
                let k = 1.0 - src[3];
                for j in 0..4 {
                    c[j] = src[j] + c[j] * k;
                }
            };
            over(&mut c, [ol, ol, ol, ol]);
            let p = &px[i * 4..i * 4 + 4];
            over(&mut c, [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]);
            for j in 0..4 {
                px[i * 4 + j] = (c[j] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="currentColor"/></svg>"#;

    fn at(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    /// R28: an SVG cannot show a file from this computer.
    #[test]
    fn an_svg_cannot_open_local_files() {
        const RED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4" fill="red"/></svg>"#;
        let dir = std::env::temp_dir().join(format!("zebra-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret.svg");
        std::fs::write(&file, RED).unwrap();
        let with = |href: &str| {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><image x="0" y="0" width="10" height="10" href="{href}"/></svg>"#);
            rasterize(svg.as_bytes(), &SvgStyle::default(), 10, 10).unwrap()
        };
        // The file would be drawn with usvg's own options: the test means something.
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><image x="0" y="0" width="10" height="10" href="{}"/></svg>"#, file.display());
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        assert!(tree.root().has_children(), "usvg's default opens the file");
        assert!(with(&file.display().to_string()).iter().all(|b| *b == 0), "nothing of the file is drawn");
        // A picture inside the SVG itself still shows.
        use base64::Engine;
        let px = with(&format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(RED)));
        assert_eq!(at(&px, 10, 5, 5), [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bitmap inside an SVG shows: an EMF or WMF picture imported from
    /// PowerPoint is an SVG, and its bitmaps are PNG or JPEG data links.
    #[test]
    fn bitmaps_inside_an_svg_show() {
        use base64::Engine;
        let mut png = vec![];
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 160, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><image width="10" height="10" preserveAspectRatio="none" href="data:image/png;base64,{}"/></svg>"#,
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let px = rasterize(svg.as_bytes(), &SvgStyle::default(), 10, 10).unwrap();
        assert_eq!(at(&px, 10, 5, 5), [0, 160, 0, 255]);
    }

    #[test]
    fn one_colour_symbols_take_the_colour() {
        let px = rasterize(DOT, &SvgStyle { color: Some([200, 0, 0, 255]), ..Default::default() }, 40, 40).unwrap();
        assert_eq!(at(&px, 40, 20, 20), [200, 0, 0, 255]);
        assert_eq!(at(&px, 40, 1, 1)[3], 0);
    }

    #[test]
    fn a_sticker_has_a_white_edge_and_a_shadow() {
        let style = SvgStyle { color: Some([0, 0, 200, 255]), sticker: true, ..Default::default() };
        let px = rasterize(DOT, &style, 100, 100).unwrap();
        assert_eq!(at(&px, 100, 50, 50), [0, 0, 200, 255]);
        // Just outside the dot (radius 4/10 of the drawn box): white.
        let edge = at(&px, 100, 50, 50 - 36);
        assert!(edge[0] > 200 && edge[3] > 200, "{edge:?}");
        // Corners stay clear.
        assert_eq!(at(&px, 100, 0, 0)[3], 0);
    }

    #[test]
    fn two_colour_symbols_swap_outline_and_fill() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="5" height="10" fill="#2F88FF"/><rect x="5" width="5" height="10" fill="#000"/></svg>"##;
        let style = SvgStyle { color: Some([10, 20, 30, 255]), color2: Some([200, 100, 0, 255]), sticker: false };
        let px = rasterize(svg, &style, 20, 20).unwrap();
        assert_eq!(at(&px, 20, 2, 10), [200, 100, 0, 255]);
        assert_eq!(at(&px, 20, 17, 10), [10, 20, 30, 255]);
    }

    /// Hostile-input audit H5: this 8 KB SVG (feMorphology radius 100000)
    /// pinned a core for minutes. Now it is refused at once.
    #[test]
    fn a_morphology_bomb_is_refused_quickly() {
        let svg = include_bytes!("../tests/data/blur.svg");
        let t = std::time::Instant::now();
        assert!(rasterize(svg, &SvgStyle::default(), 400, 400).is_none());
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }

    /// Ordinary filters still draw, a big blur too (box blurs cost the same
    /// at any deviation).
    #[test]
    fn ordinary_filters_still_draw() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feGaussianBlur stdDeviation="30"/><feMorphology radius="1"/></filter><rect x="20" y="20" width="60" height="60" fill="red" filter="url(#f)"/></svg>"#;
        let px = rasterize(svg, &SvgStyle::default(), 1000, 1000).unwrap();
        assert!(at(&px, 1000, 500, 500)[3] > 0);
    }

    /// Audit T-C2: a pattern tile of 5,000,000 units (and a fuzzed EMF's
    /// conversion) made resvg allocate terabytes and abort.
    #[test]
    fn huge_pattern_tiles_are_refused() {
        for svg in [&include_bytes!("../tests/data/pattern-bomb.svg")[..], &include_bytes!("../tests/data/emf-pattern-bomb.svg")[..]] {
            assert!(rasterize(svg, &SvgStyle::default(), 400, 400).is_none());
        }
        let ok = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs><pattern id="p" patternUnits="userSpaceOnUse" width="10" height="10"><rect width="5" height="5" fill="red"/></pattern></defs><rect width="100" height="100" fill="url(#p)"/></svg>"#;
        let px = rasterize(ok, &SvgStyle::default(), 400, 400).unwrap();
        assert!(at(&px, 400, 5, 5)[3] > 0);
    }

    #[test]
    fn too_many_nodes_are_refused() {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
        for _ in 0..100_001 {
            svg.push_str(r#"<rect width="1" height="1"/>"#);
        }
        svg.push_str("</svg>");
        assert!(rasterize(svg.as_bytes(), &SvgStyle::default(), 10, 10).is_none());
    }

    #[test]
    fn unreadable_svg_is_none() {
        assert!(rasterize(b"<svg", &SvgStyle::default(), 10, 10).is_none());
    }
}
