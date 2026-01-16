use super::{Page, PageOutput, add_gradient_quad_h};

pub struct ColorRamps;

impl Page for ColorRamps {
    fn name(&self) -> &'static str {
        "Color Ramps"
    }

    fn render(&self, _width: u32, _height: u32, max_brightness_nits: f32, _time: f32) -> PageOutput {
        let mut vertices = Vec::new();

        let colors: [[f32; 3]; 6] = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
        ];

        let bar_count = colors.len();
        let bar_height = 2.0 / bar_count as f32;
        let max_scrgb = max_brightness_nits / 80.0;
        let segments = 32;

        for (i, base_color) in colors.iter().enumerate() {
            let y0 = 1.0 - i as f32 * bar_height;
            let y1 = y0 - bar_height;

            for seg in 0..segments {
                let t0 = seg as f32 / segments as f32;
                let t1 = (seg + 1) as f32 / segments as f32;

                let x0 = -1.0 + t0 * 2.0;
                let x1 = -1.0 + t1 * 2.0;

                let g0 = t0.powf(2.2);
                let g1 = t1.powf(2.2);

                let color0 = compute_ramp_color(base_color, g0, max_scrgb);
                let color1 = compute_ramp_color(base_color, g1, max_scrgb);

                add_gradient_quad_h(&mut vertices, x0, y0, x1, y1, color0, color1);
            }
        }

        PageOutput { vertices, labels: Vec::new() }
    }
}

fn compute_ramp_color(base: &[f32; 3], t: f32, max_scrgb: f32) -> [f32; 4] {
    if t < 0.5 {
        let intensity = t * 2.0;
        [
            base[0] * intensity * max_scrgb,
            base[1] * intensity * max_scrgb,
            base[2] * intensity * max_scrgb,
            1.0,
        ]
    } else {
        let blend = (t - 0.5) * 2.0;
        [
            (base[0] + (1.0 - base[0]) * blend) * max_scrgb,
            (base[1] + (1.0 - base[1]) * blend) * max_scrgb,
            (base[2] + (1.0 - base[2]) * blend) * max_scrgb,
            1.0,
        ]
    }
}
