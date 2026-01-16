use crate::ui::HdrTextLabel;
use super::{Page, PageOutput, add_quad};

pub struct BrightnessGrid;

impl Page for BrightnessGrid {
    fn name(&self) -> &'static str {
        "Brightness Grid"
    }

    fn render(&self, width: u32, height: u32, _max_brightness_nits: f32, _time: f32) -> PageOutput {
        let mut vertices = Vec::new();
        let mut labels = Vec::new();

        let scale = height.min(width) as f32 / 1080.0;
        let font_size = (scale * 18.0).max(12.0);

        let nit_values: [f32; 16] = [
            0.0, 1.0, 2.0, 2.5,
            5.0, 10.0, 20.0, 40.0,
            80.0, 160.0, 320.0, 640.0,
            1000.0, 2000.0, 4000.0, 10000.0,
        ];

        let cols = 4;
        let rows = 4;
        let padding = 0.05f32;
        let margin = 0.08f32;
        let label_height = 0.05f32;

        let available_width = 2.0 - 2.0 * margin;
        let available_height = 2.0 - 2.0 * margin;

        let cell_width = (available_width - (cols - 1) as f32 * padding) / cols as f32;
        let cell_height = (available_height - (rows - 1) as f32 * padding - rows as f32 * label_height) / rows as f32;

        for row in 0..rows {
            for col in 0..cols {
                let index = row * cols + col;
                let nits = nit_values[index];
                let scrgb_value = nits / 80.0;

                let x0 = -1.0 + margin + col as f32 * (cell_width + padding);
                let y0 = 1.0 - margin - row as f32 * (cell_height + padding + label_height);
                let x1 = x0 + cell_width;
                let y1 = y0 - cell_height;

                let color = [scrgb_value, scrgb_value, scrgb_value, 1.0];
                add_quad(&mut vertices, x0, y0, x1, y1, color);

                let nits_str = if nits == nits.floor() {
                    format!("Nits:{:.0}", nits)
                } else {
                    format!("Nits:{:.1}", nits)
                };

                labels.push(HdrTextLabel {
                    text: nits_str,
                    x: x0,
                    y: y1 - 0.01,
                    nits: 40.0,
                    size: font_size,
                });
            }
        }

        PageOutput { vertices, labels }
    }
}
