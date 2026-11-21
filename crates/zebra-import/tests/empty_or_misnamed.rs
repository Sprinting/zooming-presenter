//! Mac pass 21 Nov, M9: an empty .pptx was refused as "Zebra can't
//! import this kind of file", which is wrong: it is a .pptx, and it is
//! empty. A file named as a kind Zebra imports says what is wrong with
//! it; only a file of another kind is "not supported".
use zebra_import::ImportError;

fn open(name: &str, bytes: &[u8]) -> Option<ImportError> {
    let dir = std::env::temp_dir().join(format!("zebra-import-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    let r = zebra_import::open(&p).err();
    let _ = std::fs::remove_file(&p);
    r
}

#[test]
fn an_empty_file_is_empty() {
    for name in ["empty.pptx", "empty.PPTX", "empty.pdf", "empty.ppsx", "empty.txt", "empty"] {
        assert_eq!(open(name, b""), Some(ImportError::Empty), "{name}");
    }
    assert_eq!(ImportError::Empty.to_string(), "the file is empty");
}

#[test]
fn a_file_named_as_a_kind_zebra_imports_but_isnt_is_damaged() {
    let text = b"this is a text file, not a presentation";
    for name in ["text.pptx", "text.pptm", "text.potx", "text.ppsx", "Deck.PPTX"] {
        assert_eq!(open(name, text), Some(ImportError::Damaged("PowerPoint")), "{name}");
    }
    assert_eq!(open("text.pdf", text), Some(ImportError::Damaged("PDF")));
    // Another kind of file is still one Zebra doesn't import.
    for name in ["text.txt", "text.key", "text"] {
        assert_eq!(open(name, text), Some(ImportError::NotSupported), "{name}");
    }
    // An old PowerPoint file named .pptx is still told to be saved as .pptx.
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.resize(512, 0);
    ole.extend("PowerPoint Document".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(open("old.pptx", &ole), Some(ImportError::OldPowerPoint));
}
