//! GPU canvas: vello renders the display list into a wgpu texture on eframe's
//! device (Metal on macOS) and it is shown inside egui. STUB.
use crate::DisplayList;

pub struct VelloCanvas {}

impl VelloCanvas {
    /// Create once, from `frame.wgpu_render_state()`.
    pub fn new(_rs: &egui_wgpu::RenderState) -> Self {
        VelloCanvas {}
    }
    /// Paint `list` (built at `rect.size() * pixels_per_point`) into `rect`.
    /// Call every frame the canvas is visible.
    pub fn paint(&mut self, _ui: &mut egui::Ui, _rect: egui::Rect, _list: &DisplayList, _rs: &egui_wgpu::RenderState) {}
}
