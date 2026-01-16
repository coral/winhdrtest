use crate::ui::HdrTextLabel;
use super::{Page, PageOutput, add_quad};

pub struct SplitCompare;

impl Page for SplitCompare {
    fn name(&self) -> &'static str {
        "Split Compare (SDR | HDR)"
    }

    fn render(&self, width: u32, height: u32, max_brightness_nits: f32, _time: f32) -> PageOutput {
        let mut vertices = Vec::new();

        let scale = height.min(width) as f32 / 1080.0;
        let font_size = (scale * 24.0).max(14.0);
        let max_scrgb = max_brightness_nits / 80.0;
        let bands = 8;
        let band_height = 2.0 / bands as f32;

        for i in 0..bands {
            let y0 = 1.0 - i as f32 * band_height;
            let y1 = y0 - band_height;
            let brightness = (i + 1) as f32 / bands as f32;

            let sdr_value = (brightness * max_scrgb).min(1.0);
            add_quad(&mut vertices, -1.0, y0, 0.0, y1, [sdr_value, sdr_value, sdr_value, 1.0]);

            let hdr_value = brightness * max_scrgb;
            add_quad(&mut vertices, 0.0, y0, 1.0, y1, [hdr_value, hdr_value, hdr_value, 1.0]);
        }

        let line_width = 0.005;
        add_quad(&mut vertices, -line_width, 1.0, line_width, -1.0, [0.5, 0.5, 0.5, 1.0]);

        let labels = vec![
            HdrTextLabel {
                text: "SDR (clamped)".to_string(),
                x: -0.9,
                y: 0.95,
                nits: 40.0,
                size: font_size,
            },
            HdrTextLabel {
                text: "HDR (full range)".to_string(),
                x: 0.1,
                y: 0.95,
                nits: 40.0,
                size: font_size,
            },
        ];

        PageOutput { vertices, labels }
    }
}
