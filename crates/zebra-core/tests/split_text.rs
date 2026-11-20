//! Drag text out of a box (spec 18c): `split_text` takes selected words
//! into a box of their own, as one undo step.
use serde_json::json;
use zebra_core::model::{ListKind, ParaFormat, SpanFormat, TextSpan};
use zebra_core::*;

const H: Origin = Origin::Human;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn ed_with(text: &str) -> Editor {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_text","id":"t1","text":text,"x":0,"y":0})), H).unwrap();
    ed
}

fn text(ed: &Editor, id: &str) -> TextData {
    match &ed.doc.get(id).unwrap_or_else(|| panic!("{id} is gone")).kind {
        ElementKind::Text(t) => t.clone(),
        _ => panic!("{id} is not a text"),
    }
}

fn split(ed: &mut Editor, j: serde_json::Value) -> String {
    let mut j = j;
    j["op"] = json!("split_text");
    let out = ed.apply(cmd(j), H).unwrap();
    assert_eq!(out.created.len(), 1, "one new box");
    out.created[0].clone()
}

/// Character offset of `needle` in `ed`'s t1.
fn at(ed: &Editor, needle: &str) -> (usize, usize) {
    let t = text(ed, "t1").text;
    let b = t.find(needle).unwrap();
    let s = t[..b].chars().count();
    (s, s + needle.chars().count())
}

#[test]
fn word_formatting_moves_with_the_words() {
    let mut ed = ed_with("Zebras have bold stripes and links");
    let fmt = |f: serde_json::Value, find: &str| cmd(json!({"op":"format_text","id":"t1","find":find,"format":f}));
    ed.apply_batch(
        vec![
            fmt(json!({"bold":true}), "bold"),
            fmt(json!({"italic":true,"underline":true}), "stripes"),
            fmt(json!({"color":"#cc3333","size":1.5}), "and"),
            fmt(json!({"link":"https://example.com"}), "links"),
        ],
        None,
        H,
    )
    .unwrap();
    let (s, e) = at(&ed, "bold stripes and links");
    let new = split(&mut ed, json!({"id":"t1","start":s,"end":e,"x":0,"y":500}));
    let t = text(&ed, &new);
    assert_eq!(t.text, "bold stripes and links");
    let span = |start, end, format| TextSpan { start, end, format };
    assert_eq!(
        t.spans,
        vec![
            span(0, 4, SpanFormat { bold: Some(true), ..Default::default() }),
            span(5, 12, SpanFormat { italic: Some(true), underline: Some(true), ..Default::default() }),
            span(13, 16, SpanFormat { color: Some("#cc3333".into()), size: Some(1.5), ..Default::default() }),
            span(17, 22, SpanFormat { link: Some("https://example.com".into()), ..Default::default() }),
        ]
    );
    assert_eq!(text(&ed, "t1").text, "Zebras have", "no space left at the end");
    assert!(text(&ed, "t1").spans.is_empty(), "the formatting went with the words");
}

#[test]
fn the_new_box_keeps_style_scale_rotation_and_box_settings() {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_text","id":"t1","text":"Big title words","style":"title","scale":2.5,"x":0,"y":0,"color":"theme:accent"})), H).unwrap();
    ed.apply_batch(
        vec![
            cmd(json!({"op":"rotate","ids":["t1"],"degrees":20})),
            cmd(json!({"op":"update","id":"t1","patch":{"italic":true,"align":"center"}})),
        ],
        None,
        H,
    )
    .unwrap();
    let old = ed.doc.get("t1").unwrap().clone();
    let new = split(&mut ed, json!({"id":"t1","start":4,"end":9,"x":300,"y":400}));
    let e = ed.doc.get(&new).unwrap();
    let t = text(&ed, &new);
    assert_eq!(t.text, "title");
    assert_eq!((t.style, t.italic, t.align, t.color.as_deref()), (TextStyle::Title, true, Align::Center, Some("theme:accent")));
    assert_eq!(e.transform.scale, old.transform.scale);
    assert_eq!(e.transform.rotation, old.transform.rotation);
    assert_eq!((e.transform.x, e.transform.y), (300.0, 400.0), "x, y is the centre");
    assert!(!t.fixed_width, "wraps automatically");
    assert!(e.group.is_none() && !e.locked && !e.placeholder);
}

#[test]
fn whole_paragraphs_keep_lists_and_a_part_comes_out_plain() {
    let mut ed = ed_with("Intro line\nfirst point\nsecond point\nthird point");
    ed.apply_batch(
        vec![
            cmd(json!({"op":"format_paragraphs","id":"t1","first":1,"last":3,"list":"bullet"})),
            cmd(json!({"op":"format_paragraphs","id":"t1","first":2,"last":2,"indent":1})),
        ],
        None,
        H,
    )
    .unwrap();
    // Whole paragraphs 1 and 2, the selection running into the line break.
    let (s, _) = at(&ed, "first point");
    let (_, e) = at(&ed, "second point\n");
    let new = split(&mut ed, json!({"id":"t1","start":s,"end":e,"x":0,"y":600}));
    let t = text(&ed, &new);
    assert_eq!(t.text, "first point\nsecond point", "no empty paragraph at the end");
    assert_eq!(rich::para(&t, 0), (ListKind::Bullet, 0, Align::Left));
    assert_eq!(rich::para(&t, 1), (ListKind::Bullet, 1, Align::Left));
    let left = text(&ed, "t1");
    assert_eq!(left.text, "Intro line\nthird point", "the paragraphs' lines went too");
    assert_eq!(rich::para(&left, 1), (ListKind::Bullet, 0, Align::Left), "third point is still a bullet");

    // Part of a paragraph: plain, even in a box whose every paragraph has a bullet.
    let mut ed = ed_with("one two\nthree");
    ed.apply(cmd(json!({"op":"update","id":"t1","patch":{"bullets":true,"paras":[{"indent":2}]}})), H).unwrap();
    let new = split(&mut ed, json!({"id":"t1","start":4,"end":7,"x":0,"y":600}));
    let t = text(&ed, &new);
    assert_eq!(t.text, "two");
    assert_eq!(rich::para(&t, 0), (ListKind::None, 0, Align::Left), "a part of a paragraph is plain");
    assert_eq!(rich::para(&text(&ed, "t1"), 0), (ListKind::Bullet, 2, Align::Left), "the rest keeps its paragraph");

    // A selection from mid-paragraph into the next whole one: the part is
    // plain, the whole one keeps its marker.
    let mut ed = ed_with("lead words\nnext one");
    ed.apply(cmd(json!({"op":"format_paragraphs","id":"t1","first":0,"last":1,"list":"number"})), H).unwrap();
    let new = split(&mut ed, json!({"id":"t1","start":5,"end":19}));
    let t = text(&ed, &new);
    assert_eq!(t.text, "words\nnext one");
    assert_eq!(t.paras, vec![ParaFormat::default(), ParaFormat { list: Some(ListKind::Number), ..Default::default() }]);
}

#[test]
fn smart_spaces_are_tidied_where_the_words_left() {
    let cases = [
        ("The quick brown fox", "quick", "The brown fox"),
        ("The quick brown fox", "quick ", "The brown fox"),
        ("The quick brown fox", " quick", "The brown fox"),
        ("The quick brown fox", "The", "quick brown fox"),
        ("The quick brown fox", "fox", "The quick brown"),
        ("Stripes are mostly black.", "mostly black", "Stripes are."),
        ("Hello there\nbig world", "big", "Hello there\nworld"),
    ];
    for (body, cut, want) in cases {
        let mut ed = ed_with(body);
        let (s, e) = at(&ed, cut);
        let new = split(&mut ed, json!({"id":"t1","start":s,"end":e}));
        assert_eq!(text(&ed, "t1").text, want, "cutting {cut:?} out of {body:?}");
        assert_eq!(text(&ed, &new).text, cut.trim(), "the new box has no spaces at its ends");
    }
}

#[test]
fn a_box_emptied_by_the_split_is_deleted() {
    let mut ed = ed_with("All of it");
    ed.apply(cmd(json!({"op":"add_step","target":{"kind":"element","id":"t1"}})), H).unwrap();
    let new = split(&mut ed, json!({"id":"t1","start":0,"end":9,"x":10,"y":10}));
    assert!(ed.doc.get("t1").is_none(), "the old box went");
    assert!(ed.doc.path.is_empty(), "with its path step");
    assert_eq!(text(&ed, &new).text, "All of it");
    // Spaces alone don't keep a box.
    let mut ed = ed_with("  words  ");
    split(&mut ed, json!({"id":"t1","start":2,"end":7}));
    assert!(ed.doc.get("t1").is_none(), "{:?}", ed.doc.get("t1").map(|e| e.kind.clone()));
}

#[test]
fn a_fixed_width_is_kept_and_an_auto_width_stays_auto() {
    let mut ed = ed_with("Some words that wrap in a narrow box");
    ed.apply(cmd(json!({"op":"update","id":"t1","patch":{"w":120}})), H).unwrap();
    let new = split(&mut ed, json!({"id":"t1","start":11,"end":36}));
    let (e, t) = (ed.doc.get(&new).unwrap(), text(&ed, &new));
    assert!(t.fixed_width);
    assert_eq!(e.size.w, 120.0);
    let mut ed = ed_with("Some words that grow");
    let new = split(&mut ed, json!({"id":"t1","start":5,"end":20}));
    assert!(!text(&ed, &new).fixed_width);
}

#[test]
fn the_new_box_belongs_to_the_frame_it_lands_in() {
    let mut ed = Editor::default();
    ed.apply_batch(
        vec![
            cmd(json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600})),
            cmd(json!({"op":"add_frame","id":"f2","x":3000,"y":0,"w":800,"h":600})),
            cmd(json!({"op":"add_text","id":"t1","text":"Move these words","x":0,"y":0})),
        ],
        None,
        H,
    )
    .unwrap();
    assert_eq!(ed.doc.parent_frame("t1").as_deref(), Some("f1"));
    let into = split(&mut ed, json!({"id":"t1","start":5,"end":10,"x":3000,"y":0}));
    assert_eq!(ed.doc.parent_frame(&into).as_deref(), Some("f2"));
    let out = split(&mut ed, json!({"id":"t1","start":5,"end":10,"x":9000,"y":0}));
    assert_eq!(ed.doc.parent_frame(&out), None, "on empty canvas: in no frame");
    // Moving f2 now carries its new box.
    let before = ed.doc.get(&into).unwrap().transform.x;
    ed.apply(cmd(json!({"op":"move","ids":["f2"],"dx":100,"dy":0})), H).unwrap();
    assert_eq!(ed.doc.get(&into).unwrap().transform.x, before + 100.0);
}

#[test]
fn it_is_one_undo_step_named_move_text() {
    let mut ed = ed_with("Zebras have stripes");
    let before = ed.doc.clone();
    let steps = ed.history().len();
    let new = split(&mut ed, json!({"id":"t1","start":7,"end":19,"x":0,"y":300}));
    assert_eq!(ed.history().len(), steps + 1, "one step");
    assert_eq!(ed.history().last().unwrap().label, "Move text");
    assert_eq!(ed.undo(H).unwrap().as_deref(), Some("Move text"));
    assert_eq!(ed.doc, before, "undo gives back the box as it was, and no new box");
    ed.redo(H).unwrap();
    assert_eq!(text(&ed, &new).text, "have stripes");
    assert_eq!(text(&ed, "t1").text, "Zebras");
    // Emptying (and so deleting) the box is still one step.
    let mut ed = ed_with("Everything");
    let before = ed.doc.clone();
    split(&mut ed, json!({"id":"t1","start":0,"end":10}));
    ed.undo(H).unwrap();
    assert_eq!(ed.doc, before);
}

#[test]
fn copy_keeps_the_source_as_it_was() {
    let mut ed = ed_with("Keep all of this");
    let old = ed.doc.get("t1").unwrap().clone();
    let new = split(&mut ed, json!({"id":"t1","start":5,"end":8,"x":0,"y":400,"copy":true}));
    assert_eq!(ed.doc.get("t1").unwrap(), &old, "the source is untouched");
    assert_eq!(text(&ed, &new).text, "all");
    assert_eq!(ed.history().last().unwrap().label, "Copy text");
    // Copying out of a locked box is allowed; moving out of one is not.
    ed.apply(cmd(json!({"op":"update","id":"t1","patch":{"locked":true}})), H).unwrap();
    assert!(ed.apply(cmd(json!({"op":"split_text","id":"t1","start":0,"end":4,"copy":true})), H).is_ok());
    let e = ed.apply(cmd(json!({"op":"split_text","id":"t1","start":0,"end":4})), H).unwrap_err();
    assert_eq!(e.code, "locked");
}

#[test]
fn without_a_position_the_new_box_goes_just_below() {
    let mut ed = ed_with("First line words\nsecond");
    let new = split(&mut ed, json!({"id":"t1","start":11,"end":16}));
    let (o, n) = (ed.doc.get("t1").unwrap().clone(), ed.doc.get(&new).unwrap().clone());
    let (ob, nb) = (o.bounds(), n.bounds());
    assert!(nb.y0 > ob.y1, "below: {nb:?} under {ob:?}");
    assert!(nb.y0 - ob.y1 < ob.y1 - ob.y0, "just below, not far off");
    assert!((nb.x0 - ob.x0).abs() < 1e-6, "left edges line up");
}

#[test]
fn bad_requests_are_refused() {
    let mut ed = ed_with("Words");
    ed.apply(cmd(json!({"op":"add_shape","id":"s1","shape":"rectangle"})), H).unwrap();
    let before = ed.doc.clone();
    for (j, code) in [
        (json!({"op":"split_text","id":"t1","start":3,"end":3}), "invalid_params"),
        (json!({"op":"split_text","id":"t1","start":0,"end":9}), "invalid_params"),
        (json!({"op":"split_text","id":"t1","start":0,"end":2,"x":5}), "invalid_params"),
        (json!({"op":"split_text","id":"s1","start":0,"end":2}), "invalid_params"),
        (json!({"op":"split_text","id":"nope","start":0,"end":2}), "not_found"),
    ] {
        let e = ed.apply(cmd(j.clone()), H).unwrap_err();
        assert_eq!(e.code, code, "{j}");
    }
    let mut sp = ed_with("a   b");
    assert_eq!(sp.apply(cmd(json!({"op":"split_text","id":"t1","start":1,"end":4})), H).unwrap_err().code, "invalid_params");
    assert_eq!(ed.doc, before);
}

// ---- moving words inside a box (the reorder half of the drag)

fn moved(body: &str, sel: &str, to_before: &str, copy: bool) -> (String, String) {
    let mut t = TextData { text: body.into(), ..Default::default() };
    let b = body.find(sel).unwrap();
    let s = body[..b].chars().count();
    let to = body[..body.find(to_before).unwrap()].chars().count();
    let (a, e) = rich::move_within(&mut t, s, s + sel.chars().count(), to, copy).unwrap();
    let got: String = t.text.chars().skip(a).take(e - a).collect();
    (t.text, got)
}

#[test]
fn words_move_inside_a_box_with_their_spaces_tidied() {
    assert_eq!(moved("The quick brown fox", "quick", "fox", false), ("The brown quick fox".into(), "quick".into()));
    assert_eq!(moved("The quick brown fox", "brown ", "quick", false), ("The brown quick fox".into(), "brown".into()));
    assert_eq!(moved("The quick brown fox", "The", "fox", false).0, "quick brown The fox");
    assert_eq!(moved("The quick brown fox", "quick", "The", true).0, "quick The quick brown fox", "Option copies");
    let mut t = TextData { text: "abc".into(), ..Default::default() };
    assert_eq!(rich::move_within(&mut t, 0, 2, 1, false), None, "dropped on itself: nothing");
    assert_eq!(t.text, "abc");
}

#[test]
fn whole_paragraphs_move_as_paragraphs() {
    let mut t = TextData { text: "one\ntwo\nthree".into(), ..Default::default() };
    rich::format_paragraphs(&mut t, 2, 2, Some(ListKind::Bullet), Some(1), 0, None);
    // "three" to the start of "one".
    let r = rich::move_within(&mut t, 8, 13, 0, false).unwrap();
    assert_eq!(t.text, "three\none\ntwo");
    assert_eq!(r, (0, 5));
    assert_eq!(rich::para(&t, 0), (ListKind::Bullet, 1, Align::Left), "its bullet came along");
    assert_eq!(rich::para(&t, 1), (ListKind::None, 0, Align::Left));
    // "three" to the end of "two" (after the last word).
    let r = rich::move_within(&mut t, 0, 6, 13, false).unwrap();
    assert_eq!(t.text, "one\ntwo\nthree");
    assert_eq!(rich::para(&t, 2), (ListKind::Bullet, 1, Align::Left));
    assert_eq!(rich::para(&t, 0), (ListKind::None, 0, Align::Left));
    assert_eq!(r, (8, 13));
}

#[test]
fn moved_words_keep_their_formatting_and_dont_take_their_neighbours() {
    let mut t = TextData { text: "plain bold end".into(), ..Default::default() };
    rich::apply_format(&mut t, 6, 14, &SpanFormat { bold: Some(true), ..Default::default() });
    // "plain" into the middle of the bold run, before "end".
    let (a, b) = rich::move_within(&mut t, 0, 5, 11, false).unwrap();
    assert_eq!(t.text, "bold plain end");
    assert_eq!((a, b), (5, 10));
    assert_eq!(rich::common_format(&t, a, b), SpanFormat::default(), "the moved word stays plain inside the bold run");
    assert_eq!(rich::common_format(&t, 11, 14).bold, Some(true));
    assert_eq!(rich::common_format(&t, 0, 4).bold, Some(true));
}

/// W2 of the 20 Nov audit: a range that ends or starts inside a grapheme
/// takes the whole grapheme, so no box is left with a stray accent, half a
/// flag or half a ZWJ family.
#[test]
fn splits_take_whole_graphemes() {
    for (body, start, end, want_new, want_left) in [
        ("Cafe\u{301} au lait", 3, 4, "e\u{301}", "Caf au lait"),
        ("Cafe\u{301} au lait", 4, 5, "e\u{301}", "Caf au lait"),
        ("flag 🇬🇧 here", 5, 6, "🇬🇧", "flag here"),
        ("us 👨\u{200d}👩\u{200d}👧 too", 4, 6, "👨\u{200d}👩\u{200d}👧", "us too"),
    ] {
        let mut ed = ed_with(body);
        let new = split(&mut ed, json!({"id":"t1","start":start,"end":end}));
        assert_eq!(text(&ed, &new).text, want_new, "{body}");
        assert_eq!(text(&ed, "t1").text, want_left, "{body}");
    }
    assert_eq!(rich::whole_graphemes("abc", 2, 1), (2, 1), "backwards ranges are left alone");
    let mut t = TextData { text: "x e\u{301} y".into(), ..Default::default() };
    let (a, e) = rich::move_within(&mut t, 2, 3, 0, false).unwrap();
    assert_eq!(t.text.chars().skip(a).take(e - a).collect::<String>(), "e\u{301}", "a move takes the accent too");
}
