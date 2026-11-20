//! What happens to media nothing uses any more, to the files Zebra makes
//! for it, and to what a crash leaves behind. Own test binary: the spill
//! folder is process-wide.
use serde_json::json;
use zebra_core::*;

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

fn files(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

#[test]
fn media_lifecycle() {
    let root = std::env::temp_dir().join(format!("zebra-lifecycle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let spill = root.join("spill");
    std::fs::create_dir_all(&spill).unwrap();
    media::set_spill_dir(Some(spill.clone()));
    let h = Origin::Human;

    // A clip inserted from a file is copied to a spill file named for what
    // it is (a .mov that holds an MP4 is renamed .mp4), which a player opens
    // as it is: no second copy.
    let mut clip = media::fixtures::tiny_mp4(640, 360, 5, true);
    clip.resize(200_000, 0);
    let src = root.join("Holiday.MOV");
    std::fs::write(&src, &clip).unwrap();
    let mut ed = Editor::default();
    let out = ed.apply(cmd(json!({"op":"add_video","path":src,"x":0,"y":0})), h.clone()).unwrap();
    let v = out.created[0].clone();
    let key = match &ed.doc.get(&v).unwrap().kind {
        ElementKind::Video(d) => d.asset.clone(),
        _ => unreachable!(),
    };
    let a = ed.doc.assets[&key].clone();
    let own = a.bytes.own_file().unwrap().to_path_buf();
    assert_eq!(own.extension().unwrap(), a.ext.as_str(), "{own:?}");
    let play = media::materialize(&key, &a.ext, &a.bytes).unwrap();
    assert_eq!(play.path(), own.as_path(), "played from its spill file, not a copy");
    drop(play);
    drop(a);
    // The original can go: the document has its own copy.
    std::fs::remove_file(&src).unwrap();

    // Delete it: kept while Undo can bring it back...
    ed.apply(cmd(json!({"op":"delete","ids":[v]})), h.clone()).unwrap();
    assert!(ed.prune_assets().contains(&key));
    assert!(ed.doc.assets.contains_key(&key));
    // ...and while Redo can (undone, then not redone yet).
    ed.undo(h.clone()).unwrap();
    ed.undo(h.clone()).unwrap();
    assert!(ed.doc.elements.is_empty());
    assert!(ed.prune_assets().contains(&key));
    // A new change drops the redo steps: nothing can bring the clip back,
    // so it goes, and its spill file with it.
    ed.apply(cmd(json!({"op":"add_text","text":"next","x":0,"y":0})), h.clone()).unwrap();
    assert!(ed.prune_assets().is_empty());
    assert!(!ed.doc.assets.contains_key(&key));
    assert!(!own.exists(), "the spill file went with the last use of the clip");

    // A sound set by key (as an import does) needs an audio asset the
    // document has.
    let r = ed.apply(Command::SetSoundtrack { data_base64: String::new(), path: None, looping: None, volume: None, name: None, asset: Some(key.clone()) }, h.clone());
    assert!(r.is_err());

    // What crashed saves and opens left beside a document goes; what a
    // running Zebra is writing stays; other files are never touched.
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    for n in ["talk.zebra", ".talk.zebra.saving-999999999", ".talk.zebra.open-999999999", "talk.zebra.tmp", "notes.tmp", "other.zebra.tmp"] {
        std::fs::write(docs.join(n), b"x").unwrap();
    }
    let mine = format!(".talk.zebra.saving-{}", std::process::id());
    std::fs::write(docs.join(&mine), b"x").unwrap();
    io::sweep_temp(&docs, |p| p == std::process::id());
    let mut want = vec![mine, "notes.tmp".to_string(), "other.zebra.tmp".to_string(), "talk.zebra".to_string()];
    want.sort();
    assert_eq!(files(&docs), want);

    // Players' copies: a run that died leaves its own and older builds'
    // (`<key>.<ext>`, the sample clip); the next launch sweeps them, never
    // a running Zebra's.
    let cache = root.join("cache");
    // SAFETY: this binary's only test, nothing reads it concurrently.
    unsafe { std::env::set_var("ZEBRA_MEDIA_CACHE", &cache) };
    std::fs::create_dir_all(&cache).unwrap();
    let live = format!("{}-abc.mp4", std::process::id());
    for n in ["999999999-abc.mp4", "abc.mp4", "demo-clip.mp4", live.as_str()] {
        std::fs::write(cache.join(n), b"x").unwrap();
    }
    media::sweep_cache(|p| p == std::process::id());
    assert_eq!(files(&cache), vec![live]);
    std::fs::remove_dir_all(&root).unwrap();
}
