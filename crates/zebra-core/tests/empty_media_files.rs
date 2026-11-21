//! Mac pass 21 Nov, M9: an empty .png given to `add_image` was refused
//! as "unrecognised media format" with a list of formats to use, though
//! it is named PNG; a person picking it with Insert › Image… read
//! "Couldn't do that.". An empty file says it is empty; a file named as
//! a kind Zebra takes that isn't one says it is damaged or isn't that
//! kind (as an import does: "the file is damaged or isn't a PDF file").
use serde_json::json;
use zebra_core::*;

fn add(op: &str, name: &str, bytes: &[u8]) -> ZError {
    let dir = std::env::temp_dir().join(format!("zebra-empty-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    let mut v = json!({"op": op, "path": p.to_string_lossy()});
    if op != "set_soundtrack" {
        v["x"] = json!(0);
        v["y"] = json!(0);
    }
    let cmd: Command = serde_json::from_value(v).unwrap();
    let mut ed = Editor::default();
    let e = ed.apply(cmd, Origin::Human).expect_err(name);
    assert!(ed.doc.assets.is_empty() && ed.doc.elements.is_empty(), "{name}: nothing added");
    let _ = std::fs::remove_file(&p);
    e
}

#[test]
fn an_empty_media_file_says_it_is_empty() {
    for (op, name) in [("add_image", "empty.png"), ("add_image", "empty.heic"), ("add_video", "empty.mp4"), ("set_soundtrack", "empty.m4a"), ("add_image", "empty")] {
        let e = add(op, name, b"");
        assert_eq!((e.code, e.message.as_str()), ("damaged", "the file is empty"), "{name}");
        assert!(e.hint.is_none(), "{name}: no list of formats to use: {e:?}");
    }
}

#[test]
fn a_file_named_as_media_that_isnt_says_so() {
    let text = b"hello, this is a text file";
    for (op, name, ext) in [("add_image", "text-named-picture.png", ".png"), ("add_image", "photo.JPG", ".jpg"), ("add_video", "clip.mov", ".mov"), ("set_soundtrack", "song.mp3", ".mp3")] {
        let e = add(op, name, text);
        assert_eq!(e.code, "damaged", "{name}: {e:?}");
        assert_eq!(e.message, format!("the file is damaged or isn't a {ext} file"), "{name}");
    }
    // A disk image named as a picture: the same, from its first bytes.
    let e = add("add_image", "disk-image-named.png", b"koly-this-is-a-disk-image");
    assert_eq!(e.code, "damaged");
    // A file of a kind Zebra doesn't take is still "unrecognised", with
    // the formats to use.
    let e = add("add_image", "notes.txt", text);
    assert_eq!((e.code, e.message.as_str()), ("invalid_params", "unrecognised media format"));
    assert!(e.hint.is_some());
    // So is a PowerPoint file (a zip): it isn't a damaged picture.
    let e = add("add_image", "deck.pptx", b"PK\x03\x04 a zip");
    assert_eq!((e.code, e.message.as_str()), ("invalid_params", "unrecognised media format"));
}
