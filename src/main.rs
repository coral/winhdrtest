mod app;
mod dx12;
mod pages;
mod ui;

use anyhow::Result;
use app::AppState;
use dx12::Dx12State;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use ui::UiState;
use windows::Win32::Foundation::HWND;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

struct App {
    window: Option<Window>,
    dx12: Option<Dx12State>,
    app_state: AppState,
    ui_state: UiState,
    modifiers: ModifiersState,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            dx12: None,
            app_state: AppState::new(),
            ui_state: UiState::new(),
            modifiers: ModifiersState::empty(),
        }
    }

    fn render(&mut self) -> Result<()> {
        let dx12 = self.dx12.as_mut().unwrap();
        let width = dx12.width;
        let height = dx12.height;

        // Update app state (auto-cycle, etc.)
        self.app_state.update();

        // Begin frame
        dx12.begin_frame()?;

        // Clear to true black background
        dx12.clear_render_target([0.0, 0.0, 0.0, 1.0]);

        // Always run egui to ensure fonts are initialized (even if UI is hidden)
        let ui_output = self.ui_state.run(&mut self.app_state, width, height);

        // Update font texture if needed (required for HDR text labels too)
        dx12.update_font_texture(&ui_output.textures_delta)?;

        // Render current HDR test page
        let page_output = self.app_state.render_current_page(width, height);
        dx12.render_quads(&page_output.vertices);

        // Render HDR text labels if any
        if !page_output.labels.is_empty() {
            let label_vertices = self.ui_state.render_hdr_labels(&page_output.labels, width, height);
            dx12.render_hdr_text(&label_vertices);
        }

        // Render UI if visible
        if self.app_state.show_ui {
            // Clear SDR render target
            dx12.clear_sdr_target();

            dx12.render_ui_quads(&ui_output.vertices);

            // Composite UI onto HDR backbuffer
            dx12.composite_ui(self.app_state.paper_white_nits);
        }

        // End frame and present
        dx12.end_frame()?;

        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title("HDR Test Application")
            .with_inner_size(PhysicalSize::new(1920, 1080));

        match event_loop.create_window(window_attrs) {
            Ok(window) => {
                let size = window.inner_size();

                // Get HWND from window handle
                let hwnd = match window.window_handle() {
                    Ok(handle) => match handle.as_raw() {
                        RawWindowHandle::Win32(h) => HWND(h.hwnd.get() as *mut _),
                        _ => {
                            eprintln!("Unsupported window handle type");
                            event_loop.exit();
                            return;
                        }
                    },
                    Err(e) => {
                        eprintln!("Failed to get window handle: {}", e);
                        event_loop.exit();
                        return;
                    }
                };

                // Initialize DX12
                match Dx12State::new(hwnd, size.width, size.height) {
                    Ok(dx12) => {
                        self.dx12 = Some(dx12);
                        self.window = Some(window);
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize DX12: {}", e);
                        event_loop.exit();
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(dx12) = &mut self.dx12 {
                    if let Err(e) = dx12.resize(size.width, size.height) {
                        eprintln!("Failed to resize: {} (continuing with old size)", e);
                    }
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.ui_state.on_mouse_move(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let egui_button = match button {
                    MouseButton::Left => egui::PointerButton::Primary,
                    MouseButton::Right => egui::PointerButton::Secondary,
                    MouseButton::Middle => egui::PointerButton::Middle,
                    _ => return,
                };
                self.ui_state.on_mouse_button(egui_button, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 20.0, y * 20.0),
                    MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };
                self.ui_state.on_mouse_wheel(dx, dy);
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key,
                    state: ElementState::Pressed,
                    ..
                },
                ..
            } => {
                match &logical_key {
                    Key::Named(NamedKey::PageUp) => {
                        self.app_state.prev_page();
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.app_state.next_page();
                    }
                    Key::Character(c) if c.eq_ignore_ascii_case("u") && self.modifiers.control_key() => {
                        self.app_state.toggle_ui();
                    }
                    Key::Named(NamedKey::Escape) => {
                        event_loop.exit();
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render() {
                    eprintln!("Render error: {}", e);
                    // Don't continue rendering if we have an error
                    event_loop.exit();
                    return;
                }
                // Request another frame
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app)?;

    Ok(())
}
