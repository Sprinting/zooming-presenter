//! Spec 18a/18b: a weight per theme style, set in the Theme Editor's Weight
//! pop-up, and Bold (⌘B, the B button, Format › Bold) against a heavy
//! style, as in Keynote and Pages.
use egui::{Key, Modifiers};
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, ElementKind, Origin, TextData, TextStyle, Theme};

const H: Origin = Origin::Human;

fn harness() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

/// The presentation's theme with these title and subtitle weights.
fn set_weights(h: &mut Harness<'static, ZebraApp>, title: u16, subtitle: u16) {
    let mut t = Theme::resolve("classic");
    t.id = "heavy".into();
    t.title.weight = title;
    t.subtitle.weight = subtitle;
    h.state_mut().editor.apply(Command::SaveTheme { theme: t, apply: true, to_library: false }, H).unwrap();
    h.run_steps(2);
}

/// A title text at the middle of the view, selected.
fn title(h: &mut Harness<'static, ZebraApp>, words: &str) -> String {
    let c = {
        let a = h.state();
        a.canvas.s2w(&a.editor, a.canvas.rect.center())
    };
    let app = h.state_mut();
    let scale = 1.0 / app.editor.view.camera.zoom;
    let out = app
        .editor
        .apply(Command::AddText { text: words.into(), x: c.x, y: c.y, style: TextStyle::Title, scale: Some(scale), place: None, id: None, on: None, color: None }, H)
        .unwrap();
    let id = out.created[0].to_string();
    app.select(vec![id.clone().into()]);
    h.run_steps(3);
    id
}

fn text(h: &Harness<'static, ZebraApp>, id: &str) -> TextData {
    match &h.state().editor.doc.get(id).unwrap().kind {
        ElementKind::Text(t) => t.clone(),
        _ => panic!("not a text"),
    }
}

fn bold_ticked(h: &Harness<'static, ZebraApp>) -> bool {
    zebra_app::menus::state(h.state(), &Action::Bold).checked == Some(true)
}

#[test]
fn the_weight_popup_sets_the_style_weight_in_one_undo_step() {
    let mut h = harness();
    title(&mut h, "Zebra");
    h.state_mut().queued.push(Action::ThemeEditor);
    // The window finds its size and place.
    h.run_steps(10);
    let undo = h.state().editor.history().len();
    // Title row: its Weight pop-up shows Regular, the theme's weight.
    let pops: Vec<_> = h.query_all_by_label("Weight").collect();
    assert_eq!(pops.len(), 3, "a Weight pop-up on each style row");
    assert_eq!(h.query_all(by().label("Weight").value("Regular")).count(), 3);
    h.query_all_by_label("Weight").next().unwrap().click();
    h.run_steps(2);
    for name in ["Ultralight", "Thin", "Light", "Medium", "Semibold", "Bold", "Heavy", "Black"] {
        assert!(h.query_by_label(name).is_some(), "Inter offers {name}");
    }
    shot(&mut h, "theme-editor-weight-popup.png");
    h.get_by_label("Semibold").click();
    h.run_steps(60);
    let th = h.state().editor.theme();
    assert_eq!((th.title.weight, th.subtitle.weight, th.body.weight), (600, 400, 400));
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
    assert_eq!(h.query_all(by().label("Weight").value("Semibold")).count(), 1, "the title's pop-up shows the new weight");
    // The inspector names the face.
    assert!(h.query_all(by().value("Inter Semibold (from the theme)")).count() > 0);
}

#[test]
fn a_font_without_the_weight_takes_its_nearest_face() {
    let mut h = harness();
    set_weights(&mut h, 600, 400);
    h.state_mut().queued.push(Action::ThemeEditor);
    h.run_steps(10);
    // Title row: Font pop-up, then Hand (Caveat: Regular and Bold only).
    // This machine's font list may hold names (Japanese...) the test's UI
    // fonts can't draw: boxes, as in the app, not a panic.
    h.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    h.query_all(by().value("Sans (Inter)")).next().unwrap().click();
    h.run_steps(2);
    h.get_by_label("Hand").click();
    h.run_steps(60);
    let th = h.state().editor.theme();
    assert_eq!((th.title.font.as_str(), th.title.weight), ("Hand", 700), "Semibold becomes Bold, the nearest");
    // Caveat's pop-up lists its two faces only.
    h.query_all_by_label("Weight").next().unwrap().click();
    h.run_steps(2);
    assert!(h.query_by_label("Regular").is_some());
    assert!(h.query_by_label("Semibold").is_none());
}

#[test]
fn bold_in_a_heavy_style_turns_the_words_regular() {
    let mut h = harness();
    set_weights(&mut h, 800, 400);
    let id = title(&mut h, "Big title");
    assert!(bold_ticked(&h), "a Heavy title is already bold");
    let undo = h.state().editor.history().len();
    h.state_mut().queued.push(Action::Bold);
    h.run_steps(2);
    let t = text(&h, &id);
    assert!(!zebra_core::rich::all_bold(&t, 800, 0, 0), "Regular now: {t:?}");
    assert_eq!(zebra_core::rich::weight_at(&t, 800, 0), 400);
    assert!(!bold_ticked(&h));
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
    assert_eq!(h.state().editor.history().last().unwrap().label, Action::Bold.label());
    h.state_mut().queued.push(Action::Bold);
    h.run_steps(2);
    let t = text(&h, &id);
    assert_eq!(zebra_core::rich::weight_at(&t, 800, 0), 800, "back to the style's Heavy, not Bold");
    assert!(bold_ticked(&h));
    // A Regular style: Bold is Bold, as before.
    set_weights(&mut h, 400, 400);
    let id2 = title(&mut h, "Plain");
    h.state_mut().queued.push(Action::Bold);
    h.run_steps(2);
    let t2 = text(&h, &id2);
    assert!(t2.bold && t2.spans.is_empty(), "the box setting, as before: {t2:?}");
}

#[test]
fn command_b_on_words_in_a_heavy_style() {
    let mut h = harness();
    set_weights(&mut h, 600, 400);
    let id = title(&mut h, "Big title");
    zebra_app::canvas::begin_edit_existing(h.state_mut(), id.clone().into());
    h.run_steps(3);
    h.key_press(Key::End);
    h.run_steps(1);
    for _ in 0..5 {
        h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(h.state().canvas.text.as_ref().unwrap().sel, (4, 9));
    assert!(bold_ticked(&h), "Format › Bold is ticked for Semibold words");
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run_steps(2);
    assert!(!bold_ticked(&h));
    h.key_press(Key::Escape);
    h.run_steps(3);
    let t = text(&h, &id);
    assert_eq!(zebra_core::rich::weight_at(&t, 600, 0), 600, "\"Big\" stays Semibold");
    assert_eq!(zebra_core::rich::weight_at(&t, 600, 5), 400, "\"title\" is Regular");
    assert_eq!(t.spans.len(), 1);
    assert_eq!(t.spans[0].format.bold, Some(false));
}

/// Screenshot for review: a deck with a Semibold title and a Heavy
/// subtitle (Inter), and one with a Bold Caveat title.
#[test]
fn weights_on_the_canvas_screenshot() {
    if std::env::var_os("ZEBRA_SHOTS").is_none() {
        return;
    }
    let mut h = harness();
    set_weights(&mut h, 600, 800);
    let id = title(&mut h, "Semibold title");
    let (c, w) = {
        let e = h.state().editor.doc.get(&id).unwrap();
        (e.transform.center(), e.world_size())
    };
    let k = 1.0 / h.state().editor.view.camera.zoom;
    h.state_mut()
        .editor
        .apply(Command::AddText { text: "Heavy subtitle".into(), x: c.x, y: c.y + w.1 * 1.2, style: TextStyle::Subtitle, scale: Some(k), place: None, id: None, on: None, color: None }, H)
        .unwrap();
    h.state_mut()
        .editor
        .apply(Command::AddText { text: "Regular body text".into(), x: c.x, y: c.y + w.1 * 2.2, style: TextStyle::Body, scale: Some(k), place: None, id: None, on: None, color: None }, H)
        .unwrap();
    h.run_steps(5);
    shot(&mut h, "deck-semibold-heavy.png");
    // Words made Regular in the Semibold title, while editing.
    zebra_app::canvas::begin_edit_existing(h.state_mut(), id.into());
    h.run_steps(3);
    h.key_press(Key::Home);
    for _ in 0..8 {
        h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
        h.run_steps(1);
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run_steps(3);
    shot(&mut h, "deck-semibold-editing-cmd-b.png");
}

/// Screenshot for review: the Fonts rows with the longest bundled names.
#[test]
fn theme_editor_long_font_names_screenshot() {
    if std::env::var_os("ZEBRA_SHOTS").is_none() {
        return;
    }
    let mut h = harness();
    let mut t = Theme::resolve("classic");
    t.id = "long".into();
    t.title.font = "Mono".into();
    t.title.weight = 800;
    t.subtitle.font = "Hand".into();
    t.subtitle.weight = 700;
    t.body.font = "A Very Long Installed Font Name Condensed".into();
    t.body.weight = 500;
    h.state_mut().editor.apply(Command::SaveTheme { theme: t, apply: true, to_library: false }, H).unwrap();
    h.state_mut().queued.push(Action::ThemeEditor);
    h.run_steps(30);
    shot(&mut h, "theme-editor-long-names.png");
}
