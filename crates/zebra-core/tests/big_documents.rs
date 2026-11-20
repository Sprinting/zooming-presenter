//! B1: Zebra has no document-size limit beyond memory, as Keynote. A deck
//! it saves always reopens (saved, mapped, autosaved); only a file built to
//! take all memory is refused, before it takes it.
use std::io::Write;
use zebra_core::*;

/// The budget is process-wide: tests that change it, or need the real
/// one, run one at a time.
static BUDGET: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn budget_lock() -> std::sync::MutexGuard<'static, ()> {
    BUDGET.lock().unwrap_or_else(|e| e.into_inner())
}

fn heap() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("RssAnon:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zebra-bigdoc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 400 text boxes of 100,000 characters: 40 MB of text, past the old
/// 32 MB refusal.
fn big_deck() -> Document {
    let mut ed = Editor::default();
    for i in 0..400 {
        let text: String = format!("Box {i}. ").chars().cycle().take(100_000).collect();
        ed.apply(serde_json::from_value(serde_json::json!({"op":"add_text","text":text,"x":i * 10,"y":0})).unwrap(), Origin::Human).unwrap();
    }
    ed.doc
}

fn texts(d: &Document) -> Vec<String> {
    let mut v: Vec<String> = d.elements.values().filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some(t.text.clone()) } else { None }).collect();
    v.sort();
    v
}

#[test]
fn a_deck_zebra_saves_always_reopens() {
    let _b = budget_lock();
    let doc = big_deck();
    let want = texts(&doc);
    assert_eq!(want.len(), 400);
    assert!(io::to_json(&doc).len() > 32 << 20, "past the old 32 MB refusal");

    // In memory.
    let bytes = io::save_bytes(&doc).unwrap();
    assert_eq!(texts(&io::load_bytes(&bytes).unwrap()), want);
    // Saved to a file and opened (mapped).
    let d = dir("reopen");
    let path = d.join("Big.zebra");
    let mut saved = doc.clone();
    io::save(&mut saved, &path).unwrap();
    let t = std::time::Instant::now();
    assert_eq!(texts(&io::load(&path).unwrap()), want);
    eprintln!("40 MB deck reopened in {:?}", t.elapsed());
    // Autosaved, and recovered after a crash.
    let auto = d.join("Big.autosave.zebra");
    io::write_autosave(&doc, &auto, Some(&path)).unwrap();
    assert_eq!(texts(&io::load_autosave(&auto, Some(&path)).unwrap()), want);
    // As a plain .json document.
    let json = d.join("Big.json");
    std::fs::write(&json, io::to_json(&doc)).unwrap();
    assert_eq!(texts(&io::load(&json).unwrap()), want);
    let _ = std::fs::remove_dir_all(&d);
}

/// `{"a":[0,0,0,…]}`: 2 bytes of JSON but 48 parsed, deflated a
/// thousandfold. Refused once what it would take passes the budget, in
/// constant memory, with the person told it needs more memory.
#[test]
fn a_document_built_to_take_all_memory_is_refused_before_it_does() {
    let _b = budget_lock();
    io::set_memory_budget(Some(64 << 20));
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    z.start_file("document.json", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true)).unwrap();
    z.write_all(b"{\"format_version\":1,\"a\":[").unwrap();
    let chunk = "0,".repeat(1 << 19);
    for _ in 0..200 {
        z.write_all(chunk.as_bytes()).unwrap();
    }
    z.write_all(b"0]}").unwrap();
    let bomb = z.finish().unwrap().into_inner();
    assert!(bomb.len() < 4 << 20, "{}", bomb.len());
    let before = heap();
    let t = std::time::Instant::now();
    let e = io::load_bytes(&bomb).unwrap_err();
    assert_eq!(e.code, "bad_file");
    assert_eq!(e.about, Some(error::About::Document), "told it needs more memory, not that it's damaged: {e:?}");
    assert!(e.message.contains("memory"), "{}", e.message);
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 32 << 20, "heap grew {} MB", (b - a) >> 20);
    }
    eprintln!("refused in {:?}", t.elapsed());

    // The same as a .json document.
    let d = dir("bomb");
    let p = d.join("bomb.json");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&p).unwrap());
    f.write_all(b"{\"a\":[").unwrap();
    for _ in 0..40 {
        f.write_all(chunk.as_bytes()).unwrap();
    }
    f.write_all(b"0]}").unwrap();
    drop(f);
    assert_eq!(io::load(&p).unwrap_err().about, Some(error::About::Document));

    // A small document under the same budget opens.
    assert!(io::load_bytes(&io::save_bytes(&Document::default()).unwrap()).is_ok());
    io::set_memory_budget(None);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_budget_is_half_of_memory_and_at_least_1_gb() {
    let _b = budget_lock();
    io::set_memory_budget(None);
    let b = io::memory_budget();
    assert!(b >= 1 << 30, "{b}");
    #[cfg(target_os = "linux")]
    {
        let total: u64 = std::fs::read_to_string("/proc/meminfo").unwrap().lines().find_map(|l| l.strip_prefix("MemTotal:")).unwrap().trim().trim_end_matches("kB").trim().parse::<u64>().unwrap() * 1024;
        assert_eq!(b, (total / 2).max(1 << 30));
    }
}
