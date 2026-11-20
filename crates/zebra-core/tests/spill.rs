//! New media goes to a spill file and is memory-mapped, not kept on the
//! heap (own test binary: the spill folder is process-wide).
use serde_json::json;
use zebra_core::*;

#[test]
fn new_media_is_mapped_from_a_spill_file_that_leaves_nothing_behind() {
    let dir = std::env::temp_dir().join(format!("zebra-spill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    zebra_core::media::set_spill_dir(Some(dir.clone()));
    // A picture well over the spill threshold (noise doesn't compress).
    let mut img = image::RgbImage::new(256, 256);
    let mut seed = 1u32;
    for p in img.pixels_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *p = image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8]);
    }
    let mut png = std::io::Cursor::new(vec![]);
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    assert!(png.len() >= zebra_core::media::SPILL_MIN);
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let mut ed = Editor::default();
    ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64,"x":0,"y":0})).unwrap(), Origin::agent("a")).unwrap();
    let a = ed.doc.assets.values().next().unwrap();
    assert!(a.bytes.is_mapped(), "spilled and mapped");
    assert_eq!(&a.bytes[..], &png[..]);
    let spilled = |dir: &std::path::Path| std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".png")).count();
    assert_eq!(spilled(&dir), 1, "one spill file, named for its process and key");
    assert_eq!(a.bytes.own_file().and_then(|p| p.extension()).and_then(|e| e.to_str()), Some("png"));
    // Undo and redo keep the picture; a save writes it and reads back the
    // same, and the spill file goes: the saved file holds the bytes now.
    ed.undo(Origin::Human).unwrap();
    ed.redo(Origin::Human).unwrap();
    let file = dir.join("deck.zebra");
    ed.save_now(&file).unwrap();
    assert_eq!(spilled(&dir), 0, "the spill file goes once the save holds the picture");
    let back = zebra_core::io::load(&file).unwrap();
    assert_eq!(&back.assets.values().next().unwrap().bytes[..], &png[..]);
    // An imported file's pictures and PDF are spilled the same way.
    let (_, imported) = zebra_core::import::asset_from(png.clone()).unwrap();
    assert!(imported.bytes.is_mapped());
    assert_eq!((&imported.bytes[..], imported.width, imported.ext.as_str()), (&png[..], 256, "png"));
    drop(imported);
    assert_eq!(spilled(&dir), 0, "a spill file goes with the last asset using it");
    let mut pdf = b"%PDF-1.4\n".to_vec();
    pdf.extend(std::iter::repeat_n(b'x', zebra_core::media::SPILL_MIN));
    let (_, imported) = zebra_core::import::asset_from(pdf.clone()).unwrap();
    assert!(imported.bytes.is_mapped());
    assert_eq!((&imported.bytes[..], imported.ext.as_str()), (&pdf[..], "pdf"));
    // Small media stays in memory.
    let key = ed.add_svg_asset(b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>".to_vec());
    assert!(!ed.doc.assets[&key].bytes.is_mapped());
    // A leftover of a run that died is swept; this run's are kept.
    std::fs::write(dir.join("999999999-0-abc.spill"), b"x").unwrap();
    std::fs::write(dir.join("999999999-3-abc.mp4"), b"x").unwrap();
    std::fs::write(dir.join(format!("{}-7-abc.spill", std::process::id())), b"x").unwrap();
    zebra_core::media::sweep_spill(|pid| pid == std::process::id());
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.contains("-abc.")).collect();
    left.sort();
    assert_eq!(left, vec![format!("{}-7-abc.spill", std::process::id())]);
    let _ = std::fs::remove_dir_all(&dir);
}
