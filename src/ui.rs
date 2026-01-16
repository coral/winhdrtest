use crate::app::AppState;
use crate::dx12::Vertex;
use egui::{Context, Event, FontId, PointerButton, RawInput, Pos2, Rect, TextureId, Vec2, ViewportId, ViewportInfo};
use std::time::Instant;
pub use egui::TexturesDelta;

/// Text label for HDR content
pub struct HdrTextLabel {
    pub text: String,
    pub x: f32,      // NDC x position (-1 to 1)
    pub y: f32,      // NDC y position (-1 to 1)
    pub nits: f32,   // Brightness in nits
    pub size: f32,   // Font size in pixels
}

pub struct UiState {
    pub ctx: Context,
    pub pixels_per_point: f32,
    // Input state
    pub pointer_pos: Option<Pos2>,
    pub events: Vec<Event>,
    // Time tracking
    start_time: Instant,
}

pub struct UiOutput {
    pub vertices: Vec<Vertex>,
    pub textures_delta: TexturesDelta,
}

impl UiState {
    pub fn new() -> Self {
        let ctx = Context::default();
        ctx.set_pixels_per_point(1.0);

        Self {
            ctx,
            pixels_per_point: 1.0,
            pointer_pos: None,
            events: Vec::new(),
            start_time: Instant::now(),
        }
    }

    /// Called when mouse moves
    pub fn on_mouse_move(&mut self, x: f32, y: f32) {
        self.pointer_pos = Some(Pos2::new(x, y));
        self.events.push(Event::PointerMoved(Pos2::new(x, y)));
    }

    /// Called when mouse button is pressed/released
    pub fn on_mouse_button(&mut self, button: PointerButton, pressed: bool) {
        if let Some(pos) = self.pointer_pos {
            self.events.push(Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            });
        }
    }

    /// Called when mouse wheel scrolls
    pub fn on_mouse_wheel(&mut self, delta_x: f32, delta_y: f32) {
        self.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(delta_x, delta_y),
            modifiers: Default::default(),
        });
    }

    /// Render text labels for HDR content
    /// Returns vertices that can be rendered directly to the HDR backbuffer
    /// Note: Must be called after run() has been called at least once to initialize fonts
    pub fn render_hdr_labels(&mut self, labels: &[HdrTextLabel], width: u32, height: u32) -> Vec<Vertex> {
        if labels.is_empty() {
            return Vec::new();
        }

        let mut clipped_shapes = Vec::new();
        let scrgb = labels[0].nits / 80.0;

        for label in labels {
            let screen_x = (label.x + 1.0) / 2.0 * width as f32;
            let screen_y = (1.0 - label.y) / 2.0 * height as f32;

            let font_id = FontId::proportional(label.size);
            let galley = self.ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(label.text.clone(), font_id, egui::Color32::WHITE)
            });

            let shape = egui::epaint::Shape::galley(Pos2::new(screen_x, screen_y), galley, egui::Color32::WHITE);
            clipped_shapes.push(egui::epaint::ClippedShape {
                clip_rect: Rect::EVERYTHING,
                shape,
            });
        }

        let meshes = self.ctx.tessellate(clipped_shapes, 1.0);

        // Convert meshes to HDR vertices
        let mut all_vertices = Vec::new();
        for primitive in meshes {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                // Only process font texture meshes
                if mesh.texture_id != TextureId::Managed(0) {
                    continue;
                }

                for idx in mesh.indices.chunks(3) {
                    if idx.len() != 3 {
                        continue;
                    }
                    for &i in idx {
                        let v = &mesh.vertices[i as usize];
                        // Convert screen coordinates to NDC
                        let x = (v.pos.x / width as f32) * 2.0 - 1.0;
                        let y = 1.0 - (v.pos.y / height as f32) * 2.0;

                        // Use HDR brightness (grayscale text)
                        let a = v.color.a() as f32 / 255.0;

                        all_vertices.push(Vertex {
                            position: [x, y],
                            uv: [v.uv.x, v.uv.y],
                            color: [scrgb, scrgb, scrgb, a],
                        });
                    }
                }
            }
        }

        all_vertices
    }

    pub fn run(&mut self, app: &mut AppState, width: u32, height: u32) -> UiOutput {
        let mut input = RawInput::default();

        let mut viewport_info = ViewportInfo::default();
        viewport_info.native_pixels_per_point = Some(1.0);
        input.viewports.insert(ViewportId::ROOT, viewport_info);
        input.screen_rect = Some(Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(width as f32, height as f32),
        ));

        // Set time for proper event handling
        input.time = Some(self.start_time.elapsed().as_secs_f64());
        input.focused = true;

        // Add accumulated events
        input.events = std::mem::take(&mut self.events);

        let output = self.ctx.run(input, |ctx| {
            render_ui(ctx, app);
        });

        // Convert egui shapes to our vertex format
        // Only process meshes that use the font texture (Managed(0))
        let primitives = self.ctx.tessellate(output.shapes, self.pixels_per_point);
        let vertices = shapes_to_vertices(&primitives, width, height, TextureId::Managed(0));

        UiOutput {
            vertices,
            textures_delta: output.textures_delta,
        }
    }
}

fn render_ui(ctx: &Context, app: &mut AppState) {
    egui::Window::new("HDR Test Controls")
        .default_pos([10.0, 10.0])
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Settings");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Max Brightness (nits):");
                ui.add(egui::Slider::new(&mut app.max_brightness_nits, 100.0..=10000.0).logarithmic(true));
            });

            ui.horizontal(|ui| {
                ui.label("Paper White (nits):");
                ui.add(egui::Slider::new(&mut app.paper_white_nits, 80.0..=500.0));
            });

            ui.separator();
            ui.heading("Pages");

            ui.horizontal(|ui| {
                if ui.button("< Prev").clicked() {
                    app.prev_page();
                }
                ui.label(format!("Page {}/{}", app.current_page + 1, app.page_count()));
                if ui.button("Next >").clicked() {
                    app.next_page();
                }
            });

            ui.label(format!("Current: {}", app.current_page_name()));

            ui.separator();

            ui.checkbox(&mut app.auto_cycle, "Auto-cycle pages");
            if app.auto_cycle {
                ui.horizontal(|ui| {
                    ui.label("Interval (seconds):");
                    ui.add(egui::Slider::new(&mut app.cycle_interval, 1.0..=30.0));
                });
            }

            ui.separator();
            ui.label("Controls:");
            ui.label("  PageUp/PageDown: Change page");
            ui.label("  Ctrl+U: Toggle UI");
        });
}

fn shapes_to_vertices(
    primitives: &[egui::ClippedPrimitive],
    width: u32,
    height: u32,
    expected_texture: TextureId,
) -> Vec<Vertex> {
    let mut vertices = Vec::new();

    for primitive in primitives {
        if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive {
            // Only process meshes that use our expected texture
            if mesh.texture_id != expected_texture {
                continue;
            }

            for idx in mesh.indices.chunks(3) {
                if idx.len() != 3 {
                    continue;
                }

                for &i in idx {
                    let v = &mesh.vertices[i as usize];

                    // Convert screen coordinates to NDC (-1 to 1)
                    let x = (v.pos.x / width as f32) * 2.0 - 1.0;
                    let y = 1.0 - (v.pos.y / height as f32) * 2.0;

                    // Convert egui color (sRGB u8) to linear float
                    let r = srgb_to_linear(v.color.r());
                    let g = srgb_to_linear(v.color.g());
                    let b = srgb_to_linear(v.color.b());
                    let a = v.color.a() as f32 / 255.0;

                    vertices.push(Vertex {
                        position: [x, y],
                        uv: [v.uv.x, v.uv.y],
                        color: [r, g, b, a],
                    });
                }
            }
        }
    }

    vertices
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
