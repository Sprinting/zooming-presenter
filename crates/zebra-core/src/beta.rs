//! Beta features: parts of Zebra that are new and may be unstable, behind
//! one switch (Settings › General › Use beta features). It is off until
//! the person turns it on, and only the person can: agents read it
//! (`state.beta_features`, the `beta_features` event) but nothing lets
//! them change it.
//!
//! The host sets it with [`crate::Editor::set_beta_features`]. Whatever
//! does a beta feature asks [`crate::Editor::check_beta`] first, and
//! refuses with [`BetaFeature::error`] while the switch is off, so the app
//! and agents meet the same rule. The app offers to turn it on there.
use crate::error::ZError;

/// What the switch covers. More features may join.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BetaFeature {
    /// Importing a PowerPoint deck, all of it (File › Import…, and the
    /// `import` call of the API and zebractl).
    PptxImport,
}

/// The error code for something the switch covers while it is off.
pub const CODE: &str = "beta_off";

impl BetaFeature {
    /// Every feature the switch covers.
    pub const ALL: [BetaFeature; 1] = [BetaFeature::PptxImport];

    /// The refusal while the switch is off. Plain words, for agents as
    /// much as people: no names of the parts behind it.
    pub fn error(self) -> ZError {
        let what = match self {
            BetaFeature::PptxImport => "importing PowerPoint is",
        };
        ZError::new(CODE, format!("{what} a beta feature, and beta features are off"))
            .hint("only the person can turn on beta features, in Zebra's Settings, under General")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_name_no_parts() {
        for f in BetaFeature::ALL {
            let e = f.error();
            assert_eq!(e.code, CODE);
            let words = format!("{} {}", e.message, e.hint.unwrap_or_default()).to_lowercase();
            for bad in ["ironcalc", "engine", "drawingml", "betteroffice", "crate", "prezi", "flag"] {
                assert!(!words.contains(bad), "{bad} in {words}");
            }
            assert!(words.contains("settings"), "{words}");
        }
    }
}
