//! M23 (Mac pass, 21 Nov): an animated GIF in a window hidden behind a
//! full-screen app kept repainting about 40 times a second, at 130% CPU.
//! Nobody could see it. Now the canvas asks for the GIF's next frame only
//! while its window can be seen; showing the window again goes on.
//!
//! Needs a GPU (software Vulkan will do): only the GPU canvas animates.
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};

fn gif() -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let mut gif = vec![];
    {
        let mut enc = GifEncoder::new(&mut gif);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for c in [[230u8, 0, 0, 255], [0, 200, 0, 255]] {
            let img = image::RgbaImage::from_pixel(64, 64, image::Rgba(c));
            enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(50, 1))).unwrap();
        }
    }
    gif
}

/// Whether the canvas asked for a repaint in the pass before the last one
/// (egui reports a pass's causes once the next pass starts) (for a GIF's
/// next frame: nothing else on this canvas animates here).
fn canvas_wants_frames(h: &Harness<'_, ZebraApp>) -> bool {
    h.ctx.repaint_causes().iter().any(|c| c.file.ends_with("canvas.rs"))
}

#[test]
fn a_hidden_window_does_not_animate_its_gifs() {
    let mut h = Harness::builder()
        .wgpu()
        .with_size(egui::vec2(800.0, 600.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, cc.wgpu_render_state.clone(), None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    assert!(h.state().gpu_canvas.is_some(), "no GPU canvas");
    let at = h.state().canvas_center_world();
    h.state_mut().insert_image_bytes(gif(), at, "Animated".into());
    // Let the frames decode, then check it animates while seen.
    let t0 = std::time::Instant::now();
    loop {
        h.step();
        if canvas_wants_frames(&h) {
            break;
        }
        assert!(t0.elapsed().as_secs() < 20, "the GIF never asked for its next frame while seen");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Covered (another app, or another Space behind a full-screen app).
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().occluded = Some(true);
    h.step();
    for _ in 0..5 {
        h.step();
        assert!(!canvas_wants_frames(&h), "a covered window still repaints for its GIF: {:?}", h.ctx.repaint_causes());
    }
    // Minimised.
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().occluded = Some(false);
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().minimized = Some(true);
    h.step();
    for _ in 0..5 {
        h.step();
        assert!(!canvas_wants_frames(&h), "a minimised window still repaints for its GIF");
    }
    // Seen again: it goes on.
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().minimized = Some(false);
    h.step();
    h.step();
    h.step();
    assert!(canvas_wants_frames(&h), "the GIF didn't go on when the window was seen again");
}
