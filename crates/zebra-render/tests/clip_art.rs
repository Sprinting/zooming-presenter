//! Test-drive bug T2: some Tango clip-art (edit-cut, media-playback-start,
//! battery-caution) drew as a grey "can't be shown" box. Their soft
//! shadows are blurs the SVG cost checks used to refuse whole. Each now
//! draws exactly as resvg draws it. (Tango icons are public domain.)

use resvg::{tiny_skia, usvg};

#[test]
fn tango_clip_art_draws_whole() {
    for f in ["edit-cut", "media-playback-start", "battery-caution"] {
        let b = std::fs::read(format!("{}/tests/data/tango/{f}.svg", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let ours = zebra_render::svg::rasterize(&b, &Default::default(), 256, 256).unwrap_or_else(|| panic!("{f} didn't draw"));
        let tree = usvg::Tree::from_data(&b, &usvg::Options::default()).unwrap();
        let mut pm = tiny_skia::Pixmap::new(256, 256).unwrap();
        let k = 256.0 / tree.size().width().max(tree.size().height());
        resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
        let differ = ours.iter().zip(pm.data()).filter(|(a, b)| a.abs_diff(**b) > 8).count();
        assert_eq!(differ, 0, "{f}: {differ} bytes differ from resvg's drawing");
        assert!(ours.chunks(4).filter(|p| p[3] > 0).count() > 10_000, "{f} is mostly empty");
    }
}
