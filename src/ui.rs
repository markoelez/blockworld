use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use crate::world::BlockType;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct UIVertex {
    position: [f32; 2],     // 2D screen coordinates
    tex_coords: [f32; 2],   // Texture coordinates
    color: [f32; 4],        // RGBA color/tint
    use_texture: f32,       // 0.0 = use color, 1.0+ = block type for texture
}

pub struct Inventory {
    pub slots: [Option<BlockType>; 9],
    pub selected_slot: usize,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            slots: [
                Some(BlockType::Grass),
                Some(BlockType::Dirt), 
                Some(BlockType::Stone),
                Some(BlockType::Wood),
                Some(BlockType::Leaves),
                None,
                None,
                None,
                None,
            ],
            selected_slot: 0,
        }
    }
    
    pub fn select_slot(&mut self, slot: usize) {
        if slot < self.slots.len() {
            self.selected_slot = slot;
        }
    }
    
    pub fn get_selected_block(&self) -> Option<BlockType> {
        self.slots[self.selected_slot]
    }
}

pub struct UIRenderer {
    ui_render_pipeline: wgpu::RenderPipeline,
    crosshair_vertex_buffer: wgpu::Buffer,
    crosshair_index_buffer: wgpu::Buffer,
    inventory_vertex_buffer: wgpu::Buffer,
    inventory_index_buffer: wgpu::Buffer,
    crosshair_indices: u32,
    inventory_indices: u32,
}

impl UIRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, config: &wgpu::SurfaceConfiguration, _texture_bind_group: &wgpu::BindGroup, texture_bind_group_layout: &wgpu::BindGroupLayout) -> Self {
        // Create UI shader
        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui_shader.wgsl").into()),
        });
        
        // Create render pipeline layout that includes texture binding
        let ui_render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI Render Pipeline Layout"),
            bind_group_layouts: &[texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        
        // Create UI render pipeline
        let ui_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI Render Pipeline"),
            layout: Some(&ui_render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ui_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UIVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &ui_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        // Create crosshair with corrected aspect ratio handling
        let aspect_ratio = config.width as f32 / config.height as f32;
        let crosshair_size = 0.025; // Normalized size
        let crosshair_thickness = 0.002; // Thinner for precision
        let vert_thickness = crosshair_thickness / aspect_ratio;
        let gap = 0.005; // Small gap from center
        let h_line_length = crosshair_size / aspect_ratio;
        let h_gap = gap / aspect_ratio;
        let crosshair_color = [1.0, 1.0, 1.0, 0.8]; // Slightly transparent white
        
        let crosshair_vertices = vec![
            // Vertical top line
            UIVertex { position: [-vert_thickness, gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, gap + crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-vert_thickness, gap + crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Vertical bottom line
            UIVertex { position: [-vert_thickness, -gap - crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, -gap - crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, -gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-vert_thickness, -gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Horizontal left line
            UIVertex { position: [-h_gap - h_line_length, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap - h_line_length, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Horizontal right line
            UIVertex { position: [h_gap, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap + h_line_length, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap + h_line_length, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
        ];
        
        let crosshair_indices: Vec<u16> = vec![
            0, 1, 2, 0, 2, 3,   // Top
            4, 5, 6, 4, 6, 7,   // Bottom
            8, 9, 10, 8, 10, 11, // Left
            12, 13, 14, 12, 14, 15, // Right
        ];
        
        let crosshair_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Crosshair Vertex Buffer"),
            contents: bytemuck::cast_slice(&crosshair_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let crosshair_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Crosshair Index Buffer"),
            contents: bytemuck::cast_slice(&crosshair_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        
        // Create Minecraft-style hotbar
        let (mut inventory_vertices, mut inventory_indices) = Self::create_minecraft_hotbar();
        
        // Add space for selection indicator (4 vertices)
        let selection_vertices = vec![
            UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
            UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
            UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
            UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
        ];
        let selection_indices = vec![0u16, 1, 2, 0, 2, 3];
        
        // Insert selection at the beginning
        inventory_vertices.splice(0..0, selection_vertices);
        inventory_indices.splice(0..0, selection_indices);
        for i in 6..inventory_indices.len() {
            inventory_indices[i] += 4;
        }
        
        let inventory_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Inventory Vertex Buffer"),
            size: (inventory_vertices.len() * 2 * std::mem::size_of::<UIVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        queue.write_buffer(&inventory_vertex_buffer, 0, bytemuck::cast_slice(&inventory_vertices));
        
        let inventory_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Inventory Index Buffer"),
            contents: bytemuck::cast_slice(&inventory_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        
        Self {
            ui_render_pipeline,
            crosshair_vertex_buffer,
            crosshair_index_buffer,
            inventory_vertex_buffer,
            inventory_index_buffer,
            crosshair_indices: crosshair_indices.len() as u32,
            inventory_indices: inventory_indices.len() as u32,
        }
    }
    
    fn create_minecraft_hotbar() -> (Vec<UIVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        
        // Minecraft hotbar dimensions (based on 182x22 pixels in a 256x256 GUI texture)
        let _aspect_ratio = 1.0; // Normalize coordinates (will adjust in shader if needed)
        let slot_size = 0.09; // Single slot is 20x20 pixels, scaled to normalized coords
        let slot_gap = 0.004; // 4-pixel gap between slots
        let total_slots_width = 9.0 * (slot_size * 2.0) + 8.0 * slot_gap;
        let start_x = -total_slots_width / 2.0;
        let hotbar_y = -0.82; // Position near bottom of screen
        
        // Hotbar background (semi-transparent dark gray with light gray border)
        let bg_padding = 0.01; // Matches Minecraft's 2-pixel padding
        let bg_color = [0.1, 0.1, 0.1, 0.5]; // Dark gray, semi-transparent
        let border_color = [0.8, 0.8, 0.8, 1.0]; // Light gray border
        
        let bg_left = start_x - slot_size - bg_padding;
        let bg_right = start_x + total_slots_width + slot_size + bg_padding;
        let bg_top = hotbar_y + slot_size + bg_padding;
        let bg_bottom = hotbar_y - slot_size - bg_padding;
        
        // Outer border
        let border_thickness = 0.005; // Matches Minecraft's 1-pixel border
        let base_idx = vertices.len() as u16;
        vertices.extend_from_slice(&[
            UIVertex { position: [bg_left - border_thickness, bg_bottom - border_thickness], tex_coords: [0.0, 0.0], color: border_color, use_texture: 0.0 },
            UIVertex { position: [bg_right + border_thickness, bg_bottom - border_thickness], tex_coords: [0.0, 0.0], color: border_color, use_texture: 0.0 },
            UIVertex { position: [bg_right + border_thickness, bg_top + border_thickness], tex_coords: [0.0, 0.0], color: border_color, use_texture: 0.0 },
            UIVertex { position: [bg_left - border_thickness, bg_top + border_thickness], tex_coords: [0.0, 0.0], color: border_color, use_texture: 0.0 },
        ]);
        indices.extend_from_slice(&[base_idx, base_idx + 1, base_idx + 2, base_idx, base_idx + 2, base_idx + 3]);
        
        // Inner background
        let bg_idx = vertices.len() as u16;
        vertices.extend_from_slice(&[
            UIVertex { position: [bg_left, bg_bottom], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 },
            UIVertex { position: [bg_right, bg_bottom], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 },
            UIVertex { position: [bg_right, bg_top], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 },
            UIVertex { position: [bg_left, bg_top], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 },
        ]);
        indices.extend_from_slice(&[bg_idx, bg_idx + 1, bg_idx + 2, bg_idx, bg_idx + 2, bg_idx + 3]);
        
        // Individual slots
        for i in 0..9 {
            let slot_x = start_x + slot_size + i as f32 * (slot_size * 2.0 + slot_gap);
            let slot_idx = vertices.len() as u16;
            
            // Slot border (light gray)
            let slot_border_color = [0.8, 0.8, 0.8, 1.0];
            let border_size = slot_size + 0.005; // 1-pixel border
            vertices.extend_from_slice(&[
                UIVertex { position: [slot_x - border_size, hotbar_y - border_size], tex_coords: [0.0, 0.0], color: slot_border_color, use_texture: 0.0 },
                UIVertex { position: [slot_x + border_size, hotbar_y - border_size], tex_coords: [0.0, 0.0], color: slot_border_color, use_texture: 0.0 },
                UIVertex { position: [slot_x + border_size, hotbar_y + border_size], tex_coords: [0.0, 0.0], color: slot_border_color, use_texture: 0.0 },
                UIVertex { position: [slot_x - border_size, hotbar_y + border_size], tex_coords: [0.0, 0.0], color: slot_border_color, use_texture: 0.0 },
            ]);
            indices.extend_from_slice(&[slot_idx, slot_idx + 1, slot_idx + 2, slot_idx, slot_idx + 2, slot_idx + 3]);
            
            // Slot background (dark gray)
            let slot_bg_color = [0.2, 0.2, 0.2, 1.0];
            let slot_bg_idx = vertices.len() as u16;
            vertices.extend_from_slice(&[
                UIVertex { position: [slot_x - slot_size, hotbar_y - slot_size], tex_coords: [0.0, 0.0], color: slot_bg_color, use_texture: 0.0 },
                UIVertex { position: [slot_x + slot_size, hotbar_y - slot_size], tex_coords: [0.0, 0.0], color: slot_bg_color, use_texture: 0.0 },
                UIVertex { position: [slot_x + slot_size, hotbar_y + slot_size], tex_coords: [0.0, 0.0], color: slot_bg_color, use_texture: 0.0 },
                UIVertex { position: [slot_x - slot_size, hotbar_y + slot_size], tex_coords: [0.0, 0.0], color: slot_bg_color, use_texture: 0.0 },
            ]);
            indices.extend_from_slice(&[slot_bg_idx, slot_bg_idx + 1, slot_bg_idx + 2, slot_bg_idx, slot_bg_idx + 2, slot_bg_idx + 3]);
            
            // Block icon (if slot has item)
            if i < 5 {
                let icon_size = slot_size * 0.8; // Matches Minecraft's 16x16 item in 20x20 slot
                let icon_idx = vertices.len() as u16;
                let block_type = match i {
                    0 => 1.0, // Grass
                    1 => 2.0, // Dirt
                    2 => 3.0, // Stone
                    3 => 4.0, // Wood
                    4 => 5.0, // Leaves
                    _ => 0.0,
                };
                let icon_color = [1.0, 1.0, 1.0, 1.0]; // Full white for proper texture color
                vertices.extend_from_slice(&[
                    UIVertex { position: [slot_x - icon_size, hotbar_y - icon_size], tex_coords: [0.0, 1.0], color: icon_color, use_texture: block_type },
                    UIVertex { position: [slot_x + icon_size, hotbar_y - icon_size], tex_coords: [1.0, 1.0], color: icon_color, use_texture: block_type },
                    UIVertex { position: [slot_x + icon_size, hotbar_y + icon_size], tex_coords: [1.0, 0.0], color: icon_color, use_texture: block_type },
                    UIVertex { position: [slot_x - icon_size, hotbar_y + icon_size], tex_coords: [0.0, 0.0], color: icon_color, use_texture: block_type },
                ]);
                indices.extend_from_slice(&[icon_idx, icon_idx + 1, icon_idx + 2, icon_idx, icon_idx + 2, icon_idx + 3]);
            }
        }
        
        (vertices, indices)
    }
    
    pub fn resize(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
        // Update crosshair for new aspect ratio
        let aspect_ratio = config.width as f32 / config.height as f32;
        let crosshair_size = 0.025;
        let crosshair_thickness = 0.002;
        let vert_thickness = crosshair_thickness / aspect_ratio;
        let gap = 0.005;
        let h_line_length = crosshair_size / aspect_ratio;
        let h_gap = gap / aspect_ratio;
        let crosshair_color = [1.0, 1.0, 1.0, 0.8];
        
        let crosshair_vertices = vec![
            // Vertical top line
            UIVertex { position: [-vert_thickness, gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, gap + crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-vert_thickness, gap + crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Vertical bottom line
            UIVertex { position: [-vert_thickness, -gap - crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, -gap - crosshair_size], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [vert_thickness, -gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-vert_thickness, -gap], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Horizontal left line
            UIVertex { position: [-h_gap - h_line_length, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [-h_gap - h_line_length, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            
            // Horizontal right line
            UIVertex { position: [h_gap, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap + h_line_length, -crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap + h_line_length, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
            UIVertex { position: [h_gap, crosshair_thickness], tex_coords: [0.0, 0.0], color: crosshair_color, use_texture: 0.0 },
        ];
        
        self.crosshair_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Crosshair Vertex Buffer"),
            contents: bytemuck::cast_slice(&crosshair_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    }
    
    pub fn update_inventory_selection(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, inventory: &Inventory) {
        let (mut vertices, _indices) = Self::create_minecraft_hotbar();
        
        // Update selection indicator
        let selected = inventory.selected_slot;
        if selected < 9 {
            let slot_size = 0.09;
            let slot_gap = 0.004;
            let total_slots_width = 9.0 * (slot_size * 2.0) + 8.0 * slot_gap;
            let start_x = -total_slots_width / 2.0;
            let hotbar_y = -0.82;
            
            let slot_x = start_x + slot_size + selected as f32 * (slot_size * 2.0 + slot_gap);
            let selection_size = slot_size + 0.01; // Matches Minecraft's 2-pixel thick white border
            let selection_color = [1.0, 1.0, 1.0, 1.0]; // White, fully opaque
            
            vertices[0] = UIVertex { position: [slot_x - selection_size, hotbar_y - selection_size], tex_coords: [0.0, 0.0], color: selection_color, use_texture: 0.0 };
            vertices[1] = UIVertex { position: [slot_x + selection_size, hotbar_y - selection_size], tex_coords: [0.0, 0.0], color: selection_color, use_texture: 0.0 };
            vertices[2] = UIVertex { position: [slot_x + selection_size, hotbar_y + selection_size], tex_coords: [0.0, 0.0], color: selection_color, use_texture: 0.0 };
            vertices[3] = UIVertex { position: [slot_x - selection_size, hotbar_y + selection_size], tex_coords: [0.0, 0.0], color: selection_color, use_texture: 0.0 };
        } else {
            let transparent = [0.0, 0.0, 0.0, 0.0];
            vertices[0] = UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: transparent, use_texture: 0.0 };
            vertices[1] = UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: transparent, use_texture: 0.0 };
            vertices[2] = UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: transparent, use_texture: 0.0 };
            vertices[3] = UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: transparent, use_texture: 0.0 };
        }
        
        // Update slot contents based on inventory
        let slot_size = 0.09;
        let slot_gap = 0.004;
        let total_slots_width = 9.0 * (slot_size * 2.0) + 8.0 * slot_gap;
        let start_x = -total_slots_width / 2.0;
        let hotbar_y = -0.82;
        let icon_size = slot_size * 0.8;
        let icon_color = [1.0, 1.0, 1.0, 1.0];
        
        let mut _vertex_offset = 12; // After background (8) and selection (4)
        for i in 0..9 {
            // Skip border and background vertices (8 per slot)
            _vertex_offset += 8;
            
            if let Some(block_type) = inventory.slots[i] {
                let slot_x = start_x + slot_size + i as f32 * (slot_size * 2.0 + slot_gap);
                let block_type_val = match block_type {
                    BlockType::Grass => 1.0,
                    BlockType::Dirt => 2.0,
                    BlockType::Stone => 3.0,
                    BlockType::Wood => 4.0,
                    BlockType::Leaves => 5.0,
                    BlockType::Water => 6.0,
                    BlockType::Air | BlockType::Barrier => 0.0,
                };
                
                vertices.extend_from_slice(&[
                    UIVertex { position: [slot_x - icon_size, hotbar_y - icon_size], tex_coords: [0.0, 1.0], color: icon_color, use_texture: block_type_val },
                    UIVertex { position: [slot_x + icon_size, hotbar_y - icon_size], tex_coords: [1.0, 1.0], color: icon_color, use_texture: block_type_val },
                    UIVertex { position: [slot_x + icon_size, hotbar_y + icon_size], tex_coords: [1.0, 0.0], color: icon_color, use_texture: block_type_val },
                    UIVertex { position: [slot_x - icon_size, hotbar_y + icon_size], tex_coords: [0.0, 0.0], color: icon_color, use_texture: block_type_val },
                ]);
            } else {
                // Add transparent vertices for empty slots
                vertices.extend_from_slice(&[
                    UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
                    UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
                    UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
                    UIVertex { position: [0.0, 0.0], tex_coords: [0.0, 0.0], color: [0.0, 0.0, 0.0, 0.0], use_texture: 0.0 },
                ]);
            }
            _vertex_offset += 4;
        }
        
        queue.write_buffer(&self.inventory_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }
    
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, texture_bind_group: &'a wgpu::BindGroup) {
        render_pass.set_pipeline(&self.ui_render_pipeline);
        render_pass.set_bind_group(0, texture_bind_group, &[]);
        
        // Render crosshair
        render_pass.set_vertex_buffer(0, self.crosshair_vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.crosshair_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.crosshair_indices, 0, 0..1);
        
        // Render inventory
        render_pass.set_vertex_buffer(0, self.inventory_vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.inventory_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.inventory_indices, 0, 0..1);
    }
}