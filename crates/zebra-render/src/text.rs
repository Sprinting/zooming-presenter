//! Bundled fonts + parley layout. STUB.
pub(crate) struct Fonts;
impl Fonts {
    pub fn new() -> Self { Fonts }
    pub fn measurer(&self) -> Box<dyn zebra_core::TextMeasure> { Box::new(zebra_core::editor::EstimateMeasure) }
}
