//! Renderer-neutral display list. STUB: the render agent implements this.
use crate::{Renderer, SceneOptions};
use zebra_core::{Camera, Editor};

/// Commands in device pixels (already rebased to the camera, f32-safe).
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub width_px: u32,
    pub height_px: u32,
    pub background: [u8; 4],
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    // filled in by the render agent
}

pub(crate) fn build(_r: &Renderer, _e: &Editor, _c: &Camera, _vp: [f64; 2], _ppp: f64, _o: &SceneOptions) -> DisplayList {
    todo!("render agent")
}
