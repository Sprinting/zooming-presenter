//! The clip-art pack on disk (format 2): every symbol a zstd frame
//! compressed with its set's dictionary.
//!
//! tests/data/symbols-v1 is a slice of Tabler Icons (MIT, every 30th
//! symbol) in the old raw format; tests/data/symbols-v2 is the same slice
//! as the real builder writes it:
//!     python3 -P tools/symbols/build_pack.py --from-pack \
//!         crates/zebra-core/tests/data/symbols-v1 --out crates/zebra-core/tests/data/symbols-v2
use std::path::{Path, PathBuf};

use zebra_core::symbols::{is_svg, Library};

fn data(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(dir)
}

/// (name, svg) of every symbol in a format 1 set, read directly.
fn raw_set(dir: &Path, key: &str) -> Vec<(String, Vec<u8>)> {
    let dat = std::fs::read(dir.join(format!("{key}.dat"))).unwrap();
    std::fs::read_to_string(dir.join(format!("{key}.idx")))
        .unwrap()
        .lines()
        .map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            let (off, len): (usize, usize) = (p[2].parse().unwrap(), p[3].parse().unwrap());
            (p[0].to_string(), dat[off..off + len].to_vec())
        })
        .collect()
}

#[test]
fn every_symbol_reads_back_byte_for_byte() {
    let lib = Library::open(&data("symbols-v2")).unwrap();
    let raw = raw_set(&data("symbols-v1"), "tabler");
    assert_eq!(lib.total(), raw.len());
    assert!(data("symbols-v2/tabler.dict").is_file(), "the builder trained a dictionary");
    for (name, svg) in &raw {
        let r = lib.find("tabler", name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(&*lib.svg(r).unwrap(), &svg[..], "{name}");
    }
    // Search still works on the index alone.
    let hits = lib.search("arrow", None, 100);
    assert!(!hits.is_empty() && hits.iter().all(|r| lib.name(*r).contains("arrow") || lib.keywords(*r).contains("arrow")));
}

#[test]
fn the_dictionary_is_dropped_with_the_svgs() {
    let lib = Library::open(&data("symbols-v2")).unwrap();
    assert_eq!(lib.dictionaries(), 0, "nothing is read until a symbol is drawn");
    let r = lib.all_in(0).next().unwrap();
    assert!(is_svg(&lib.svg(r).unwrap()));
    assert_eq!((lib.cached(), lib.dictionaries()), (1, 1));
    lib.clear();
    assert_eq!((lib.cached(), lib.dictionaries()), (0, 0));
    assert!(is_svg(&lib.svg(r).unwrap()), "and read again when needed");
}

#[test]
fn old_and_unknown_pack_formats_are_refused() {
    let err = Library::open(&data("symbols-v1")).err().expect("format 1 is refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("format 1") && err.to_string().contains("build_pack.py"), "{err}");

    let dir = std::env::temp_dir().join(format!("zebra-symbols-format-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sets.json"), r#"{"format": 3, "sets": []}"#).unwrap();
    let err = Library::open(&dir).err().expect("a newer format is refused");
    assert!(err.to_string().contains("format 3"), "{err}");
    std::fs::write(dir.join("sets.json"), r#"{"sets": []}"#).unwrap();
    assert!(Library::open(&dir).is_err(), "no format at all");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_damaged_frame_is_an_error_not_a_panic() {
    let dir = std::env::temp_dir().join(format!("zebra-symbols-damaged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = data("symbols-v2");
    for f in ["sets.json", "tabler.idx", "tabler.dict"] {
        std::fs::copy(src.join(f), dir.join(f)).unwrap();
    }
    let mut dat = std::fs::read(src.join("tabler.dat")).unwrap();
    for b in dat.iter_mut().skip(200).step_by(7) {
        *b ^= 0x5a;
    }
    std::fs::write(dir.join("tabler.dat"), dat).unwrap();
    let lib = Library::open(&dir).unwrap();
    let errors = lib.all_in(0).filter(|r| lib.svg(*r).is_err()).count();
    assert!(errors > 0);
    // Without its dictionary a set can't be read either.
    std::fs::remove_file(dir.join("tabler.dict")).unwrap();
    let lib = Library::open(&dir).unwrap();
    assert!(lib.svg(lib.all_in(0).next().unwrap()).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

/// The real pack, when it is unpacked in assets/symbols (it isn't in git):
/// the first, middle and last symbol of every set read back as SVG.
#[test]
fn the_installed_pack_reads_when_present() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/symbols");
    if !dir.join("sets.json").is_file() {
        eprintln!("assets/symbols not installed; skipped");
        return;
    }
    let lib = match Library::open(&dir) {
        Ok(l) => l,
        // An old pack left in a checkout: the app says so; unzip the new one.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            eprintln!("assets/symbols: {e}; skipped");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(lib.total() > 0);
    for (si, s) in lib.sets().enumerate() {
        let refs: Vec<_> = lib.all_in(si as u16).collect();
        assert!(!refs.is_empty(), "{}", s.key);
        for r in [refs[0], refs[refs.len() / 2], refs[refs.len() - 1]] {
            let svg = lib.svg(r).unwrap_or_else(|e| panic!("{} {}: {e}", s.key, lib.name(r)));
            assert!(is_svg(&svg), "{} {}", s.key, lib.name(r));
        }
    }
}
