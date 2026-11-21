//! B12: undo keeps as many steps as memory allows (Keynote has no step
//! count), dropping the oldest past `undo_budget`. Own binary: it sets the
//! process's memory budget.
use zebra_core::{Command, Editor, Origin};

#[test]
fn undo_drops_the_oldest_steps_only_past_its_memory_budget() {
    let mut ed = Editor::default();
    let add = |ed: &mut Editor, i: usize| {
        let c: Command = serde_json::from_value(serde_json::json!({"op":"add_text","text":"x".repeat(10_000),"x":i as f64,"y":0})).unwrap();
        ed.apply(c, Origin::Human).unwrap();
    };
    for i in 0..1000 {
        add(&mut ed, i);
    }
    assert_eq!(ed.history().len(), 1000, "a thousand steps fit");
    let one = ed.history().last().unwrap().approx_bytes() as u64;
    // A budget of about 100 such steps: the oldest go, the newest stay.
    zebra_core::io::set_memory_budget(Some(one * 100 * 4));
    add(&mut ed, 1000);
    let kept = ed.history().len();
    assert!((95..=101).contains(&kept), "{kept}");
    assert!(ed.history_bytes() as u64 <= one * 101);
    let mut n = 0;
    while ed.undo(Origin::Human).unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, kept);
    assert_eq!(ed.doc.elements.len(), 1001 - kept, "the oldest steps can't be undone; the rest could");
    // The newest step always stays, however big.
    zebra_core::io::set_memory_budget(Some(1));
    add(&mut ed, 2000);
    assert_eq!(ed.history().len(), 1);
    zebra_core::io::set_memory_budget(None);
}
