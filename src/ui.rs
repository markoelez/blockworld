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
    pub slots: [Option<(BlockType, u32)>; HOTBAR_NUM_SLOTS],
    pub selected_slot: usize,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            slots: [None; HOTBAR_NUM_SLOTS],
            selected_slot: 0,
        }
    }
}

pub struct DebugInfo {
    pub visible: bool,
}

impl DebugInfo {
    pub fn new() -> Self {
        Self { visible: false }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

pub struct PauseMenu {
    pub visible: bool,
    pub selected_option: usize,
}

impl PauseMenu {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected_option: 0,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.selected_option = 0;
        }
    }

    pub fn navigate(&mut self, delta: i32) {
        let num_options = 3i32;
        self.selected_option = ((self.selected_option as i32 + delta).rem_euclid(num_options)) as usize;
    }

    pub fn get_selected_action(&self) -> &'static str {
        match self.selected_option {
            0 => "RESUME",
            1 => "OPTIONS",
            2 => "QUIT",
            _ => "RESUME",
        }
    }
}

pub struct ChestUI {
    pub open: bool,
    pub chest_pos: Option<(i32, i32, i32)>,
    pub selected_slot: usize,
    pub in_chest_section: bool,  // true = chest slots, false = player inventory
}

impl ChestUI {
    pub fn new() -> Self {
        Self {
            open: false,
            chest_pos: None,
            selected_slot: 0,
            in_chest_section: true,
        }
    }

    pub fn open_chest(&mut self, pos: (i32, i32, i32)) {
        self.open = true;
        self.chest_pos = Some(pos);
        self.selected_slot = 0;
        self.in_chest_section = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.chest_pos = None;
    }

    pub fn navigate(&mut self, dx: i32, dy: i32) {
        // Both sections now have 9 slots
        let max_slot = 8;

        // Handle horizontal navigation
        let new_slot = (self.selected_slot as i32 + dx).clamp(0, max_slot as i32) as usize;
        self.selected_slot = new_slot;

        // Handle vertical navigation (switch between chest and inventory sections)
        if dy != 0 {
            self.in_chest_section = !self.in_chest_section;
        }
    }
}

impl Inventory {
    pub fn select_slot(&mut self, slot: usize) {
        if slot < self.slots.len() {
            self.selected_slot = slot;
        }
    }
    
    pub fn get_selected_block(&self) -> Option<BlockType> {
        self.slots[self.selected_slot].and_then(|(bt, qty)| if qty > 0 { Some(bt) } else { None })
    }

    pub fn decrement_selected(&mut self) {
        if let Some((_, qty)) = &mut self.slots[self.selected_slot] {
            if *qty > 0 {
                *qty -= 1;
                if *qty == 0 {
                    self.slots[self.selected_slot] = None;
                }
            }
        }
    }

    pub fn add_block(&mut self, block_type: BlockType) -> bool {
        // First try to add to existing slot
        for slot in self.slots.iter_mut() {
            if let Some((bt, qty)) = slot {
                if *bt == block_type {
                    *qty += 1;
                    return true;
                }
            }
        }
        // Then try to add to empty slot
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some((block_type, 1));
                return true;
            }
        }
        false
    }
}

// Hotbar layout constants
const HOTBAR_SLOT_SIZE: f32 = 0.04;
const HOTBAR_NUM_SLOTS: usize = 9;  // Expanded to 9 slots like Minecraft
const HOTBAR_DIVIDER_WIDTH: f32 = 0.002;
const HOTBAR_Y: f32 = -0.92;
const HOTBAR_BG_PADDING: f32 = 0.006;

// UI Atlas constants (256x256 atlas)
const ATLAS_SIZE: f32 = 256.0;
const GLYPH_SIZE: f32 = 8.0;       // 8x8 pixel font glyphs
const GLYPH_CELL: f32 = 16.0;      // Glyphs are centered in 16x16 cells
const FONT_START_Y: f32 = 64.0;    // Font atlas starts at Y=64
const GLYPHS_PER_ROW: usize = 16;
const FIRST_CHAR: u8 = 32;         // ASCII space

// UI Atlas element positions (in 16x16 cell coordinates)
const SLOT_EMPTY_UV: (f32, f32) = (0.0, 0.0);      // Row 0, Col 0
const SLOT_SELECTED_UV: (f32, f32) = (16.0, 0.0);  // Row 0, Col 1
const SLOT_HOVER_UV: (f32, f32) = (32.0, 0.0);     // Row 0, Col 2

// 9-slice panel pieces (Row 1)
const PANEL_TL_UV: (f32, f32) = (0.0, 16.0);
const PANEL_T_UV: (f32, f32) = (16.0, 16.0);
const PANEL_TR_UV: (f32, f32) = (32.0, 16.0);
const PANEL_L_UV: (f32, f32) = (48.0, 16.0);
const PANEL_C_UV: (f32, f32) = (64.0, 16.0);
const PANEL_R_UV: (f32, f32) = (80.0, 16.0);
const PANEL_BL_UV: (f32, f32) = (96.0, 16.0);
const PANEL_B_UV: (f32, f32) = (112.0, 16.0);
const PANEL_BR_UV: (f32, f32) = (128.0, 16.0);

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
    
    fn hotbar_total_width() -> f32 {
        HOTBAR_NUM_SLOTS as f32 * (HOTBAR_SLOT_SIZE * 2.0) + (HOTBAR_NUM_SLOTS - 1) as f32 * HOTBAR_DIVIDER_WIDTH
    }

    fn hotbar_start_x() -> f32 {
        -Self::hotbar_total_width() / 2.0 + HOTBAR_SLOT_SIZE
    }

    fn block_type_to_ui_index(block_type: BlockType) -> f32 {
        match block_type {
            BlockType::Grass => 1.0,
            BlockType::Dirt => 2.0,
            BlockType::Stone => 3.0,
            BlockType::Wood => 4.0,
            BlockType::Leaves => 5.0,
            BlockType::Water => 6.0,
            BlockType::Sand => 7.0,
            BlockType::Snow => 8.0,
            BlockType::Ice => 9.0,
            BlockType::Cobblestone => 10.0,
            BlockType::Coal => 11.0,
            BlockType::Iron => 12.0,
            BlockType::Gold => 13.0,
            BlockType::Diamond => 14.0,
            BlockType::Gravel => 15.0,
            BlockType::Clay => 16.0,
            BlockType::Torch => 24.0,
            BlockType::Chest => 26.0,
            BlockType::Air | BlockType::Barrier => 0.0,
        }
    }

    /// Calculate UV coordinates for a character in the font atlas
    fn char_to_uv(c: char) -> (f32, f32, f32, f32) {
        let ascii = c as u8;
        let index = if ascii >= FIRST_CHAR && ascii <= 126 {
            (ascii - FIRST_CHAR) as usize
        } else {
            0 // Default to space for unsupported chars
        };

        let col = index % GLYPHS_PER_ROW;
        let row = index / GLYPHS_PER_ROW;

        // UV coordinates (normalized 0-1)
        let u0 = (col as f32 * GLYPH_CELL + 4.0) / ATLAS_SIZE; // +4 to center 8x8 in 16x16 cell
        let v0 = (FONT_START_Y + row as f32 * GLYPH_CELL + 4.0) / ATLAS_SIZE;
        let u1 = u0 + GLYPH_SIZE / ATLAS_SIZE;
        let v1 = v0 + GLYPH_SIZE / ATLAS_SIZE;

        (u0, v0, u1, v1)
    }

    /// Calculate the width of text at a given scale
    fn text_width(text: &str, scale: f32) -> f32 {
        let char_width = scale * 0.6; // Characters are slightly narrower than tall
        text.len() as f32 * char_width
    }

    /// Generate vertices for bitmap font text
    /// Returns (vertices, indices) that can be appended to existing buffers
    /// use_texture = -1.0 signals font mode in shader (uses alpha from atlas, color from vertex)
    fn generate_text_vertices(
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
        base_index: u16,
    ) -> (Vec<UIVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        let char_width = scale * 0.6;
        let char_height = scale;

        for (i, c) in text.chars().enumerate() {
            let (u0, v0, u1, v1) = Self::char_to_uv(c);
            let cx = x + i as f32 * char_width;

            let idx = base_index + (i as u16 * 4);
            vertices.extend_from_slice(&[
                UIVertex { position: [cx, y], tex_coords: [u0, v1], color, use_texture: -1.0 },
                UIVertex { position: [cx + char_width, y], tex_coords: [u1, v1], color, use_texture: -1.0 },
                UIVertex { position: [cx + char_width, y + char_height], tex_coords: [u1, v0], color, use_texture: -1.0 },
                UIVertex { position: [cx, y + char_height], tex_coords: [u0, v0], color, use_texture: -1.0 },
            ]);
            indices.extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
        }

        (vertices, indices)
    }

    /// Generate centered text at a given Y position
    fn generate_centered_text(
        text: &str,
        center_x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
        base_index: u16,
    ) -> (Vec<UIVertex>, Vec<u16>) {
        let text_w = Self::text_width(text, scale);
        let start_x = center_x - text_w / 2.0;
        Self::generate_text_vertices(text, start_x, y, scale, color, base_index)
    }

    /// Generate text with a shadow effect (like Minecraft item counts)
    /// Renders dark text offset down-right, then white text on top
    fn generate_text_with_shadow(
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
        base_index: u16,
    ) -> (Vec<UIVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        let shadow_offset = scale * 0.12;  // Shadow offset
        let shadow_color = [0.15, 0.15, 0.15, color[3]];  // Dark shadow

        // Shadow (rendered first, behind)
        let (shadow_verts, shadow_inds) = Self::generate_text_vertices(
            text, x + shadow_offset, y - shadow_offset, scale,
            shadow_color, base_index
        );
        vertices.extend(shadow_verts);
        indices.extend(shadow_inds);

        // Main text (rendered on top)
        let (text_verts, text_inds) = Self::generate_text_vertices(
            text, x, y, scale,
            color, base_index + vertices.len() as u16
        );
        vertices.extend(text_verts);
        indices.extend(text_inds);

        (vertices, indices)
    }

    /// Generate vertices for a slot from the UI atlas
    /// slot_type: 0 = empty, 1 = selected, 2 = hovered
    fn generate_slot_vertices(
        x: f32,
        y: f32,
        size: f32,
        slot_type: u8,
        color: [f32; 4],
        base_index: u16,
    ) -> (Vec<UIVertex>, Vec<u16>) {
        let (atlas_x, atlas_y) = match slot_type {
            1 => SLOT_SELECTED_UV,
            2 => SLOT_HOVER_UV,
            _ => SLOT_EMPTY_UV,
        };

        // UV coordinates for 16x16 slot texture
        let u0 = atlas_x / ATLAS_SIZE;
        let v0 = atlas_y / ATLAS_SIZE;
        let u1 = (atlas_x + 16.0) / ATLAS_SIZE;
        let v1 = (atlas_y + 16.0) / ATLAS_SIZE;

        let vertices = vec![
            UIVertex { position: [x - size, y - size], tex_coords: [u0, v1], color, use_texture: -2.0 },
            UIVertex { position: [x + size, y - size], tex_coords: [u1, v1], color, use_texture: -2.0 },
            UIVertex { position: [x + size, y + size], tex_coords: [u1, v0], color, use_texture: -2.0 },
            UIVertex { position: [x - size, y + size], tex_coords: [u0, v0], color, use_texture: -2.0 },
        ];

        let indices = vec![base_index, base_index + 1, base_index + 2, base_index, base_index + 2, base_index + 3];
        (vertices, indices)
    }

    /// Generate vertices for a 9-slice panel
    /// The panel stretches the center while keeping corners and edges fixed
    fn generate_nine_slice_panel(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        border: f32,  // Size of corners/edges in screen coords
        color: [f32; 4],
        base_index: u16,
    ) -> (Vec<UIVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        // UV size for each piece (16x16 in atlas)
        let uv_size = 16.0 / ATLAS_SIZE;

        // Helper to add a quad
        let mut add_quad = |px: f32, py: f32, pw: f32, ph: f32, atlas_x: f32, atlas_y: f32| {
            let u0 = atlas_x / ATLAS_SIZE;
            let v0 = atlas_y / ATLAS_SIZE;
            let u1 = u0 + uv_size;
            let v1 = v0 + uv_size;

            let idx = base_index + vertices.len() as u16;
            vertices.extend_from_slice(&[
                UIVertex { position: [px, py], tex_coords: [u0, v1], color, use_texture: -2.0 },
                UIVertex { position: [px + pw, py], tex_coords: [u1, v1], color, use_texture: -2.0 },
                UIVertex { position: [px + pw, py + ph], tex_coords: [u1, v0], color, use_texture: -2.0 },
                UIVertex { position: [px, py + ph], tex_coords: [u0, v0], color, use_texture: -2.0 },
            ]);
            indices.extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
        };

        let inner_w = width - 2.0 * border;
        let inner_h = height - 2.0 * border;

        // Bottom-left corner
        add_quad(x, y, border, border, PANEL_BL_UV.0, PANEL_BL_UV.1);
        // Bottom edge
        add_quad(x + border, y, inner_w, border, PANEL_B_UV.0, PANEL_B_UV.1);
        // Bottom-right corner
        add_quad(x + border + inner_w, y, border, border, PANEL_BR_UV.0, PANEL_BR_UV.1);

        // Left edge
        add_quad(x, y + border, border, inner_h, PANEL_L_UV.0, PANEL_L_UV.1);
        // Center
        add_quad(x + border, y + border, inner_w, inner_h, PANEL_C_UV.0, PANEL_C_UV.1);
        // Right edge
        add_quad(x + border + inner_w, y + border, border, inner_h, PANEL_R_UV.0, PANEL_R_UV.1);

        // Top-left corner
        add_quad(x, y + border + inner_h, border, border, PANEL_TL_UV.0, PANEL_TL_UV.1);
        // Top edge
        add_quad(x + border, y + border + inner_h, inner_w, border, PANEL_T_UV.0, PANEL_T_UV.1);
        // Top-right corner
        add_quad(x + border + inner_w, y + border + inner_h, border, border, PANEL_TR_UV.0, PANEL_TR_UV.1);

        (vertices, indices)
    }

    fn create_minecraft_hotbar() -> (Vec<UIVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        let total_width = Self::hotbar_total_width();
        let start_x = Self::hotbar_start_x();

        // 9-slice panel background
        let panel_padding = 0.008;
        let panel_left = start_x - HOTBAR_SLOT_SIZE - panel_padding;
        let panel_bottom = HOTBAR_Y - HOTBAR_SLOT_SIZE - panel_padding;
        let panel_width = total_width + panel_padding * 2.0;
        let panel_height = HOTBAR_SLOT_SIZE * 2.0 + panel_padding * 2.0;
        let panel_border = 0.008;
        let panel_color = [1.0, 1.0, 1.0, 0.95];

        let (panel_verts, panel_inds) = Self::generate_nine_slice_panel(
            panel_left, panel_bottom, panel_width, panel_height,
            panel_border, panel_color, vertices.len() as u16
        );
        vertices.extend(panel_verts);
        indices.extend(panel_inds);

        // Render slot backgrounds
        let slot_color = [1.0, 1.0, 1.0, 1.0];
        for i in 0..HOTBAR_NUM_SLOTS {
            let slot_x = start_x + i as f32 * (HOTBAR_SLOT_SIZE * 2.0 + HOTBAR_DIVIDER_WIDTH);
            let (slot_verts, slot_inds) = Self::generate_slot_vertices(
                slot_x, HOTBAR_Y, HOTBAR_SLOT_SIZE * 0.95,
                0, // Empty slot
                slot_color, vertices.len() as u16
            );
            vertices.extend(slot_verts);
            indices.extend(slot_inds);
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
    
    pub fn update_inventory_selection(&mut self, device: &wgpu::Device, inventory: &Inventory) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        let total_width = Self::hotbar_total_width();
        let start_x = Self::hotbar_start_x();

        // 9-slice panel background
        let panel_padding = 0.008;
        let panel_left = start_x - HOTBAR_SLOT_SIZE - panel_padding;
        let panel_bottom = HOTBAR_Y - HOTBAR_SLOT_SIZE - panel_padding;
        let panel_width = total_width + panel_padding * 2.0;
        let panel_height = HOTBAR_SLOT_SIZE * 2.0 + panel_padding * 2.0;
        let panel_border = 0.008;
        let panel_color = [1.0, 1.0, 1.0, 0.95];

        let (panel_verts, panel_inds) = Self::generate_nine_slice_panel(
            panel_left, panel_bottom, panel_width, panel_height,
            panel_border, panel_color, vertices.len() as u16
        );
        vertices.extend(panel_verts);
        indices.extend(panel_inds);

        let selected = inventory.selected_slot;
        let icon_size = HOTBAR_SLOT_SIZE * 0.7;
        let icon_color = [1.0, 1.0, 1.0, 1.0];

        for i in 0..HOTBAR_NUM_SLOTS {
            let slot_x = start_x + i as f32 * (HOTBAR_SLOT_SIZE * 2.0 + HOTBAR_DIVIDER_WIDTH);
            let is_selected = i == selected;

            // Slot background from atlas (selected or empty)
            let slot_type = if is_selected { 1 } else { 0 };
            let (slot_verts, slot_inds) = Self::generate_slot_vertices(
                slot_x, HOTBAR_Y, HOTBAR_SLOT_SIZE * 0.95,
                slot_type, [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
            );
            vertices.extend(slot_verts);
            indices.extend(slot_inds);

            // Slot content (block texture)
            if let Some((block_type, qty)) = inventory.slots[i] {
                if qty > 0 {
                    let block_type_val = Self::block_type_to_ui_index(block_type);
                    let icon_base = vertices.len() as u16;
                    vertices.extend_from_slice(&[
                        UIVertex { position: [slot_x - icon_size, HOTBAR_Y - icon_size], tex_coords: [0.0, 1.0], color: icon_color, use_texture: block_type_val },
                        UIVertex { position: [slot_x + icon_size, HOTBAR_Y - icon_size], tex_coords: [1.0, 1.0], color: icon_color, use_texture: block_type_val },
                        UIVertex { position: [slot_x + icon_size, HOTBAR_Y + icon_size], tex_coords: [1.0, 0.0], color: icon_color, use_texture: block_type_val },
                        UIVertex { position: [slot_x - icon_size, HOTBAR_Y + icon_size], tex_coords: [0.0, 0.0], color: icon_color, use_texture: block_type_val },
                    ]);
                    indices.extend_from_slice(&[icon_base, icon_base + 1, icon_base + 2, icon_base, icon_base + 2, icon_base + 3]);

                    // Quantity in bottom-right corner with shadow (Minecraft style)
                    if qty > 1 {
                        let qty_str = qty.to_string();
                        let text_scale = HOTBAR_SLOT_SIZE * 0.7;  // Larger text
                        let text_w = Self::text_width(&qty_str, text_scale);
                        let text_x = slot_x + HOTBAR_SLOT_SIZE * 0.85 - text_w;
                        let text_y = HOTBAR_Y - HOTBAR_SLOT_SIZE * 0.85;
                        let (text_verts, text_inds) = Self::generate_text_with_shadow(
                            &qty_str, text_x, text_y, text_scale,
                            [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
                        );
                        vertices.extend(text_verts);
                        indices.extend(text_inds);
                    }
                }
            }
        }

        self.inventory_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Inventory Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.inventory_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Inventory Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.inventory_indices = indices.len() as u32;
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

    fn get_digit_vertices(digit: usize, x: f32, y: f32, size: f32, color: [f32;4]) -> (Vec<UIVertex>, Vec<u16>) {
        // Simple line-based digits
        let h = size;
        let w = size * 0.5;
        let t = size * 0.1; // thickness
        let mut verts = Vec::new();
        let mut inds = Vec::new();
        let mut add_line = |x1: f32, y1: f32, x2: f32, y2: f32| {
            let dx = x2 - x1;
            let dy = y2 - y1;
            let len = (dx*dx + dy*dy).sqrt();
            let nx = -dy / len * t/2.0;
            let ny = dx / len * t/2.0;
            let base = verts.len() as u16;
            verts.push(UIVertex { position: [x + x1 + nx, y + y1 + ny], tex_coords: [0.0,0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x2 + nx, y + y2 + ny], tex_coords: [0.0,0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x2 - nx, y + y2 - ny], tex_coords: [0.0,0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x1 - nx, y + y1 - ny], tex_coords: [0.0,0.0], color, use_texture: 0.0 });
            inds.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
        };
        match digit {
            0 => { add_line(0.0,0.0,0.0,h); add_line(0.0,h,w,h); add_line(w,h,w,0.0); add_line(0.0,0.0,w,0.0); }
            1 => { add_line(w/2.0,0.0,w/2.0,h); }
            2 => { add_line(0.0,h,w,h); add_line(w,h,w,h/2.0); add_line(0.0,h/2.0,w,h/2.0); add_line(0.0,0.0,0.0,h/2.0); add_line(0.0,0.0,w,0.0); }
            3 => { add_line(0.0,h,w,h); add_line(w,h,w,0.0); add_line(0.0,0.0,w,0.0); add_line(0.0,h/2.0,w,h/2.0); }
            4 => { add_line(0.0,h,0.0,h/2.0); add_line(0.0,h/2.0,w,h/2.0); add_line(w,h,w,0.0); add_line(w,h/2.0,w,h); }
            5 => { add_line(w,h,0.0,h); add_line(0.0,h,0.0,h/2.0); add_line(0.0,h/2.0,w,h/2.0); add_line(w,h/2.0,w,0.0); add_line(0.0,0.0,w,0.0); }
            6 => { add_line(0.0,0.0,0.0,h); add_line(0.0,h,w,h); add_line(w,h/2.0,w,0.0); add_line(0.0,h/2.0,w,h/2.0); add_line(0.0,0.0,w,0.0); }
            7 => { add_line(0.0,h,w,h); add_line(w,h,w,0.0); }
            8 => { add_line(0.0,0.0,0.0,h); add_line(0.0,h,w,h); add_line(w,h,w,0.0); add_line(0.0,0.0,w,0.0); add_line(0.0,h/2.0,w,h/2.0); }
            9 => { add_line(0.0,h,w,h); add_line(w,h,w,h/2.0); add_line(0.0,h/2.0,w,h/2.0); add_line(0.0,0.0,0.0,h/2.0); add_line(w,0.0,0.0,0.0); }
            _ => {},
        }
        (verts, inds)
    }

    /// Render a simple loading screen with progress bar and message
    pub fn render_loading_screen(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        config: &wgpu::SurfaceConfiguration,
        texture_bind_group: &wgpu::BindGroup,
        progress: f32,
        _message: &str
    ) {
        let mut vertices: Vec<UIVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        let _aspect = config.width as f32 / config.height as f32;

        // Progress bar dimensions
        let bar_width = 0.6;
        let bar_height = 0.04;
        let bar_x = -bar_width / 2.0;
        let bar_y = -0.1;

        // Background of progress bar (dark gray)
        let bg_color = [0.2, 0.2, 0.2, 1.0];
        let base = vertices.len() as u16;
        vertices.push(UIVertex { position: [bar_x, bar_y], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x + bar_width, bar_y], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x + bar_width, bar_y + bar_height], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x, bar_y + bar_height], tex_coords: [0.0, 0.0], color: bg_color, use_texture: 0.0 });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

        // Foreground of progress bar (green)
        let fg_color = [0.2, 0.8, 0.3, 1.0];
        let filled_width = bar_width * progress.clamp(0.0, 1.0);
        let padding = 0.005;
        let base = vertices.len() as u16;
        vertices.push(UIVertex { position: [bar_x + padding, bar_y + padding], tex_coords: [0.0, 0.0], color: fg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x + padding + filled_width - 2.0 * padding, bar_y + padding], tex_coords: [0.0, 0.0], color: fg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x + padding + filled_width - 2.0 * padding, bar_y + bar_height - padding], tex_coords: [0.0, 0.0], color: fg_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [bar_x + padding, bar_y + bar_height - padding], tex_coords: [0.0, 0.0], color: fg_color, use_texture: 0.0 });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

        // Loading text using simple lines (draw "LOADING..." above bar)
        let text_color = [1.0, 1.0, 1.0, 1.0];
        let char_size = 0.05;
        let char_spacing = char_size * 0.7;
        let text_y = bar_y + bar_height + 0.05;

        // Draw "LOADING" using line segments
        let letters = ['L', 'O', 'A', 'D', 'I', 'N', 'G'];
        let text_width = letters.len() as f32 * char_spacing;
        let mut text_x = -text_width / 2.0;

        for letter in letters {
            let (letter_verts, letter_inds) = Self::get_letter_vertices(letter, text_x, text_y, char_size, text_color, vertices.len() as u16);
            vertices.extend(letter_verts);
            indices.extend(letter_inds);
            text_x += char_spacing;
        }

        // Draw percentage text below bar
        let percent = (progress * 100.0) as usize;
        let percent_str = format!("{}", percent.min(100));
        let digit_size = 0.03;
        let digit_spacing = digit_size * 0.7;
        let percent_width = percent_str.len() as f32 * digit_spacing + digit_size * 0.5; // +0.5 for %
        let mut digit_x = -percent_width / 2.0;
        let digit_y = bar_y - 0.06;

        for c in percent_str.chars() {
            if let Some(digit) = c.to_digit(10) {
                let (digit_verts, digit_inds) = Self::get_digit_vertices(digit as usize, digit_x, digit_y, digit_size, text_color);
                let base = vertices.len() as u16;
                for v in digit_verts {
                    vertices.push(v);
                }
                for i in digit_inds {
                    indices.push(base + i);
                }
                digit_x += digit_spacing;
            }
        }

        // Draw % symbol
        let (percent_verts, percent_inds) = Self::get_letter_vertices('%', digit_x, digit_y, digit_size, text_color, vertices.len() as u16);
        vertices.extend(percent_verts);
        indices.extend(percent_inds);

        // Create buffers and render
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Loading Screen Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Loading Screen Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Loading UI Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Loading UI Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(&self.ui_render_pipeline);
            render_pass.set_bind_group(0, texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    fn get_letter_vertices(letter: char, x: f32, y: f32, size: f32, color: [f32; 4], base_index: u16) -> (Vec<UIVertex>, Vec<u16>) {
        let h = size;
        let w = size * 0.5;
        let t = size * 0.1;
        let mut verts = Vec::new();
        let mut inds = Vec::new();
        let mut idx = base_index;

        let mut add_line = |x1: f32, y1: f32, x2: f32, y2: f32| {
            let dx = x2 - x1;
            let dy = y2 - y1;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            let nx = -dy / len * t / 2.0;
            let ny = dx / len * t / 2.0;
            verts.push(UIVertex { position: [x + x1 + nx, y + y1 + ny], tex_coords: [0.0, 0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x2 + nx, y + y2 + ny], tex_coords: [0.0, 0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x2 - nx, y + y2 - ny], tex_coords: [0.0, 0.0], color, use_texture: 0.0 });
            verts.push(UIVertex { position: [x + x1 - nx, y + y1 - ny], tex_coords: [0.0, 0.0], color, use_texture: 0.0 });
            inds.extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
            idx += 4;
        };

        match letter {
            'L' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, 0.0, w, 0.0); }
            'O' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, 0.0); add_line(0.0, 0.0, w, 0.0); }
            'A' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, 0.0); add_line(0.0, h / 2.0, w, h / 2.0); }
            'D' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w * 0.7, h); add_line(w * 0.7, h, w, h * 0.7); add_line(w, h * 0.7, w, h * 0.3); add_line(w, h * 0.3, w * 0.7, 0.0); add_line(w * 0.7, 0.0, 0.0, 0.0); }
            'I' => { add_line(w / 2.0, 0.0, w / 2.0, h); add_line(0.0, 0.0, w, 0.0); add_line(0.0, h, w, h); }
            'N' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, 0.0); add_line(w, 0.0, w, h); }
            'G' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, h / 2.0); add_line(w / 2.0, h / 2.0, w, h / 2.0); add_line(0.0, 0.0, w, 0.0); }
            'F' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(0.0, h / 2.0, w * 0.7, h / 2.0); }
            'P' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, h / 2.0); add_line(0.0, h / 2.0, w, h / 2.0); }
            'S' => { add_line(w, h, 0.0, h); add_line(0.0, h, 0.0, h / 2.0); add_line(0.0, h / 2.0, w, h / 2.0); add_line(w, h / 2.0, w, 0.0); add_line(0.0, 0.0, w, 0.0); }
            'X' => { add_line(0.0, 0.0, w, h); add_line(0.0, h, w, 0.0); }
            'Y' => { add_line(0.0, h, w / 2.0, h / 2.0); add_line(w, h, w / 2.0, h / 2.0); add_line(w / 2.0, h / 2.0, w / 2.0, 0.0); }
            'Z' => { add_line(0.0, h, w, h); add_line(w, h, 0.0, 0.0); add_line(0.0, 0.0, w, 0.0); }
            'C' => { add_line(w, h, 0.0, h); add_line(0.0, h, 0.0, 0.0); add_line(0.0, 0.0, w, 0.0); }
            'H' => { add_line(0.0, 0.0, 0.0, h); add_line(w, 0.0, w, h); add_line(0.0, h / 2.0, w, h / 2.0); }
            'U' => { add_line(0.0, h, 0.0, 0.0); add_line(0.0, 0.0, w, 0.0); add_line(w, 0.0, w, h); }
            'K' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h / 2.0, w, h); add_line(0.0, h / 2.0, w, 0.0); }
            'T' => { add_line(0.0, h, w, h); add_line(w / 2.0, h, w / 2.0, 0.0); }
            'R' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, h / 2.0); add_line(0.0, h / 2.0, w, h / 2.0); add_line(0.0, h / 2.0, w, 0.0); }
            'E' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(0.0, h / 2.0, w * 0.7, h / 2.0); add_line(0.0, 0.0, w, 0.0); }
            'M' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w / 2.0, h / 2.0); add_line(w / 2.0, h / 2.0, w, h); add_line(w, h, w, 0.0); }
            'W' => { add_line(0.0, h, 0.0, 0.0); add_line(0.0, 0.0, w / 2.0, h / 2.0); add_line(w / 2.0, h / 2.0, w, 0.0); add_line(w, 0.0, w, h); }
            'B' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, h / 2.0); add_line(0.0, h / 2.0, w, h / 2.0); add_line(w, h / 2.0, w, 0.0); add_line(0.0, 0.0, w, 0.0); }
            'Q' => { add_line(0.0, 0.0, 0.0, h); add_line(0.0, h, w, h); add_line(w, h, w, 0.0); add_line(0.0, 0.0, w, 0.0); add_line(w * 0.5, h * 0.3, w, 0.0); }
            'V' => { add_line(0.0, h, w / 2.0, 0.0); add_line(w / 2.0, 0.0, w, h); }
            ':' => { add_line(w / 2.0, h * 0.7, w / 2.0 + t, h * 0.7 + t); add_line(w / 2.0, h * 0.3, w / 2.0 + t, h * 0.3 + t); }
            '/' => { add_line(0.0, 0.0, w, h); }
            '+' => { add_line(w / 2.0, 0.0, w / 2.0, h); add_line(0.0, h / 2.0, w, h / 2.0); }
            '-' => { add_line(0.0, h / 2.0, w, h / 2.0); }
            '.' => { add_line(w / 2.0 - t, t, w / 2.0 + t, t); }
            '(' => { add_line(w * 0.7, h, w * 0.3, h * 0.5); add_line(w * 0.3, h * 0.5, w * 0.7, 0.0); }
            ')' => { add_line(w * 0.3, h, w * 0.7, h * 0.5); add_line(w * 0.7, h * 0.5, w * 0.3, 0.0); }
            '>' => { add_line(0.0, h, w, h / 2.0); add_line(w, h / 2.0, 0.0, 0.0); }
            '<' => { add_line(w, h, 0.0, h / 2.0); add_line(0.0, h / 2.0, w, 0.0); }
            ' ' => {}
            '%' => {
                // Small circles and diagonal
                add_line(0.0, h * 0.8, w * 0.2, h * 0.8); add_line(0.0, h * 0.8, 0.0, h); add_line(0.0, h, w * 0.2, h); add_line(w * 0.2, h, w * 0.2, h * 0.8);
                add_line(0.0, 0.0, w, h);
                add_line(w * 0.8, 0.0, w, 0.0); add_line(w * 0.8, 0.0, w * 0.8, h * 0.2); add_line(w * 0.8, h * 0.2, w, h * 0.2); add_line(w, h * 0.2, w, 0.0);
            }
            _ => {}
        }

        (verts, inds)
    }

    /// Render debug overlay with FPS, position, facing direction, etc.
    pub fn render_debug_overlay(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        texture_bind_group: &wgpu::BindGroup,
        fps: f32,
        position: cgmath::Point3<f32>,
        facing: &str,
        chunk_count: usize,
        particle_count: usize,
    ) {
        let mut vertices: Vec<UIVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        let text_color = [1.0, 1.0, 1.0, 1.0];
        let char_size = 0.025;
        let line_height = char_size * 1.4;
        let start_x = -0.95;
        let start_y = 0.92;
        let padding = 0.015;

        // Build debug lines
        let lines = vec![
            format!("FPS: {}", fps as i32),
            format!("XYZ: {:.1} / {:.1} / {:.1}", position.x, position.y, position.z),
            format!("Facing: {}", facing),
            format!("Chunks: {}", chunk_count),
            format!("Particles: {}", particle_count),
        ];

        // Calculate background size
        let max_line_len = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let bg_width = Self::text_width(&"X".repeat(max_line_len), char_size) + padding * 2.0;
        let bg_height = lines.len() as f32 * line_height + padding * 2.0;

        // Draw 9-slice panel background
        let (panel_verts, panel_inds) = Self::generate_nine_slice_panel(
            start_x - padding, start_y - bg_height,
            bg_width + padding, bg_height + padding,
            0.01, [0.1, 0.1, 0.15, 0.85], vertices.len() as u16
        );
        vertices.extend(panel_verts);
        indices.extend(panel_inds);

        // Draw text lines using bitmap font
        for (line_idx, line) in lines.iter().enumerate() {
            let text_y = start_y - (line_idx as f32 + 1.0) * line_height + char_size * 0.3;
            let (text_verts, text_inds) = Self::generate_text_vertices(
                line, start_x, text_y, char_size, text_color, vertices.len() as u16
            );
            vertices.extend(text_verts);
            indices.extend(text_inds);
        }

        // Create buffers and render
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Debug Overlay Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Debug Overlay Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Debug Overlay Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Debug Overlay Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(&self.ui_render_pipeline);
            render_pass.set_bind_group(0, texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Render pause menu overlay
    pub fn render_pause_menu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        texture_bind_group: &wgpu::BindGroup,
        selected_option: usize,
    ) {
        let mut vertices: Vec<UIVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        // Full screen dark overlay
        let overlay_color = [0.0, 0.0, 0.0, 0.6];
        let base = vertices.len() as u16;
        vertices.push(UIVertex { position: [-1.0, -1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [1.0, -1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [1.0, 1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [-1.0, 1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

        // Panel background
        let panel_width = 0.5;
        let panel_height = 0.55;
        let panel_x = -panel_width / 2.0;
        let panel_y = -0.15;
        let panel_border = 0.02;

        let (panel_verts, panel_inds) = Self::generate_nine_slice_panel(
            panel_x, panel_y, panel_width, panel_height,
            panel_border, [1.0, 1.0, 1.0, 0.95], vertices.len() as u16
        );
        vertices.extend(panel_verts);
        indices.extend(panel_inds);

        // Title "Game Paused" using bitmap font
        let (title_verts, title_inds) = Self::generate_centered_text(
            "Game Paused", 0.0, 0.28, 0.06, [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
        );
        vertices.extend(title_verts);
        indices.extend(title_inds);

        // Menu options using bitmap font
        let options = ["Resume", "Options", "Quit"];
        let option_size = 0.04;
        let option_line_height = 0.1;
        let option_color = [0.85, 0.85, 0.85, 1.0];
        let selected_color = [1.0, 1.0, 0.5, 1.0];

        for (idx, option) in options.iter().enumerate() {
            let color = if idx == selected_option { selected_color } else { option_color };
            let option_y = 0.1 - idx as f32 * option_line_height;

            // Selection indicator
            if idx == selected_option {
                let (arrow_verts, arrow_inds) = Self::generate_text_vertices(
                    ">", -0.15, option_y, option_size, color, vertices.len() as u16
                );
                vertices.extend(arrow_verts);
                indices.extend(arrow_inds);
            }

            let (opt_verts, opt_inds) = Self::generate_centered_text(
                option, 0.0, option_y, option_size, color, vertices.len() as u16
            );
            vertices.extend(opt_verts);
            indices.extend(opt_inds);
        }

        // Create buffers and render
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Pause Menu Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Pause Menu Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Pause Menu Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pause Menu Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(&self.ui_render_pipeline);
            render_pass.set_bind_group(0, texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Render chest UI overlay
    pub fn render_chest_ui(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        texture_bind_group: &wgpu::BindGroup,
        chest_ui: &ChestUI,
        chest_contents: &[Option<(BlockType, u32)>; 9],
        inventory: &Inventory,
    ) {
        let mut vertices: Vec<UIVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        // Full screen dark overlay
        let overlay_color = [0.0, 0.0, 0.0, 0.6];
        let base = vertices.len() as u16;
        vertices.push(UIVertex { position: [-1.0, -1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [1.0, -1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [1.0, 1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        vertices.push(UIVertex { position: [-1.0, 1.0], tex_coords: [0.0, 0.0], color: overlay_color, use_texture: 0.0 });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

        // Layout constants
        let slot_size = 0.055;
        let slot_spacing = 0.075;
        let num_slots = 9;
        let panel_border = 0.015;
        let row_width = num_slots as f32 * slot_spacing;

        // Main container panel
        let panel_width = row_width + 0.08;
        let panel_height = 0.65;
        let panel_x = -panel_width / 2.0;
        let panel_y = -0.35;

        let (panel_verts, panel_inds) = Self::generate_nine_slice_panel(
            panel_x, panel_y, panel_width, panel_height,
            panel_border, [1.0, 1.0, 1.0, 0.95], vertices.len() as u16
        );
        vertices.extend(panel_verts);
        indices.extend(panel_inds);

        // Title "Chest" using bitmap font
        let (title_verts, title_inds) = Self::generate_centered_text(
            "Chest", 0.0, 0.22, 0.05, [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
        );
        vertices.extend(title_verts);
        indices.extend(title_inds);

        // Chest section
        let chest_row_y = 0.1;
        let row_start_x = -(num_slots as f32 * slot_spacing) / 2.0 + slot_spacing / 2.0;

        for i in 0..num_slots {
            let slot_x = row_start_x + i as f32 * slot_spacing;
            let is_selected = chest_ui.in_chest_section && chest_ui.selected_slot == i;

            // Slot background from atlas
            let slot_type = if is_selected { 1 } else { 0 };
            let (slot_verts, slot_inds) = Self::generate_slot_vertices(
                slot_x, chest_row_y, slot_size, slot_type,
                [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
            );
            vertices.extend(slot_verts);
            indices.extend(slot_inds);

            // Draw item if present
            if let Some((block_type, qty)) = chest_contents[i] {
                let block_type_val = Self::block_type_to_ui_index(block_type);
                let icon_size = slot_size * 0.7;
                let icon_base = vertices.len() as u16;
                vertices.push(UIVertex { position: [slot_x - icon_size, chest_row_y - icon_size], tex_coords: [0.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x + icon_size, chest_row_y - icon_size], tex_coords: [1.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x + icon_size, chest_row_y + icon_size], tex_coords: [1.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x - icon_size, chest_row_y + icon_size], tex_coords: [0.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                indices.extend_from_slice(&[icon_base, icon_base + 1, icon_base + 2, icon_base, icon_base + 2, icon_base + 3]);

                // Quantity display with shadow (Minecraft style) - bottom right corner
                if qty > 1 {
                    let qty_str = qty.to_string();
                    let text_scale = slot_size * 0.6;
                    // Position at bottom-right, offset from center
                    let text_x = slot_x - slot_size * 0.1;
                    let text_y = chest_row_y - slot_size * 0.75;
                    let (qty_verts, qty_inds) = Self::generate_text_with_shadow(
                        &qty_str, text_x, text_y, text_scale,
                        [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
                    );
                    vertices.extend(qty_verts);
                    indices.extend(qty_inds);
                }
            }
        }

        // "Inventory" label using bitmap font
        let (inv_label_verts, inv_label_inds) = Self::generate_centered_text(
            "Inventory", 0.0, -0.05, 0.035, [0.9, 0.9, 0.9, 1.0], vertices.len() as u16
        );
        vertices.extend(inv_label_verts);
        indices.extend(inv_label_inds);

        // Player inventory section (9 slots)
        let inv_row_y = -0.15;

        for i in 0..num_slots {
            let slot_x = row_start_x + i as f32 * slot_spacing;
            let is_selected = !chest_ui.in_chest_section && chest_ui.selected_slot == i;

            // Slot background from atlas
            let slot_type = if is_selected { 1 } else { 0 };
            let (slot_verts, slot_inds) = Self::generate_slot_vertices(
                slot_x, inv_row_y, slot_size, slot_type,
                [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
            );
            vertices.extend(slot_verts);
            indices.extend(slot_inds);

            // Draw item if present
            if let Some((block_type, qty)) = inventory.slots[i] {
                let block_type_val = Self::block_type_to_ui_index(block_type);
                let icon_size = slot_size * 0.7;
                let icon_base = vertices.len() as u16;
                vertices.push(UIVertex { position: [slot_x - icon_size, inv_row_y - icon_size], tex_coords: [0.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x + icon_size, inv_row_y - icon_size], tex_coords: [1.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x + icon_size, inv_row_y + icon_size], tex_coords: [1.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                vertices.push(UIVertex { position: [slot_x - icon_size, inv_row_y + icon_size], tex_coords: [0.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], use_texture: block_type_val });
                indices.extend_from_slice(&[icon_base, icon_base + 1, icon_base + 2, icon_base, icon_base + 2, icon_base + 3]);

                // Quantity display with shadow (Minecraft style) - bottom right corner
                if qty > 1 {
                    let qty_str = qty.to_string();
                    let text_scale = slot_size * 0.6;
                    // Position at bottom-right, offset from center
                    let text_x = slot_x - slot_size * 0.1;
                    let text_y = inv_row_y - slot_size * 0.75;
                    let (qty_verts, qty_inds) = Self::generate_text_with_shadow(
                        &qty_str, text_x, text_y, text_scale,
                        [1.0, 1.0, 1.0, 1.0], vertices.len() as u16
                    );
                    vertices.extend(qty_verts);
                    indices.extend(qty_inds);
                }
            }
        }

        // Instructions at bottom using bitmap font
        let (inst_verts, inst_inds) = Self::generate_centered_text(
            "Arrows: Move  Enter: Transfer  Esc: Close",
            0.0, -0.28, 0.022, [0.7, 0.7, 0.7, 1.0], vertices.len() as u16
        );
        vertices.extend(inst_verts);
        indices.extend(inst_inds);

        // Create buffers and render
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Chest UI Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Chest UI Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Chest UI Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Chest UI Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            render_pass.set_pipeline(&self.ui_render_pipeline);
            render_pass.set_bind_group(0, texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}