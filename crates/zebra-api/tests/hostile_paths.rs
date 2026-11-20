//! An agent's `add_image {path}` naming something that never ends when
//! read (a device, a FIFO, a folder) is refused at once.
use serde_json::json;

#[test]
fn add_image_paths_that_are_not_files_are_refused() {
    let dir = std::env::temp_dir().join(format!("zebra-api-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("pipe.png");
    let mut paths = vec!["/dev/zero".to_string(), "/dev/random".to_string(), dir.to_string_lossy().to_string()];
    if std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
        paths.push(fifo.to_string_lossy().to_string());
    }
    let t = std::time::Instant::now();
    for p in paths {
        let r = zebra_api::protocol::parse_command(json!({"op":"add_image","path":p,"x":0,"y":0}));
        assert!(r.is_err(), "{p}");
    }
    assert!(t.elapsed().as_secs() < 2);
}

/// Agents can add what a person can drop: by path, a picture of any file
/// size (Kartik, 21 Nov: no cap; it's shown scaled down). Inline base64
/// keeps a transport bound, with a hint to use a path.
#[test]
fn add_image_by_path_has_no_size_cap() {
    let dir = std::env::temp_dir().join(format!("zebra-api-big-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sparse = |name: &str, len: u64| {
        let p = dir.join(name);
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(len).unwrap();
        p.to_string_lossy().to_string()
    };
    for (name, len) in [("picture-100MB.png", 100u64 << 20), ("picture-3GB.png", 3 << 30)] {
        let p = sparse(name, len);
        assert!(zebra_api::protocol::parse_command(json!({"op":"add_image","path":p,"x":0,"y":0})).is_ok(), "{name}");
    }
    // Inline: a request this big is refused with a pointer to `path`.
    let b64 = "A".repeat(((zebra_api::protocol::MAX_INLINE_IMAGE / 3) * 4 + 8) as usize);
    let e = zebra_api::protocol::parse_command(json!({"op":"add_image","data_base64":b64,"x":0,"y":0})).unwrap_err();
    assert_eq!(e.kind(), "too_big");
    assert!(e.hint_text().is_some_and(|h| h.contains("path")), "{:?}", e.hint_text());
    let _ = std::fs::remove_dir_all(&dir);
}
