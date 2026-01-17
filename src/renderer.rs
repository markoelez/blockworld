use wgpu::util::DeviceExt;
use winit::window::Window;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use image::GenericImageView;
use cgmath::SquareMatrix;
use std::time::Instant;
use cgmath::{Matrix4, Deg, Vector3, Vector4, InnerSpace, Matrix, Point3};
use rayon::prelude::*;
use rand::Rng;

use crate::camera::Camera;
use crate::world::{World, BlockType, TorchFace, ItemStack, Tool, ToolType, ToolMaterial};
use crate::ui::{Inventory, UIRenderer, DebugInfo, PauseMenu, ChestUI, CraftingUI, RecipeRegistry};
use crate::entity::{EntityManager, Villager, VillagerState, VILLAGER_HEIGHT};
use crate::particle::{ParticleSystem, LightningSystem, WeatherState, WeatherType};

// Shadow map resolution
const SHADOW_MAP_SIZE: u32 = 2048;
// HDR format for main render target
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
    normal: [f32; 3],
    block_type: f32,
    damage: f32,  // 0.0 to 1.0 normalized damage
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct SkyVertex {
    position: [f32; 3],
    brightness: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CloudVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ParticleVertex {
    position: [f32; 3],
    offset: [f32; 2],
    color: [f32; 4],
    size: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ParticleUniform {
    view_proj: [[f32; 4]; 4],
    camera_right: [f32; 3],
    _pad1: f32,
    camera_up: [f32; 3],
    _pad2: f32,
}

struct ChunkMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

// Frustum plane representation: ax + by + cz + d = 0
#[derive(Copy, Clone)]
struct Plane {
    normal: Vector3<f32>,
    d: f32,
}

impl Plane {
    fn distance_to_point(&self, point: Vector3<f32>) -> f32 {
        self.normal.dot(point) + self.d
    }
}

// View frustum for culling
struct Frustum {
    planes: [Plane; 6], // Left, Right, Bottom, Top, Near, Far
}

impl Frustum {
    // Extract frustum planes from view-projection matrix
    fn from_view_proj(vp: &Matrix4<f32>) -> Self {
        let m = vp;

        // Extract planes from columns of the transposed VP matrix
        let left = Plane {
            normal: Vector3::new(m[0][3] + m[0][0], m[1][3] + m[1][0], m[2][3] + m[2][0]),
            d: m[3][3] + m[3][0],
        };
        let right = Plane {
            normal: Vector3::new(m[0][3] - m[0][0], m[1][3] - m[1][0], m[2][3] - m[2][0]),
            d: m[3][3] - m[3][0],
        };
        let bottom = Plane {
            normal: Vector3::new(m[0][3] + m[0][1], m[1][3] + m[1][1], m[2][3] + m[2][1]),
            d: m[3][3] + m[3][1],
        };
        let top = Plane {
            normal: Vector3::new(m[0][3] - m[0][1], m[1][3] - m[1][1], m[2][3] - m[2][1]),
            d: m[3][3] - m[3][1],
        };
        let near = Plane {
            normal: Vector3::new(m[0][3] + m[0][2], m[1][3] + m[1][2], m[2][3] + m[2][2]),
            d: m[3][3] + m[3][2],
        };
        let far = Plane {
            normal: Vector3::new(m[0][3] - m[0][2], m[1][3] - m[1][2], m[2][3] - m[2][2]),
            d: m[3][3] - m[3][2],
        };

        // Normalize planes
        let normalize = |p: Plane| -> Plane {
            let len = p.normal.magnitude();
            Plane {
                normal: p.normal / len,
                d: p.d / len,
            }
        };

        Frustum {
            planes: [
                normalize(left),
                normalize(right),
                normalize(bottom),
                normalize(top),
                normalize(near),
                normalize(far),
            ],
        }
    }

    // Check if an AABB intersects the frustum
    fn intersects_aabb(&self, min: Vector3<f32>, max: Vector3<f32>) -> bool {
        for plane in &self.planes {
            // Find the corner of the AABB that's most in the direction of the plane normal
            let p = Vector3::new(
                if plane.normal.x > 0.0 { max.x } else { min.x },
                if plane.normal.y > 0.0 { max.y } else { min.y },
                if plane.normal.z > 0.0 { max.z } else { min.z },
            );

            // If this corner is behind the plane, the AABB is outside
            if plane.distance_to_point(p) < 0.0 {
                return false;
            }
        }
        true
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inverse_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    time_of_day: f32,
    sun_direction: [f32; 3],
    ambient_intensity: f32,
    light_view_proj: [[f32; 4]; 4],
    sun_color: [f32; 3],
    fog_density: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ShadowUniform {
    light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct PostProcessUniform {
    exposure: f32,
    bloom_intensity: f32,
    saturation: f32,
    contrast: f32,
    sun_screen_pos: [f32; 2],  // Sun position in screen space for god rays
    god_ray_intensity: f32,
    god_ray_decay: f32,
    screen_size: [f32; 2],    // Screen dimensions for SSAO
    ssao_intensity: f32,
    ssao_radius: f32,
    underwater: f32,          // 1.0 if underwater, 0.0 otherwise
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct BloomUniform {
    threshold: f32,
    soft_threshold: f32,
    blur_direction: [f32; 2],
    texel_size: [f32; 2],
    _padding: [f32; 2],
}

const MAX_POINT_LIGHTS: usize = 32;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct PointLight {
    position: [f32; 3],
    radius: f32,
    color: [f32; 3],
    intensity: f32,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            radius: 0.0,
            color: [0.0, 0.0, 0.0],
            intensity: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct LightingUniform {
    point_lights: [PointLight; MAX_POINT_LIGHTS],
    num_lights: u32,
    _padding: [u32; 3],
}

pub struct Renderer {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    cloud_pipeline: wgpu::RenderPipeline,
    held_item_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
    depth_texture: wgpu::TextureView,
    ui_renderer: UIRenderer,
    outline_vertex_buffer: wgpu::Buffer,
    outline_index_buffer: wgpu::Buffer,
    sky_vertex_buffer: wgpu::Buffer,
    sky_index_buffer: wgpu::Buffer,
    sky_vertex_count: u32,
    cloud_vertex_buffer: wgpu::Buffer,
    cloud_index_buffer: wgpu::Buffer,
    cloud_index_count: u32,
    transparent_pipeline: wgpu::RenderPipeline,
    chunk_meshes_opaque: HashMap<(i32, i32), ChunkMesh>,
    chunk_meshes_transparent: HashMap<(i32, i32), ChunkMesh>,
    arm_swing_progress: f32,
    held_item_index_count: u32,
    last_render: Instant,
    held_item_vertex_buffer: wgpu::Buffer,
    held_item_index_buffer: wgpu::Buffer,
    // HDR rendering
    hdr_texture: wgpu::Texture,
    hdr_texture_view: wgpu::TextureView,
    hdr_depth_texture: wgpu::TextureView,
    // Shadow mapping
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_texture: wgpu::Texture,
    shadow_texture_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    shadow_texture_bind_group: wgpu::BindGroup,
    // Post-processing
    post_process_pipeline: wgpu::RenderPipeline,
    post_process_bind_group: wgpu::BindGroup,
    post_process_uniform_buffer: wgpu::Buffer,
    // Bloom
    bloom_extract_pipeline: wgpu::RenderPipeline,
    bloom_blur_pipeline: wgpu::RenderPipeline,
    bloom_textures: [wgpu::Texture; 2],  // Ping-pong buffers
    bloom_texture_views: [wgpu::TextureView; 2],
    bloom_bind_groups: [wgpu::BindGroup; 3],  // Extract, blur H, blur V
    bloom_uniform_buffer: wgpu::Buffer,
    // Time tracking
    start_time: Instant,
    time_offset: f32,  // Random starting time offset (0.0-1.0)
    time_of_day: f32,
    // Villager rendering
    villager_vertex_buffer: wgpu::Buffer,
    villager_index_buffer: wgpu::Buffer,
    villager_index_count: u32,
    // Particle rendering
    particle_pipeline: wgpu::RenderPipeline,
    particle_vertex_buffer: wgpu::Buffer,
    particle_uniform_buffer: wgpu::Buffer,
    particle_bind_group: wgpu::BindGroup,
    particle_vertex_count: u32,
    // Point lighting
    lighting_buffer: wgpu::Buffer,
    lighting_bind_group: wgpu::BindGroup,
    // Block preview (ghost block)
    preview_vertex_buffer: wgpu::Buffer,
    preview_index_buffer: wgpu::Buffer,
    preview_index_count: u32,
    preview_visible: bool,
    // Dropped item rendering
    dropped_item_vertex_buffer: wgpu::Buffer,
    dropped_item_index_buffer: wgpu::Buffer,
    dropped_item_index_count: u32,
    // Animal rendering
    animal_vertex_buffer: wgpu::Buffer,
    animal_index_buffer: wgpu::Buffer,
    animal_index_count: u32,
    // Hostile mob rendering
    hostile_mob_vertex_buffer: wgpu::Buffer,
    hostile_mob_index_buffer: wgpu::Buffer,
    hostile_mob_index_count: u32,
    // Plane rendering
    plane_vertex_buffer: wgpu::Buffer,
    plane_index_buffer: wgpu::Buffer,
    plane_index_count: u32,
    // Missile rendering
    missile_vertex_buffer: wgpu::Buffer,
    missile_index_buffer: wgpu::Buffer,
    missile_index_count: u32,
    // Bomb rendering
    bomb_vertex_buffer: wgpu::Buffer,
    bomb_index_buffer: wgpu::Buffer,
    bomb_index_count: u32,
}

impl Renderer {
    fn load_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &str,
    ) -> Result<wgpu::Texture, Box<dyn std::error::Error>> {
        let img = image::open(path)?;
        let rgba = img.to_rgba8();
        let dimensions = img.dimensions();
        
        let size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };
        
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(path),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            size,
        );
        
        Ok(texture)
    }

    fn create_fallback_texture(device: &wgpu::Device, queue: &wgpu::Queue, color: [u8; 4]) -> wgpu::Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Fallback Texture"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        // Create a 16x16 solid color texture
        let data = vec![color; 16 * 16];
        
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * 16),
                rows_per_image: Some(16),
            },
            wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        
        texture
    }

    /// Generate a 256x256 UI atlas texture with:
    /// - Rows 0-3 (Y=0-63): UI elements (slots, 9-slice panels) in 16x16 cells
    /// - Rows 4-15 (Y=64-255): Bitmap font (8x8 glyphs in 16x16 cells)
    fn generate_ui_atlas(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        const ATLAS_SIZE: usize = 256;
        let mut data = vec![[0u8, 0, 0, 0]; ATLAS_SIZE * ATLAS_SIZE];

        // Helper to set a pixel
        let set_pixel = |data: &mut Vec<[u8; 4]>, x: usize, y: usize, color: [u8; 4]| {
            if x < ATLAS_SIZE && y < ATLAS_SIZE {
                data[y * ATLAS_SIZE + x] = color;
            }
        };

        // Colors
        let slot_bg = [40u8, 40, 48, 255];           // Dark gray slot background
        let slot_border = [60u8, 60, 70, 255];       // Slot border
        let slot_inner = [30u8, 30, 35, 255];        // Inner slot darker
        let selected_border = [255u8, 255, 200, 255]; // Bright yellow selection
        let selected_glow = [255u8, 255, 180, 100];  // Subtle glow
        let panel_dark = [25u8, 25, 30, 240];        // Panel background
        let panel_border = [70u8, 70, 80, 255];      // Panel border
        let panel_highlight = [90u8, 90, 100, 255];  // Panel highlight edge

        // === SLOT TEXTURES (Row 0) ===
        // [0,0] Empty slot - 16x16
        for y in 0..16 {
            for x in 0..16 {
                let is_border = x == 0 || x == 15 || y == 0 || y == 15;
                let is_inner_border = x == 1 || x == 14 || y == 1 || y == 14;
                let color = if is_border {
                    slot_border
                } else if is_inner_border {
                    slot_inner
                } else {
                    slot_bg
                };
                set_pixel(&mut data, x, y, color);
            }
        }

        // [1,0] Selected slot - 16x16 (at x=16)
        for y in 0..16 {
            for x in 0..16 {
                let is_outer = x == 0 || x == 15 || y == 0 || y == 15;
                let is_border = x == 1 || x == 14 || y == 1 || y == 14;
                let color = if is_outer {
                    selected_glow
                } else if is_border {
                    selected_border
                } else {
                    slot_bg
                };
                set_pixel(&mut data, x + 16, y, color);
            }
        }

        // [2,0] Hovered slot - 16x16 (at x=32)
        for y in 0..16 {
            for x in 0..16 {
                let is_border = x == 0 || x == 15 || y == 0 || y == 15;
                let color = if is_border {
                    [100u8, 100, 120, 255]
                } else {
                    [50u8, 50, 60, 255]
                };
                set_pixel(&mut data, x + 32, y, color);
            }
        }

        // === 9-SLICE PANEL PIECES (Row 1, Y=16-31) ===
        // Layout: [TL][T][TR][L][C][R][BL][B][BR] - each 16x16
        // Top-left corner [0,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x < 2 || y < 2 {
                    panel_highlight
                } else if x < 3 || y < 3 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x, y + 16, color);
            }
        }

        // Top edge [1,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if y < 2 {
                    panel_highlight
                } else if y < 3 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 16, y + 16, color);
            }
        }

        // Top-right corner [2,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x > 13 || y < 2 {
                    if y < 2 { panel_highlight } else { panel_border }
                } else if x > 12 || y < 3 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 32, y + 16, color);
            }
        }

        // Left edge [3,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x < 2 {
                    panel_highlight
                } else if x < 3 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 48, y + 16, color);
            }
        }

        // Center [4,1]
        for y in 0..16 {
            for x in 0..16 {
                set_pixel(&mut data, x + 64, y + 16, panel_dark);
            }
        }

        // Right edge [5,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x > 13 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 80, y + 16, color);
            }
        }

        // Bottom-left corner [6,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x < 2 {
                    panel_highlight
                } else if x < 3 || y > 12 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 96, y + 16, color);
            }
        }

        // Bottom edge [7,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if y > 13 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 112, y + 16, color);
            }
        }

        // Bottom-right corner [8,1]
        for y in 0..16 {
            for x in 0..16 {
                let color = if x > 13 || y > 13 {
                    panel_border
                } else {
                    panel_dark
                };
                set_pixel(&mut data, x + 128, y + 16, color);
            }
        }

        // === BITMAP FONT (Rows 4-15, Y=64-255) ===
        // 8x8 pixel glyphs in 16x16 cells, ASCII 32-127
        // Using a simple pixel font definition
        let font_data: std::collections::HashMap<char, [u8; 8]> = [
            // Space
            (' ', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ('!', [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00]),
            ('"', [0x6C, 0x6C, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ('#', [0x24, 0x7E, 0x24, 0x24, 0x7E, 0x24, 0x00, 0x00]),
            ('$', [0x10, 0x3C, 0x50, 0x38, 0x14, 0x78, 0x10, 0x00]),
            ('%', [0x62, 0x64, 0x08, 0x10, 0x26, 0x46, 0x00, 0x00]),
            ('&', [0x30, 0x48, 0x30, 0x56, 0x88, 0x88, 0x76, 0x00]),
            ('\'', [0x18, 0x18, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ('(', [0x08, 0x10, 0x20, 0x20, 0x20, 0x10, 0x08, 0x00]),
            (')', [0x20, 0x10, 0x08, 0x08, 0x08, 0x10, 0x20, 0x00]),
            ('*', [0x00, 0x24, 0x18, 0x7E, 0x18, 0x24, 0x00, 0x00]),
            ('+', [0x00, 0x10, 0x10, 0x7C, 0x10, 0x10, 0x00, 0x00]),
            (',', [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x10]),
            ('-', [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00]),
            ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00]),
            ('/', [0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x00]),
            ('0', [0x3C, 0x46, 0x4A, 0x52, 0x62, 0x3C, 0x00, 0x00]),
            ('1', [0x18, 0x38, 0x18, 0x18, 0x18, 0x7E, 0x00, 0x00]),
            ('2', [0x3C, 0x42, 0x02, 0x1C, 0x20, 0x7E, 0x00, 0x00]),
            ('3', [0x3C, 0x42, 0x0C, 0x02, 0x42, 0x3C, 0x00, 0x00]),
            ('4', [0x08, 0x18, 0x28, 0x48, 0x7E, 0x08, 0x00, 0x00]),
            ('5', [0x7E, 0x40, 0x7C, 0x02, 0x42, 0x3C, 0x00, 0x00]),
            ('6', [0x1C, 0x20, 0x7C, 0x42, 0x42, 0x3C, 0x00, 0x00]),
            ('7', [0x7E, 0x02, 0x04, 0x08, 0x10, 0x10, 0x00, 0x00]),
            ('8', [0x3C, 0x42, 0x3C, 0x42, 0x42, 0x3C, 0x00, 0x00]),
            ('9', [0x3C, 0x42, 0x42, 0x3E, 0x04, 0x38, 0x00, 0x00]),
            (':', [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00]),
            (';', [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x10, 0x00]),
            ('<', [0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x00]),
            ('=', [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00]),
            ('>', [0x20, 0x10, 0x08, 0x04, 0x08, 0x10, 0x20, 0x00]),
            ('?', [0x3C, 0x42, 0x04, 0x08, 0x00, 0x08, 0x00, 0x00]),
            ('@', [0x3C, 0x42, 0x5E, 0x52, 0x5E, 0x40, 0x3C, 0x00]),
            ('A', [0x18, 0x24, 0x42, 0x7E, 0x42, 0x42, 0x00, 0x00]),
            ('B', [0x7C, 0x42, 0x7C, 0x42, 0x42, 0x7C, 0x00, 0x00]),
            ('C', [0x3C, 0x42, 0x40, 0x40, 0x42, 0x3C, 0x00, 0x00]),
            ('D', [0x78, 0x44, 0x42, 0x42, 0x44, 0x78, 0x00, 0x00]),
            ('E', [0x7E, 0x40, 0x7C, 0x40, 0x40, 0x7E, 0x00, 0x00]),
            ('F', [0x7E, 0x40, 0x7C, 0x40, 0x40, 0x40, 0x00, 0x00]),
            ('G', [0x3C, 0x42, 0x40, 0x4E, 0x42, 0x3C, 0x00, 0x00]),
            ('H', [0x42, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x00, 0x00]),
            ('I', [0x7E, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00, 0x00]),
            ('J', [0x1E, 0x04, 0x04, 0x04, 0x44, 0x38, 0x00, 0x00]),
            ('K', [0x42, 0x44, 0x78, 0x48, 0x44, 0x42, 0x00, 0x00]),
            ('L', [0x40, 0x40, 0x40, 0x40, 0x40, 0x7E, 0x00, 0x00]),
            ('M', [0x42, 0x66, 0x5A, 0x42, 0x42, 0x42, 0x00, 0x00]),
            ('N', [0x42, 0x62, 0x52, 0x4A, 0x46, 0x42, 0x00, 0x00]),
            ('O', [0x3C, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00]),
            ('P', [0x7C, 0x42, 0x42, 0x7C, 0x40, 0x40, 0x00, 0x00]),
            ('Q', [0x3C, 0x42, 0x42, 0x4A, 0x44, 0x3A, 0x00, 0x00]),
            ('R', [0x7C, 0x42, 0x42, 0x7C, 0x44, 0x42, 0x00, 0x00]),
            ('S', [0x3C, 0x40, 0x3C, 0x02, 0x42, 0x3C, 0x00, 0x00]),
            ('T', [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00]),
            ('U', [0x42, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00]),
            ('V', [0x42, 0x42, 0x42, 0x42, 0x24, 0x18, 0x00, 0x00]),
            ('W', [0x42, 0x42, 0x42, 0x5A, 0x66, 0x42, 0x00, 0x00]),
            ('X', [0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x00, 0x00]),
            ('Y', [0x42, 0x42, 0x24, 0x18, 0x18, 0x18, 0x00, 0x00]),
            ('Z', [0x7E, 0x04, 0x08, 0x10, 0x20, 0x7E, 0x00, 0x00]),
            ('[', [0x3C, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00, 0x00]),
            ('\\', [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x00]),
            (']', [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00, 0x00]),
            ('^', [0x10, 0x28, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ('_', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00]),
            ('`', [0x30, 0x30, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ('a', [0x00, 0x00, 0x3C, 0x02, 0x3E, 0x42, 0x3E, 0x00]),
            ('b', [0x40, 0x40, 0x7C, 0x42, 0x42, 0x7C, 0x00, 0x00]),
            ('c', [0x00, 0x00, 0x3C, 0x40, 0x40, 0x3C, 0x00, 0x00]),
            ('d', [0x02, 0x02, 0x3E, 0x42, 0x42, 0x3E, 0x00, 0x00]),
            ('e', [0x00, 0x00, 0x3C, 0x42, 0x7E, 0x40, 0x3C, 0x00]),
            ('f', [0x0C, 0x10, 0x3C, 0x10, 0x10, 0x10, 0x00, 0x00]),
            ('g', [0x00, 0x00, 0x3E, 0x42, 0x3E, 0x02, 0x3C, 0x00]),
            ('h', [0x40, 0x40, 0x7C, 0x42, 0x42, 0x42, 0x00, 0x00]),
            ('i', [0x18, 0x00, 0x38, 0x18, 0x18, 0x3C, 0x00, 0x00]),
            ('j', [0x04, 0x00, 0x0C, 0x04, 0x04, 0x44, 0x38, 0x00]),
            ('k', [0x40, 0x40, 0x44, 0x78, 0x48, 0x44, 0x00, 0x00]),
            ('l', [0x38, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00, 0x00]),
            ('m', [0x00, 0x00, 0x64, 0x5A, 0x42, 0x42, 0x00, 0x00]),
            ('n', [0x00, 0x00, 0x7C, 0x42, 0x42, 0x42, 0x00, 0x00]),
            ('o', [0x00, 0x00, 0x3C, 0x42, 0x42, 0x3C, 0x00, 0x00]),
            ('p', [0x00, 0x00, 0x7C, 0x42, 0x7C, 0x40, 0x40, 0x00]),
            ('q', [0x00, 0x00, 0x3E, 0x42, 0x3E, 0x02, 0x02, 0x00]),
            ('r', [0x00, 0x00, 0x5C, 0x60, 0x40, 0x40, 0x00, 0x00]),
            ('s', [0x00, 0x00, 0x3E, 0x40, 0x3C, 0x02, 0x7C, 0x00]),
            ('t', [0x10, 0x10, 0x3C, 0x10, 0x10, 0x0C, 0x00, 0x00]),
            ('u', [0x00, 0x00, 0x42, 0x42, 0x42, 0x3E, 0x00, 0x00]),
            ('v', [0x00, 0x00, 0x42, 0x42, 0x24, 0x18, 0x00, 0x00]),
            ('w', [0x00, 0x00, 0x42, 0x42, 0x5A, 0x24, 0x00, 0x00]),
            ('x', [0x00, 0x00, 0x42, 0x24, 0x18, 0x24, 0x42, 0x00]),
            ('y', [0x00, 0x00, 0x42, 0x42, 0x3E, 0x02, 0x3C, 0x00]),
            ('z', [0x00, 0x00, 0x7E, 0x04, 0x18, 0x20, 0x7E, 0x00]),
            ('{', [0x0C, 0x10, 0x10, 0x20, 0x10, 0x10, 0x0C, 0x00]),
            ('|', [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00]),
            ('}', [0x30, 0x08, 0x08, 0x04, 0x08, 0x08, 0x30, 0x00]),
            ('~', [0x00, 0x32, 0x4C, 0x00, 0x00, 0x00, 0x00, 0x00]),
        ].iter().cloned().collect();

        // Render font glyphs starting at Y=64
        let font_color = [255u8, 255, 255, 255]; // White font
        for ascii in 32u8..=126u8 {
            let c = ascii as char;
            let glyph_index = (ascii - 32) as usize;
            let cell_x = (glyph_index % 16) * 16;
            let cell_y = 64 + (glyph_index / 16) * 16;

            if let Some(glyph_data) = font_data.get(&c) {
                // Center 8x8 glyph in 16x16 cell (offset by 4,4)
                for row in 0..8 {
                    let row_data = glyph_data[row];
                    for col in 0..8 {
                        if (row_data >> (7 - col)) & 1 == 1 {
                            let px = cell_x + 4 + col;
                            let py = cell_y + 4 + row;
                            set_pixel(&mut data, px, py, font_color);
                        }
                    }
                }
            }
        }

        // Create texture
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Atlas Texture"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE as u32,
                height: ATLAS_SIZE as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * ATLAS_SIZE as u32),
                rows_per_image: Some(ATLAS_SIZE as u32),
            },
            wgpu::Extent3d {
                width: ATLAS_SIZE as u32,
                height: ATLAS_SIZE as u32,
                depth_or_array_layers: 1,
            },
        );

        texture
    }

    pub async fn new(window: &Window) -> Self {
        let size = window.inner_size();
        
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        
        let surface = unsafe { instance.create_surface(window) }.unwrap();
        
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();
        
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    features: wgpu::Features::empty(),
                    limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .unwrap();
        
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        
        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Outline Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("outline_shader.wgsl").into()),
        });
        
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sky Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky_shader.wgsl").into()),
        });
        
        let cloud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cloud Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cloud_shader.wgsl").into()),
        });
        
        let held_item_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Held Item Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("held_item_shader.wgsl").into()),
        });

        // New shaders for realistic rendering
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shadow Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow_shader.wgsl").into()),
        });

        let post_process_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post Process Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_process_shader.wgsl").into()),
        });

        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Bloom Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bloom_shader.wgsl").into()),
        });

        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("particle_shader.wgsl").into()),
        });

        let uniform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("uniform_bind_group_layout"),
        });
        
        // Create texture bind group layout
        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // UI Atlas texture
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // UI Atlas sampler (nearest-neighbor for crisp pixels)
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
            label: Some("texture_bind_group_layout"),
        });
        
        // Shadow map bind group layout
        let shadow_uniform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("shadow_uniform_bind_group_layout"),
        });

        // Shadow texture bind group layout (for sampling in main pass)
        let shadow_texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
            label: Some("shadow_texture_bind_group_layout"),
        });

        // Lighting bind group layout (for point lights)
        let lighting_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("lighting_bind_group_layout"),
        });

        // Post-process bind group layout
        let post_process_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Depth texture for SSAO (non-filtering)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                // Non-filtering sampler for depth
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
            label: Some("post_process_bind_group_layout"),
        });

        // Bloom bind group layout
        let bloom_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("bloom_bind_group_layout"),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout, &shadow_texture_bind_group_layout, &lighting_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shadow pipeline layout
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow Pipeline Layout"),
            bind_group_layouts: &[&shadow_uniform_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Post-process pipeline layout
        let post_process_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Post Process Pipeline Layout"),
            bind_group_layouts: &[&post_process_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Bloom pipeline layout
        let bloom_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom Pipeline Layout"),
            bind_group_layouts: &[&bloom_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Sky and cloud pipeline layout (only uniform, no textures)
        let simple_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Simple Pipeline Layout"),
            bind_group_layouts: &[&uniform_bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        let transparent_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Transparent Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        // Create outline render pipeline
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Outline Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList, // Use triangles for thicker lines
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: -1,
                    slope_scale: -1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        // Create sky render pipeline  
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Sky Render Pipeline"),
            layout: Some(&simple_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SkyVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, // Stars are far away
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        // Create cloud render pipeline
        let cloud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cloud Render Pipeline"),
            layout: Some(&simple_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cloud_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CloudVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &cloud_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, // Clouds don't write depth
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        // Create held item render pipeline
        let held_item_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Held Item Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &held_item_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &held_item_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        // Shadow pipeline (depth-only)
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shadow Render Pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress, shader_location: 3, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, shader_location: 4, format: wgpu::VertexFormat::Float32 },
                    ],
                }],
            },
            fragment: None,  // Depth-only, no fragment shader output
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Post-process pipeline (renders to swapchain)
        let post_process_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Post Process Pipeline"),
            layout: Some(&post_process_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &post_process_shader,
                entry_point: "vs_main",
                buffers: &[],  // Full-screen triangle generated in shader
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_process_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,  // Swapchain format
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Bloom extract pipeline
        let bloom_extract_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bloom Extract Pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bloom_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &bloom_shader,
                entry_point: "fs_extract",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Bloom blur pipeline
        let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bloom Blur Pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bloom_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &bloom_shader,
                entry_point: "fs_blur",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Create outline buffers with proper size for triangulated edges
        let outline_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline Vertex Buffer"),
            size: (48 * std::mem::size_of::<[f32; 3]>()) as u64, // 12 edges * 4 vertices per edge
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let outline_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline Index Buffer"),
            size: (72 * std::mem::size_of::<u16>()) as u64, // 12 edges * 6 indices per edge (2 triangles)
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        // Chunk meshes will be created on demand
        
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("uniform_bind_group"),
        });
        
        // Load textures
        let grass_texture = Self::load_texture(&device, &queue, "src/textures/grass.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [85, 140, 85, 255]));
        let grass_top_texture = Self::load_texture(&device, &queue, "src/textures/grass_top.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [85, 140, 85, 255]));
        let dirt_texture = Self::load_texture(&device, &queue, "src/textures/dirt.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [133, 94, 66, 255]));
        let stone_texture = Self::load_texture(&device, &queue, "src/textures/stone.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [128, 128, 128, 255]));
        let wood_texture = Self::load_texture(&device, &queue, "src/textures/wood.png")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [165, 128, 77, 255]));
        let leaves_texture = Self::load_texture(&device, &queue, "src/textures/leaves.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [64, 115, 64, 255]));
        let water_texture = Self::create_fallback_texture(&device, &queue, [63, 118, 228, 255]);
        let sand_texture = Self::load_texture(&device, &queue, "src/textures/sand.jpg")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [244, 205, 140, 255]));
        let snow_texture = Self::load_texture(&device, &queue, "src/textures/snow.png")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [250, 250, 250, 255]));
        let torch_texture = Self::load_texture(&device, &queue, "src/textures/torch.png")
            .unwrap_or_else(|_| Self::create_fallback_texture(&device, &queue, [255, 180, 100, 255]));

        // Generate UI atlas programmatically
        let ui_atlas_texture = Self::generate_ui_atlas(&device, &queue);

        // Create texture views
        let grass_view = grass_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let grass_top_view = grass_top_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let dirt_view = dirt_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let stone_view = stone_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let wood_view = wood_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let leaves_view = leaves_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let water_view = water_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sand_view = sand_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let snow_view = snow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let torch_view = torch_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ui_atlas_view = ui_atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        
        // Create texture bind group
        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&grass_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&grass_top_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&dirt_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&stone_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&wood_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&leaves_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&water_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&sand_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&snow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&torch_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(&ui_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::Sampler(&sampler), // Reuse nearest-neighbor sampler
                },
            ],
            label: Some("texture_bind_group"),
        });

        // Generate sky quad for gradient background
        let (sky_vertices, sky_indices) = Self::generate_sky_quad();
        let sky_vertex_count = sky_indices.len() as u32;
        
        let sky_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sky Vertex Buffer"),
            contents: bytemuck::cast_slice(&sky_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let sky_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sky Index Buffer"),
            contents: bytemuck::cast_slice(&sky_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        
        // Generate Minecraft-style 3D clouds
        let (cloud_vertices, cloud_indices) = Self::generate_minecraft_clouds();
        let cloud_index_count = cloud_indices.len() as u32;
        
        let cloud_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cloud Vertex Buffer"),
            contents: bytemuck::cast_slice(&cloud_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let cloud_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cloud Index Buffer"),
            contents: bytemuck::cast_slice(&cloud_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let depth_texture = Self::create_depth_texture(&device, &config);

        // Create HDR render target
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HDR Texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hdr_texture_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hdr_depth_texture = Self::create_depth_texture(&device, &config);

        // Create shadow map
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shadow Map"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_texture_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Shadow Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // Create bloom textures (half resolution for efficiency)
        let bloom_width = config.width / 2;
        let bloom_height = config.height / 2;
        let bloom_textures: [wgpu::Texture; 2] = [
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Bloom Texture 0"),
                size: wgpu::Extent3d {
                    width: bloom_width.max(1),
                    height: bloom_height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Bloom Texture 1"),
                size: wgpu::Extent3d {
                    width: bloom_width.max(1),
                    height: bloom_height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
        ];
        let bloom_texture_views: [wgpu::TextureView; 2] = [
            bloom_textures[0].create_view(&wgpu::TextureViewDescriptor::default()),
            bloom_textures[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];

        // Create linear sampler for post-processing
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Linear Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create nearest sampler for depth (non-filtering)
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Nearest Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Shadow uniform buffer
        let shadow_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shadow Uniform Buffer"),
            size: std::mem::size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &shadow_uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("shadow_bind_group"),
        });

        let shadow_texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &shadow_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
            label: Some("shadow_texture_bind_group"),
        });

        // Post-process uniform buffer
        let post_process_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Post Process Uniform Buffer"),
            contents: bytemuck::cast_slice(&[PostProcessUniform {
                exposure: 1.2,
                bloom_intensity: 0.2,
                saturation: 1.08,
                contrast: 1.05,
                sun_screen_pos: [0.5, 0.3],
                god_ray_intensity: 0.3,
                god_ray_decay: 0.97,
                screen_size: [config.width as f32, config.height as f32],
                ssao_intensity: 0.0,  // Disabled - causing visual artifacts
                ssao_radius: 0.25,
                underwater: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
                _pad3: 0.0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let post_process_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &post_process_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_texture_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: post_process_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&hdr_depth_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&nearest_sampler),
                },
            ],
            label: Some("post_process_bind_group"),
        });

        // Bloom uniform buffer
        let bloom_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bloom Uniform Buffer"),
            contents: bytemuck::cast_slice(&[BloomUniform {
                threshold: 0.8,
                soft_threshold: 0.5,
                blur_direction: [1.0, 0.0],
                texel_size: [1.0 / bloom_width as f32, 1.0 / bloom_height as f32],
                _padding: [0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Bloom bind groups
        let bloom_bind_groups: [wgpu::BindGroup; 3] = [
            // Extract pass - sample from HDR texture
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bloom_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&hdr_texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: bloom_uniform_buffer.as_entire_binding(),
                    },
                ],
                label: Some("bloom_extract_bind_group"),
            }),
            // Horizontal blur - sample from bloom texture 0
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bloom_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&bloom_texture_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: bloom_uniform_buffer.as_entire_binding(),
                    },
                ],
                label: Some("bloom_blur_h_bind_group"),
            }),
            // Vertical blur - sample from bloom texture 1
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bloom_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&bloom_texture_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: bloom_uniform_buffer.as_entire_binding(),
                    },
                ],
                label: Some("bloom_blur_v_bind_group"),
            }),
        ];

        let ui_renderer = UIRenderer::new(&device, &queue, &config, &texture_bind_group, &texture_bind_group_layout);
        let start_time = Instant::now();
        
        // Create indices for up to 10 cubes (tool parts + arm) - enough for any tool
        let max_cubes = 10;
        let mut held_indices: Vec<u16> = vec![];
        let face_indices = &[0u16, 1, 2, 2, 3, 0];

        for cube in 0..max_cubes {
            let vert_base = (cube * 24) as u16;
            for face in 0..6 {
                let base = vert_base + (face * 4) as u16;
                for &i in face_indices {
                    held_indices.push(base + i);
                }
            }
        }
        let held_item_index_count = held_indices.len() as u32;

        let held_item_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Held Item Index Buffer"),
            contents: bytemuck::cast_slice(&held_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let held_item_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Held Item Vertex Buffer"),
            size: (max_cubes * 24 * std::mem::size_of::<Vertex>()) as u64, // max_cubes * 24 vertices each
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let arm_swing_progress = 0.0;
        let last_render = Instant::now();

        // Villager buffers (allocate max for ~50 villagers * 6 parts * 24 vertices each)
        let max_villager_vertices = 50 * 6 * 24;
        let villager_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Villager Vertex Buffer"),
            size: (max_villager_vertices * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let villager_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Villager Index Buffer"),
            size: (max_villager_vertices * 6 / 4 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Particle system setup
        let particle_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("particle_bind_group_layout"),
        });

        let particle_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Uniform Buffer"),
            size: std::mem::size_of::<ParticleUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let particle_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &particle_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: particle_uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("particle_bind_group"),
        });

        // Particle vertex buffer (2000 particles * 4 vertices each)
        let max_particle_vertices = 2000 * 4;
        let particle_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Vertex Buffer"),
            size: (max_particle_vertices * std::mem::size_of::<ParticleVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let particle_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Particle Pipeline Layout"),
            bind_group_layouts: &[&particle_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Lighting buffer for point lights
        let lighting_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lighting Buffer"),
            size: std::mem::size_of::<LightingUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &lighting_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lighting_buffer.as_entire_binding(),
                },
            ],
            label: Some("lighting_bind_group"),
        });

        // Block preview buffers (6 faces * 4 vertices = 24 vertices, 36 indices)
        let preview_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview Vertex Buffer"),
            size: (24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let preview_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview Index Buffer"),
            size: (36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Dropped item buffers (200 items max * 24 vertices, 36 indices each)
        let dropped_item_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dropped Item Vertex Buffer"),
            size: (200 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let dropped_item_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dropped Item Index Buffer"),
            size: (200 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Animal buffers (200 animals max, each with ~10 body parts * 24 vertices for wings/fins)
        let animal_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Animal Vertex Buffer"),
            size: (200 * 10 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let animal_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Animal Index Buffer"),
            size: (200 * 10 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Hostile mob buffers (30 mobs max, each with ~8 body parts * 24 vertices)
        let hostile_mob_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Hostile Mob Vertex Buffer"),
            size: (30 * 8 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let hostile_mob_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Hostile Mob Index Buffer"),
            size: (30 * 8 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Plane buffers (20 planes max, each with ~10 body parts * 24 vertices)
        let plane_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Plane Vertex Buffer"),
            size: (20 * 10 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let plane_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Plane Index Buffer"),
            size: (20 * 10 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Missile buffers (50 missiles max, simple elongated cube)
        let missile_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Missile Vertex Buffer"),
            size: (50 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let missile_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Missile Index Buffer"),
            size: (50 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Bomb buffers (100 bombs max, simple sphere-ish shape)
        let bomb_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bomb Vertex Buffer"),
            size: (100 * 24 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bomb_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bomb Index Buffer"),
            size: (100 * 36 * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let particle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Particle Pipeline"),
            layout: Some(&particle_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &particle_shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ParticleVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 20,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 36,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &particle_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // No culling for particles
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false, // Particles don't write to depth
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        Self {
            surface,
            device,
            queue,
            config,
            render_pipeline,
            outline_pipeline,
            sky_pipeline,
            cloud_pipeline,
            held_item_pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group,
            depth_texture,
            ui_renderer,
            outline_vertex_buffer,
            outline_index_buffer,
            sky_vertex_buffer,
            sky_index_buffer,
            sky_vertex_count,
            cloud_vertex_buffer,
            cloud_index_buffer,
            cloud_index_count,
            transparent_pipeline,
            chunk_meshes_opaque: HashMap::new(),
            chunk_meshes_transparent: HashMap::new(),
            arm_swing_progress,
            held_item_index_count,
            last_render,
            held_item_vertex_buffer,
            held_item_index_buffer,
            // HDR rendering
            hdr_texture,
            hdr_texture_view,
            hdr_depth_texture,
            // Shadow mapping
            shadow_pipeline,
            shadow_texture,
            shadow_texture_view,
            shadow_sampler,
            shadow_uniform_buffer,
            shadow_bind_group,
            shadow_texture_bind_group,
            // Post-processing
            post_process_pipeline,
            post_process_bind_group,
            post_process_uniform_buffer,
            // Bloom
            bloom_extract_pipeline,
            bloom_blur_pipeline,
            bloom_textures,
            bloom_texture_views,
            bloom_bind_groups,
            bloom_uniform_buffer,
            // Time tracking
            start_time,
            time_offset: rand::thread_rng().gen_range(0.0..1.0),  // Random starting time
            time_of_day: 0.0,
            // Villager rendering
            villager_vertex_buffer,
            villager_index_buffer,
            villager_index_count: 0,
            // Particle rendering
            particle_pipeline,
            particle_vertex_buffer,
            particle_uniform_buffer,
            particle_bind_group,
            particle_vertex_count: 0,
            // Point lighting
            lighting_buffer,
            lighting_bind_group,
            // Block preview
            preview_vertex_buffer,
            preview_index_buffer,
            preview_index_count: 0,
            preview_visible: false,
            // Dropped items
            dropped_item_vertex_buffer,
            dropped_item_index_buffer,
            dropped_item_index_count: 0,
            // Animals
            animal_vertex_buffer,
            animal_index_buffer,
            animal_index_count: 0,
            // Hostile mobs
            hostile_mob_vertex_buffer,
            hostile_mob_index_buffer,
            hostile_mob_index_count: 0,
            // Planes
            plane_vertex_buffer,
            plane_index_buffer,
            plane_index_count: 0,
            // Missiles
            missile_vertex_buffer,
            missile_index_buffer,
            missile_index_count: 0,
            // Bombs
            bomb_vertex_buffer,
            bomb_index_buffer,
            bomb_index_count: 0,
        }
    }

    fn collect_torch_lights(world: &World, camera_pos: cgmath::Point3<f32>) -> LightingUniform {
        use crate::world::BlockType;

        let mut lights = [PointLight::default(); MAX_POINT_LIGHTS];
        let mut num_lights = 0u32;

        let search_radius = 48i32;
        let cam_x = camera_pos.x as i32;
        let cam_y = camera_pos.y as i32;
        let cam_z = camera_pos.z as i32;

        // Search nearby blocks for torches and lava
        for x in (cam_x - search_radius)..(cam_x + search_radius) {
            for z in (cam_z - search_radius)..(cam_z + search_radius) {
                for y in (cam_y - 20).max(0)..(cam_y + 20).min(World::CHUNK_HEIGHT as i32) {
                    if let Some(block) = world.get_block(x, y, z) {
                        let light = match block {
                            BlockType::Torch => Some(PointLight {
                                position: [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5],
                                radius: 12.0,
                                color: [1.0, 0.8, 0.4],  // Warm orange glow
                                intensity: 1.5,
                            }),
                            BlockType::Lava => Some(PointLight {
                                position: [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5],
                                radius: 15.0,  // Larger than torch
                                color: [1.0, 0.4, 0.1],  // Orange-red glow
                                intensity: 2.0,  // Brighter than torch
                            }),
                            _ => None,
                        };

                        if let Some(point_light) = light {
                            if (num_lights as usize) < MAX_POINT_LIGHTS {
                                lights[num_lights as usize] = point_light;
                                num_lights += 1;
                            }
                        }
                    }
                }
            }
        }

        LightingUniform {
            point_lights: lights,
            num_lights,
            _padding: [0; 3],
        }
    }

    fn generate_sky_quad() -> (Vec<SkyVertex>, Vec<u16>) {
        // Create a simple full-screen quad that renders at maximum depth
        let vertices = vec![
            SkyVertex { position: [-1.0, -1.0, 0.999], brightness: 1.0 }, // Bottom-left (near max depth)
            SkyVertex { position: [1.0, -1.0, 0.999], brightness: 1.0 },  // Bottom-right
            SkyVertex { position: [1.0, 1.0, 0.999], brightness: 1.0 },   // Top-right
            SkyVertex { position: [-1.0, 1.0, 0.999], brightness: 1.0 },  // Top-left
        ];
        let indices = vec![0, 1, 2, 2, 3, 0];
        
        (vertices, indices)
    }

    fn generate_minecraft_clouds() -> (Vec<CloudVertex>, Vec<u16>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        
        // Minecraft cloud generation parameters
        let cloud_height = 100.0; // Height above ground
        let cloud_size = 8.0; // Size of each cloud block (doubled for larger clouds)
        let cloud_spacing = 16.0; // Distance between cloud centers (increased spacing)
        let cloud_density = 0.4; // Probability of cloud block existing
        
        // Generate clouds in a grid pattern around origin
        let grid_size = 20; // 20x20 grid of potential cloud positions
        
        // Simple pseudo-random function for consistent cloud patterns
        let noise = |x: i32, z: i32| -> f32 {
            let mut n = (x as u32).wrapping_mul(374761393).wrapping_add((z as u32).wrapping_mul(668265263));
            n = n ^ (n >> 13);
            n = n.wrapping_mul(1274126177) ^ (n >> 16);
            (n as f32) / (u32::MAX as f32)
        };
        
        for x in -grid_size/2..grid_size/2 {
            for z in -grid_size/2..grid_size/2 {
                let cloud_x = x as f32 * cloud_spacing;
                let cloud_z = z as f32 * cloud_spacing;
                
                // Check if cloud exists at this position
                if noise(x, z) < cloud_density {
                    // Generate cloud cluster (3x2x3 blocks for larger, more substantial clouds)
                    for dx in 0..3 {
                        for dy in 0..2 {
                            for dz in 0..3 {
                                if noise(x * 8 + dx * 2, z * 8 + dz * 2 + dy * 4) < 0.75 {
                                    let block_x = cloud_x + (dx as f32 - 1.0) * cloud_size;
                                    let block_z = cloud_z + (dz as f32 - 1.0) * cloud_size;
                                    let block_y = cloud_height + dy as f32 * cloud_size * 0.5 + noise(x * 12 + dx * 3, z * 12 + dz * 3) * 4.0;
                                    
                                    // Add cloud block (6 faces)
                                    Self::add_cloud_cube(&mut vertices, &mut indices, block_x, block_y, block_z, cloud_size);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        (vertices, indices)
    }
    
    fn add_cloud_cube(vertices: &mut Vec<CloudVertex>, indices: &mut Vec<u16>, x: f32, y: f32, z: f32, size: f32) {
        let half_size = size / 2.0;
        
        // Define the 8 vertices of the cube
        let cube_vertices = [
            [x - half_size, y - half_size, z - half_size], // 0: bottom-left-back
            [x + half_size, y - half_size, z - half_size], // 1: bottom-right-back  
            [x + half_size, y + half_size, z - half_size], // 2: top-right-back
            [x - half_size, y + half_size, z - half_size], // 3: top-left-back
            [x - half_size, y - half_size, z + half_size], // 4: bottom-left-front
            [x + half_size, y - half_size, z + half_size], // 5: bottom-right-front
            [x + half_size, y + half_size, z + half_size], // 6: top-right-front
            [x - half_size, y + half_size, z + half_size], // 7: top-left-front
        ];
        
        // Add vertices for all 6 faces with proper normals
        let faces = [
            // Front face (z+)
            ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
            // Back face (z-)
            ([1, 0, 3, 2], [0.0, 0.0, -1.0]),
            // Right face (x+)
            ([5, 1, 2, 6], [1.0, 0.0, 0.0]),
            // Left face (x-)
            ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),
            // Top face (y+)
            ([3, 7, 6, 2], [0.0, 1.0, 0.0]),
            // Bottom face (y-)
            ([4, 0, 1, 5], [0.0, -1.0, 0.0]),
        ];
        
        for (face_verts, normal) in faces.iter() {
            let face_start = vertices.len() as u16;
            
            // Add 4 vertices for this face
            for &vert_idx in face_verts.iter() {
                vertices.push(CloudVertex {
                    position: cube_vertices[vert_idx],
                    normal: *normal,
                });
            }
            
            // Add 2 triangles for this face
            indices.extend_from_slice(&[
                face_start, face_start + 1, face_start + 2,
                face_start, face_start + 2, face_start + 3,
            ]);
        }
    }

    fn create_depth_texture(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        
        depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
    }
    
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            self.depth_texture = Self::create_depth_texture(&self.device, &self.config);

            // Update UI elements for new aspect ratio
            self.ui_renderer.resize(&self.device, &self.config);
        }
    }

    pub fn get_time_of_day(&self) -> f32 {
        self.time_of_day
    }

    /// Set time of day (0.0 = midnight, 0.25 = dawn, 0.5 = noon, 0.75 = dusk)
    pub fn set_time_of_day(&mut self, time: f32) {
        // Adjust time_offset to achieve the desired time
        let elapsed = (Instant::now() - self.start_time).as_secs_f32();
        self.time_offset = time - elapsed / 600.0;
        self.time_of_day = time.fract();
    }

    pub fn render(&mut self, camera: &Camera, world: &mut World, inventory: &Inventory, targeted_block: Option<(i32, i32, i32)>, entity_manager: &EntityManager, particle_system: &ParticleSystem, underwater: bool, debug_info: &DebugInfo, pause_menu: &PauseMenu, chest_ui: &ChestUI, crafting_ui: &CraftingUI, furnace_ui: &crate::ui::FurnaceUI, recipe_registry: &RecipeRegistry, lightning_system: &LightningSystem, weather_state: &WeatherState) {
        let now = Instant::now();
        let dt = (now - self.last_render).as_secs_f32();
        self.last_render = now;
        self.arm_swing_progress = (self.arm_swing_progress - dt * 4.0).max(0.0);

        // Update time of day (full cycle every 10 minutes)
        let elapsed = (now - self.start_time).as_secs_f32();
        self.time_of_day = (self.time_offset + elapsed / 600.0).fract();

        // Calculate sun direction based on time of day
        let sun_angle = self.time_of_day * 2.0 * std::f32::consts::PI - std::f32::consts::PI / 2.0;
        let sun_direction = Vector3::new(
            0.3 * sun_angle.cos(),
            sun_angle.sin(),
            0.5 * sun_angle.cos(),
        ).normalize();

        // Day/night factor for lighting
        let day_factor = (sun_direction.y + 0.1).max(0.0).min(1.0) / 0.4;
        // Much higher minimum ambient at night so terrain is always visible
        let mut ambient_intensity = 0.35 + 0.25 * day_factor;

        // Weather affects ambient light (darken during storms)
        let weather_factor = match weather_state.weather_type {
            WeatherType::Thunderstorm => 0.5,  // Much darker during thunderstorms
            WeatherType::Rain => 0.75,         // Slightly darker during rain
            _ => 1.0,
        };
        ambient_intensity *= weather_factor;

        // Lightning flash brightens everything temporarily
        if lightning_system.sky_flash > 0.0 {
            ambient_intensity = ambient_intensity.max(0.8 * lightning_system.sky_flash);
        }

        // Sun color changes with time of day
        let sun_color = if sun_direction.y < 0.0 {
            [0.1, 0.1, 0.2]  // Moonlight (blue-ish)
        } else if sun_direction.y < 0.2 {
            // Sunrise/sunset - orange
            let t = sun_direction.y / 0.2;
            [1.0 - 0.2 * t, 0.6 + 0.3 * t, 0.3 + 0.5 * t]
        } else {
            [1.0, 0.95, 0.85]  // Daylight
        };

        // Update chunk meshes for loaded chunks
        self.update_chunk_meshes(world);

        // Update villager mesh
        self.update_villager_mesh(entity_manager.get_villagers());

        // Update animal mesh
        self.update_animal_mesh(entity_manager.get_animals());

        // Update hostile mob mesh (including projectiles)
        self.update_hostile_mob_mesh(entity_manager.get_hostile_mobs(), entity_manager.get_projectiles());

        // Update plane mesh
        self.update_plane_mesh(entity_manager.get_planes());

        // Update missile mesh
        self.update_missile_mesh(entity_manager.get_missiles());

        // Update bomb mesh
        self.update_bomb_mesh(entity_manager.get_bombs());

        // Update dropped item mesh
        self.update_dropped_items(entity_manager.get_dropped_items());

        // Update particle mesh
        self.update_particle_mesh(camera, particle_system);

        let output = self.surface.get_current_texture().unwrap();
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let view_proj = camera.view_proj;
        let inverse_view_proj = view_proj.invert().unwrap();

        // Calculate light view-projection matrix for shadow mapping
        let light_view_proj = Self::calculate_light_matrix(&camera.position, &sun_direction);

        let uniform = Uniform {
            view_proj: view_proj.into(),
            inverse_view_proj: inverse_view_proj.into(),
            camera_pos: [camera.position.x, camera.position.y, camera.position.z],
            time_of_day: self.time_of_day,
            sun_direction: [sun_direction.x, sun_direction.y, sun_direction.z],
            ambient_intensity,
            light_view_proj: light_view_proj.into(),
            sun_color,
            fog_density: Self::calculate_fog_density(self.time_of_day, weather_state, lightning_system.sky_flash),
        };

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        // Update shadow uniform
        let shadow_uniform = ShadowUniform {
            light_view_proj: light_view_proj.into(),
        };
        self.queue.write_buffer(&self.shadow_uniform_buffer, 0, bytemuck::cast_slice(&[shadow_uniform]));

        // Calculate sun position in screen space for god rays
        let sun_world_pos = Point3::new(
            camera.position.x + sun_direction.x * 1000.0,
            camera.position.y + sun_direction.y * 1000.0,
            camera.position.z + sun_direction.z * 1000.0,
        );
        let sun_clip = view_proj * Vector4::new(sun_world_pos.x, sun_world_pos.y, sun_world_pos.z, 1.0);
        let sun_screen_pos = if sun_clip.w > 0.0 {
            let ndc_x = sun_clip.x / sun_clip.w;
            let ndc_y = sun_clip.y / sun_clip.w;
            [(ndc_x + 1.0) * 0.5, (1.0 - ndc_y) * 0.5]
        } else {
            [-10.0, -10.0] // Sun behind camera
        };

        // Update post-process uniform with sun position and underwater effect
        let post_uniform = PostProcessUniform {
            exposure: 1.3,
            bloom_intensity: 0.35,
            saturation: 1.08,
            contrast: 1.04,
            sun_screen_pos,
            god_ray_intensity: 0.4 * day_factor,  // Sun glow effect (scales with daytime)
            god_ray_decay: 0.97,
            screen_size: [self.config.width as f32, self.config.height as f32],
            ssao_intensity: 0.0,  // Disabled - causing visual artifacts
            ssao_radius: 0.25,
            underwater: if underwater { 1.0 } else { 0.0 },
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
        };
        self.queue.write_buffer(&self.post_process_uniform_buffer, 0, bytemuck::cast_slice(&[post_uniform]));

        // Collect torch lights from nearby chunks
        let lighting_uniform = Self::collect_torch_lights(world, camera.position);
        self.queue.write_buffer(&self.lighting_buffer, 0, bytemuck::cast_slice(&[lighting_uniform]));

        // Update outline buffers if a block is targeted
        if let Some((block_x, block_y, block_z)) = targeted_block {
            self.update_outline_buffers(block_x, block_y, block_z);
        }
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        // === SHADOW MAP PASS ===
        {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });

            shadow_pass.set_pipeline(&self.shadow_pipeline);
            shadow_pass.set_bind_group(0, &self.shadow_bind_group, &[]);

            // Render opaque chunks to shadow map
            for chunk_mesh in self.chunk_meshes_opaque.values() {
                if chunk_mesh.index_count > 0 {
                    shadow_pass.set_vertex_buffer(0, chunk_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(chunk_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    shadow_pass.draw_indexed(0..chunk_mesh.index_count, 0, 0..1);
                }
            }

            // Render villagers to shadow map
            if self.villager_index_count > 0 {
                shadow_pass.set_vertex_buffer(0, self.villager_vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(self.villager_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                shadow_pass.draw_indexed(0..self.villager_index_count, 0, 0..1);
            }

            // Render animals to shadow map
            if self.animal_index_count > 0 {
                shadow_pass.set_vertex_buffer(0, self.animal_vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(self.animal_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                shadow_pass.draw_indexed(0..self.animal_index_count, 0, 0..1);
            }

            // Render hostile mobs to shadow map
            if self.hostile_mob_index_count > 0 {
                shadow_pass.set_vertex_buffer(0, self.hostile_mob_vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(self.hostile_mob_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                shadow_pass.draw_indexed(0..self.hostile_mob_index_count, 0, 0..1);
            }
        }

        // === MAIN HDR RENDER PASS ===
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HDR Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.hdr_depth_texture,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });

            // Render sky gradient background
            render_pass.set_pipeline(&self.sky_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.sky_vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.sky_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.sky_vertex_count, 0, 0..1);

            // Render terrain blocks with shadows
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            render_pass.set_bind_group(2, &self.shadow_texture_bind_group, &[]);
            render_pass.set_bind_group(3, &self.lighting_bind_group, &[]);

            // Create frustum for culling
            let frustum = Frustum::from_view_proj(&view_proj);
            let chunk_size = World::CHUNK_SIZE as f32;
            let chunk_height = World::CHUNK_HEIGHT as f32;

            // Render opaque chunks with frustum culling
            for (&(chunk_x, chunk_z), chunk_mesh) in &self.chunk_meshes_opaque {
                if chunk_mesh.index_count > 0 {
                    // Calculate chunk AABB
                    let min = Vector3::new(
                        chunk_x as f32 * chunk_size,
                        0.0,
                        chunk_z as f32 * chunk_size,
                    );
                    let max = Vector3::new(
                        (chunk_x as f32 + 1.0) * chunk_size,
                        chunk_height,
                        (chunk_z as f32 + 1.0) * chunk_size,
                    );

                    // Skip if outside frustum
                    if !frustum.intersects_aabb(min, max) {
                        continue;
                    }

                    render_pass.set_vertex_buffer(0, chunk_mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(chunk_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..chunk_mesh.index_count, 0, 0..1);
                }
            }

            // Render villagers (after opaque terrain, before transparent)
            if self.villager_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.villager_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.villager_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.villager_index_count, 0, 0..1);
            }

            // Render animals
            if self.animal_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.animal_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.animal_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.animal_index_count, 0, 0..1);
            }

            // Render hostile mobs
            if self.hostile_mob_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.hostile_mob_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.hostile_mob_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.hostile_mob_index_count, 0, 0..1);
            }

            // Render planes
            if self.plane_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.plane_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.plane_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.plane_index_count, 0, 0..1);
            }

            // Render missiles
            if self.missile_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.missile_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.missile_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.missile_index_count, 0, 0..1);
            }

            // Render bombs
            if self.bomb_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.bomb_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.bomb_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.bomb_index_count, 0, 0..1);
            }

            // Render dropped items
            if self.dropped_item_index_count > 0 {
                render_pass.set_vertex_buffer(0, self.dropped_item_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.dropped_item_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.dropped_item_index_count, 0, 0..1);
            }

            // Render transparent chunks with frustum culling
            render_pass.set_pipeline(&self.transparent_pipeline);
            render_pass.set_bind_group(2, &self.shadow_texture_bind_group, &[]);
            render_pass.set_bind_group(3, &self.lighting_bind_group, &[]);
            for (&(chunk_x, chunk_z), chunk_mesh) in &self.chunk_meshes_transparent {
                if chunk_mesh.index_count > 0 {
                    // Calculate chunk AABB
                    let min = Vector3::new(
                        chunk_x as f32 * chunk_size,
                        0.0,
                        chunk_z as f32 * chunk_size,
                    );
                    let max = Vector3::new(
                        (chunk_x as f32 + 1.0) * chunk_size,
                        chunk_height,
                        (chunk_z as f32 + 1.0) * chunk_size,
                    );

                    // Skip if outside frustum
                    if !frustum.intersects_aabb(min, max) {
                        continue;
                    }

                    render_pass.set_vertex_buffer(0, chunk_mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(chunk_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..chunk_mesh.index_count, 0, 0..1);
                }
            }

            // Render block preview (ghost block)
            if self.preview_visible && self.preview_index_count > 0 {
                // Uses same transparent pipeline (already set)
                render_pass.set_vertex_buffer(0, self.preview_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.preview_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.preview_index_count, 0, 0..1);
            }

            // Render particles
            if self.particle_vertex_count > 0 {
                render_pass.set_pipeline(&self.particle_pipeline);
                render_pass.set_bind_group(0, &self.particle_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.particle_vertex_buffer.slice(..));
                render_pass.draw(0..self.particle_vertex_count, 0..1);
            }

            // Render 3D clouds
            if self.cloud_index_count > 0 {
                render_pass.set_pipeline(&self.cloud_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.cloud_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.cloud_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.cloud_index_count, 0, 0..1);
            }

            // Render selected block outline
            if targeted_block.is_some() {
                render_pass.set_pipeline(&self.outline_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.outline_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.outline_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..72, 0, 0..1);
            }

            // Only render held item when not piloting a plane
            if !camera.is_piloting() {
                let opt_item = inventory.get_selected_item();
                let vertices = Self::create_held_item_vertices(camera, opt_item, self.arm_swing_progress);
                self.queue.write_buffer(&self.held_item_vertex_buffer, 0, bytemuck::cast_slice(&vertices));

                // Calculate index count based on actual vertices (24 verts per cube, 36 indices per cube)
                let num_cubes = vertices.len() / 24;
                let actual_index_count = (num_cubes * 36) as u32;

                // Render held item
                render_pass.set_pipeline(&self.held_item_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.held_item_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.held_item_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..actual_index_count, 0, 0..1);
            }
        }

        // === BLOOM EXTRACT PASS ===
        {
            // Update bloom uniform for extract
            self.queue.write_buffer(&self.bloom_uniform_buffer, 0, bytemuck::cast_slice(&[BloomUniform {
                threshold: 0.8,
                soft_threshold: 0.5,
                blur_direction: [0.0, 0.0],
                texel_size: [1.0 / (self.config.width / 2) as f32, 1.0 / (self.config.height / 2) as f32],
                _padding: [0.0, 0.0],
            }]));

            let mut bloom_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Extract Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_texture_views[0],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            bloom_pass.set_pipeline(&self.bloom_extract_pipeline);
            bloom_pass.set_bind_group(0, &self.bloom_bind_groups[0], &[]);
            bloom_pass.draw(0..3, 0..1);
        }

        // === BLOOM HORIZONTAL BLUR ===
        {
            self.queue.write_buffer(&self.bloom_uniform_buffer, 0, bytemuck::cast_slice(&[BloomUniform {
                threshold: 0.8,
                soft_threshold: 0.5,
                blur_direction: [1.0, 0.0],
                texel_size: [1.0 / (self.config.width / 2) as f32, 1.0 / (self.config.height / 2) as f32],
                _padding: [0.0, 0.0],
            }]));

            let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom H Blur Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_texture_views[1],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            blur_pass.set_pipeline(&self.bloom_blur_pipeline);
            blur_pass.set_bind_group(0, &self.bloom_bind_groups[1], &[]);
            blur_pass.draw(0..3, 0..1);
        }

        // === BLOOM VERTICAL BLUR ===
        {
            self.queue.write_buffer(&self.bloom_uniform_buffer, 0, bytemuck::cast_slice(&[BloomUniform {
                threshold: 0.8,
                soft_threshold: 0.5,
                blur_direction: [0.0, 1.0],
                texel_size: [1.0 / (self.config.width / 2) as f32, 1.0 / (self.config.height / 2) as f32],
                _padding: [0.0, 0.0],
            }]));

            let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom V Blur Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_texture_views[0],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            blur_pass.set_pipeline(&self.bloom_blur_pipeline);
            blur_pass.set_bind_group(0, &self.bloom_bind_groups[2], &[]);
            blur_pass.draw(0..3, 0..1);
        }

        // === POST-PROCESS PASS (tone mapping, bloom composite) ===
        {
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Post Process Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            post_pass.set_pipeline(&self.post_process_pipeline);
            post_pass.set_bind_group(0, &self.post_process_bind_group, &[]);
            post_pass.draw(0..3, 0..1);
        }

        // === UI PASS ===
        // Only render hotbar/crosshair when crafting UI is not open
        if !crafting_ui.open {
            let mut ui_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            self.ui_renderer.update_inventory_selection(&self.device, &inventory);
            self.ui_renderer.render(&mut ui_render_pass, &self.texture_bind_group);
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Render debug overlay (F3 screen)
        if debug_info.visible {
            let fps = 1.0 / dt;
            self.ui_renderer.render_debug_overlay(
                &self.device,
                &self.queue,
                &view,
                &self.texture_bind_group,
                fps,
                camera.position,
                camera.get_facing_direction(),
                world.chunks.len(),
                particle_system.len(),
            );
        }

        // Render pause menu
        if pause_menu.visible {
            self.ui_renderer.render_pause_menu(
                &self.device,
                &self.queue,
                &view,
                &self.texture_bind_group,
                pause_menu.selected_option,
            );
        }

        // Render chest UI
        if chest_ui.open {
            if let Some(chest_pos) = chest_ui.chest_pos {
                let empty_chest: [Option<(crate::world::BlockType, u32)>; 9] = [None; 9];
                let chest_contents = world.chest_contents.get(&chest_pos).unwrap_or(&empty_chest);
                self.ui_renderer.render_chest_ui(
                    &self.device,
                    &self.queue,
                    &view,
                    &self.texture_bind_group,
                    chest_ui,
                    chest_contents,
                    inventory,
                );
            }
        }

        // Render crafting UI
        if crafting_ui.open {
            let recipe_result = recipe_registry.find_match(&crafting_ui.grid, crafting_ui.grid_size)
                .map(|r| &r.result);
            self.ui_renderer.render_crafting_ui(
                &self.device,
                &self.queue,
                &view,
                &self.texture_bind_group,
                crafting_ui,
                inventory,
                recipe_result,
            );
        }

        // Render furnace UI
        if furnace_ui.open {
            if let Some(furnace_pos) = furnace_ui.furnace_pos {
                if let Some(furnace_data) = world.get_furnace(furnace_pos.0, furnace_pos.1, furnace_pos.2) {
                    self.ui_renderer.render_furnace_ui(
                        &self.device,
                        &self.queue,
                        &view,
                        &self.texture_bind_group,
                        furnace_ui,
                        furnace_data,
                        inventory,
                    );
                }
            }
        }

        // Render survival UI (health, hunger, air)
        if !pause_menu.visible && !chest_ui.open && !crafting_ui.open {
            self.ui_renderer.render_survival_ui(
                &self.device,
                &self.queue,
                &view,
                &self.texture_bind_group,
                camera.health,
                camera.max_health,
                camera.hunger,
                camera.air_supply,
                underwater,
                camera.damage_flash > 0.0,
            );
        }

        // Render death screen
        if camera.is_dead {
            self.ui_renderer.render_death_screen(
                &self.device,
                &self.queue,
                &view,
                &self.texture_bind_group,
            );
        }

        output.present();
    }

    fn calculate_light_matrix(camera_pos: &cgmath::Point3<f32>, sun_direction: &Vector3<f32>) -> Matrix4<f32> {
        // Calculate orthographic projection for directional light shadows
        let shadow_distance = 100.0;
        let light_pos = Point3::new(
            camera_pos.x + sun_direction.x * shadow_distance,
            camera_pos.y + sun_direction.y * shadow_distance,
            camera_pos.z + sun_direction.z * shadow_distance,
        );

        let light_target = Point3::new(camera_pos.x, camera_pos.y, camera_pos.z);
        let up = if sun_direction.y.abs() > 0.99 {
            Vector3::new(0.0, 0.0, 1.0)
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        };

        let light_view = Matrix4::look_at_rh(light_pos, light_target, up);

        // Orthographic projection for the shadow map
        let shadow_size = 80.0;
        let light_proj = cgmath::ortho(
            -shadow_size, shadow_size,
            -shadow_size, shadow_size,
            0.1, shadow_distance * 2.0,
        );

        light_proj * light_view
    }

    fn calculate_fog_density(time_of_day: f32, weather_state: &WeatherState, sky_flash: f32) -> f32 {
        // Base fog density
        let mut fog_density = 0.002;

        // Morning mist (thicker fog at dawn: 0.2-0.35 time)
        if time_of_day > 0.2 && time_of_day < 0.35 {
            // Peak fog at 0.275 (middle of dawn)
            let dawn_progress = (time_of_day - 0.2) / 0.15;
            let mist_factor = if dawn_progress < 0.5 {
                dawn_progress * 2.0  // Fog increasing
            } else {
                (1.0 - dawn_progress) * 2.0  // Fog decreasing
            };
            fog_density += 0.006 * mist_factor;  // Up to 0.008 total during peak mist
        }

        // Weather-based fog
        match weather_state.weather_type {
            WeatherType::Thunderstorm => {
                // Heavy fog during thunderstorms
                fog_density += 0.012 * weather_state.intensity;
            }
            WeatherType::Rain => {
                // Light fog during rain
                fog_density += 0.004 * weather_state.intensity;
            }
            WeatherType::Snow => {
                // Moderate fog during snow
                fog_density += 0.005 * weather_state.intensity;
            }
            WeatherType::Clear => {}
        }

        // Lightning flash briefly reduces fog visibility (scene becomes clearer)
        if sky_flash > 0.0 {
            fog_density *= 1.0 - (sky_flash * 0.5);
        }

        fog_density
    }

    fn update_chunk_meshes(&mut self, world: &mut World) {
        // With parallel mesh generation, we can process more chunks per frame
        const MAX_CHUNKS_PER_FRAME: usize = 4;

        let loaded_chunks: HashMap<(i32, i32), ()> = world.get_loaded_chunks()
            .map(|chunk| ((chunk.position.x, chunk.position.z), ()))
            .collect();

        self.chunk_meshes_opaque.retain(|key, _| loaded_chunks.contains_key(key));
        self.chunk_meshes_transparent.retain(|key, _| loaded_chunks.contains_key(key));

        let dirty_chunks: Vec<(i32, i32)> = world.get_loaded_chunks()
            .filter(|chunk| chunk.dirty || !chunk.mesh_generated)
            .map(|chunk| (chunk.position.x, chunk.position.z))
            .take(MAX_CHUNKS_PER_FRAME)
            .collect();

        if dirty_chunks.is_empty() {
            return;
        }

        // Generate mesh data in parallel (CPU-bound work)
        let mesh_results: Vec<_> = dirty_chunks
            .par_iter()
            .filter_map(|&(chunk_x, chunk_z)| {
                let chunk_key = (chunk_x, chunk_z);
                world.chunks.get(&chunk_key).map(|chunk| {
                    let (opaque_vertices, opaque_indices, trans_vertices, trans_indices) =
                        Self::create_vertices_for_chunk(world, chunk);
                    (chunk_key, opaque_vertices, opaque_indices, trans_vertices, trans_indices)
                })
            })
            .collect();

        // Create GPU buffers sequentially (GPU operations must be on main thread)
        for (chunk_key, opaque_vertices, opaque_indices, trans_vertices, trans_indices) in mesh_results {
            if !opaque_vertices.is_empty() {
                let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Opaque Vertex Buffer"),
                    contents: bytemuck::cast_slice(&opaque_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Opaque Index Buffer"),
                    contents: bytemuck::cast_slice(&opaque_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let chunk_mesh = ChunkMesh {
                    vertex_buffer,
                    index_buffer,
                    index_count: opaque_indices.len() as u32,
                };

                self.chunk_meshes_opaque.insert(chunk_key, chunk_mesh);
            }

            if !trans_vertices.is_empty() {
                let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Transparent Vertex Buffer"),
                    contents: bytemuck::cast_slice(&trans_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Transparent Index Buffer"),
                    contents: bytemuck::cast_slice(&trans_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let chunk_mesh = ChunkMesh {
                    vertex_buffer,
                    index_buffer,
                    index_count: trans_indices.len() as u32,
                };

                self.chunk_meshes_transparent.insert(chunk_key, chunk_mesh);
            }
        }

        for (chunk_x, chunk_z) in &dirty_chunks {
            let chunk_key = (*chunk_x, *chunk_z);
            if let Some(chunk) = world.chunks.get_mut(&chunk_key) {
                chunk.dirty = false;
                chunk.mesh_generated = true;
            }
        }
    }

    /// Force generate all chunk meshes without rate limiting (for initial loading)
    /// Uses parallel processing for faster initial load
    pub fn force_generate_all_meshes(&mut self, world: &mut World) {
        let loaded_chunks: HashMap<(i32, i32), ()> = world.get_loaded_chunks()
            .map(|chunk| ((chunk.position.x, chunk.position.z), ()))
            .collect();

        self.chunk_meshes_opaque.retain(|key, _| loaded_chunks.contains_key(key));
        self.chunk_meshes_transparent.retain(|key, _| loaded_chunks.contains_key(key));

        // Get ALL dirty/ungenerated chunks without limit
        let dirty_chunks: Vec<(i32, i32)> = world.get_loaded_chunks()
            .filter(|chunk| chunk.dirty || !chunk.mesh_generated)
            .map(|chunk| (chunk.position.x, chunk.position.z))
            .collect();

        if dirty_chunks.is_empty() {
            return;
        }

        // Generate mesh data in parallel (CPU-bound work)
        let mesh_results: Vec<_> = dirty_chunks
            .par_iter()
            .filter_map(|&(chunk_x, chunk_z)| {
                let chunk_key = (chunk_x, chunk_z);
                world.chunks.get(&chunk_key).map(|chunk| {
                    let (opaque_vertices, opaque_indices, trans_vertices, trans_indices) =
                        Self::create_vertices_for_chunk(world, chunk);
                    (chunk_key, opaque_vertices, opaque_indices, trans_vertices, trans_indices)
                })
            })
            .collect();

        // Create GPU buffers sequentially (GPU operations must be on main thread)
        for (chunk_key, opaque_vertices, opaque_indices, trans_vertices, trans_indices) in mesh_results {
            if !opaque_vertices.is_empty() {
                let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Opaque Vertex Buffer"),
                    contents: bytemuck::cast_slice(&opaque_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Opaque Index Buffer"),
                    contents: bytemuck::cast_slice(&opaque_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let chunk_mesh = ChunkMesh {
                    vertex_buffer,
                    index_buffer,
                    index_count: opaque_indices.len() as u32,
                };

                self.chunk_meshes_opaque.insert(chunk_key, chunk_mesh);
            }

            if !trans_vertices.is_empty() {
                let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Transparent Vertex Buffer"),
                    contents: bytemuck::cast_slice(&trans_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk Transparent Index Buffer"),
                    contents: bytemuck::cast_slice(&trans_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let chunk_mesh = ChunkMesh {
                    vertex_buffer,
                    index_buffer,
                    index_count: trans_indices.len() as u32,
                };

                self.chunk_meshes_transparent.insert(chunk_key, chunk_mesh);
            }
        }

        for (chunk_x, chunk_z) in &dirty_chunks {
            let chunk_key = (*chunk_x, *chunk_z);
            if let Some(chunk) = world.chunks.get_mut(&chunk_key) {
                chunk.dirty = false;
                chunk.mesh_generated = true;
            }
        }
    }

    /// Render a simple loading screen with progress
    pub fn render_loading_screen(&mut self, progress: f32, message: &str) {
        let output = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(_) => return,
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Loading Screen Encoder"),
        });

        // Clear to dark blue background
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Loading Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Use UI renderer to draw loading text and progress bar
        self.ui_renderer.render_loading_screen(&self.device, &self.queue, &view, &self.config, &self.texture_bind_group, progress, message);

        output.present();
    }

    fn create_vertices_for_chunk(world: &World, chunk: &crate::world::Chunk) -> (Vec<Vertex>, Vec<u16>, Vec<Vertex>, Vec<u16>) {
        let mut opaque_vertices = Vec::new();
        let mut opaque_indices = Vec::new();
        let mut trans_vertices = Vec::new();
        let mut trans_indices = Vec::new();

        let chunk_x_offset = chunk.position.x * World::CHUNK_SIZE as i32;
        let chunk_z_offset = chunk.position.z * World::CHUNK_SIZE as i32;

        // Greedy meshing for each face direction
        // Process Top faces (Y+)
        Self::greedy_mesh_horizontal(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |y| y + 1, // neighbor check direction
            Face::Top,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Process Bottom faces (Y-)
        Self::greedy_mesh_horizontal(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |y| y - 1,
            Face::Bottom,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Process Right faces (X+)
        Self::greedy_mesh_vertical_x(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |x| x + 1,
            Face::Right,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Process Left faces (X-)
        Self::greedy_mesh_vertical_x(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |x| x - 1,
            Face::Left,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Process Front faces (Z+)
        Self::greedy_mesh_vertical_z(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |z| z + 1,
            Face::Front,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Process Back faces (Z-)
        Self::greedy_mesh_vertical_z(
            world, chunk, chunk_x_offset, chunk_z_offset,
            |z| z - 1,
            Face::Back,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Render damaged blocks separately (not greedy meshed) so they show crack effects
        Self::render_damaged_blocks(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
            &mut trans_vertices, &mut trans_indices,
        );

        // Render torches with special geometry
        Self::render_torches(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render slabs with half-height geometry
        Self::render_slabs(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render stairs with L-shaped geometry
        Self::render_stairs(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render ladders with flat panel geometry
        Self::render_ladders(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render trapdoors with horizontal/vertical slab geometry
        Self::render_trapdoors(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render fences with center post and connecting bars
        Self::render_fences(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut opaque_vertices, &mut opaque_indices,
        );

        // Render glass panes with thin connecting panels (transparent)
        Self::render_glass_panes(
            world, chunk, chunk_x_offset, chunk_z_offset,
            &mut trans_vertices, &mut trans_indices,
        );

        (opaque_vertices, opaque_indices, trans_vertices, trans_indices)
    }

    // Render blocks with damage separately so crack effects are visible
    fn render_damaged_blocks(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
        trans_vertices: &mut Vec<Vertex>,
        trans_indices: &mut Vec<u16>,
    ) {
        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch
                       || block_type == BlockType::Ladder || block_type.is_trapdoor() || block_type.is_fence() || block_type == BlockType::GlassPane
                       || block_type.is_bottom_slab() || block_type.is_top_slab() || block_type.is_stairs() {
                        continue;  // Torches, slabs, stairs, ladders, and trapdoors are rendered separately with special geometry
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let damage = world.get_block_damage(world_x, world_y, world_z);
                    if damage <= 0.0 {
                        continue; // Only render damaged blocks here
                    }

                    let hardness = crate::world::World::get_hardness(block_type);
                    let normalized_damage = if hardness > 0.0 { damage / hardness } else { 0.0 };

                    let pos = Vector3::new(world_x as f32, world_y as f32, world_z as f32);
                    let is_transparent = Self::is_transparent(block_type);

                    // Add faces for damaged block (these will overdraw the greedy mesh but show cracks)
                    if Self::is_face_exposed(world, world_x, world_y + 1, world_z, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Top, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Top, block_type, normalized_damage);
                        }
                    }
                    if Self::is_face_exposed(world, world_x, world_y - 1, world_z, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Bottom, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Bottom, block_type, normalized_damage);
                        }
                    }
                    if Self::is_face_exposed(world, world_x + 1, world_y, world_z, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Right, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Right, block_type, normalized_damage);
                        }
                    }
                    if Self::is_face_exposed(world, world_x - 1, world_y, world_z, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Left, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Left, block_type, normalized_damage);
                        }
                    }
                    if Self::is_face_exposed(world, world_x, world_y, world_z + 1, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Front, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Front, block_type, normalized_damage);
                        }
                    }
                    if Self::is_face_exposed(world, world_x, world_y, world_z - 1, block_type) {
                        if is_transparent {
                            Self::add_face(trans_vertices, trans_indices, pos, Face::Back, block_type, normalized_damage);
                        } else {
                            Self::add_face(opaque_vertices, opaque_indices, pos, Face::Back, block_type, normalized_damage);
                        }
                    }
                }
            }
        }
    }

    // Render torches as 3D sticks attached to block faces
    fn render_torches(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        let block_type_f = Self::block_type_to_float(BlockType::Torch);

        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    if chunk.blocks[x][y][z] != BlockType::Torch {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    // Get torch orientation (default to Top if not found)
                    let face = world.get_torch_face(world_x, world_y, world_z)
                        .unwrap_or(TorchFace::Top);

                    // Torch dimensions
                    let torch_width = 0.125;   // 2/16 blocks wide
                    let torch_height = 0.625;  // 10/16 blocks tall
                    let hw = torch_width / 2.0;

                    // Calculate torch base position and tilt based on face
                    match face {
                        TorchFace::Top => {
                            // Standing torch - centered on top of block below
                            let cx = world_x as f32 + 0.5;
                            let cy = world_y as f32;
                            let cz = world_z as f32 + 0.5;

                            Self::add_torch_stick(
                                opaque_vertices, opaque_indices,
                                cx, cy, cz, hw, torch_height,
                                0.0, 0.0,  // No tilt
                                block_type_f,
                            );
                        }
                        TorchFace::North => {
                            // Torch in -Z air block, solid at +Z, tilts toward -Z (toward player)
                            let cx = world_x as f32 + 0.5;
                            let cy = world_y as f32 + 0.15;
                            let cz = world_z as f32 + 0.9;  // Near +Z edge (close to solid)

                            Self::add_torch_stick(
                                opaque_vertices, opaque_indices,
                                cx, cy, cz, hw, torch_height * 0.85,
                                0.0, -0.35,  // Tilt toward -Z
                                block_type_f,
                            );
                        }
                        TorchFace::South => {
                            // Torch in +Z air block, solid at -Z, tilts toward +Z (toward player)
                            let cx = world_x as f32 + 0.5;
                            let cy = world_y as f32 + 0.15;
                            let cz = world_z as f32 + 0.1;  // Near -Z edge (close to solid)

                            Self::add_torch_stick(
                                opaque_vertices, opaque_indices,
                                cx, cy, cz, hw, torch_height * 0.85,
                                0.0, 0.35,  // Tilt toward +Z
                                block_type_f,
                            );
                        }
                        TorchFace::East => {
                            // Torch in +X air block, solid at -X, tilts toward +X (toward player)
                            let cx = world_x as f32 + 0.1;  // Near -X edge (close to solid)
                            let cy = world_y as f32 + 0.15;
                            let cz = world_z as f32 + 0.5;

                            Self::add_torch_stick(
                                opaque_vertices, opaque_indices,
                                cx, cy, cz, hw, torch_height * 0.85,
                                0.35, 0.0,  // Tilt toward +X
                                block_type_f,
                            );
                        }
                        TorchFace::West => {
                            // Torch in -X air block, solid at +X, tilts toward -X (toward player)
                            let cx = world_x as f32 + 0.9;  // Near +X edge (close to solid)
                            let cy = world_y as f32 + 0.15;
                            let cz = world_z as f32 + 0.5;

                            Self::add_torch_stick(
                                opaque_vertices, opaque_indices,
                                cx, cy, cz, hw, torch_height * 0.85,
                                -0.35, 0.0,  // Tilt toward -X
                                block_type_f,
                            );
                        }
                    }
                }
            }
        }
    }

    // Add a 3D torch stick (rectangular prism, optionally tilted) with flame on top
    fn add_torch_stick(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        cx: f32, cy: f32, cz: f32,  // Base center position
        hw: f32,                     // Half-width
        height: f32,                 // Height
        tilt_x: f32, tilt_z: f32,   // Tilt offset at top
        block_type: f32,
    ) {
        // 8 corners of the torch stick
        // Bottom 4 corners
        let b0 = [cx - hw, cy, cz - hw];
        let b1 = [cx + hw, cy, cz - hw];
        let b2 = [cx + hw, cy, cz + hw];
        let b3 = [cx - hw, cy, cz + hw];

        // Top 4 corners (with tilt applied)
        let top_cx = cx + tilt_x;
        let top_cz = cz + tilt_z;
        let top_y = cy + height;
        let t0 = [top_cx - hw, top_y, top_cz - hw];
        let t1 = [top_cx + hw, top_y, top_cz - hw];
        let t2 = [top_cx + hw, top_y, top_cz + hw];
        let t3 = [top_cx - hw, top_y, top_cz + hw];

        // Add 5 faces of the stick (excluding top, flame goes there)
        Self::add_quad_face(vertices, indices, b3, b2, t2, t3, [0.0, 0.0, 1.0], block_type, 0.0);
        Self::add_quad_face(vertices, indices, b1, b0, t0, t1, [0.0, 0.0, -1.0], block_type, 0.0);
        Self::add_quad_face(vertices, indices, b2, b1, t1, t2, [1.0, 0.0, 0.0], block_type, 0.0);
        Self::add_quad_face(vertices, indices, b0, b3, t3, t0, [-1.0, 0.0, 0.0], block_type, 0.0);
        Self::add_quad_face(vertices, indices, b3, b2, b1, b0, [0.0, -1.0, 0.0], block_type, 0.0);

        // Add flame on top of the torch
        let flame_type = 25.0;
        let flame_hw = hw * 1.2;
        let flame_height = 0.15;

        let f_b0 = [top_cx - flame_hw, top_y, top_cz - flame_hw];
        let f_b1 = [top_cx + flame_hw, top_y, top_cz - flame_hw];
        let f_b2 = [top_cx + flame_hw, top_y, top_cz + flame_hw];
        let f_b3 = [top_cx - flame_hw, top_y, top_cz + flame_hw];

        let flame_top_y = top_y + flame_height;
        let f_t0 = [top_cx - flame_hw * 0.5, flame_top_y, top_cz - flame_hw * 0.5];
        let f_t1 = [top_cx + flame_hw * 0.5, flame_top_y, top_cz - flame_hw * 0.5];
        let f_t2 = [top_cx + flame_hw * 0.5, flame_top_y, top_cz + flame_hw * 0.5];
        let f_t3 = [top_cx - flame_hw * 0.5, flame_top_y, top_cz + flame_hw * 0.5];

        // Add flame faces (tapered shape)
        Self::add_quad_face(vertices, indices, f_b3, f_b2, f_t2, f_t3, [0.0, 0.0, 1.0], flame_type, 0.0);
        Self::add_quad_face(vertices, indices, f_b1, f_b0, f_t0, f_t1, [0.0, 0.0, -1.0], flame_type, 0.0);
        Self::add_quad_face(vertices, indices, f_b2, f_b1, f_t1, f_t2, [1.0, 0.0, 0.0], flame_type, 0.0);
        Self::add_quad_face(vertices, indices, f_b0, f_b3, f_t3, f_t0, [-1.0, 0.0, 0.0], flame_type, 0.0);
        Self::add_quad_face(vertices, indices, f_t0, f_t1, f_t2, f_t3, [0.0, 1.0, 0.0], flame_type, 0.0);
        Self::add_quad_face(vertices, indices, f_b3, f_b2, f_b1, f_b0, [0.0, -1.0, 0.0], flame_type, 0.0);
    }

    // Add a single quad face with configurable damage value
    // Use damage = -1.0 for semi-transparent preview blocks
    fn add_quad_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], v3: [f32; 3],
        normal: [f32; 3],
        block_type: f32,
        damage: f32,
    ) {
        let base = vertices.len() as u16;

        let tex_coords = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let positions = [v0, v1, v2, v3];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage,
            });
        }

        // Two triangles for the quad
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
        indices.push(base);
        indices.push(base + 2);
        indices.push(base + 3);
    }

    // Render slabs with half-height geometry
    fn render_slabs(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];

                    let (is_bottom_slab, is_top_slab) = match block_type {
                        BlockType::StoneSlabBottom | BlockType::WoodSlabBottom | BlockType::CobblestoneSlabBottom => (true, false),
                        BlockType::StoneSlabTop | BlockType::WoodSlabTop | BlockType::CobblestoneSlabTop => (false, true),
                        _ => continue,
                    };

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(block_type);

                    // Y offset for slab position
                    let (y_min, y_max) = if is_bottom_slab {
                        (0.0, 0.5)
                    } else {
                        (0.5, 1.0)
                    };

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Add top face
                    let top_exposed = if is_top_slab {
                        Self::is_face_exposed(world, world_x, world_y + 1, world_z, block_type)
                    } else {
                        // Bottom slab top at y+0.5 is exposed if block above is air or top slab
                        world.get_block(world_x, world_y, world_z)
                            .map(|b| b == BlockType::Air || b.is_top_slab())
                            .unwrap_or(true) || Self::is_face_exposed(world, world_x, world_y + 1, world_z, block_type)
                    };
                    if top_exposed {
                        Self::add_slab_face(opaque_vertices, opaque_indices,
                            base_x, base_y + y_max, base_z, Face::Top, block_type_f);
                    }

                    // Add bottom face
                    let bottom_exposed = if is_bottom_slab {
                        Self::is_face_exposed(world, world_x, world_y - 1, world_z, block_type)
                    } else {
                        // Top slab bottom at y+0.5 is exposed if block is air or bottom slab
                        world.get_block(world_x, world_y, world_z)
                            .map(|b| b == BlockType::Air || b.is_bottom_slab())
                            .unwrap_or(true) || Self::is_face_exposed(world, world_x, world_y - 1, world_z, block_type)
                    };
                    if bottom_exposed {
                        Self::add_slab_face(opaque_vertices, opaque_indices,
                            base_x, base_y + y_min, base_z, Face::Bottom, block_type_f);
                    }

                    // Add side faces (half height)
                    // Right (+X)
                    if Self::is_face_exposed(world, world_x + 1, world_y, world_z, block_type) {
                        Self::add_slab_side_face(opaque_vertices, opaque_indices,
                            base_x + 1.0, base_y + y_min, base_z, 0.5, Face::Right, block_type_f);
                    }
                    // Left (-X)
                    if Self::is_face_exposed(world, world_x - 1, world_y, world_z, block_type) {
                        Self::add_slab_side_face(opaque_vertices, opaque_indices,
                            base_x, base_y + y_min, base_z, 0.5, Face::Left, block_type_f);
                    }
                    // Front (+Z)
                    if Self::is_face_exposed(world, world_x, world_y, world_z + 1, block_type) {
                        Self::add_slab_side_face(opaque_vertices, opaque_indices,
                            base_x, base_y + y_min, base_z + 1.0, 0.5, Face::Front, block_type_f);
                    }
                    // Back (-Z)
                    if Self::is_face_exposed(world, world_x, world_y, world_z - 1, block_type) {
                        Self::add_slab_side_face(opaque_vertices, opaque_indices,
                            base_x, base_y + y_min, base_z, 0.5, Face::Back, block_type_f);
                    }
                }
            }
        }
    }

    // Add a horizontal slab face (top or bottom)
    fn add_slab_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        face: Face,
        block_type: f32,
    ) {
        let base_index = vertices.len() as u16;
        let (positions, normal) = match face {
            Face::Top => (
                [[x, y, z], [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x, y, z + 1.0]],
                [0.0, 1.0, 0.0]
            ),
            Face::Bottom => (
                [[x, y, z], [x, y, z + 1.0], [x + 1.0, y, z + 1.0], [x + 1.0, y, z]],
                [0.0, -1.0, 0.0]
            ),
            _ => return,
        };

        let tex_coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    // Add a vertical slab side face with specified height
    fn add_slab_side_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,  // Base position
        height: f32,
        face: Face,
        block_type: f32,
    ) {
        let base_index = vertices.len() as u16;
        let (positions, normal) = match face {
            Face::Right => (  // +X face
                [[x, y, z], [x, y, z + 1.0], [x, y + height, z + 1.0], [x, y + height, z]],
                [1.0, 0.0, 0.0]
            ),
            Face::Left => (   // -X face
                [[x, y, z], [x, y + height, z], [x, y + height, z + 1.0], [x, y, z + 1.0]],
                [-1.0, 0.0, 0.0]
            ),
            Face::Front => (  // +Z face
                [[x, y, z], [x, y + height, z], [x + 1.0, y + height, z], [x + 1.0, y, z]],
                [0.0, 0.0, 1.0]
            ),
            Face::Back => (   // -Z face
                [[x, y, z], [x + 1.0, y, z], [x + 1.0, y + height, z], [x, y + height, z]],
                [0.0, 0.0, -1.0]
            ),
            _ => return,
        };

        // Tex coords: bottom half of texture for half-height slab
        let tex_coords = [[0.0, 1.0], [0.0, 0.5], [1.0, 0.5], [1.0, 1.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    // Render stairs with L-shaped geometry
    fn render_stairs(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        use crate::world::BlockFacing;

        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];

                    if !block_type.is_stairs() {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(block_type);

                    // Get stair data (default: facing north, not upside down)
                    let stair_data = world.get_stair_data(world_x, world_y, world_z);
                    let facing = stair_data.map(|d| d.facing).unwrap_or(BlockFacing::North);
                    let upside_down = stair_data.map(|d| d.upside_down).unwrap_or(false);

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Stair geometry: bottom half (full) + back half (half height)
                    // For normal stairs (not upside down):
                    //   Bottom part: y=0 to y=0.5, full XZ
                    //   Top part: y=0.5 to y=1.0, back half (depends on facing)
                    // For upside down:
                    //   Top part: y=0.5 to y=1.0, full XZ
                    //   Bottom part: y=0 to y=0.5, back half

                    if !upside_down {
                        // Normal stairs
                        Self::add_stair_normal(opaque_vertices, opaque_indices,
                            base_x, base_y, base_z, facing, block_type_f, world, world_x, world_y, world_z);
                    } else {
                        // Upside down stairs
                        Self::add_stair_upside_down(opaque_vertices, opaque_indices,
                            base_x, base_y, base_z, facing, block_type_f, world, world_x, world_y, world_z);
                    }
                }
            }
        }
    }

    fn add_stair_normal(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
        world: &World,
        world_x: i32, world_y: i32, world_z: i32,
    ) {
        // Bottom half (y=0 to y=0.5) - always full width
        // Top face of bottom half at y+0.5 (partially exposed based on facing)
        // This is the step surface

        // Bottom face at y=0
        if Self::is_face_exposed(world, world_x, world_y - 1, world_z, BlockType::Stone) {
            Self::add_slab_face(vertices, indices, x, y, z, Face::Bottom, block_type);
        }

        // Step surface (top of bottom half) - full width
        Self::add_slab_face(vertices, indices, x, y + 0.5, z, Face::Top, block_type);

        // Top of upper half
        if Self::is_face_exposed(world, world_x, world_y + 1, world_z, BlockType::Stone) {
            // Partial top based on facing
            Self::add_stair_partial_top(vertices, indices, x, y + 1.0, z, facing, block_type);
        }

        // Side faces for bottom half (0-0.5) and upper half (0.5-1.0)
        Self::add_stair_side_faces(vertices, indices, x, y, z, facing, block_type, world, world_x, world_y, world_z, false);
    }

    fn add_stair_upside_down(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
        world: &World,
        world_x: i32, world_y: i32, world_z: i32,
    ) {
        // Top half (y=0.5 to y=1.0) - always full width
        // Bottom face of top half at y+0.5 (the step surface)

        // Top face at y=1.0
        if Self::is_face_exposed(world, world_x, world_y + 1, world_z, BlockType::Stone) {
            Self::add_slab_face(vertices, indices, x, y + 1.0, z, Face::Top, block_type);
        }

        // Step surface (bottom of top half) - full width
        Self::add_slab_face(vertices, indices, x, y + 0.5, z, Face::Bottom, block_type);

        // Bottom of lower half
        if Self::is_face_exposed(world, world_x, world_y - 1, world_z, BlockType::Stone) {
            Self::add_stair_partial_bottom(vertices, indices, x, y, z, facing, block_type);
        }

        // Side faces
        Self::add_stair_side_faces(vertices, indices, x, y, z, facing, block_type, world, world_x, world_y, world_z, true);
    }

    fn add_stair_partial_top(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
    ) {
        use crate::world::BlockFacing;
        let base_index = vertices.len() as u16;

        // Top face for back half only
        let positions = match facing {
            BlockFacing::North => [[x, y, z], [x + 1.0, y, z], [x + 1.0, y, z + 0.5], [x, y, z + 0.5]],
            BlockFacing::South => [[x, y, z + 0.5], [x + 1.0, y, z + 0.5], [x + 1.0, y, z + 1.0], [x, y, z + 1.0]],
            BlockFacing::East => [[x + 0.5, y, z], [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x + 0.5, y, z + 1.0]],
            BlockFacing::West => [[x, y, z], [x + 0.5, y, z], [x + 0.5, y, z + 1.0], [x, y, z + 1.0]],
        };

        let tex_coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.5], [0.0, 0.5]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal: [0.0, 1.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    fn add_stair_partial_bottom(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
    ) {
        use crate::world::BlockFacing;
        let base_index = vertices.len() as u16;

        // Bottom face for back half only
        let positions = match facing {
            BlockFacing::North => [[x, y, z], [x, y, z + 0.5], [x + 1.0, y, z + 0.5], [x + 1.0, y, z]],
            BlockFacing::South => [[x, y, z + 0.5], [x, y, z + 1.0], [x + 1.0, y, z + 1.0], [x + 1.0, y, z + 0.5]],
            BlockFacing::East => [[x + 0.5, y, z], [x + 0.5, y, z + 1.0], [x + 1.0, y, z + 1.0], [x + 1.0, y, z]],
            BlockFacing::West => [[x, y, z], [x, y, z + 1.0], [x + 0.5, y, z + 1.0], [x + 0.5, y, z]],
        };

        let tex_coords = [[0.0, 0.0], [0.0, 0.5], [1.0, 0.5], [1.0, 0.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal: [0.0, -1.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    fn add_stair_side_faces(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
        world: &World,
        world_x: i32, world_y: i32, world_z: i32,
        upside_down: bool,
    ) {
        // Full height side faces
        if Self::is_face_exposed(world, world_x + 1, world_y, world_z, BlockType::Stone) {
            Self::add_stair_side(vertices, indices, x, y, z, facing, Face::Right, block_type, upside_down);
        }
        if Self::is_face_exposed(world, world_x - 1, world_y, world_z, BlockType::Stone) {
            Self::add_stair_side(vertices, indices, x, y, z, facing, Face::Left, block_type, upside_down);
        }
        if Self::is_face_exposed(world, world_x, world_y, world_z + 1, BlockType::Stone) {
            Self::add_stair_side(vertices, indices, x, y, z, facing, Face::Front, block_type, upside_down);
        }
        if Self::is_face_exposed(world, world_x, world_y, world_z - 1, BlockType::Stone) {
            Self::add_stair_side(vertices, indices, x, y, z, facing, Face::Back, block_type, upside_down);
        }

        // Inner step face (vertical face of the step)
        Self::add_stair_step_face(vertices, indices, x, y, z, facing, block_type, upside_down);
    }

    fn add_stair_side(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        stair_facing: crate::world::BlockFacing,
        side_face: Face,
        block_type: f32,
        upside_down: bool,
    ) {
        use crate::world::BlockFacing;

        // Determine if this side needs an L-shaped cutout based on stair facing
        let needs_cutout = match (stair_facing, side_face) {
            (BlockFacing::North, Face::Right | Face::Left) => true,
            (BlockFacing::South, Face::Right | Face::Left) => true,
            (BlockFacing::East, Face::Front | Face::Back) => true,
            (BlockFacing::West, Face::Front | Face::Back) => true,
            _ => false,
        };

        if needs_cutout {
            // L-shaped side face - render as two quads
            Self::add_stair_l_side(vertices, indices, x, y, z, stair_facing, side_face, block_type, upside_down);
        } else {
            // Full rectangle side face
            Self::add_slab_side_face(vertices, indices,
                match side_face {
                    Face::Right => x + 1.0,
                    _ => x,
                },
                y,
                match side_face {
                    Face::Front => z + 1.0,
                    _ => z,
                },
                1.0, side_face, block_type);
        }
    }

    fn add_stair_l_side(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        stair_facing: crate::world::BlockFacing,
        side_face: Face,
        block_type: f32,
        upside_down: bool,
    ) {
        // For L-shaped sides, we need two rectangles
        // Bottom part (full width, half height) and top part (half width, half height)

        let (y_low, y_mid, y_high) = if upside_down {
            (y, y + 0.5, y + 1.0)
        } else {
            (y, y + 0.5, y + 1.0)
        };

        let base_index = vertices.len() as u16;

        // Simplified L-shape: just render full side for now (can be refined later)
        let (positions, normal): ([[f32; 3]; 4], [f32; 3]) = match side_face {
            Face::Right => (
                [[x + 1.0, y_low, z], [x + 1.0, y_low, z + 1.0], [x + 1.0, y_high, z + 1.0], [x + 1.0, y_high, z]],
                [1.0, 0.0, 0.0]
            ),
            Face::Left => (
                [[x, y_low, z], [x, y_high, z], [x, y_high, z + 1.0], [x, y_low, z + 1.0]],
                [-1.0, 0.0, 0.0]
            ),
            Face::Front => (
                [[x, y_low, z + 1.0], [x, y_high, z + 1.0], [x + 1.0, y_high, z + 1.0], [x + 1.0, y_low, z + 1.0]],
                [0.0, 0.0, 1.0]
            ),
            Face::Back => (
                [[x, y_low, z], [x + 1.0, y_low, z], [x + 1.0, y_high, z], [x, y_high, z]],
                [0.0, 0.0, -1.0]
            ),
            _ => return,
        };

        let tex_coords = [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    fn add_stair_step_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        block_type: f32,
        upside_down: bool,
    ) {
        use crate::world::BlockFacing;
        let base_index = vertices.len() as u16;

        let step_y = if upside_down { y + 0.5 } else { y + 0.5 };

        // Vertical face at the step edge
        let (positions, normal): ([[f32; 3]; 4], [f32; 3]) = match facing {
            BlockFacing::North => (
                [[x, step_y, z + 0.5], [x + 1.0, step_y, z + 0.5],
                 [x + 1.0, step_y + 0.5, z + 0.5], [x, step_y + 0.5, z + 0.5]],
                [0.0, 0.0, 1.0]
            ),
            BlockFacing::South => (
                [[x, step_y, z + 0.5], [x, step_y + 0.5, z + 0.5],
                 [x + 1.0, step_y + 0.5, z + 0.5], [x + 1.0, step_y, z + 0.5]],
                [0.0, 0.0, -1.0]
            ),
            BlockFacing::East => (
                [[x + 0.5, step_y, z], [x + 0.5, step_y + 0.5, z],
                 [x + 0.5, step_y + 0.5, z + 1.0], [x + 0.5, step_y, z + 1.0]],
                [-1.0, 0.0, 0.0]
            ),
            BlockFacing::West => (
                [[x + 0.5, step_y, z], [x + 0.5, step_y, z + 1.0],
                 [x + 0.5, step_y + 0.5, z + 1.0], [x + 0.5, step_y + 0.5, z]],
                [1.0, 0.0, 0.0]
            ),
        };

        let tex_coords = [[0.0, 0.5], [1.0, 0.5], [1.0, 0.0], [0.0, 0.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);
    }

    // Render ladders as flat panels against walls
    fn render_ladders(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    if chunk.blocks[x][y][z] != BlockType::Ladder {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(BlockType::Ladder);

                    // Get ladder facing from torch_orientations
                    let face = world.get_torch_face(world_x, world_y, world_z)
                        .unwrap_or(TorchFace::North);

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Ladder is a thin panel 0.125 blocks from the wall
                    let offset = 0.125;

                    Self::add_ladder_panel(opaque_vertices, opaque_indices,
                        base_x, base_y, base_z, face, offset, block_type_f);
                }
            }
        }
    }

    fn add_ladder_panel(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        face: TorchFace,
        offset: f32,
        block_type: f32,
    ) {
        let base_index = vertices.len() as u16;

        // Position the ladder panel slightly away from the wall
        let (positions, normal): ([[f32; 3]; 4], [f32; 3]) = match face {
            TorchFace::North => (
                // Ladder on +Z wall, facing -Z
                [[x, y, z + 1.0 - offset], [x + 1.0, y, z + 1.0 - offset],
                 [x + 1.0, y + 1.0, z + 1.0 - offset], [x, y + 1.0, z + 1.0 - offset]],
                [0.0, 0.0, -1.0]
            ),
            TorchFace::South => (
                // Ladder on -Z wall, facing +Z
                [[x, y, z + offset], [x, y + 1.0, z + offset],
                 [x + 1.0, y + 1.0, z + offset], [x + 1.0, y, z + offset]],
                [0.0, 0.0, 1.0]
            ),
            TorchFace::East => (
                // Ladder on -X wall, facing +X
                [[x + offset, y, z], [x + offset, y + 1.0, z],
                 [x + offset, y + 1.0, z + 1.0], [x + offset, y, z + 1.0]],
                [1.0, 0.0, 0.0]
            ),
            TorchFace::West => (
                // Ladder on +X wall, facing -X
                [[x + 1.0 - offset, y, z], [x + 1.0 - offset, y, z + 1.0],
                 [x + 1.0 - offset, y + 1.0, z + 1.0], [x + 1.0 - offset, y + 1.0, z]],
                [-1.0, 0.0, 0.0]
            ),
            TorchFace::Top => return, // Ladders don't have a top placement
        };

        let tex_coords = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);

        // Add back face as well (so ladder is visible from both sides)
        let back_base = vertices.len() as u16;
        let back_normal = [-normal[0], -normal[1], -normal[2]];

        for i in (0..4).rev() {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[3 - i],
                normal: back_normal,
                block_type,
                damage: 0.0,
            });
        }

        indices.extend_from_slice(&[
            back_base, back_base + 1, back_base + 2,
            back_base + 2, back_base + 3, back_base,
        ]);
    }

    // Render trapdoors as horizontal slabs (closed) or vertical panels (open)
    fn render_trapdoors(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        use crate::world::BlockFacing;

        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];
                    if !block_type.is_trapdoor() {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(block_type);

                    // Get trapdoor data (open state, facing, top/bottom)
                    let trapdoor_data = world.get_trapdoor_data(world_x, world_y, world_z);
                    let (open, facing, top_half) = if let Some(data) = trapdoor_data {
                        (data.open, data.facing, data.top_half)
                    } else {
                        (false, BlockFacing::North, false)
                    };

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Trapdoor thickness
                    let thickness = 0.1875;

                    if open {
                        // Open trapdoor: vertical panel against hinge side
                        Self::add_trapdoor_open(opaque_vertices, opaque_indices,
                            base_x, base_y, base_z, facing, thickness, block_type_f);
                    } else {
                        // Closed trapdoor: horizontal slab
                        Self::add_trapdoor_closed(opaque_vertices, opaque_indices,
                            base_x, base_y, base_z, top_half, thickness, block_type_f);
                    }
                }
            }
        }
    }

    // Add closed trapdoor mesh (horizontal slab)
    fn add_trapdoor_closed(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        top_half: bool,
        thickness: f32,
        block_type: f32,
    ) {
        let (y_min, y_max) = if top_half {
            (y + 1.0 - thickness, y + 1.0)
        } else {
            (y, y + thickness)
        };

        let base_index = vertices.len() as u16;

        // Top face
        let top_positions = [
            [x, y_max, z], [x, y_max, z + 1.0],
            [x + 1.0, y_max, z + 1.0], [x + 1.0, y_max, z],
        ];
        let tex_coords = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        for i in 0..4 {
            vertices.push(Vertex {
                position: top_positions[i],
                tex_coords: tex_coords[i],
                normal: [0.0, 1.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);

        // Bottom face
        let bottom_base = vertices.len() as u16;
        let bottom_positions = [
            [x, y_min, z], [x + 1.0, y_min, z],
            [x + 1.0, y_min, z + 1.0], [x, y_min, z + 1.0],
        ];
        for i in 0..4 {
            vertices.push(Vertex {
                position: bottom_positions[i],
                tex_coords: tex_coords[i],
                normal: [0.0, -1.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            bottom_base, bottom_base + 1, bottom_base + 2,
            bottom_base + 2, bottom_base + 3, bottom_base,
        ]);

        // Side faces (4 sides)
        // North face (-Z)
        let north_base = vertices.len() as u16;
        let north_positions = [
            [x, y_min, z], [x, y_max, z],
            [x + 1.0, y_max, z], [x + 1.0, y_min, z],
        ];
        let side_tex = [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        for i in 0..4 {
            vertices.push(Vertex {
                position: north_positions[i],
                tex_coords: side_tex[i],
                normal: [0.0, 0.0, -1.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            north_base, north_base + 1, north_base + 2,
            north_base + 2, north_base + 3, north_base,
        ]);

        // South face (+Z)
        let south_base = vertices.len() as u16;
        let south_positions = [
            [x, y_min, z + 1.0], [x + 1.0, y_min, z + 1.0],
            [x + 1.0, y_max, z + 1.0], [x, y_max, z + 1.0],
        ];
        for i in 0..4 {
            vertices.push(Vertex {
                position: south_positions[i],
                tex_coords: side_tex[i],
                normal: [0.0, 0.0, 1.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            south_base, south_base + 1, south_base + 2,
            south_base + 2, south_base + 3, south_base,
        ]);

        // West face (-X)
        let west_base = vertices.len() as u16;
        let west_positions = [
            [x, y_min, z], [x, y_min, z + 1.0],
            [x, y_max, z + 1.0], [x, y_max, z],
        ];
        for i in 0..4 {
            vertices.push(Vertex {
                position: west_positions[i],
                tex_coords: side_tex[i],
                normal: [-1.0, 0.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            west_base, west_base + 1, west_base + 2,
            west_base + 2, west_base + 3, west_base,
        ]);

        // East face (+X)
        let east_base = vertices.len() as u16;
        let east_positions = [
            [x + 1.0, y_min, z], [x + 1.0, y_max, z],
            [x + 1.0, y_max, z + 1.0], [x + 1.0, y_min, z + 1.0],
        ];
        for i in 0..4 {
            vertices.push(Vertex {
                position: east_positions[i],
                tex_coords: side_tex[i],
                normal: [1.0, 0.0, 0.0],
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            east_base, east_base + 1, east_base + 2,
            east_base + 2, east_base + 3, east_base,
        ]);
    }

    // Add open trapdoor mesh (vertical panel against hinge side)
    fn add_trapdoor_open(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        facing: crate::world::BlockFacing,
        thickness: f32,
        block_type: f32,
    ) {
        use crate::world::BlockFacing;

        // Open trapdoor is a vertical panel against the hinge side
        let (positions, normal): ([[f32; 3]; 8], [f32; 3]) = match facing {
            BlockFacing::North => {
                // Hinge on +Z side, opens toward -Z
                let z_pos = z + 1.0 - thickness;
                ([
                    // Front face
                    [x, y, z_pos], [x + 1.0, y, z_pos], [x + 1.0, y + 1.0, z_pos], [x, y + 1.0, z_pos],
                    // Back face
                    [x, y, z + 1.0], [x, y + 1.0, z + 1.0], [x + 1.0, y + 1.0, z + 1.0], [x + 1.0, y, z + 1.0],
                ], [0.0, 0.0, -1.0])
            }
            BlockFacing::South => {
                // Hinge on -Z side, opens toward +Z
                let z_pos = z + thickness;
                ([
                    // Front face
                    [x, y, z_pos], [x, y + 1.0, z_pos], [x + 1.0, y + 1.0, z_pos], [x + 1.0, y, z_pos],
                    // Back face
                    [x, y, z], [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z],
                ], [0.0, 0.0, 1.0])
            }
            BlockFacing::East => {
                // Hinge on -X side, opens toward +X
                let x_pos = x + thickness;
                ([
                    // Front face
                    [x_pos, y, z], [x_pos, y + 1.0, z], [x_pos, y + 1.0, z + 1.0], [x_pos, y, z + 1.0],
                    // Back face
                    [x, y, z], [x, y, z + 1.0], [x, y + 1.0, z + 1.0], [x, y + 1.0, z],
                ], [1.0, 0.0, 0.0])
            }
            BlockFacing::West => {
                // Hinge on +X side, opens toward -X
                let x_pos = x + 1.0 - thickness;
                ([
                    // Front face
                    [x_pos, y, z], [x_pos, y, z + 1.0], [x_pos, y + 1.0, z + 1.0], [x_pos, y + 1.0, z],
                    // Back face
                    [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x + 1.0, y + 1.0, z + 1.0], [x + 1.0, y, z + 1.0],
                ], [-1.0, 0.0, 0.0])
            }
        };

        let base_index = vertices.len() as u16;
        let tex_coords = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        // Front face (first 4 vertices)
        for i in 0..4 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i],
                normal,
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            base_index, base_index + 1, base_index + 2,
            base_index + 2, base_index + 3, base_index,
        ]);

        // Back face (vertices 4-7)
        let back_base = vertices.len() as u16;
        let back_normal = [-normal[0], -normal[1], -normal[2]];
        for i in 4..8 {
            vertices.push(Vertex {
                position: positions[i],
                tex_coords: tex_coords[i - 4],
                normal: back_normal,
                block_type,
                damage: 0.0,
            });
        }
        indices.extend_from_slice(&[
            back_base, back_base + 1, back_base + 2,
            back_base + 2, back_base + 3, back_base,
        ]);
    }

    // Render fences with center post and connecting bars
    fn render_fences(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
    ) {
        use crate::world::BlockFacing;

        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];
                    if !block_type.is_fence() {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(block_type);

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Center post dimensions: 0.25 x 1.0 x 0.25
                    let post_min = 0.375;
                    let post_max = 0.625;

                    // Add center post (always present)
                    Self::add_fence_post(opaque_vertices, opaque_indices,
                        base_x + post_min, base_y, base_z + post_min,
                        post_max - post_min, 1.0, post_max - post_min,
                        block_type_f);

                    // Check connections in each direction and add bars
                    let connects_north = world.fence_connects(world_x, world_y, world_z, BlockFacing::North);
                    let connects_south = world.fence_connects(world_x, world_y, world_z, BlockFacing::South);
                    let connects_east = world.fence_connects(world_x, world_y, world_z, BlockFacing::East);
                    let connects_west = world.fence_connects(world_x, world_y, world_z, BlockFacing::West);

                    // Add connecting bars at y=0.375 (lower) and y=0.75 (upper)
                    let bar_height = 0.125;
                    let bar_width = 0.125;
                    let bar_y_positions = [0.375, 0.75];

                    for &bar_y in &bar_y_positions {
                        // North connection (+Z)
                        if connects_north {
                            Self::add_fence_post(opaque_vertices, opaque_indices,
                                base_x + post_min, base_y + bar_y, base_z + post_max,
                                bar_width, bar_height, 1.0 - post_max,
                                block_type_f);
                        }

                        // South connection (-Z)
                        if connects_south {
                            Self::add_fence_post(opaque_vertices, opaque_indices,
                                base_x + post_min, base_y + bar_y, base_z,
                                bar_width, bar_height, post_min,
                                block_type_f);
                        }

                        // East connection (+X)
                        if connects_east {
                            Self::add_fence_post(opaque_vertices, opaque_indices,
                                base_x + post_max, base_y + bar_y, base_z + post_min,
                                1.0 - post_max, bar_height, bar_width,
                                block_type_f);
                        }

                        // West connection (-X)
                        if connects_west {
                            Self::add_fence_post(opaque_vertices, opaque_indices,
                                base_x, base_y + bar_y, base_z + post_min,
                                post_min, bar_height, bar_width,
                                block_type_f);
                        }
                    }
                }
            }
        }
    }

    // Add a fence post (vertical beam)
    fn add_fence_post(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        width: f32, height: f32, depth: f32,
        block_type: f32,
    ) {
        let base_index = vertices.len() as u16;

        // Define the 8 corners
        let corners = [
            [x, y, z],                             // 0: bottom-front-left
            [x + width, y, z],                     // 1: bottom-front-right
            [x + width, y, z + depth],             // 2: bottom-back-right
            [x, y, z + depth],                     // 3: bottom-back-left
            [x, y + height, z],                    // 4: top-front-left
            [x + width, y + height, z],            // 5: top-front-right
            [x + width, y + height, z + depth],    // 6: top-back-right
            [x, y + height, z + depth],            // 7: top-back-left
        ];

        // 6 faces: top, bottom, front, back, left, right
        let faces = [
            // (corner indices, normal)
            ([4, 5, 6, 7], [0.0f32, 1.0, 0.0]),   // top
            ([0, 3, 2, 1], [0.0, -1.0, 0.0]),     // bottom
            ([0, 1, 5, 4], [0.0, 0.0, -1.0]),     // front (-Z)
            ([2, 3, 7, 6], [0.0, 0.0, 1.0]),      // back (+Z)
            ([3, 0, 4, 7], [-1.0, 0.0, 0.0]),     // left (-X)
            ([1, 2, 6, 5], [1.0, 0.0, 0.0]),      // right (+X)
        ];

        let tex_coords = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

        for (corner_indices, normal) in &faces {
            let face_base = vertices.len() as u16;
            for (i, &ci) in corner_indices.iter().enumerate() {
                vertices.push(Vertex {
                    position: corners[ci],
                    tex_coords: tex_coords[i],
                    normal: *normal,
                    block_type,
                    damage: 0.0,
                });
            }
            indices.extend_from_slice(&[
                face_base, face_base + 1, face_base + 2,
                face_base + 2, face_base + 3, face_base,
            ]);
        }
    }

    // Render glass panes with thin connecting panels
    fn render_glass_panes(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        trans_vertices: &mut Vec<Vertex>,
        trans_indices: &mut Vec<u16>,
    ) {
        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    if chunk.blocks[x][y][z] != BlockType::GlassPane {
                        continue;
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_y = y as i32;
                    let world_z = chunk_z_offset + z as i32;

                    let block_type_f = Self::block_type_to_float(BlockType::GlassPane);

                    let base_x = world_x as f32;
                    let base_y = world_y as f32;
                    let base_z = world_z as f32;

                    // Get connections in each direction
                    let (north, south, east, west) = world.pane_connections(world_x, world_y, world_z);

                    // Pane thickness
                    let thickness = 0.125;
                    let half_thick = thickness / 2.0;
                    let center = 0.5;

                    // Check if panes form a straight line (no center post needed)
                    let is_straight_line = (north && south && !east && !west) || (east && west && !north && !south);

                    // Add center post unless panes form a straight line
                    if !is_straight_line {
                        Self::add_fence_post(trans_vertices, trans_indices,
                            base_x + center - half_thick, base_y, base_z + center - half_thick,
                            thickness, 1.0, thickness,
                            block_type_f);
                    }

                    // Add panels to connected sides
                    if north {
                        Self::add_fence_post(trans_vertices, trans_indices,
                            base_x + center - half_thick, base_y, base_z + center,
                            thickness, 1.0, 0.5,
                            block_type_f);
                    }

                    if south {
                        Self::add_fence_post(trans_vertices, trans_indices,
                            base_x + center - half_thick, base_y, base_z,
                            thickness, 1.0, center,
                            block_type_f);
                    }

                    if east {
                        Self::add_fence_post(trans_vertices, trans_indices,
                            base_x + center, base_y, base_z + center - half_thick,
                            0.5, 1.0, thickness,
                            block_type_f);
                    }

                    if west {
                        Self::add_fence_post(trans_vertices, trans_indices,
                            base_x, base_y, base_z + center - half_thick,
                            center, 1.0, thickness,
                            block_type_f);
                    }
                }
            }
        }
    }

    // Greedy mesh for horizontal faces (top/bottom) - iterates Y layers, merges in XZ plane
    fn greedy_mesh_horizontal<F>(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        neighbor_y: F,
        face: Face,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
        trans_vertices: &mut Vec<Vertex>,
        trans_indices: &mut Vec<u16>,
    ) where F: Fn(i32) -> i32 {
        let size = World::CHUNK_SIZE;
        let height = World::CHUNK_HEIGHT;

        for y in 0..height {
            // Build mask of exposed faces at this Y level
            let mut mask: [[Option<BlockType>; 16]; 16] = [[None; 16]; 16];

            for x in 0..size {
                for z in 0..size {
                    let block_type = chunk.blocks[x][y][z];
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch
                       || block_type == BlockType::Ladder || block_type.is_trapdoor() || block_type.is_fence() || block_type == BlockType::GlassPane
                       || block_type.is_bottom_slab() || block_type.is_top_slab() || block_type.is_stairs() {
                        continue;  // Torches, slabs, stairs, ladders, and trapdoors are rendered separately with special geometry
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_z = chunk_z_offset + z as i32;
                    let check_y = neighbor_y(y as i32);

                    // Skip damaged blocks - they're rendered separately with crack effects
                    if world.get_block_damage(world_x, y as i32, world_z) > 0.0 {
                        continue;
                    }

                    if Self::is_face_exposed(world, world_x, check_y, world_z, block_type) {
                        mask[x][z] = Some(block_type);
                    }
                }
            }

            // Greedy merge
            let mut visited = [[false; 16]; 16];

            for start_x in 0..size {
                for start_z in 0..size {
                    if visited[start_x][start_z] || mask[start_x][start_z].is_none() {
                        continue;
                    }

                    let block_type = mask[start_x][start_z].unwrap();
                    let is_water = block_type == BlockType::Water;

                    // Water blocks skip merging - each needs individual depth value
                    let (width, depth) = if is_water {
                        (1, 1)
                    } else {
                        // Find width (extend in X)
                        let mut w = 1;
                        while start_x + w < size
                            && !visited[start_x + w][start_z]
                            && mask[start_x + w][start_z] == Some(block_type)
                        {
                            w += 1;
                        }

                        // Find depth (extend in Z)
                        let mut d = 1;
                        'outer: while start_z + d < size {
                            for dx in 0..w {
                                if visited[start_x + dx][start_z + d]
                                    || mask[start_x + dx][start_z + d] != Some(block_type)
                                {
                                    break 'outer;
                                }
                            }
                            d += 1;
                        }
                        (w, d)
                    };

                    // Mark as visited
                    for dx in 0..width {
                        for dz in 0..depth {
                            visited[start_x + dx][start_z + dz] = true;
                        }
                    }

                    // Generate merged quad
                    let world_x = chunk_x_offset + start_x as i32;
                    let world_z = chunk_z_offset + start_z as i32;
                    let water_depth = if is_water {
                        world.get_water_depth(world_x, y as i32, world_z) as f32
                    } else {
                        0.0
                    };

                    // Water uses transparent buffer, everything else uses opaque
                    if is_water {
                        // Get water level for variable height (8 = source/full, 1-7 = flowing)
                        let water_level = world.get_water_level(world_x, y as i32, world_z);
                        Self::add_greedy_face_horizontal_water(
                            trans_vertices, trans_indices,
                            world_x as f32, y as f32, world_z as f32,
                            width as f32, depth as f32,
                            face, block_type, water_depth, water_level,
                        );
                    } else {
                        Self::add_greedy_face_horizontal(
                            opaque_vertices, opaque_indices,
                            world_x as f32, y as f32, world_z as f32,
                            width as f32, depth as f32,
                            face, block_type, water_depth,
                        );
                    }
                }
            }
        }
    }

    // Greedy mesh for vertical X faces (left/right) - iterates X layers, merges in YZ plane
    fn greedy_mesh_vertical_x<F>(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        neighbor_x: F,
        face: Face,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
        trans_vertices: &mut Vec<Vertex>,
        trans_indices: &mut Vec<u16>,
    ) where F: Fn(i32) -> i32 {
        let size = World::CHUNK_SIZE;
        let height = World::CHUNK_HEIGHT;

        for x in 0..size {
            // Build mask of exposed faces at this X level
            let mut mask: Vec<Vec<Option<BlockType>>> = vec![vec![None; size]; height];

            for y in 0..height {
                for z in 0..size {
                    let block_type = chunk.blocks[x][y][z];
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch
                       || block_type == BlockType::Ladder || block_type.is_trapdoor() || block_type.is_fence() || block_type == BlockType::GlassPane
                       || block_type.is_bottom_slab() || block_type.is_top_slab() || block_type.is_stairs() {
                        continue;  // Torches, slabs, stairs, ladders, and trapdoors are rendered separately with special geometry
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_z = chunk_z_offset + z as i32;
                    let check_x = neighbor_x(world_x);

                    // Skip damaged blocks - they're rendered separately with crack effects
                    if world.get_block_damage(world_x, y as i32, world_z) > 0.0 {
                        continue;
                    }

                    if Self::is_face_exposed(world, check_x, y as i32, world_z, block_type) {
                        mask[y][z] = Some(block_type);
                    }
                }
            }

            // Greedy merge in YZ plane
            let mut visited: Vec<Vec<bool>> = vec![vec![false; size]; height];

            for start_y in 0..height {
                for start_z in 0..size {
                    if visited[start_y][start_z] || mask[start_y][start_z].is_none() {
                        continue;
                    }

                    let block_type = mask[start_y][start_z].unwrap();
                    let is_transparent = Self::is_transparent(block_type);

                    // Find width (extend in Z)
                    let mut width = 1;
                    while start_z + width < size
                        && !visited[start_y][start_z + width]
                        && mask[start_y][start_z + width] == Some(block_type)
                    {
                        width += 1;
                    }

                    // Find height (extend in Y)
                    let mut h = 1;
                    'outer: while start_y + h < height {
                        for dz in 0..width {
                            if visited[start_y + h][start_z + dz]
                                || mask[start_y + h][start_z + dz] != Some(block_type)
                            {
                                break 'outer;
                            }
                        }
                        h += 1;
                    }

                    // Mark as visited
                    for dy in 0..h {
                        for dz in 0..width {
                            visited[start_y + dy][start_z + dz] = true;
                        }
                    }

                    // Generate merged quad
                    let world_x = chunk_x_offset + x as i32;
                    let world_z = chunk_z_offset + start_z as i32;

                    if is_transparent {
                        Self::add_greedy_face_vertical_x(
                            trans_vertices, trans_indices,
                            world_x as f32, start_y as f32, world_z as f32,
                            width as f32, h as f32,
                            face, block_type,
                        );
                    } else {
                        Self::add_greedy_face_vertical_x(
                            opaque_vertices, opaque_indices,
                            world_x as f32, start_y as f32, world_z as f32,
                            width as f32, h as f32,
                            face, block_type,
                        );
                    }
                }
            }
        }
    }

    // Greedy mesh for vertical Z faces (front/back) - iterates Z layers, merges in XY plane
    fn greedy_mesh_vertical_z<F>(
        world: &World,
        chunk: &crate::world::Chunk,
        chunk_x_offset: i32,
        chunk_z_offset: i32,
        neighbor_z: F,
        face: Face,
        opaque_vertices: &mut Vec<Vertex>,
        opaque_indices: &mut Vec<u16>,
        trans_vertices: &mut Vec<Vertex>,
        trans_indices: &mut Vec<u16>,
    ) where F: Fn(i32) -> i32 {
        let size = World::CHUNK_SIZE;
        let height = World::CHUNK_HEIGHT;

        for z in 0..size {
            // Build mask of exposed faces at this Z level
            let mut mask: Vec<Vec<Option<BlockType>>> = vec![vec![None; size]; height];

            for y in 0..height {
                for x in 0..size {
                    let block_type = chunk.blocks[x][y][z];
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch
                       || block_type == BlockType::Ladder || block_type.is_trapdoor() || block_type.is_fence() || block_type == BlockType::GlassPane
                       || block_type.is_bottom_slab() || block_type.is_top_slab() || block_type.is_stairs() {
                        continue;  // Torches, slabs, stairs, ladders, and trapdoors are rendered separately with special geometry
                    }

                    let world_x = chunk_x_offset + x as i32;
                    let world_z = chunk_z_offset + z as i32;

                    // Skip damaged blocks - they're rendered separately with crack effects
                    if world.get_block_damage(world_x, y as i32, world_z) > 0.0 {
                        continue;
                    }

                    let check_z = neighbor_z(world_z);

                    if Self::is_face_exposed(world, world_x, y as i32, check_z, block_type) {
                        mask[y][x] = Some(block_type);
                    }
                }
            }

            // Greedy merge in XY plane
            let mut visited: Vec<Vec<bool>> = vec![vec![false; size]; height];

            for start_y in 0..height {
                for start_x in 0..size {
                    if visited[start_y][start_x] || mask[start_y][start_x].is_none() {
                        continue;
                    }

                    let block_type = mask[start_y][start_x].unwrap();
                    let is_transparent = Self::is_transparent(block_type);

                    // Find width (extend in X)
                    let mut width = 1;
                    while start_x + width < size
                        && !visited[start_y][start_x + width]
                        && mask[start_y][start_x + width] == Some(block_type)
                    {
                        width += 1;
                    }

                    // Find height (extend in Y)
                    let mut h = 1;
                    'outer: while start_y + h < height {
                        for dx in 0..width {
                            if visited[start_y + h][start_x + dx]
                                || mask[start_y + h][start_x + dx] != Some(block_type)
                            {
                                break 'outer;
                            }
                        }
                        h += 1;
                    }

                    // Mark as visited
                    for dy in 0..h {
                        for dx in 0..width {
                            visited[start_y + dy][start_x + dx] = true;
                        }
                    }

                    // Generate merged quad
                    let world_x = chunk_x_offset + start_x as i32;
                    let world_z = chunk_z_offset + z as i32;

                    if is_transparent {
                        Self::add_greedy_face_vertical_z(
                            trans_vertices, trans_indices,
                            world_x as f32, start_y as f32, world_z as f32,
                            width as f32, h as f32,
                            face, block_type,
                        );
                    } else {
                        Self::add_greedy_face_vertical_z(
                            opaque_vertices, opaque_indices,
                            world_x as f32, start_y as f32, world_z as f32,
                            width as f32, h as f32,
                            face, block_type,
                        );
                    }
                }
            }
        }
    }

    // Add a greedy-merged horizontal face for water with variable height
    fn add_greedy_face_horizontal_water(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        width: f32, depth: f32,
        face: Face,
        block_type: BlockType,
        water_depth: f32,
        water_level: u8,  // 1-8, where 8 is full/source
    ) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);
        let is_top = matches!(face, Face::Top);
        let normal = if is_top { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] };

        // For water top face, height depends on water level
        // Level 8 (source) = 0.875 height, Level 1 = 0.125
        let y_offset = if is_top {
            (water_level as f32 / 8.0) * 0.875
        } else {
            0.0
        };
        let y_pos = y + y_offset;

        vertices.push(Vertex { position: [x, y_pos, z], tex_coords: [0.0, 0.0], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x + width, y_pos, z], tex_coords: [width, 0.0], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x + width, y_pos, z + depth], tex_coords: [width, depth], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x, y_pos, z + depth], tex_coords: [0.0, depth], normal, block_type: block_type_f, damage: water_depth });

        let idx = if is_top { [0, 1, 2, 2, 3, 0] } else { [0, 3, 2, 2, 1, 0] };
        for i in idx {
            indices.push(base_index + i);
        }
    }

    // Add a greedy-merged horizontal face (top/bottom)
    fn add_greedy_face_horizontal(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        width: f32, depth: f32,
        face: Face,
        block_type: BlockType,
        water_depth: f32,  // For water blocks: depth of water column below (0.0 for non-water)
    ) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);
        let is_top = matches!(face, Face::Top);
        let normal = if is_top { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] };
        let y_offset = if is_top { 1.0 } else { 0.0 };
        let y_pos = y + y_offset;

        // Water depth is passed via damage field (already 0.0 for non-water blocks)
        vertices.push(Vertex { position: [x, y_pos, z], tex_coords: [0.0, 0.0], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x + width, y_pos, z], tex_coords: [width, 0.0], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x + width, y_pos, z + depth], tex_coords: [width, depth], normal, block_type: block_type_f, damage: water_depth });
        vertices.push(Vertex { position: [x, y_pos, z + depth], tex_coords: [0.0, depth], normal, block_type: block_type_f, damage: water_depth });

        let idx = if is_top { [0, 1, 2, 2, 3, 0] } else { [0, 3, 2, 2, 1, 0] };
        for i in idx {
            indices.push(base_index + i);
        }
    }

    // Add a greedy-merged vertical X face (left/right)
    fn add_greedy_face_vertical_x(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        width: f32, height: f32,  // width = Z extent, height = Y extent
        face: Face,
        block_type: BlockType,
    ) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);
        let normal = match face {
            Face::Right => [1.0, 0.0, 0.0],
            Face::Left => [-1.0, 0.0, 0.0],
            _ => [1.0, 0.0, 0.0],
        };
        let x_offset = if matches!(face, Face::Right) { 1.0 } else { 0.0 };

        // Tiled texture coordinates
        vertices.push(Vertex { position: [x + x_offset, y, z], tex_coords: [0.0, height], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x + x_offset, y, z + width], tex_coords: [width, height], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x + x_offset, y + height, z + width], tex_coords: [width, 0.0], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x + x_offset, y + height, z], tex_coords: [0.0, 0.0], normal, block_type: block_type_f, damage: 0.0 });

        let idx = if matches!(face, Face::Right) {
            [0, 1, 2, 2, 3, 0]
        } else {
            [0, 3, 2, 2, 1, 0]
        };
        for i in idx {
            indices.push(base_index + i);
        }
    }

    // Add a greedy-merged vertical Z face (front/back)
    fn add_greedy_face_vertical_z(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        x: f32, y: f32, z: f32,
        width: f32, height: f32,  // width = X extent, height = Y extent
        face: Face,
        block_type: BlockType,
    ) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);
        let normal = match face {
            Face::Front => [0.0, 0.0, 1.0],
            Face::Back => [0.0, 0.0, -1.0],
            _ => [0.0, 0.0, 1.0],
        };
        let z_offset = if matches!(face, Face::Front) { 1.0 } else { 0.0 };

        // Tiled texture coordinates
        vertices.push(Vertex { position: [x, y, z + z_offset], tex_coords: [0.0, height], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x, y + height, z + z_offset], tex_coords: [0.0, 0.0], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x + width, y + height, z + z_offset], tex_coords: [width, 0.0], normal, block_type: block_type_f, damage: 0.0 });
        vertices.push(Vertex { position: [x + width, y, z + z_offset], tex_coords: [width, height], normal, block_type: block_type_f, damage: 0.0 });

        let idx = if matches!(face, Face::Front) {
            [0, 1, 2, 2, 3, 0]
        } else {
            [0, 3, 2, 2, 1, 0]
        };
        for i in idx {
            indices.push(base_index + i);
        }
    }

    fn create_held_item_vertices(_camera: &Camera, opt_item: Option<&ItemStack>, progress: f32) -> Vec<Vertex> {
        let face_verts = [
            TOP_FACE_VERTICES,
            BOTTOM_FACE_VERTICES,
            RIGHT_FACE_VERTICES,
            LEFT_FACE_VERTICES,
            FRONT_FACE_VERTICES,
            BACK_FACE_VERTICES,
        ];

        let mut verts: Vec<Vertex> = vec![];

        // Generate item vertices based on what's held
        match opt_item {
            Some(ItemStack::Block(block_type, _)) => {
                // Render block as cube
                let block_type_f = Self::block_type_to_float(*block_type);
                let size = 0.4;
                for &face_vert in face_verts.iter() {
                    for v in face_vert.iter() {
                        let mut new_v = *v;
                        new_v.position[0] = (new_v.position[0] - 0.5) * size;
                        new_v.position[1] = (new_v.position[1] - 0.5) * size;
                        new_v.position[2] = (new_v.position[2] - 0.5) * size;
                        new_v.block_type = block_type_f;
                        new_v.damage = 0.0;
                        verts.push(new_v);
                    }
                }
            }
            Some(ItemStack::Tool(tool)) => {
                // Render tool model
                Self::generate_tool_vertices(&mut verts, tool);
            }
            None => {
                // Add dummy vertices for the item cube if nothing is selected
                let dummy = Vertex {
                    position: [0.0, 0.0, 0.0],
                    tex_coords: [0.0, 0.0],
                    normal: [0.0, 0.0, 0.0],
                    block_type: -1.0,
                    damage: 0.0,
                };
                for _ in 0..24 {
                    verts.push(dummy);
                }
            }
        }

        // Generate arm vertices
        let arm_size_x = 0.25;
        let arm_size_y = 0.5;
        let arm_size_z = 0.25;
        let arm_offset = [0.0f32, -0.3, 0.0];
        for &face_vert in face_verts.iter() {
            for v in face_vert.iter() {
                let mut new_v = *v;
                new_v.position[0] = (new_v.position[0] - 0.5) * arm_size_x + arm_offset[0];
                new_v.position[1] = (new_v.position[1] - 0.5) * arm_size_y + arm_offset[1];
                new_v.position[2] = (new_v.position[2] - 0.5) * arm_size_z + arm_offset[2];
                new_v.block_type = 6.0;  // Skin color
                new_v.damage = 0.0;
                verts.push(new_v);
            }
        }

        // Position in view space
        let view_offset = Vector3::new(0.8, -0.6, -1.5);

        // Apply swing animation rotation
        let angle = -progress.powi(2) * 80.0;
        let rotation = Matrix4::from_axis_angle(Vector3::unit_x(), Deg(angle));

        // Apply base tilt
        let tilt_rotation = Matrix4::from_axis_angle(Vector3::unit_x(), Deg(-30.0)) *
                          Matrix4::from_axis_angle(Vector3::unit_y(), Deg(20.0));

        // Combine transformations in view space
        let model = Matrix4::from_translation(view_offset) * rotation * tilt_rotation;
        let normal_mat = model.invert().unwrap().transpose();

        // Transform all vertices to view space
        for v in verts.iter_mut() {
            let local_pos = Vector3::from(v.position);
            let transformed = (model * Vector4::new(local_pos.x, local_pos.y, local_pos.z, 1.0)).truncate();
            v.position = [transformed.x, transformed.y, transformed.z];

            let local_normal = Vector3::from(v.normal);
            let transformed_normal = (normal_mat * Vector4::new(local_normal.x, local_normal.y, local_normal.z, 0.0)).truncate().normalize();
            v.normal = [transformed_normal.x, transformed_normal.y, transformed_normal.z];
        }

        verts
    }

    /// Generate vertices for a tool model (pickaxe, sword, axe, shovel)
    fn generate_tool_vertices(verts: &mut Vec<Vertex>, tool: &Tool) {
        // Get material color (block type index for texturing)
        // Using visually distinct colors for each material tier:
        // - Wood (4): Brown wood log texture
        // - Stone (11): Gray cobblestone - looks crafted
        // - Iron (9): White snow texture - appears silvery/metallic
        // - Gold (14): Gold ore - golden yellow tones
        // - Diamond (15): Diamond ore - cyan/teal blue
        let material_color = match tool.material {
            ToolMaterial::Wood => 4.0,      // Wood log - brown
            ToolMaterial::Stone => 11.0,    // Cobblestone - gray, crafted look
            ToolMaterial::Iron => 9.0,      // Snow - white/silver metallic
            ToolMaterial::Gold => 14.0,     // Gold ore - golden/yellow
            ToolMaterial::Diamond => 15.0,  // Diamond ore - cyan/light blue
        };
        let handle_color = 4.0;  // Wood log for handle (brown)

        let face_verts = [
            TOP_FACE_VERTICES,
            BOTTOM_FACE_VERTICES,
            RIGHT_FACE_VERTICES,
            LEFT_FACE_VERTICES,
            FRONT_FACE_VERTICES,
            BACK_FACE_VERTICES,
        ];

        // Helper to add a box
        let add_box = |verts: &mut Vec<Vertex>,
                       offset: [f32; 3],
                       size: [f32; 3],
                       color: f32| {
            for &face_vert in face_verts.iter() {
                for v in face_vert.iter() {
                    let mut new_v = *v;
                    new_v.position[0] = (new_v.position[0] - 0.5) * size[0] + offset[0];
                    new_v.position[1] = (new_v.position[1] - 0.5) * size[1] + offset[1];
                    new_v.position[2] = (new_v.position[2] - 0.5) * size[2] + offset[2];
                    new_v.block_type = color;
                    new_v.damage = 0.0;
                    verts.push(new_v);
                }
            }
        };

        match tool.tool_type {
            ToolType::Sword => {
                // Sword: Long blade + crossguard + handle
                // Handle (bottom)
                add_box(verts, [0.0, -0.25, 0.0], [0.06, 0.2, 0.06], handle_color);
                // Crossguard
                add_box(verts, [0.0, -0.1, 0.0], [0.2, 0.04, 0.04], material_color);
                // Blade (long, thin, tapered)
                add_box(verts, [0.0, 0.25, 0.0], [0.08, 0.6, 0.02], material_color);
                // Blade tip
                add_box(verts, [0.0, 0.58, 0.0], [0.04, 0.1, 0.02], material_color);
            }
            ToolType::Pickaxe => {
                // Pickaxe: Handle + T-shaped head
                // Handle
                add_box(verts, [0.0, -0.15, 0.0], [0.06, 0.5, 0.06], handle_color);
                // Horizontal head bar
                add_box(verts, [0.0, 0.2, 0.0], [0.5, 0.1, 0.06], material_color);
                // Left pick point
                add_box(verts, [-0.28, 0.15, 0.0], [0.08, 0.12, 0.04], material_color);
                // Right pick point
                add_box(verts, [0.28, 0.15, 0.0], [0.08, 0.12, 0.04], material_color);
            }
            ToolType::Axe => {
                // Axe: Handle + blade head on one side
                // Handle
                add_box(verts, [0.0, -0.15, 0.0], [0.06, 0.5, 0.06], handle_color);
                // Axe head (one-sided blade)
                add_box(verts, [0.12, 0.18, 0.0], [0.25, 0.22, 0.05], material_color);
                // Axe head back
                add_box(verts, [-0.04, 0.18, 0.0], [0.08, 0.12, 0.06], material_color);
            }
            ToolType::Shovel => {
                // Shovel: Long handle + spade head
                // Handle
                add_box(verts, [0.0, -0.1, 0.0], [0.05, 0.55, 0.05], handle_color);
                // Spade head
                add_box(verts, [0.0, 0.28, 0.0], [0.14, 0.2, 0.03], material_color);
                // Spade edge (rounded look)
                add_box(verts, [0.0, 0.4, 0.0], [0.1, 0.06, 0.025], material_color);
            }
        }
    }

    pub fn start_arm_swing(&mut self) {
        self.arm_swing_progress = 1.0;
    }
    
    fn update_outline_buffers(&mut self, block_x: i32, block_y: i32, block_z: i32) {
        let x = block_x as f32;
        let y = block_y as f32;
        let z = block_z as f32;
        
        // Offset to make outline slightly larger than block
        let offset = 0.005;
        let x0 = x - offset;
        let x1 = x + 1.0 + offset;
        let y0 = y - offset;
        let y1 = y + 1.0 + offset;
        let z0 = z - offset;
        let z1 = z + 1.0 + offset;
        
        // Define the 8 vertices of a cube with slight expansion
        let vertices: Vec<[f32; 3]> = vec![
            [x0, y0, z0],  // 0: bottom-left-back
            [x1, y0, z0],  // 1: bottom-right-back
            [x1, y1, z0],  // 2: top-right-back
            [x0, y1, z0],  // 3: top-left-back
            [x0, y0, z1],  // 4: bottom-left-front
            [x1, y0, z1],  // 5: bottom-right-front
            [x1, y1, z1],  // 6: top-right-front
            [x0, y1, z1],  // 7: top-left-front
        ];
        
        // Create triangulated faces (only render the edges as thick quads)
        let thickness = 0.02;
        let mut outline_vertices = Vec::new();
        let mut outline_indices = Vec::new();
        
        // Helper to add a line segment as a quad
        let add_line = |verts: &mut Vec<[f32; 3]>, indices: &mut Vec<u16>, p1: [f32; 3], p2: [f32; 3]| {
            let base_idx = verts.len() as u16;
            
            // Calculate perpendicular vectors for thickness
            let dir = [p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]];
            let len = (dir[0]*dir[0] + dir[1]*dir[1] + dir[2]*dir[2]).sqrt();
            let norm_dir = [dir[0]/len, dir[1]/len, dir[2]/len];
            
            // Create a perpendicular vector for thickness
            let perp = if norm_dir[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            
            let cross = [
                norm_dir[1] * perp[2] - norm_dir[2] * perp[1],
                norm_dir[2] * perp[0] - norm_dir[0] * perp[2],
                norm_dir[0] * perp[1] - norm_dir[1] * perp[0],
            ];
            let cross_len = (cross[0]*cross[0] + cross[1]*cross[1] + cross[2]*cross[2]).sqrt();
            let offset = [cross[0]/cross_len * thickness, cross[1]/cross_len * thickness, cross[2]/cross_len * thickness];
            
            // Add 4 vertices for the line quad
            verts.push([p1[0] - offset[0], p1[1] - offset[1], p1[2] - offset[2]]);
            verts.push([p1[0] + offset[0], p1[1] + offset[1], p1[2] + offset[2]]);
            verts.push([p2[0] + offset[0], p2[1] + offset[1], p2[2] + offset[2]]);
            verts.push([p2[0] - offset[0], p2[1] - offset[1], p2[2] - offset[2]]);
            
            // Add 2 triangles
            indices.extend_from_slice(&[
                base_idx, base_idx + 1, base_idx + 2,
                base_idx, base_idx + 2, base_idx + 3,
            ]);
        };
        
        // Add all 12 edges
        // Bottom edges
        add_line(&mut outline_vertices, &mut outline_indices, vertices[0], vertices[1]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[1], vertices[5]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[5], vertices[4]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[4], vertices[0]);
        // Top edges
        add_line(&mut outline_vertices, &mut outline_indices, vertices[3], vertices[2]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[2], vertices[6]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[6], vertices[7]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[7], vertices[3]);
        // Vertical edges
        add_line(&mut outline_vertices, &mut outline_indices, vertices[0], vertices[3]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[1], vertices[2]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[5], vertices[6]);
        add_line(&mut outline_vertices, &mut outline_indices, vertices[4], vertices[7]);
        
        // Update the buffers
        self.queue.write_buffer(&self.outline_vertex_buffer, 0, bytemuck::cast_slice(&outline_vertices));
        self.queue.write_buffer(&self.outline_index_buffer, 0, bytemuck::cast_slice(&outline_indices));
    }
    
    fn is_face_exposed(world: &World, x: i32, y: i32, z: i32, current_type: BlockType) -> bool {
        world.get_block(x, y, z).map_or(true, |block| {
            if current_type == BlockType::Water {
                block != BlockType::Water
            } else {
                // Torch doesn't occlude faces - it's a small object, not a full block
                block == BlockType::Air || block == BlockType::Torch
            }
        })
    }
    
    fn block_type_to_float(block_type: BlockType) -> f32 {
        match block_type {
            BlockType::Grass => 0.0,
            BlockType::Dirt => 1.0,
            BlockType::Stone => 2.0,
            BlockType::Wood => 3.0,
            BlockType::Leaves => 4.0,
            BlockType::Water => 5.0,
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
            BlockType::Torch => 24.0,  // Torch uses texture
            BlockType::Chest => 26.0,  // Chest uses wood-like color
            // New block types for world generation
            BlockType::Lava => 41.0,             // Emissive orange
            BlockType::MobSpawner => 42.0,       // Dark cage-like
            BlockType::Rail => 43.0,             // Metal rails
            BlockType::Planks => 44.0,           // Light wood
            BlockType::Fence => 45.0,            // Wood fence
            BlockType::Brick => 46.0,            // Stone brick
            BlockType::MossyCobblestone => 47.0, // Mossy green cobblestone
            // Slabs use their base block textures
            BlockType::StoneSlabBottom | BlockType::StoneSlabTop => 2.0, // Stone texture
            BlockType::WoodSlabBottom | BlockType::WoodSlabTop => 44.0,  // Planks texture
            BlockType::CobblestoneSlabBottom | BlockType::CobblestoneSlabTop => 10.0, // Cobblestone texture
            // Stairs use their base block textures
            BlockType::StoneStairs => 2.0,
            BlockType::WoodStairs => 44.0,
            BlockType::CobblestoneStairs => 10.0,
            BlockType::BrickStairs => 46.0,
            // Other building blocks
            BlockType::Ladder => 44.0,        // Wood-like
            BlockType::WoodTrapdoor => 44.0,  // Wood texture
            BlockType::IronTrapdoor => 12.0,  // Iron texture
            BlockType::SignPost | BlockType::WallSign => 44.0, // Wood texture
            BlockType::WoodFence => 44.0,     // Wood texture
            BlockType::StoneFence => 2.0,     // Stone texture
            BlockType::FenceGate => 44.0,     // Wood texture
            BlockType::GlassPane => 9.0,      // Ice/glass-like
            _ => 0.0,
        }
    }
    
    fn add_face(vertices: &mut Vec<Vertex>, indices: &mut Vec<u16>, pos: Vector3<f32>, face: Face, block_type: BlockType, damage: f32) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);

        let face_vertices = match face {
            Face::Top => TOP_FACE_VERTICES,
            Face::Bottom => BOTTOM_FACE_VERTICES,
            Face::Right => RIGHT_FACE_VERTICES,
            Face::Left => LEFT_FACE_VERTICES,
            Face::Front => FRONT_FACE_VERTICES,
            Face::Back => BACK_FACE_VERTICES,
        };

        for v in face_vertices {
            vertices.push(Vertex {
                position: [
                    pos.x + v.position[0],
                    pos.y + v.position[1],
                    pos.z + v.position[2],
                ],
                tex_coords: v.tex_coords,
                normal: v.normal,
                block_type: block_type_f,
                damage,
            });
        }

        for i in FACE_INDICES {
            indices.push(base_index + i);
        }
    }

    fn is_transparent(block_type: BlockType) -> bool {
        matches!(block_type, BlockType::Water | BlockType::Lava)
    }

    // Generate a cube mesh for a villager body part
    fn generate_villager_cube(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        pos: [f32; 3],
        size: [f32; 3],
        block_type: f32,
        rotation: f32,  // Y-axis rotation in radians
        pivot: [f32; 3],  // Pivot point for rotation
    ) {
        let base_idx = vertices.len() as u16;
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();

        let face_verts = [
            TOP_FACE_VERTICES,
            BOTTOM_FACE_VERTICES,
            RIGHT_FACE_VERTICES,
            LEFT_FACE_VERTICES,
            FRONT_FACE_VERTICES,
            BACK_FACE_VERTICES,
        ];

        for face_vert in face_verts.iter() {
            for v in face_vert.iter() {
                // Scale and offset the vertex
                let mut local_x = (v.position[0] - 0.5) * size[0] + pos[0];
                let local_y = (v.position[1] - 0.5) * size[1] + pos[1];
                let mut local_z = (v.position[2] - 0.5) * size[2] + pos[2];

                // Apply rotation around pivot
                let rx = local_x - pivot[0];
                let rz = local_z - pivot[2];
                local_x = rx * cos_r - rz * sin_r + pivot[0];
                local_z = rx * sin_r + rz * cos_r + pivot[2];

                // Rotate normal as well
                let mut norm_x = v.normal[0];
                let mut norm_z = v.normal[2];
                let new_norm_x = norm_x * cos_r - norm_z * sin_r;
                let new_norm_z = norm_x * sin_r + norm_z * cos_r;
                norm_x = new_norm_x;
                norm_z = new_norm_z;

                vertices.push(Vertex {
                    position: [local_x, local_y, local_z],
                    tex_coords: v.tex_coords,
                    normal: [norm_x, v.normal[1], norm_z],
                    block_type,
                    damage: 0.0,
                });
            }
        }

        // Add indices for all 6 faces (4 vertices each, 2 triangles)
        for face in 0..6 {
            let face_base = base_idx + (face * 4) as u16;
            indices.push(face_base);
            indices.push(face_base + 1);
            indices.push(face_base + 2);
            indices.push(face_base);
            indices.push(face_base + 2);
            indices.push(face_base + 3);
        }
    }

    pub fn update_villager_mesh(&mut self, villagers: &[Villager]) {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();

        for villager in villagers {
            let x = villager.position.x;
            let y = villager.position.y - VILLAGER_HEIGHT;  // Position is at eye level
            let z = villager.position.z;
            let yaw = villager.yaw.to_radians();
            let pivot = [x, y, z];

            // Animation swing for arms and legs
            let swing = if villager.state == VillagerState::Walking {
                (villager.animation_time * 8.0).sin() * 0.5
            } else {
                0.0
            };

            let robe = villager.robe_color;

            // Head (skin color = 17.0)
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [x, y + 1.6, z],
                [0.5, 0.5, 0.5],
                17.0,
                yaw,
                pivot,
            );

            // Body (robe color)
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [x, y + 0.95, z],
                [0.6, 0.8, 0.35],
                robe,
                yaw,
                pivot,
            );

            // Left Arm (robe color)
            let left_arm_swing = swing;
            let left_arm_x = x - 0.375;
            let left_arm_pivot = [left_arm_x, y + 1.15, z];
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [left_arm_x, y + 0.85 + left_arm_swing * 0.15, z + left_arm_swing * 0.2],
                [0.25, 0.6, 0.25],
                robe,
                yaw,
                left_arm_pivot,
            );

            // Right Arm (robe color)
            let right_arm_swing = -swing;
            let right_arm_x = x + 0.375;
            let right_arm_pivot = [right_arm_x, y + 1.15, z];
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [right_arm_x, y + 0.85 + right_arm_swing * 0.15, z + right_arm_swing * 0.2],
                [0.25, 0.6, 0.25],
                robe,
                yaw,
                right_arm_pivot,
            );

            // Left Leg (robe color)
            let left_leg_swing = -swing;
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [x - 0.125, y + 0.35, z + left_leg_swing * 0.25],
                [0.25, 0.7, 0.25],
                robe,
                yaw,
                pivot,
            );

            // Right Leg (robe color)
            let right_leg_swing = swing;
            Self::generate_villager_cube(
                &mut vertices,
                &mut indices,
                [x + 0.125, y + 0.35, z + right_leg_swing * 0.25],
                [0.25, 0.7, 0.25],
                robe,
                yaw,
                pivot,
            );
        }

        self.villager_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.villager_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.villager_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update the animal mesh (all animal types including flying)
    pub fn update_animal_mesh(&mut self, animals: &[crate::entity::Animal]) {
        use crate::entity::{AnimalState, AnimalType, MovementType};

        let mut vertices: Vec<Vertex> = Vec::with_capacity(animals.len() * 8 * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(animals.len() * 8 * 36);

        for animal in animals {
            let x = animal.position.x;
            let y = animal.position.y;
            let z = animal.position.z;
            let yaw = animal.yaw.to_radians();
            let color = animal.animal_type.color_index();
            let (width, height) = animal.animal_type.dimensions();
            let movement_type = animal.animal_type.movement_type();

            let pivot = [x, y, z];

            match movement_type {
                MovementType::Flying => {
                    // Flying animals: body + head + 2 flapping wings
                    let wing_flap = (animal.animation_time * 15.0).sin() * 0.8; // Fast wing flap

                    // Body (smaller, rounder)
                    let body_y = y - height * 0.5;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [x, body_y, z],
                        [width * 0.4, height * 0.5, width * 0.5],
                        color,
                        yaw,
                        pivot,
                    );

                    // Head (front)
                    let head_forward = width * 0.35;
                    let head_x = x - yaw.sin() * head_forward;
                    let head_z = z - yaw.cos() * head_forward;
                    let head_y = y - height * 0.3;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [head_x, head_y, head_z],
                        [width * 0.25, width * 0.25, width * 0.25],
                        color,
                        yaw,
                        pivot,
                    );

                    // Left wing (flapping)
                    let wing_width = width * 0.8;
                    let wing_height = height * 0.1;
                    let wing_depth = width * 0.4;
                    let wing_y_offset = wing_flap * 0.3; // Wing goes up and down

                    // Left wing position
                    let left_wing_dx = width * 0.4;
                    let left_rot_dx = left_wing_dx * yaw.cos();
                    let left_rot_dz = left_wing_dx * yaw.sin();
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [x + left_rot_dx, body_y + wing_y_offset, z + left_rot_dz],
                        [wing_width, wing_height, wing_depth],
                        color,
                        yaw,
                        pivot,
                    );

                    // Right wing position
                    let right_wing_dx = -width * 0.4;
                    let right_rot_dx = right_wing_dx * yaw.cos();
                    let right_rot_dz = right_wing_dx * yaw.sin();
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [x + right_rot_dx, body_y - wing_y_offset, z + right_rot_dz],
                        [wing_width, wing_height, wing_depth],
                        color,
                        yaw,
                        pivot,
                    );
                }
                MovementType::Aquatic => {
                    // Aquatic animals: streamlined body + tail + fins
                    let swim_wiggle = (animal.animation_time * 4.0).sin() * 0.15;

                    // Main body (elongated)
                    let body_y = y - height * 0.5;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [x, body_y, z],
                        [width * 0.3, height * 0.4, width * 0.8],
                        color,
                        yaw,
                        pivot,
                    );

                    // Tail (wiggling)
                    let tail_back = width * 0.6;
                    let tail_x = x + yaw.sin() * tail_back + yaw.cos() * swim_wiggle;
                    let tail_z = z + yaw.cos() * tail_back - yaw.sin() * swim_wiggle;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [tail_x, body_y, tail_z],
                        [width * 0.2, height * 0.5, width * 0.15],
                        color,
                        yaw,
                        pivot,
                    );

                    // Side fins
                    let fin_dx = width * 0.25;
                    for side in [-1.0, 1.0] {
                        let fin_rot_dx = (fin_dx * side) * yaw.cos();
                        let fin_rot_dz = (fin_dx * side) * yaw.sin();
                        Self::generate_villager_cube(
                            &mut vertices,
                            &mut indices,
                            [x + fin_rot_dx, body_y, z + fin_rot_dz],
                            [width * 0.3, height * 0.1, width * 0.2],
                            color,
                            yaw,
                            pivot,
                        );
                    }
                }
                MovementType::Ground => {
                    // Ground animals: body + head + 4 legs (original code)
                    let swing = if animal.state == AnimalState::Walking || animal.state == AnimalState::Running {
                        (animal.animation_time * 6.0).sin() * 0.25
                    } else {
                        0.0
                    };

                    let head_y_offset = if animal.state == AnimalState::Eating { -0.15 } else { 0.0 };

                    // Body
                    let body_y = y - height * 0.4;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [x, body_y, z],
                        [width * 0.5, height * 0.35, width * 0.7],
                        color,
                        yaw,
                        pivot,
                    );

                    // Head
                    let head_forward = width * 0.5 + width * 0.2;
                    let head_x = x - yaw.sin() * head_forward;
                    let head_z = z - yaw.cos() * head_forward;
                    let head_y = y - height * 0.25 + head_y_offset;
                    Self::generate_villager_cube(
                        &mut vertices,
                        &mut indices,
                        [head_x, head_y, head_z],
                        [width * 0.3, width * 0.3, width * 0.35],
                        color,
                        yaw,
                        pivot,
                    );

                    // 4 Legs
                    let leg_height = height * 0.35;
                    let leg_y = y - height + leg_height * 0.5;
                    let leg_offsets = [
                        (-0.15 * width, 0.25 * width, 1.0),
                        (0.15 * width, 0.25 * width, -1.0),
                        (-0.15 * width, -0.25 * width, -1.0),
                        (0.15 * width, -0.25 * width, 1.0),
                    ];

                    for (dx, dz, phase) in leg_offsets {
                        let rot_dx = dx * yaw.cos() - dz * yaw.sin();
                        let rot_dz = dx * yaw.sin() + dz * yaw.cos();
                        let leg_swing = swing * phase;
                        let swing_x = -yaw.sin() * leg_swing * 0.3;
                        let swing_z = -yaw.cos() * leg_swing * 0.3;

                        Self::generate_villager_cube(
                            &mut vertices,
                            &mut indices,
                            [x + rot_dx + swing_x, leg_y, z + rot_dz + swing_z],
                            [0.12, leg_height, 0.12],
                            color,
                            yaw,
                            pivot,
                        );
                    }
                }
            }
        }

        self.animal_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.animal_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.animal_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update the hostile mob mesh (zombies, skeletons, spiders, creepers) and projectiles
    pub fn update_hostile_mob_mesh(&mut self, hostile_mobs: &[crate::entity::HostileMob], projectiles: &[crate::entity::Projectile]) {
        use crate::entity::{HostileMobState, HostileMobType};

        let mut vertices: Vec<Vertex> = Vec::with_capacity(hostile_mobs.len() * 8 * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(hostile_mobs.len() * 8 * 36);

        for mob in hostile_mobs {
            let x = mob.position.x;
            let y = mob.position.y;
            let z = mob.position.z;
            let yaw = mob.yaw.to_radians();

            // Use mob-specific color, flash red when hit
            let color = if mob.damage_flash > 0.0 { 99.0 } else { mob.mob_type.color_index() };

            let (width, height) = mob.mob_type.dimensions();
            let pivot = [x, y, z];

            // Animation swing for walking/attacking
            let swing = if mob.state == HostileMobState::Chasing || mob.state == HostileMobState::Wandering {
                (mob.animation_time * 4.0).sin() * 0.3
            } else if mob.state == HostileMobState::Attacking {
                (mob.animation_time * 8.0).sin() * 0.5
            } else if mob.state == HostileMobState::Fusing {
                // Creeper fuse - pulsing/expanding effect
                (mob.animation_time * 16.0).sin() * 0.1
            } else {
                0.0
            };

            match mob.mob_type {
                HostileMobType::Spider => {
                    // Spider: wide flat body with 8 legs
                    // Body
                    let body_y = y - height * 0.5;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x, body_y, z],
                        [width * 0.5, height * 0.6, width * 0.4],
                        color, yaw, pivot,
                    );
                    // Head (smaller, in front)
                    let head_forward = -width * 0.3;
                    let head_x = x - yaw.sin() * head_forward;
                    let head_z = z - yaw.cos() * head_forward;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [head_x, body_y + 0.1, head_z],
                        [width * 0.3, height * 0.4, width * 0.3],
                        color, yaw, pivot,
                    );
                    // 8 legs (4 per side)
                    let leg_positions = [
                        (0.25, -0.3, 1.0), (0.35, 0.0, 0.5), (0.35, 0.25, -0.5), (0.25, 0.45, -1.0),
                        (-0.25, -0.3, -1.0), (-0.35, 0.0, -0.5), (-0.35, 0.25, 0.5), (-0.25, 0.45, 1.0),
                    ];
                    for (dx_mult, dz_mult, phase) in leg_positions {
                        let dx = width * dx_mult;
                        let dz = width * dz_mult;
                        let leg_swing = swing * phase;
                        let rot_dx = dx * yaw.cos() - dz * yaw.sin();
                        let rot_dz = dx * yaw.sin() + dz * yaw.cos();
                        Self::generate_villager_cube(
                            &mut vertices, &mut indices,
                            [x + rot_dx, body_y - height * 0.3 + leg_swing * 0.1, z + rot_dz],
                            [0.08, height * 0.4, 0.08],
                            color, yaw, pivot,
                        );
                    }
                }
                HostileMobType::Creeper => {
                    // Creeper: tall body, small head, 4 short legs, no arms
                    let fuse_expand = if mob.state == HostileMobState::Fusing {
                        1.0 + swing.abs() * 0.3
                    } else {
                        1.0
                    };
                    // Body (tall rectangle)
                    let body_y = y - height * 0.4;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x, body_y, z],
                        [width * 0.5 * fuse_expand, height * 0.5 * fuse_expand, width * 0.4 * fuse_expand],
                        color, yaw, pivot,
                    );
                    // Head
                    let head_y = y - height * 0.1;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x, head_y, z],
                        [width * 0.45 * fuse_expand, width * 0.45 * fuse_expand, width * 0.35 * fuse_expand],
                        color, yaw, pivot,
                    );
                    // 4 short legs
                    let leg_height = height * 0.25;
                    let leg_y = y - height + leg_height * 0.5;
                    let leg_offsets = [
                        (width * 0.15, width * 0.1, 1.0),
                        (-width * 0.15, width * 0.1, -1.0),
                        (width * 0.15, -width * 0.1, -0.5),
                        (-width * 0.15, -width * 0.1, 0.5),
                    ];
                    for (dx, dz, phase) in leg_offsets {
                        let rot_dx = dx * yaw.cos() - dz * yaw.sin();
                        let rot_dz = dx * yaw.sin() + dz * yaw.cos();
                        let leg_swing = swing * phase;
                        let swing_x = -yaw.sin() * leg_swing * 0.15;
                        let swing_z = -yaw.cos() * leg_swing * 0.15;
                        Self::generate_villager_cube(
                            &mut vertices, &mut indices,
                            [x + rot_dx + swing_x, leg_y, z + rot_dz + swing_z],
                            [0.12, leg_height, 0.12],
                            color, yaw, pivot,
                        );
                    }
                }
                HostileMobType::Zombie | HostileMobType::Skeleton => {
                    // Zombie and Skeleton: humanoid shape
                    // Skeleton is thinner
                    let thickness_mult = if mob.mob_type == HostileMobType::Skeleton { 0.6 } else { 1.0 };

                    // Body (torso)
                    let body_y = y - height * 0.4;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x, body_y, z],
                        [width * 0.6 * thickness_mult, height * 0.4, width * 0.4 * thickness_mult],
                        color, yaw, pivot,
                    );

                    // Head
                    let head_y = y - height * 0.15;
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x, head_y, z],
                        [width * 0.4, width * 0.4, width * 0.4],
                        color, yaw, pivot,
                    );

                    // Arms
                    let arm_length = 0.5;
                    let arm_swing = swing * 0.5;
                    let arm_forward = if mob.mob_type == HostileMobType::Zombie {
                        0.4 + arm_swing // Zombie arms extended forward
                    } else {
                        arm_swing * 0.3 // Skeleton arms at sides (slight swing)
                    };

                    // Left arm
                    let left_arm_dx = width * 0.4;
                    let left_rot_dx = left_arm_dx * yaw.cos() - arm_forward * yaw.sin();
                    let left_rot_dz = left_arm_dx * yaw.sin() + arm_forward * yaw.cos();
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x + left_rot_dx, body_y + 0.1, z + left_rot_dz],
                        [0.1 * thickness_mult, arm_length, 0.1 * thickness_mult],
                        color, yaw, pivot,
                    );

                    // Right arm
                    let right_arm_dx = -width * 0.4;
                    let right_rot_dx = right_arm_dx * yaw.cos() - arm_forward * yaw.sin();
                    let right_rot_dz = right_arm_dx * yaw.sin() + arm_forward * yaw.cos();
                    Self::generate_villager_cube(
                        &mut vertices, &mut indices,
                        [x + right_rot_dx, body_y + 0.1, z + right_rot_dz],
                        [0.1 * thickness_mult, arm_length, 0.1 * thickness_mult],
                        color, yaw, pivot,
                    );

                    // Legs
                    let leg_height = height * 0.4;
                    let leg_y = y - height + leg_height * 0.5;
                    let leg_offsets = [
                        (width * 0.15, width * 0.05, 1.0),
                        (-width * 0.15, -width * 0.05, -1.0),
                    ];

                    for (dx, dz, phase) in leg_offsets {
                        let rot_dx = dx * yaw.cos() - dz * yaw.sin();
                        let rot_dz = dx * yaw.sin() + dz * yaw.cos();
                        let leg_swing = swing * phase;
                        let swing_x = -yaw.sin() * leg_swing * 0.3;
                        let swing_z = -yaw.cos() * leg_swing * 0.3;

                        Self::generate_villager_cube(
                            &mut vertices, &mut indices,
                            [x + rot_dx + swing_x, leg_y, z + rot_dz + swing_z],
                            [0.12 * thickness_mult, leg_height, 0.12 * thickness_mult],
                            color, yaw, pivot,
                        );
                    }
                }
            }
        }

        // Render projectiles (arrows)
        for proj in projectiles {
            let x = proj.position.x;
            let y = proj.position.y;
            let z = proj.position.z;

            // Calculate arrow direction from velocity
            let vlen = (proj.velocity.x.powi(2) + proj.velocity.y.powi(2) + proj.velocity.z.powi(2)).sqrt();
            let yaw = if vlen > 0.01 {
                (-proj.velocity.x).atan2(-proj.velocity.z)
            } else {
                0.0
            };
            let pitch = if vlen > 0.01 {
                (proj.velocity.y / vlen).asin()
            } else {
                0.0
            };

            // Arrow color (brown/wood color, using sand block index as approximation)
            let arrow_color = 4.0; // Wood/planks color

            let pivot = [x, y, z];

            // Arrow shaft (thin long rectangle)
            let shaft_length = 0.5;
            let shaft_thickness = 0.05;

            // Calculate rotated shaft positions
            let forward_x = -yaw.sin() * pitch.cos();
            let forward_y = pitch.sin();
            let forward_z = -yaw.cos() * pitch.cos();

            // Shaft end points
            let front_x = x + forward_x * shaft_length * 0.5;
            let front_y = y + forward_y * shaft_length * 0.5;
            let front_z = z + forward_z * shaft_length * 0.5;

            Self::generate_villager_cube(
                &mut vertices, &mut indices,
                [front_x, front_y, front_z],
                [shaft_thickness, shaft_thickness, shaft_length],
                arrow_color, yaw, pivot,
            );

            // Arrowhead (small pyramid approximated as a cube)
            let head_x = x + forward_x * shaft_length * 0.7;
            let head_y = y + forward_y * shaft_length * 0.7;
            let head_z = z + forward_z * shaft_length * 0.7;
            Self::generate_villager_cube(
                &mut vertices, &mut indices,
                [head_x, head_y, head_z],
                [0.08, 0.08, 0.1],
                17.0, // Gray stone color for arrowhead
                yaw, pivot,
            );
        }

        // Hostile mobs and projectiles share the same buffer
        self.hostile_mob_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.hostile_mob_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.hostile_mob_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update the plane mesh for all planes in the world
    pub fn update_plane_mesh(&mut self, planes: &[crate::entity::Plane]) {
        let mut vertices: Vec<Vertex> = Vec::with_capacity(planes.len() * 10 * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(planes.len() * 10 * 36);

        // Constant colors
        const COCKPIT_COLOR: f32 = 10.0;   // Blue tint (ice)
        const PROPELLER_COLOR: f32 = 5.0;  // Dark (wood planks)

        for plane in planes {
            let x = plane.position.x;
            let y = plane.position.y;
            let z = plane.position.z;
            let yaw = plane.yaw.to_radians();
            let pitch = plane.pitch.to_radians();
            let roll = plane.roll.to_radians();

            // Get plane's colors
            let (fuselage_color, wing_color) = plane.color.to_color_indices();

            let pivot = [x, y, z];

            // Helper to rotate a point around all three axes
            let rotate_point = |local_x: f32, local_y: f32, local_z: f32| -> [f32; 3] {
                // Apply roll (rotation around forward/Z axis)
                let cos_roll = roll.cos();
                let sin_roll = roll.sin();
                let rx1 = local_x * cos_roll - local_y * sin_roll;
                let ry1 = local_x * sin_roll + local_y * cos_roll;
                let rz1 = local_z;

                // Apply pitch (rotation around right/X axis)
                let cos_pitch = pitch.cos();
                let sin_pitch = pitch.sin();
                let rx2 = rx1;
                let ry2 = ry1 * cos_pitch - rz1 * sin_pitch;
                let rz2 = ry1 * sin_pitch + rz1 * cos_pitch;

                // Apply yaw (rotation around up/Y axis)
                let cos_yaw = yaw.cos();
                let sin_yaw = yaw.sin();
                let rx3 = rx2 * cos_yaw + rz2 * sin_yaw;
                let ry3 = ry2;
                let rz3 = -rx2 * sin_yaw + rz2 * cos_yaw;

                [x + rx3, y + ry3, z + rz3]
            };

            // Generate a rotated cube for a plane part
            let add_plane_part = |vertices: &mut Vec<Vertex>, indices: &mut Vec<u16>,
                                  offset: [f32; 3], size: [f32; 3], color: f32| {
                let base_idx = vertices.len() as u16;

                // 8 corners of the cube in local space
                let half = [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0];
                let corners = [
                    [-half[0], -half[1], -half[2]],
                    [half[0], -half[1], -half[2]],
                    [half[0], half[1], -half[2]],
                    [-half[0], half[1], -half[2]],
                    [-half[0], -half[1], half[2]],
                    [half[0], -half[1], half[2]],
                    [half[0], half[1], half[2]],
                    [-half[0], half[1], half[2]],
                ];

                // Transform corners to world space
                let mut world_corners = [[0.0f32; 3]; 8];
                for (i, corner) in corners.iter().enumerate() {
                    world_corners[i] = rotate_point(
                        corner[0] + offset[0],
                        corner[1] + offset[1],
                        corner[2] + offset[2],
                    );
                }

                // Face definitions: [v0, v1, v2, v3] with counter-clockwise winding for outward normal
                let faces = [
                    ([4, 5, 6, 7], [0.0, 0.0, 1.0]),   // Front (+Z)
                    ([1, 0, 3, 2], [0.0, 0.0, -1.0]),  // Back (-Z)
                    ([5, 1, 2, 6], [1.0, 0.0, 0.0]),   // Right (+X)
                    ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),  // Left (-X)
                    ([7, 6, 2, 3], [0.0, 1.0, 0.0]),   // Top (+Y)
                    ([0, 1, 5, 4], [0.0, -1.0, 0.0]),  // Bottom (-Y)
                ];

                // Texture coordinates for each face vertex (counter-clockwise from bottom-left)
                let tex_coords_quad = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

                for (face_indices, normal) in faces.iter() {
                    let face_base = vertices.len() as u16;

                    for (i, &vi) in face_indices.iter().enumerate() {
                        let pos = world_corners[vi];
                        vertices.push(Vertex {
                            position: pos,
                            tex_coords: tex_coords_quad[i],
                            normal: *normal,
                            block_type: color,
                            damage: 0.0,
                        });
                    }

                    // Two triangles for the quad
                    indices.push(face_base);
                    indices.push(face_base + 1);
                    indices.push(face_base + 2);
                    indices.push(face_base);
                    indices.push(face_base + 2);
                    indices.push(face_base + 3);
                }
            };

            // Fuselage (main body) - centered
            add_plane_part(&mut vertices, &mut indices,
                [0.0, 0.0, 0.0],
                [0.8, 0.6, 4.0],
                fuselage_color);

            // Cockpit (glass canopy) - forward and above
            add_plane_part(&mut vertices, &mut indices,
                [0.0, 0.4, -0.8],
                [0.6, 0.4, 0.8],
                COCKPIT_COLOR);

            // Left wing
            add_plane_part(&mut vertices, &mut indices,
                [-1.8, 0.0, 0.3],
                [3.0, 0.15, 1.2],
                wing_color);

            // Right wing
            add_plane_part(&mut vertices, &mut indices,
                [1.8, 0.0, 0.3],
                [3.0, 0.15, 1.2],
                wing_color);

            // Tail vertical stabilizer
            add_plane_part(&mut vertices, &mut indices,
                [0.0, 0.5, 1.8],
                [0.1, 0.8, 0.6],
                wing_color);

            // Tail horizontal stabilizer
            add_plane_part(&mut vertices, &mut indices,
                [0.0, 0.15, 1.8],
                [1.5, 0.1, 0.5],
                wing_color);

            // Propeller (animated rotation)
            let prop_angle = plane.propeller_rotation.to_radians();
            let prop_cos = prop_angle.cos();
            let prop_sin = prop_angle.sin();

            // Propeller blade 1 (vertical when prop_angle = 0)
            let blade1_x = prop_cos * 0.0 - prop_sin * 0.5;
            let blade1_y = prop_sin * 0.0 + prop_cos * 0.5;
            add_plane_part(&mut vertices, &mut indices,
                [blade1_x, blade1_y, -2.1],
                [0.15, 1.0, 0.1],
                PROPELLER_COLOR);

            // Propeller blade 2 (opposite)
            let blade2_x = prop_cos * 0.0 - prop_sin * (-0.5);
            let blade2_y = prop_sin * 0.0 + prop_cos * (-0.5);
            add_plane_part(&mut vertices, &mut indices,
                [blade2_x, blade2_y, -2.1],
                [0.15, 1.0, 0.1],
                PROPELLER_COLOR);

            // Propeller hub
            add_plane_part(&mut vertices, &mut indices,
                [0.0, 0.0, -2.05],
                [0.2, 0.2, 0.15],
                PROPELLER_COLOR);
        }

        self.plane_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.plane_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.plane_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update missile mesh for rendering
    pub fn update_missile_mesh(&mut self, missiles: &[crate::entity::Missile]) {
        let mut vertices: Vec<Vertex> = Vec::with_capacity(missiles.len() * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(missiles.len() * 36);

        const MISSILE_COLOR: f32 = 7.0;  // Dark gray/black (bedrock-ish)
        const MISSILE_TIP_COLOR: f32 = 1.0;  // Red (lava)

        for missile in missiles {
            if !missile.active {
                continue;
            }

            let x = missile.position.x;
            let y = missile.position.y;
            let z = missile.position.z;

            // Calculate missile orientation from velocity
            let vel = missile.velocity;
            let speed = (vel.x * vel.x + vel.y * vel.y + vel.z * vel.z).sqrt();
            if speed < 0.01 {
                continue;
            }

            let dir_x = vel.x / speed;
            let dir_y = vel.y / speed;
            let dir_z = vel.z / speed;

            // Create a simple elongated missile shape (0.2 x 0.2 x 0.8)
            let base_idx = vertices.len() as u16;

            // Right vector (perpendicular to direction)
            let right_x = dir_z;
            let right_z = -dir_x;
            let right_len = (right_x * right_x + right_z * right_z).sqrt().max(0.001);
            let right_x = right_x / right_len * 0.1;
            let right_z = right_z / right_len * 0.1;

            // Up vector
            let up_y = 0.1f32;

            // Missile body vertices (elongated box along velocity direction)
            let half_len = 0.4;
            let front = [x + dir_x * half_len, y + dir_y * half_len, z + dir_z * half_len];
            let back = [x - dir_x * half_len, y - dir_y * half_len, z - dir_z * half_len];

            // 8 corners of the missile body
            let corners = [
                // Front face (4 corners)
                [front[0] - right_x - up_y * dir_y, front[1] - up_y, front[2] - right_z],
                [front[0] + right_x - up_y * dir_y, front[1] - up_y, front[2] + right_z],
                [front[0] + right_x + up_y * dir_y, front[1] + up_y, front[2] + right_z],
                [front[0] - right_x + up_y * dir_y, front[1] + up_y, front[2] - right_z],
                // Back face (4 corners)
                [back[0] - right_x - up_y * dir_y, back[1] - up_y, back[2] - right_z],
                [back[0] + right_x - up_y * dir_y, back[1] - up_y, back[2] + right_z],
                [back[0] + right_x + up_y * dir_y, back[1] + up_y, back[2] + right_z],
                [back[0] - right_x + up_y * dir_y, back[1] + up_y, back[2] - right_z],
            ];

            // Add vertices for all 6 faces
            let face_indices = [
                ([0, 1, 2, 3], [dir_x, dir_y, dir_z], MISSILE_TIP_COLOR),     // Front (red tip)
                ([5, 4, 7, 6], [-dir_x, -dir_y, -dir_z], MISSILE_COLOR),      // Back
                ([4, 0, 3, 7], [-right_x, 0.0, -right_z], MISSILE_COLOR),     // Left
                ([1, 5, 6, 2], [right_x, 0.0, right_z], MISSILE_COLOR),       // Right
                ([3, 2, 6, 7], [0.0, 1.0, 0.0], MISSILE_COLOR),               // Top
                ([4, 5, 1, 0], [0.0, -1.0, 0.0], MISSILE_COLOR),              // Bottom
            ];

            for (corner_idx, normal, color) in face_indices.iter() {
                let face_base = vertices.len() as u16;
                for &ci in corner_idx {
                    vertices.push(Vertex {
                        position: corners[ci],
                        tex_coords: [0.0, 0.0],
                        normal: [normal[0], normal[1], normal[2]],
                        block_type: *color,
                        damage: 0.0,
                    });
                }
                indices.extend_from_slice(&[
                    face_base, face_base + 1, face_base + 2,
                    face_base, face_base + 2, face_base + 3,
                ]);
            }
        }

        self.missile_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.missile_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.missile_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update bomb mesh for rendering (bombs are round/oval shaped)
    pub fn update_bomb_mesh(&mut self, bombs: &[crate::entity::Bomb]) {
        let mut vertices: Vec<Vertex> = Vec::with_capacity(bombs.len() * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(bombs.len() * 36);

        const BOMB_COLOR: f32 = 7.0;   // Dark (like coal/obsidian)
        const BOMB_NOSE_COLOR: f32 = 1.0;  // Red tip

        for bomb in bombs {
            if !bomb.active {
                continue;
            }

            let x = bomb.position.x;
            let y = bomb.position.y;
            let z = bomb.position.z;

            // Bomb is a simple box shape (0.4 x 0.4 x 0.6) - taller than wide
            let half_w = 0.2;
            let half_h = 0.3;

            // 8 corners of the bomb
            let corners = [
                [x - half_w, y - half_h, z - half_w],  // 0: bottom back left
                [x + half_w, y - half_h, z - half_w],  // 1: bottom back right
                [x + half_w, y - half_h, z + half_w],  // 2: bottom front right
                [x - half_w, y - half_h, z + half_w],  // 3: bottom front left
                [x - half_w, y + half_h, z - half_w],  // 4: top back left
                [x + half_w, y + half_h, z - half_w],  // 5: top back right
                [x + half_w, y + half_h, z + half_w],  // 6: top front right
                [x - half_w, y + half_h, z + half_w],  // 7: top front left
            ];

            // 6 faces with normals and colors
            let faces = [
                ([0, 1, 2, 3], [0.0, -1.0, 0.0], BOMB_NOSE_COLOR),  // Bottom (red - nose)
                ([4, 7, 6, 5], [0.0, 1.0, 0.0], BOMB_COLOR),        // Top
                ([0, 4, 5, 1], [0.0, 0.0, -1.0], BOMB_COLOR),       // Back
                ([2, 6, 7, 3], [0.0, 0.0, 1.0], BOMB_COLOR),        // Front
                ([0, 3, 7, 4], [-1.0, 0.0, 0.0], BOMB_COLOR),       // Left
                ([1, 5, 6, 2], [1.0, 0.0, 0.0], BOMB_COLOR),        // Right
            ];

            for (corner_idx, normal, color) in faces.iter() {
                let face_base = vertices.len() as u16;
                for &ci in corner_idx {
                    vertices.push(Vertex {
                        position: corners[ci],
                        tex_coords: [0.0, 0.0],
                        normal: *normal,
                        block_type: *color,
                        damage: 0.0,
                    });
                }
                indices.extend_from_slice(&[
                    face_base, face_base + 1, face_base + 2,
                    face_base, face_base + 2, face_base + 3,
                ]);
            }
        }

        self.bomb_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.bomb_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.bomb_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    /// Update the block preview mesh for placement visualization
    pub fn update_block_preview(&mut self, placement_pos: Option<(i32, i32, i32)>, block_type: Option<crate::world::BlockType>) {
        use crate::world::BlockType;

        if let (Some((x, y, z)), Some(bt)) = (placement_pos, block_type) {
            // Skip preview for non-placeable blocks
            if bt == BlockType::Air || bt == BlockType::Barrier {
                self.preview_visible = false;
                return;
            }

            let mut vertices: Vec<Vertex> = Vec::with_capacity(24);
            let mut indices: Vec<u16> = Vec::with_capacity(36);

            let x = x as f32;
            let y = y as f32;
            let z = z as f32;
            let block_type_f = bt as u32 as f32;
            let preview_damage = -1.0; // Flag for semi-transparent preview

            // Generate all 6 faces of the cube
            Self::add_quad_face(&mut vertices, &mut indices,
                [x, y, z + 1.0], [x + 1.0, y, z + 1.0], [x + 1.0, y, z], [x, y, z],
                [0.0, -1.0, 0.0], block_type_f, preview_damage);
            Self::add_quad_face(&mut vertices, &mut indices,
                [x, y + 1.0, z], [x + 1.0, y + 1.0, z], [x + 1.0, y + 1.0, z + 1.0], [x, y + 1.0, z + 1.0],
                [0.0, 1.0, 0.0], block_type_f, preview_damage);
            Self::add_quad_face(&mut vertices, &mut indices,
                [x, y, z], [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z],
                [0.0, 0.0, -1.0], block_type_f, preview_damage);
            Self::add_quad_face(&mut vertices, &mut indices,
                [x + 1.0, y, z + 1.0], [x, y, z + 1.0], [x, y + 1.0, z + 1.0], [x + 1.0, y + 1.0, z + 1.0],
                [0.0, 0.0, 1.0], block_type_f, preview_damage);
            Self::add_quad_face(&mut vertices, &mut indices,
                [x, y, z + 1.0], [x, y, z], [x, y + 1.0, z], [x, y + 1.0, z + 1.0],
                [-1.0, 0.0, 0.0], block_type_f, preview_damage);
            Self::add_quad_face(&mut vertices, &mut indices,
                [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x + 1.0, y + 1.0, z + 1.0], [x + 1.0, y + 1.0, z],
                [1.0, 0.0, 0.0], block_type_f, preview_damage);

            self.preview_index_count = indices.len() as u32;
            self.preview_visible = true;

            self.queue.write_buffer(&self.preview_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.preview_index_buffer, 0, bytemuck::cast_slice(&indices));
        } else {
            self.preview_visible = false;
        }
    }

    /// Update dropped item meshes for rendering
    pub fn update_dropped_items(&mut self, items: &[crate::entity::DroppedItem]) {
        if items.is_empty() {
            self.dropped_item_index_count = 0;
            return;
        }

        let mut vertices: Vec<Vertex> = Vec::with_capacity(items.len() * 24);
        let mut indices: Vec<u16> = Vec::with_capacity(items.len() * 36);

        let item_size = 0.3;  // Small cube size
        let half_size = item_size / 2.0;

        for item in items {
            let x = item.position.x;
            let y = item.position.y;
            let z = item.position.z;

            // Bobbing animation
            let bob = (item.bobbing_phase + self.time_of_day * 200.0).sin() * 0.1;
            let y = y + bob;

            // Rotation
            let cos_r = item.rotation.cos();
            let sin_r = item.rotation.sin();

            // Rotate cube corners around Y axis
            let rotate = |dx: f32, dz: f32| -> (f32, f32) {
                (dx * cos_r - dz * sin_r, dx * sin_r + dz * cos_r)
            };

            // 8 corners of the rotated cube
            let (rx0, rz0) = rotate(-half_size, -half_size);
            let (rx1, rz1) = rotate(half_size, -half_size);
            let (rx2, rz2) = rotate(half_size, half_size);
            let (rx3, rz3) = rotate(-half_size, half_size);
            let corners: [[f32; 3]; 8] = [
                [x + rx0, y - half_size, z + rz0],  // 0: bottom-back-left
                [x + rx1, y - half_size, z + rz1],  // 1: bottom-back-right
                [x + rx2, y - half_size, z + rz2],  // 2: bottom-front-right
                [x + rx3, y - half_size, z + rz3],  // 3: bottom-front-left
                [x + rx0, y + half_size, z + rz0],  // 4: top-back-left
                [x + rx1, y + half_size, z + rz1],  // 5: top-back-right
                [x + rx2, y + half_size, z + rz2],  // 6: top-front-right
                [x + rx3, y + half_size, z + rz3],  // 7: top-front-left
            ];

            // Get texture index for block or tool
            let block_type_f = match &item.item {
                ItemStack::Block(block_type, _) => *block_type as u32 as f32,
                ItemStack::Tool(tool) => {
                    // Use tool texture indices (60-79 range reserved for tools)
                    let type_offset = match tool.tool_type {
                        ToolType::Pickaxe => 60.0,
                        ToolType::Axe => 65.0,
                        ToolType::Shovel => 70.0,
                        ToolType::Sword => 75.0,
                    };
                    let material_offset = match tool.material {
                        ToolMaterial::Wood => 0.0,
                        ToolMaterial::Stone => 1.0,
                        ToolMaterial::Iron => 2.0,
                        ToolMaterial::Gold => 3.0,
                        ToolMaterial::Diamond => 4.0,
                    };
                    type_offset + material_offset
                }
            };

            // Add 6 faces (order: bottom, top, front, back, left, right)
            let faces = [
                ([0, 1, 2, 3], [0.0, -1.0, 0.0]),  // Bottom (Y-)
                ([4, 7, 6, 5], [0.0, 1.0, 0.0]),   // Top (Y+)
                ([3, 2, 6, 7], [0.0, 0.0, 1.0]),   // Front (Z+) rotated
                ([1, 0, 4, 5], [0.0, 0.0, -1.0]),  // Back (Z-) rotated
                ([0, 3, 7, 4], [-1.0, 0.0, 0.0]),  // Left (X-) rotated
                ([2, 1, 5, 6], [1.0, 0.0, 0.0]),   // Right (X+) rotated
            ];

            for (face_indices, normal) in faces {
                Self::add_quad_face(
                    &mut vertices, &mut indices,
                    corners[face_indices[0]], corners[face_indices[1]],
                    corners[face_indices[2]], corners[face_indices[3]],
                    normal, block_type_f, 0.0,
                );
            }
        }

        self.dropped_item_index_count = indices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.dropped_item_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.dropped_item_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
    }

    fn update_particle_mesh(&mut self, camera: &Camera, particle_system: &ParticleSystem) {
        let particles = particle_system.get_particles();
        if particles.is_empty() {
            self.particle_vertex_count = 0;
            return;
        }

        // Calculate camera right and up vectors for billboarding
        let yaw_rad = camera.yaw.to_radians();
        let pitch_rad = camera.pitch.to_radians();

        let forward = Vector3::new(
            yaw_rad.sin() * pitch_rad.cos(),
            -pitch_rad.sin(),
            -yaw_rad.cos() * pitch_rad.cos(),
        ).normalize();

        let world_up = Vector3::new(0.0, 1.0, 0.0);
        let camera_right = forward.cross(world_up).normalize();
        let camera_up = camera_right.cross(forward).normalize();

        // Update particle uniform
        let particle_uniform = ParticleUniform {
            view_proj: camera.view_proj.into(),
            camera_right: [camera_right.x, camera_right.y, camera_right.z],
            _pad1: 0.0,
            camera_up: [camera_up.x, camera_up.y, camera_up.z],
            _pad2: 0.0,
        };
        self.queue.write_buffer(&self.particle_uniform_buffer, 0, bytemuck::cast_slice(&[particle_uniform]));

        // Generate particle vertices (4 vertices per particle for a quad)
        let mut vertices: Vec<ParticleVertex> = Vec::with_capacity(particles.len() * 6);

        // Quad offsets for two triangles
        let offsets = [
            [-1.0, -1.0], [1.0, -1.0], [1.0, 1.0],  // First triangle
            [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0],  // Second triangle
        ];

        for particle in particles {
            let alpha = particle.alpha();
            let color = [particle.color[0], particle.color[1], particle.color[2], alpha];

            for offset in &offsets {
                vertices.push(ParticleVertex {
                    position: [particle.position.x, particle.position.y, particle.position.z],
                    offset: *offset,
                    color,
                    size: particle.size,
                });
            }
        }

        self.particle_vertex_count = vertices.len() as u32;

        if !vertices.is_empty() {
            self.queue.write_buffer(&self.particle_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }
}

#[derive(Copy, Clone)]
enum Face {
    Top, Bottom, Right, Left, Front, Back,
}

const TOP_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, 1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, 1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, 1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, 1.0, 0.0], block_type: 0.0, damage: 0.0 },
];

const BOTTOM_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, -1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, -1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, -1.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, -1.0, 0.0], block_type: 0.0, damage: 0.0 },
];

const RIGHT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [0.0, 1.0], normal: [1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 0.0], normal: [1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
];

const LEFT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [1.0, 1.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 0.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0, damage: 0.0 },
];

const FRONT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, 0.0, 1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 0.0], normal: [0.0, 0.0, 1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 0.0], normal: [0.0, 0.0, 1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, 0.0, 1.0], block_type: 0.0, damage: 0.0 },
];

const BACK_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [1.0, 1.0], normal: [0.0, 0.0, -1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [0.0, 1.0], normal: [0.0, 0.0, -1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, 0.0, -1.0], block_type: 0.0, damage: 0.0 },
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, 0.0, -1.0], block_type: 0.0, damage: 0.0 },
];

const FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];