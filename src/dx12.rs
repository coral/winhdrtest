use anyhow::{anyhow, Result};
use egui::TexturesDelta;
use std::ffi::CString;
use std::mem::ManuallyDrop;
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::*;

const FRAME_COUNT: u32 = 2;

pub struct Dx12State {
    pub device: ID3D12Device,
    pub command_queue: ID3D12CommandQueue,
    pub swapchain: IDXGISwapChain4,
    pub rtv_heap: ID3D12DescriptorHeap,
    pub rtv_descriptor_size: u32,
    pub render_targets: Vec<ID3D12Resource>,
    pub command_allocators: Vec<ID3D12CommandAllocator>,
    pub command_list: ID3D12GraphicsCommandList,
    pub fence: ID3D12Fence,
    pub fence_values: Vec<u64>,
    pub fence_event: HANDLE,
    pub frame_index: u32,
    pub width: u32,
    pub height: u32,
    // Pending resize to apply at frame start
    pending_resize: Option<(u32, u32)>,
    // Pipeline state for rendering
    pub root_signature: ID3D12RootSignature,
    pub quad_pso: ID3D12PipelineState,
    pub sdr_quad_pso: ID3D12PipelineState,
    pub hdr_text_pso: ID3D12PipelineState,  // Textured PSO for HDR text
    pub composite_pso: ID3D12PipelineState,
    // SDR render target for egui
    pub sdr_texture: ID3D12Resource,
    pub sdr_rtv_heap: ID3D12DescriptorHeap,
    pub sdr_srv_heap: ID3D12DescriptorHeap,
    // Upload heap for vertex data
    pub upload_buffer: ID3D12Resource,
    pub upload_buffer_ptr: *mut u8,
    // Font texture for egui
    pub font_texture: Option<ID3D12Resource>,
    pub font_srv_heap: Option<ID3D12DescriptorHeap>,
    // Keep upload buffer alive until GPU finishes copy
    font_upload_buffer: Option<ID3D12Resource>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CompositeConstants {
    pub paper_white_scale: f32,
    pub _padding: [f32; 3],
}

impl Dx12State {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        unsafe {
            // Debug layer disabled - it causes TDRs on some systems
            // To enable: uncomment and ensure Windows Graphics Tools are installed
            // #[cfg(debug_assertions)]
            // {
            //     let mut debug: Option<ID3D12Debug> = None;
            //     if D3D12GetDebugInterface(&mut debug).is_ok() {
            //         if let Some(debug) = debug {
            //             debug.EnableDebugLayer();
            //         }
            //     }
            // }

            // Create DXGI factory (no debug flags)
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;

            // Create device
            let adapter = get_hardware_adapter(&factory)?;
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device)?;
            let device = device.ok_or_else(|| anyhow!("Failed to create device"))?;

            // Create command queue
            let command_queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                ..Default::default()
            })?;

            // Create HDR swapchain
            let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT, // HDR FP16
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: FRAME_COUNT,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                ..Default::default()
            };

            let swapchain: IDXGISwapChain1 = factory.CreateSwapChainForHwnd(
                &command_queue,
                hwnd,
                &swapchain_desc,
                None,
                None,
            )?;

            // Disable Alt+Enter fullscreen
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;

            let swapchain: IDXGISwapChain4 = swapchain.cast()?;

            // Set HDR color space (scRGB linear)
            swapchain.SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709)?;

            // Create RTV descriptor heap
            let rtv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                NumDescriptors: FRAME_COUNT,
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                ..Default::default()
            })?;
            let rtv_descriptor_size = device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

            // Create render targets
            let mut render_targets = Vec::with_capacity(FRAME_COUNT as usize);
            let rtv_handle = rtv_heap.GetCPUDescriptorHandleForHeapStart();
            for i in 0..FRAME_COUNT {
                let resource: ID3D12Resource = swapchain.GetBuffer(i)?;
                let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_handle.ptr + (i * rtv_descriptor_size) as usize,
                };
                device.CreateRenderTargetView(&resource, None, handle);
                render_targets.push(resource);
            }

            // Create command allocators
            let mut command_allocators = Vec::with_capacity(FRAME_COUNT as usize);
            for _ in 0..FRAME_COUNT {
                let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
                command_allocators.push(allocator);
            }

            // Create command list
            let command_list: ID3D12GraphicsCommandList = device.CreateCommandList(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                &command_allocators[0],
                None,
            )?;
            command_list.Close()?;

            // Create fence
            let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
            let fence_values = vec![0u64; FRAME_COUNT as usize];
            let fence_event = CreateEventA(None, false, false, None)?;

            // Create root signature and PSOs
            let root_signature = create_root_signature(&device)?;
            let quad_pso = create_quad_pso(&device, &root_signature, DXGI_FORMAT_R16G16B16A16_FLOAT, false)?;
            let sdr_quad_pso = create_quad_pso(&device, &root_signature, DXGI_FORMAT_R8G8B8A8_UNORM, true)?;
            let hdr_text_pso = create_quad_pso(&device, &root_signature, DXGI_FORMAT_R16G16B16A16_FLOAT, true)?;
            let composite_pso = create_composite_pso(&device, &root_signature)?;

            // Create SDR render target for egui
            let (sdr_texture, sdr_rtv_heap, sdr_srv_heap) = create_sdr_render_target(&device, width, height)?;

            // Create upload buffer for vertex data (1MB should be enough)
            let upload_buffer_size = 1024 * 1024;
            let upload_buffer: ID3D12Resource = {
                let mut resource: Option<ID3D12Resource> = None;
                device.CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES {
                        Type: D3D12_HEAP_TYPE_UPLOAD,
                        ..Default::default()
                    },
                    D3D12_HEAP_FLAG_NONE,
                    &D3D12_RESOURCE_DESC {
                        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                        Width: upload_buffer_size,
                        Height: 1,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                        ..Default::default()
                    },
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    None,
                    &mut resource,
                )?;
                resource.ok_or_else(|| anyhow!("Failed to create upload buffer"))?
            };

            let mut upload_buffer_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            upload_buffer.Map(0, None, Some(&mut upload_buffer_ptr))?;

            let frame_index = swapchain.GetCurrentBackBufferIndex();

            Ok(Self {
                device,
                command_queue,
                swapchain,
                rtv_heap,
                rtv_descriptor_size,
                render_targets,
                command_allocators,
                command_list,
                fence,
                fence_values,
                fence_event,
                frame_index,
                width,
                height,
                root_signature,
                quad_pso,
                sdr_quad_pso,
                hdr_text_pso,
                composite_pso,
                sdr_texture,
                sdr_rtv_heap,
                sdr_srv_heap,
                upload_buffer,
                upload_buffer_ptr: upload_buffer_ptr as *mut u8,
                pending_resize: None,
                font_texture: None,
                font_srv_heap: None,
                font_upload_buffer: None,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        // Skip if size hasn't changed
        if width == self.width && height == self.height {
            return Ok(());
        }
        // Defer resize to frame boundary to avoid issues with in-flight commands
        self.pending_resize = Some((width, height));
        Ok(())
    }

    fn apply_pending_resize(&mut self) -> Result<()> {
        let (width, height) = match self.pending_resize.take() {
            Some(size) => size,
            None => return Ok(()),
        };

        if width == self.width && height == self.height {
            return Ok(());
        }

        unsafe {
            // Wait for ALL in-flight work
            self.wait_for_gpu()?;

            // Reset command allocators and command list to release internal buffer references
            for allocator in &self.command_allocators {
                allocator.Reset()?;
            }
            self.command_list.Reset(&self.command_allocators[0], None)?;
            self.command_list.Close()?;

            // Release swapchain buffer references
            self.render_targets.clear();

            // Resize swapchain
            self.swapchain.ResizeBuffers(
                FRAME_COUNT,
                width,
                height,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;

            // Recreate render targets
            let rtv_handle = self.rtv_heap.GetCPUDescriptorHandleForHeapStart();
            for i in 0..FRAME_COUNT {
                let resource: ID3D12Resource = self.swapchain.GetBuffer(i)?;
                let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_handle.ptr + (i * self.rtv_descriptor_size) as usize,
                };
                self.device.CreateRenderTargetView(&resource, None, handle);
                self.render_targets.push(resource);
            }

            // Recreate SDR render target
            let (sdr_texture, sdr_rtv_heap, sdr_srv_heap) =
                create_sdr_render_target(&self.device, width, height)?;
            self.sdr_texture = sdr_texture;
            self.sdr_rtv_heap = sdr_rtv_heap;
            self.sdr_srv_heap = sdr_srv_heap;

            // Update dimensions
            self.width = width;
            self.height = height;

            // Reset fence values to start fresh after resize
            for i in 0..FRAME_COUNT as usize {
                self.fence_values[i] = 0;
            }

            self.frame_index = self.swapchain.GetCurrentBackBufferIndex();
        }
        Ok(())
    }

    pub fn update_font_texture(&mut self, textures_delta: &TexturesDelta) -> Result<()> {
        for (id, delta) in &textures_delta.set {
            // We only handle the font texture (Managed(0))
            if *id != egui::TextureId::Managed(0) {
                continue;
            }

            // Get image dimensions and pixel data
            let egui::ImageData::Color(color) = &delta.image;
            let width = color.width() as u32;
            let height = color.height() as u32;
            let pixels: Vec<u8> = color.pixels.iter()
                .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
                .collect();

            // Check if this is a partial update or full texture
            let (dest_x, dest_y, is_partial) = match delta.pos {
                Some([x, y]) => (x as u32, y as u32, true),
                None => (0, 0, false),
            };

            unsafe {
                // For partial updates, we need the existing texture
                // For full updates, we create a new texture
                let texture = if is_partial {
                    // Use existing texture - must exist for partial update
                    match &self.font_texture {
                        Some(tex) => tex.clone(),
                        None => continue, // Skip if no texture exists yet
                    }
                } else {
                    // Create new texture for full update
                    let mut texture: Option<ID3D12Resource> = None;
                    self.device.CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_DEFAULT,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_NONE,
                        &D3D12_RESOURCE_DESC {
                            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                            Width: width as u64,
                            Height: height,
                            DepthOrArraySize: 1,
                            MipLevels: 1,
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                            ..Default::default()
                        },
                        D3D12_RESOURCE_STATE_COPY_DEST,
                        None,
                        &mut texture,
                    )?;
                    texture.ok_or_else(|| anyhow!("Failed to create font texture"))?
                };

                // Create upload buffer
                let row_pitch = (width * 4 + 255) & !255; // Align to 256 bytes
                let upload_size = row_pitch * height;
                let mut upload_buffer: Option<ID3D12Resource> = None;
                self.device.CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES {
                        Type: D3D12_HEAP_TYPE_UPLOAD,
                        ..Default::default()
                    },
                    D3D12_HEAP_FLAG_NONE,
                    &D3D12_RESOURCE_DESC {
                        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                        Width: upload_size as u64,
                        Height: 1,
                        DepthOrArraySize: 1,
                        MipLevels: 1,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                        ..Default::default()
                    },
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    None,
                    &mut upload_buffer,
                )?;
                let upload_buffer = upload_buffer.ok_or_else(|| anyhow!("Failed to create upload buffer"))?;

                // Map and copy data with proper row pitch
                let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
                upload_buffer.Map(0, None, Some(&mut mapped))?;
                let mapped = mapped as *mut u8;
                for y in 0..height {
                    let src_offset = (y * width * 4) as usize;
                    let dst_offset = (y * row_pitch) as usize;
                    std::ptr::copy_nonoverlapping(
                        pixels.as_ptr().add(src_offset),
                        mapped.add(dst_offset),
                        (width * 4) as usize,
                    );
                }
                upload_buffer.Unmap(0, None);

                // For partial updates, transition existing texture to COPY_DEST
                if is_partial {
                    resource_barrier(
                        &self.command_list,
                        &texture,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                    );
                }

                // Copy to texture at the specified position
                let dst = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: 0,
                    },
                };
                let src = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(upload_buffer.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                            Offset: 0,
                            Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                                Width: width,
                                Height: height,
                                Depth: 1,
                                RowPitch: row_pitch,
                            },
                        },
                    },
                };
                self.command_list.CopyTextureRegion(&dst, dest_x, dest_y, 0, &src, None);

                // Transition to shader resource
                resource_barrier(
                    &self.command_list,
                    &texture,
                    D3D12_RESOURCE_STATE_COPY_DEST,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                );

                // Only create new SRV heap for full texture updates
                if !is_partial {
                    let srv_heap: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        NumDescriptors: 1,
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        ..Default::default()
                    })?;

                    self.device.CreateShaderResourceView(
                        &texture,
                        Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                                Texture2D: D3D12_TEX2D_SRV {
                                    MipLevels: 1,
                                    ..Default::default()
                                },
                            },
                        }),
                        srv_heap.GetCPUDescriptorHandleForHeapStart(),
                    );

                    self.font_texture = Some(texture);
                    self.font_srv_heap = Some(srv_heap);
                }

                // Keep upload buffer alive until GPU finishes copy
                self.font_upload_buffer = Some(upload_buffer);
            }
        }
        Ok(())
    }

    /// Calculate viewport for 16:9 aspect ratio with letterboxing/pillarboxing
    pub fn get_16_9_viewport(&self) -> (D3D12_VIEWPORT, RECT) {
        const TARGET_ASPECT: f32 = 16.0 / 9.0;
        let window_aspect = self.width as f32 / self.height as f32;

        let (vp_width, vp_height, vp_x, vp_y) = if window_aspect > TARGET_ASPECT {
            // Window is wider than 16:9 - pillarbox (black bars on sides)
            let vp_height = self.height as f32;
            let vp_width = vp_height * TARGET_ASPECT;
            let vp_x = (self.width as f32 - vp_width) / 2.0;
            (vp_width, vp_height, vp_x, 0.0)
        } else {
            // Window is taller than 16:9 - letterbox (black bars on top/bottom)
            let vp_width = self.width as f32;
            let vp_height = vp_width / TARGET_ASPECT;
            let vp_y = (self.height as f32 - vp_height) / 2.0;
            (vp_width, vp_height, 0.0, vp_y)
        };

        let viewport = D3D12_VIEWPORT {
            TopLeftX: vp_x,
            TopLeftY: vp_y,
            Width: vp_width,
            Height: vp_height,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };

        let scissor = RECT {
            left: vp_x as i32,
            top: vp_y as i32,
            right: (vp_x + vp_width) as i32,
            bottom: (vp_y + vp_height) as i32,
        };

        (viewport, scissor)
    }

    pub fn begin_frame(&mut self) -> Result<()> {
        // Apply any pending resize before starting the frame
        if self.pending_resize.is_some() {
            self.apply_pending_resize()?;
        }

        unsafe {
            let allocator = &self.command_allocators[self.frame_index as usize];

            // Wait if this frame's commands haven't finished
            let fence_value = self.fence_values[self.frame_index as usize];
            if self.fence.GetCompletedValue() < fence_value {
                self.fence.SetEventOnCompletion(fence_value, self.fence_event)?;
                WaitForSingleObject(self.fence_event, INFINITE);
            }

            allocator.Reset()?;
            self.command_list.Reset(allocator, None)?;
        }
        Ok(())
    }

    pub fn clear_render_target(&self, clear_color: [f32; 4]) {
        unsafe {
            let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + (self.frame_index * self.rtv_descriptor_size) as usize,
            };

            // Transition to render target
            resource_barrier(
                &self.command_list,
                &self.render_targets[self.frame_index as usize],
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );

            self.command_list.ClearRenderTargetView(rtv_handle, &clear_color, None);
        }
    }

    pub fn render_quads(&self, vertices: &[Vertex]) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            let vertex_size = std::mem::size_of::<Vertex>();
            let buffer_size = vertices.len() * vertex_size;

            // Use frame-indexed offset to avoid race conditions
            // Frame 0: 0-256KB, Frame 1: 512KB-768KB
            let frame_offset = self.frame_index as usize * 512 * 1024;

            // Copy vertices to upload buffer
            std::ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                self.upload_buffer_ptr.add(frame_offset),
                buffer_size,
            );

            let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + (self.frame_index * self.rtv_descriptor_size) as usize,
            };

            self.command_list.SetPipelineState(&self.quad_pso);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);

            // Use 16:9 viewport with letterboxing/pillarboxing
            let (viewport, scissor) = self.get_16_9_viewport();
            self.command_list.RSSetViewports(&[viewport]);
            self.command_list.RSSetScissorRects(&[scissor]);

            self.command_list.OMSetRenderTargets(1, Some(&rtv_handle), false, None);

            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.command_list.IASetVertexBuffers(0, Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.upload_buffer.GetGPUVirtualAddress() + frame_offset as u64,
                SizeInBytes: buffer_size as u32,
                StrideInBytes: vertex_size as u32,
            }]));

            self.command_list.DrawInstanced(vertices.len() as u32, 1, 0, 0);
        }
    }

    /// Render textured HDR text directly to the HDR backbuffer
    pub fn render_hdr_text(&self, vertices: &[Vertex]) {
        if vertices.is_empty() {
            return;
        }

        // Need font texture to render text
        let font_srv_heap = match &self.font_srv_heap {
            Some(heap) => heap,
            None => return,
        };

        unsafe {
            let vertex_size = std::mem::size_of::<Vertex>();
            let buffer_size = vertices.len() * vertex_size;

            // Use a different offset region for HDR text (after UI vertices)
            // Frame 0: 384KB-512KB, Frame 1: 896KB-1MB
            let frame_offset = self.frame_index as usize * 512 * 1024 + 384 * 1024;
            std::ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                self.upload_buffer_ptr.add(frame_offset),
                buffer_size,
            );

            let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + (self.frame_index * self.rtv_descriptor_size) as usize,
            };

            // Use textured PSO for HDR
            self.command_list.SetPipelineState(&self.hdr_text_pso);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);

            // Bind font texture
            self.command_list.SetDescriptorHeaps(&[Some(font_srv_heap.clone())]);
            self.command_list.SetGraphicsRootDescriptorTable(
                1,
                font_srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );

            // Use 16:9 viewport with letterboxing/pillarboxing
            let (viewport, scissor) = self.get_16_9_viewport();
            self.command_list.RSSetViewports(&[viewport]);
            self.command_list.RSSetScissorRects(&[scissor]);

            self.command_list.OMSetRenderTargets(1, Some(&rtv_handle), false, None);

            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.command_list.IASetVertexBuffers(0, Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.upload_buffer.GetGPUVirtualAddress() + frame_offset as u64,
                SizeInBytes: buffer_size as u32,
                StrideInBytes: vertex_size as u32,
            }]));

            self.command_list.DrawInstanced(vertices.len() as u32, 1, 0, 0);
        }
    }

    pub fn clear_sdr_target(&self) {
        unsafe {
            let sdr_rtv = self.sdr_rtv_heap.GetCPUDescriptorHandleForHeapStart();

            // Transition SDR texture to render target
            resource_barrier(
                &self.command_list,
                &self.sdr_texture,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );

            // Clear with transparent black
            self.command_list.ClearRenderTargetView(sdr_rtv, &[0.0, 0.0, 0.0, 0.0], None);
        }
    }

    pub fn render_ui_quads(&self, vertices: &[Vertex]) {
        if vertices.is_empty() {
            return;
        }

        // Need font texture to render UI
        let font_srv_heap = match &self.font_srv_heap {
            Some(heap) => heap,
            None => return,
        };

        unsafe {
            let vertex_size = std::mem::size_of::<Vertex>();
            let buffer_size = vertices.len() * vertex_size;

            // Use frame-indexed offset to avoid race conditions
            // Frame 0: 256KB-512KB, Frame 1: 768KB-1MB
            let frame_offset = self.frame_index as usize * 512 * 1024 + 256 * 1024;
            std::ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                self.upload_buffer_ptr.add(frame_offset),
                buffer_size,
            );

            let sdr_rtv = self.sdr_rtv_heap.GetCPUDescriptorHandleForHeapStart();

            self.command_list.SetPipelineState(&self.sdr_quad_pso);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);

            // Bind font texture
            self.command_list.SetDescriptorHeaps(&[Some(font_srv_heap.clone())]);
            self.command_list.SetGraphicsRootDescriptorTable(
                1,
                font_srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );

            self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
                Width: self.width as f32,
                Height: self.height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]);

            self.command_list.RSSetScissorRects(&[RECT {
                right: self.width as i32,
                bottom: self.height as i32,
                ..Default::default()
            }]);

            self.command_list.OMSetRenderTargets(1, Some(&sdr_rtv), false, None);

            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.command_list.IASetVertexBuffers(0, Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.upload_buffer.GetGPUVirtualAddress() + frame_offset as u64,
                SizeInBytes: buffer_size as u32,
                StrideInBytes: vertex_size as u32,
            }]));

            self.command_list.DrawInstanced(vertices.len() as u32, 1, 0, 0);
        }
    }

    pub fn composite_ui(&self, paper_white_nits: f32) {
        unsafe {
            // Transition SDR texture to shader resource
            resource_barrier(
                &self.command_list,
                &self.sdr_texture,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );

            let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: self.rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                    + (self.frame_index * self.rtv_descriptor_size) as usize,
            };

            self.command_list.SetPipelineState(&self.composite_pso);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);

            // Set descriptor heap
            self.command_list.SetDescriptorHeaps(&[Some(self.sdr_srv_heap.clone())]);

            // Set root parameters
            let constants = CompositeConstants {
                paper_white_scale: paper_white_nits / 80.0,
                _padding: [0.0; 3],
            };
            self.command_list.SetGraphicsRoot32BitConstants(
                0,
                4,
                &constants as *const _ as *const std::ffi::c_void,
                0,
            );
            self.command_list.SetGraphicsRootDescriptorTable(
                1,
                self.sdr_srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );

            self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
                Width: self.width as f32,
                Height: self.height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]);

            self.command_list.RSSetScissorRects(&[RECT {
                right: self.width as i32,
                bottom: self.height as i32,
                ..Default::default()
            }]);

            self.command_list.OMSetRenderTargets(1, Some(&rtv_handle), false, None);
            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

            // Draw fullscreen quad (6 vertices generated in shader)
            self.command_list.DrawInstanced(6, 1, 0, 0);
        }
    }

    pub fn end_frame(&mut self) -> Result<()> {
        unsafe {
            // Transition to present
            resource_barrier(
                &self.command_list,
                &self.render_targets[self.frame_index as usize],
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );

            self.command_list.Close()?;

            let command_lists = [Some(self.command_list.cast::<ID3D12CommandList>()?)];
            self.command_queue.ExecuteCommandLists(&command_lists);

            // Present with vsync
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;

            // Signal fence
            let fence_value = self.fence_values[self.frame_index as usize] + 1;
            self.command_queue.Signal(&self.fence, fence_value)?;
            self.fence_values[self.frame_index as usize] = fence_value;

            self.frame_index = self.swapchain.GetCurrentBackBufferIndex();
        }
        Ok(())
    }

    fn wait_for_gpu(&mut self) -> Result<()> {
        unsafe {
            for i in 0..FRAME_COUNT as usize {
                let fence_value = self.fence_values[i] + 1;
                self.command_queue.Signal(&self.fence, fence_value)?;
                self.fence_values[i] = fence_value;

                if self.fence.GetCompletedValue() < fence_value {
                    self.fence.SetEventOnCompletion(fence_value, self.fence_event)?;
                    WaitForSingleObject(self.fence_event, INFINITE);
                }
            }
        }
        Ok(())
    }
}

impl Drop for Dx12State {
    fn drop(&mut self) {
        unsafe {
            let _ = self.wait_for_gpu();
            if !self.fence_event.is_invalid() {
                let _ = CloseHandle(self.fence_event);
            }
        }
    }
}

unsafe fn get_hardware_adapter(factory: &IDXGIFactory4) -> Result<IDXGIAdapter1> {
    unsafe {
        for i in 0.. {
            let adapter = match factory.EnumAdapters1(i) {
                Ok(a) => a,
                Err(_) => break,
            };

            let desc = adapter.GetDesc1()?;

            // Skip software adapter
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }

            // Check if adapter supports D3D12
            if D3D12CreateDevice(
                &adapter,
                D3D_FEATURE_LEVEL_11_0,
                std::ptr::null_mut::<Option<ID3D12Device>>(),
            ).is_ok() {
                return Ok(adapter);
            }
        }
        Err(anyhow!("No suitable GPU adapter found"))
    }
}

/// Creates a resource barrier without leaking COM references.
/// Uses a raw pointer approach to avoid incrementing refcount.
unsafe fn resource_barrier(
    command_list: &ID3D12GraphicsCommandList,
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    unsafe {
        // Get the raw interface pointer without incrementing refcount
        use windows::core::Interface;
        let raw_ptr = resource.as_raw();

        // Create a non-owning "view" of the resource by transmuting the raw pointer
        // This is safe because we only use it for the duration of this function call
        // and ResourceBarrier just reads the pointer
        let resource_view: Option<ID3D12Resource> = std::mem::transmute(raw_ptr);

        let barriers = [D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: ManuallyDrop::new(resource_view),
                    StateBefore: before,
                    StateAfter: after,
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                }),
            },
        }];

        command_list.ResourceBarrier(&barriers);

        // Since we used transmute to create resource_view without incrementing refcount,
        // we must NOT let it drop (which would decrement the refcount incorrectly).
        // ManuallyDrop already prevents this, so we don't need to do anything else.
        // Just let barriers go out of scope - ManuallyDrop prevents the destructor.
    }
}

fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature> {
    unsafe {
        // Root signature with:
        // 0: 32-bit constants (4 floats)
        // 1: Descriptor table for SRV (texture)
        let parameters = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Constants: D3D12_ROOT_CONSTANTS {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                        Num32BitValues: 4,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &D3D12_DESCRIPTOR_RANGE {
                            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                            NumDescriptors: 1,
                            BaseShaderRegister: 0,
                            RegisterSpace: 0,
                            OffsetInDescriptorsFromTableStart: 0,
                        },
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ];

        let sampler = D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            ShaderRegister: 0,
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            ..Default::default()
        };

        let desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: parameters.len() as u32,
            pParameters: parameters.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };

        let mut signature = None;
        let mut error = None;
        D3D12SerializeRootSignature(
            &desc,
            D3D_ROOT_SIGNATURE_VERSION_1,
            &mut signature,
            Some(&mut error),
        )?;

        let signature = signature.ok_or_else(|| anyhow!("Failed to serialize root signature"))?;
        let root_signature = device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(signature.GetBufferPointer() as *const u8, signature.GetBufferSize()),
        )?;

        Ok(root_signature)
    }
}

fn create_quad_pso(device: &ID3D12Device, root_signature: &ID3D12RootSignature, format: DXGI_FORMAT, textured: bool) -> Result<ID3D12PipelineState> {
    let vs_source = r#"
        struct VSInput {
            float2 position : POSITION;
            float2 uv : TEXCOORD;
            float4 color : COLOR;
        };
        struct VSOutput {
            float4 position : SV_Position;
            float2 uv : TEXCOORD;
            float4 color : COLOR;
        };
        VSOutput main(VSInput input) {
            VSOutput output;
            output.position = float4(input.position, 0.0, 1.0);
            output.uv = input.uv;
            output.color = input.color;
            return output;
        }
    "#;

    // Non-textured shader (for HDR pages)
    let ps_source_solid = r#"
        struct PSInput {
            float4 position : SV_Position;
            float2 uv : TEXCOORD;
            float4 color : COLOR;
        };
        float4 main(PSInput input) : SV_Target {
            return input.color;
        }
    "#;

    // Textured shader (for UI with font texture)
    let ps_source_textured = r#"
        Texture2D fontTexture : register(t0);
        SamplerState fontSampler : register(s0);
        struct PSInput {
            float4 position : SV_Position;
            float2 uv : TEXCOORD;
            float4 color : COLOR;
        };
        float4 main(PSInput input) : SV_Target {
            float4 texColor = fontTexture.Sample(fontSampler, input.uv);
            return input.color * texColor;
        }
    "#;

    let ps_source = if textured { ps_source_textured } else { ps_source_solid };

    let vs_blob = compile_shader(vs_source, "main", "vs_5_0")?;
    let ps_blob = compile_shader(ps_source, "main", "ps_5_0")?;

    let input_elements = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(b"POSITION\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 8,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(b"COLOR\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 16,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    unsafe {
        let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
            VS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: vs_blob.GetBufferPointer(),
                BytecodeLength: vs_blob.GetBufferSize(),
            },
            PS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: ps_blob.GetBufferPointer(),
                BytecodeLength: ps_blob.GetBufferSize(),
            },
            BlendState: D3D12_BLEND_DESC {
                RenderTarget: [
                    D3D12_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: true.into(),
                        SrcBlend: D3D12_BLEND_SRC_ALPHA,
                        DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
                        BlendOp: D3D12_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D12_BLEND_ONE,
                        DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
                        BlendOpAlpha: D3D12_BLEND_OP_ADD,
                        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                        ..Default::default()
                    },
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ],
                ..Default::default()
            },
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                ..Default::default()
            },
            InputLayout: D3D12_INPUT_LAYOUT_DESC {
                pInputElementDescs: input_elements.as_ptr(),
                NumElements: input_elements.len() as u32,
            },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: [
                format,
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };

        let pso = device.CreateGraphicsPipelineState(&pso_desc)?;
        Ok(pso)
    }
}

fn create_composite_pso(device: &ID3D12Device, root_signature: &ID3D12RootSignature) -> Result<ID3D12PipelineState> {
    // Fullscreen triangle shader - generates vertices procedurally
    let vs_source = r#"
        struct VSOutput {
            float4 position : SV_Position;
            float2 uv : TEXCOORD;
        };
        VSOutput main(uint vertexId : SV_VertexID) {
            VSOutput output;
            // Generate fullscreen quad from 6 vertices
            float2 positions[6] = {
                float2(-1, -1), float2(-1, 1), float2(1, 1),
                float2(-1, -1), float2(1, 1), float2(1, -1)
            };
            float2 uvs[6] = {
                float2(0, 1), float2(0, 0), float2(1, 0),
                float2(0, 1), float2(1, 0), float2(1, 1)
            };
            output.position = float4(positions[vertexId], 0.0, 1.0);
            output.uv = uvs[vertexId];
            return output;
        }
    "#;

    let ps_source = r#"
        cbuffer Constants : register(b0) {
            float paperWhiteScale;
            float3 padding;
        };
        Texture2D<float4> sdrTexture : register(t0);
        SamplerState linearSampler : register(s0);

        float4 main(float4 position : SV_Position, float2 uv : TEXCOORD) : SV_Target {
            float4 ui = sdrTexture.Sample(linearSampler, uv);
            // Scale SDR UI to HDR and blend
            float3 uiScaled = ui.rgb * paperWhiteScale;
            return float4(uiScaled, ui.a);
        }
    "#;

    let vs_blob = compile_shader(vs_source, "main", "vs_5_0")?;
    let ps_blob = compile_shader(ps_source, "main", "ps_5_0")?;

    unsafe {
        let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
            VS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: vs_blob.GetBufferPointer(),
                BytecodeLength: vs_blob.GetBufferSize(),
            },
            PS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: ps_blob.GetBufferPointer(),
                BytecodeLength: ps_blob.GetBufferSize(),
            },
            BlendState: D3D12_BLEND_DESC {
                RenderTarget: [
                    D3D12_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: true.into(),
                        SrcBlend: D3D12_BLEND_SRC_ALPHA,
                        DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
                        BlendOp: D3D12_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D12_BLEND_ONE,
                        DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
                        BlendOpAlpha: D3D12_BLEND_OP_ADD,
                        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                        ..Default::default()
                    },
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ],
                ..Default::default()
            },
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                ..Default::default()
            },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: [
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };

        let pso = device.CreateGraphicsPipelineState(&pso_desc)?;
        Ok(pso)
    }
}

fn compile_shader(source: &str, entry_point: &str, target: &str) -> Result<ID3DBlob> {
    unsafe {
        let entry = CString::new(entry_point)?;
        let target = CString::new(target)?;
        let mut blob = None;
        let mut error = None;

        let result = D3DCompile(
            source.as_ptr() as *const std::ffi::c_void,
            source.len(),
            None,
            None,
            None,
            PCSTR(entry.as_ptr() as *const u8),
            PCSTR(target.as_ptr() as *const u8),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut blob,
            Some(&mut error),
        );

        if let Some(error) = error {
            let error_msg = std::slice::from_raw_parts(
                error.GetBufferPointer() as *const u8,
                error.GetBufferSize(),
            );
            let error_str = String::from_utf8_lossy(error_msg);
            eprintln!("Shader compilation error: {}", error_str);
        }

        result?;
        blob.ok_or_else(|| anyhow!("Failed to compile shader"))
    }
}

fn create_sdr_render_target(
    device: &ID3D12Device,
    width: u32,
    height: u32,
) -> Result<(ID3D12Resource, ID3D12DescriptorHeap, ID3D12DescriptorHeap)> {
    unsafe {
        // Create SDR texture
        let mut texture: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: width as u64,
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                ..Default::default()
            },
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            Some(&D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                Anonymous: D3D12_CLEAR_VALUE_0 {
                    Color: [0.0, 0.0, 0.0, 0.0],
                },
            }),
            &mut texture,
        )?;
        let texture = texture.ok_or_else(|| anyhow!("Failed to create SDR texture"))?;

        // Create RTV heap for SDR texture
        let rtv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            NumDescriptors: 1,
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            ..Default::default()
        })?;

        device.CreateRenderTargetView(
            &texture,
            Some(&D3D12_RENDER_TARGET_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
                ..Default::default()
            }),
            rtv_heap.GetCPUDescriptorHandleForHeapStart(),
        );

        // Create SRV heap for SDR texture
        let srv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            NumDescriptors: 1,
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            ..Default::default()
        })?;

        device.CreateShaderResourceView(
            &texture,
            Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MipLevels: 1,
                        ..Default::default()
                    },
                },
            }),
            srv_heap.GetCPUDescriptorHandleForHeapStart(),
        );

        Ok((texture, rtv_heap, srv_heap))
    }
}
