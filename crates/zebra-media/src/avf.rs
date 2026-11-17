//! AVFoundation backend (macOS). Placeholder until the bridge lands.
use std::path::Path;

use crate::{AudioPlayer, Backend, VideoPlayer};

pub struct AvFoundation;

impl Backend for AvFoundation {
    fn name(&self) -> &'static str {
        "avfoundation"
    }
    fn can_play(&self) -> bool {
        false
    }
    fn open_video(&self, _: &Path) -> Result<Box<dyn VideoPlayer>, String> {
        Err("not built yet".into())
    }
    fn open_audio(&self, _: &Path) -> Result<Box<dyn AudioPlayer>, String> {
        Err("not built yet".into())
    }
    fn poster_png(&self, _: &Path, _: f64, _: u32) -> Result<Vec<u8>, String> {
        Err("not built yet".into())
    }
}
