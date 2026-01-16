mod animated_gradient;
mod brightness_grid;
mod color_ramps;
mod pq_levels;
mod split_compare;

use crate::dx12::Vertex;
use crate::ui::HdrTextLabel;

pub struct PageOutput {
    pub vertices: Vec<Vertex>,
    pub labels: Vec<HdrTextLabel>,
}

pub trait Page {
    fn name(&self) -> &'static str;
    fn render(&self, width: u32, height: u32, max_brightness_nits: f32, time: f32) -> PageOutput;
}

pub fn nits_to_scrgb(nits: f32) -> f32 {
    nits / 80.0
}

pub fn add_quad(vertices: &mut Vec<Vertex>, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let uv = [1.0, 1.0];
    vertices.push(Vertex {
        position: [x0, y0],
        uv,
        color,
    });
    vertices.push(Vertex {
        position: [x0, y1],
        uv,
        color,
    });
    vertices.push(Vertex {
        position: [x1, y1],
        uv,
        color,
    });
    vertices.push(Vertex {
        position: [x0, y0],
        uv,
        color,
    });
    vertices.push(Vertex {
        position: [x1, y1],
        uv,
        color,
    });
    vertices.push(Vertex {
        position: [x1, y0],
        uv,
        color,
    });
}

pub fn add_gradient_quad_h(
    vertices: &mut Vec<Vertex>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    left_color: [f32; 4],
    right_color: [f32; 4],
) {
    let uv = [1.0, 1.0];
    vertices.push(Vertex {
        position: [x0, y0],
        uv,
        color: left_color,
    });
    vertices.push(Vertex {
        position: [x0, y1],
        uv,
        color: left_color,
    });
    vertices.push(Vertex {
        position: [x1, y1],
        uv,
        color: right_color,
    });
    vertices.push(Vertex {
        position: [x0, y0],
        uv,
        color: left_color,
    });
    vertices.push(Vertex {
        position: [x1, y1],
        uv,
        color: right_color,
    });
    vertices.push(Vertex {
        position: [x1, y0],
        uv,
        color: right_color,
    });
}

pub fn get_pages() -> Vec<Box<dyn Page>> {
    vec![
        Box::new(pq_levels::PqLevels),
        //Box::new(brightness_grid::BrightnessGrid),
        Box::new(color_ramps::ColorRamps),
        Box::new(animated_gradient::AnimatedGradient),
        Box::new(split_compare::SplitCompare),
    ]
}
