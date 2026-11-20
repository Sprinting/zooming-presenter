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
