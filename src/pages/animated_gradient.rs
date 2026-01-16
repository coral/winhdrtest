use crate::ui::HdrTextLabel;
use super::{Page, PageOutput, add_gradient_quad_h};

pub struct AnimatedGradient;

impl Page for AnimatedGradient {
    fn name(&self) -> &'static str {
        "Animated Color Gradient"
    }

    fn render(&self, width: u32, height: u32, max_brightness_nits: f32, time: f32) -> PageOutput {
        let mut vertices = Vec::new();

        let scale = height.min(width) as f32 / 1080.0;
        let font_size = (scale * 24.0).max(14.0);
        let base = 0.25;
        let r = base * (time * 2.0).sin() + base;
        let g = base * (time * 1.0).sin() + base;
        let b = base * (time * 0.5).sin() + base;

        let max_scrgb = max_brightness_nits / 80.0;
        let target_color = [r * max_scrgb, g * max_scrgb, b * max_scrgb, 1.0];

        let segments = 64;

        for seg in 0..segments {
            let t0 = seg as f32 / segments as f32;
            let t1 = (seg + 1) as f32 / segments as f32;

            let x0 = -1.0 + t0 * 2.0;
            let x1 = -1.0 + t1 * 2.0;

            // Apply sRGB gamma to get perceptually uniform gradient
            let g0 = t0.powf(2.2);
            let g1 = t1.powf(2.2);

            let color0 = [
                target_color[0] * g0,
                target_color[1] * g0,
                target_color[2] * g0,
                1.0,
            ];
            let color1 = [
                target_color[0] * g1,
                target_color[1] * g1,
                target_color[2] * g1,
                1.0,
            ];

            add_gradient_quad_h(&mut vertices, x0, 1.0, x1, -1.0, color0, color1);
        }

        let labels = vec![
            HdrTextLabel {
                text: format!("R:{:.2} G:{:.2} B:{:.2}", r, g, b),
                x: -0.95,
                y: 0.92,
                nits: 80.0,
                size: font_size,
            },
        ];

        PageOutput { vertices, labels }
    }
}
