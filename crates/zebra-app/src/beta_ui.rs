//! The beta-features switch in the app (see `zebra_core::beta`): the
//! Settings row's setting, kept between launches, and the offer where a
//! person reaches a beta feature while it is off. On Turn On the offer
//! carries on with what the person was doing.
use std::path::PathBuf;

use zebra_core::beta::BetaFeature;

use crate::app::ZebraApp;
use crate::sheets::{self, SheetKind};

/// Where the setting is kept (the app's own settings, per Mac user).
pub const STORAGE_KEY: &str = "beta_features";

/// What to carry on with after Turn On.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    /// Import this file.
    Import(PathBuf),
}

/// The person's setting, as kept between launches (off when never set).
pub fn saved(storage: Option<&dyn eframe::Storage>) -> bool {
    storage.and_then(|s| eframe::get_value(s, STORAGE_KEY)).unwrap_or(false)
}

/// Turn beta features on or off (Settings, or the offer's Turn On). The
/// shell passes it on to every window.
pub fn set(app: &mut ZebraApp, on: bool) {
    app.editor.set_beta_features(on);
    if !on {
        app.beta_pending = None;
    }
}

/// Offer to turn beta features on, for `feature`, then carry on with `then`.
pub fn offer(app: &mut ZebraApp, feature: BetaFeature, then: Pending) {
    app.beta_pending = Some(then);
    sheets::request(app, SheetKind::BetaOffer(feature));
}

/// The offer's answer.
pub fn answered(app: &mut ZebraApp, turn_on: bool) {
    let then = app.beta_pending.take();
    if !turn_on {
        return;
    }
    set(app, true);
    match then {
        Some(Pending::Import(path)) => crate::import_ui::start(app, path),
        None => {}
    }
}

/// The offer's title and message.
pub fn words(feature: BetaFeature) -> (String, String) {
    let message = match feature {
        BetaFeature::PptxImport => crate::t!("beta-offer-pptx"),
    };
    (crate::t!("beta-offer-title"), message)
}
