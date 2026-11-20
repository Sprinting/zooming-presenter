//! The beta-features switch in the editor: off until the person turns it
//! on, read by agents in `state`, heard as an event, and nothing an agent
//! sends changes it.
use serde_json::json;
use zebra_core::beta::BetaFeature;
use zebra_core::editor::Event;
use zebra_core::*;

#[test]
fn off_until_turned_on() {
    let ed = Editor::default();
    assert!(!ed.view.beta_features);
    assert_eq!(ed.query(Query::State).unwrap()["beta_features"], json!(false));
    let e = ed.check_beta(BetaFeature::PptxImport).unwrap_err();
    assert_eq!(e.code, beta::CODE);
}

#[test]
fn turning_it_on_or_off_is_heard_once() {
    let mut ed = Editor::default();
    ed.set_beta_features(false);
    assert!(ed.drain_events().is_empty());
    ed.set_beta_features(true);
    let ev = ed.drain_events();
    assert!(matches!(ev.as_slice(), [Event::BetaFeatures { on: true }]), "{ev:?}");
    assert_eq!(ed.query(Query::State).unwrap()["beta_features"], json!(true));
    assert!(ed.check_beta(BetaFeature::PptxImport).is_ok());
    ed.set_beta_features(true);
    assert!(ed.drain_events().is_empty());
    ed.set_beta_features(false);
    assert!(matches!(ed.drain_events().as_slice(), [Event::BetaFeatures { on: false }]));
}

#[test]
fn nothing_an_agent_sends_turns_it_on() {
    // Not a command, not a view command, not a field of the view state an
    // agent could send.
    let mut ed = Editor::default();
    for op in ["set_beta_features", "beta_features", "set_beta"] {
        let r: Result<Command, _> = serde_json::from_value(json!({"op": op, "on": true}));
        assert!(r.is_err(), "{op} is a command");
        let r: Result<ViewCommand, _> = serde_json::from_value(json!({"op": op, "on": true}));
        assert!(r.is_err(), "{op} is a view command");
    }
    let view = serde_json::to_value(&ed.view).unwrap();
    assert!(view.get("beta_features").is_none(), "the view state carries it");
    let mut v = view.clone();
    v["beta_features"] = json!(true);
    let back: view::ViewState = serde_json::from_value(v).unwrap();
    assert!(!back.beta_features);
    // A document never carries it either.
    ed.set_beta_features(true);
    let json = io::to_json(&ed.doc);
    assert!(!json.contains("beta"), "{json}");
}
