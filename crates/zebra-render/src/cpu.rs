//! vello_cpu backend. STUB.
use crate::{CaptureTarget, Renderer};
pub(crate) fn capture(_r: &Renderer, _e: &zebra_core::Editor, _t: &CaptureTarget, _w: u32, _h: u32) -> Result<Vec<u8>, zebra_core::ZError> {
    Err(zebra_core::ZError::new("unimplemented", "capture not implemented yet"))
}
