use wgpu::util::DeviceExt;
use winit::window::Window;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use image::GenericImageView;
use cgmath::SquareMatrix;
use std::time::Instant;
use cgmath::{Matrix4, Deg, Vector3, Vector4, InnerSpace, Matrix, Point3};
use rayon::prelude::*;

use crate::camera::Camera;
use crate::world::{World, BlockType, TorchFace};
use crate::ui::{Inventory, UIRenderer, DebugInfo, PauseMenu, ChestUI};
use crate::entity::{EntityManager, Villager, VillagerState, VILLAGER_HEIGHT};
use crate::particle::ParticleSystem;

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
        
        // Create indices for up to 2 cubes (item + arm)
        let mut held_indices: Vec<u16> = vec![];
        let face_indices = &[0u16, 1, 2, 2, 3, 0];
        
        // Always create indices for 2 cubes worth of vertices
        for cube in 0..2 {
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
            size: (48 * std::mem::size_of::<Vertex>()) as u64, // 2 cubes * 24 vertices each
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
            time_of_day: 0.25,  // Start at morning
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

        // Search nearby blocks for torches
        for x in (cam_x - search_radius)..(cam_x + search_radius) {
            for z in (cam_z - search_radius)..(cam_z + search_radius) {
                for y in (cam_y - 20).max(0)..(cam_y + 20).min(World::CHUNK_HEIGHT as i32) {
                    if let Some(BlockType::Torch) = world.get_block(x, y, z) {
                        if (num_lights as usize) < MAX_POINT_LIGHTS {
                            lights[num_lights as usize] = PointLight {
                                position: [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5],
                                radius: 12.0,
                                color: [1.0, 0.8, 0.4],  // Warm orange glow
                                intensity: 1.5,
                            };
                            num_lights += 1;
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
    
    pub fn render(&mut self, camera: &Camera, world: &mut World, inventory: &Inventory, targeted_block: Option<(i32, i32, i32)>, entity_manager: &EntityManager, particle_system: &ParticleSystem, underwater: bool, debug_info: &DebugInfo, pause_menu: &PauseMenu, chest_ui: &ChestUI) {
        let now = Instant::now();
        let dt = (now - self.last_render).as_secs_f32();
        self.last_render = now;
        self.arm_swing_progress = (self.arm_swing_progress - dt * 4.0).max(0.0);

        // Update time of day (full cycle every 10 minutes)
        let elapsed = (now - self.start_time).as_secs_f32();
        self.time_of_day = (elapsed / 600.0).fract();

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
        let ambient_intensity = 0.35 + 0.25 * day_factor;

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
            fog_density: 0.002,
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

            let opt_block_type = inventory.get_selected_block();
            let vertices = Self::create_held_item_vertices(camera, opt_block_type, self.arm_swing_progress);
            self.queue.write_buffer(&self.held_item_vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            // Render held item
            render_pass.set_pipeline(&self.held_item_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.held_item_vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.held_item_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.held_item_index_count, 0, 0..1);
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
        {
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
                let chest_contents = world.chest_contents.get(&chest_pos).cloned().unwrap_or_default();
                self.ui_renderer.render_chest_ui(
                    &self.device,
                    &self.queue,
                    &view,
                    &self.texture_bind_group,
                    chest_ui,
                    &chest_contents,
                    inventory,
                );
            }
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
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch {
                        continue;  // Torches are rendered separately with special geometry
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

        // Add 6 faces of the stick (excluding top, flame goes there)
        // Front face (+Z)
        Self::add_quad_face(vertices, indices, b3, b2, t2, t3, [0.0, 0.0, 1.0], block_type);
        // Back face (-Z)
        Self::add_quad_face(vertices, indices, b1, b0, t0, t1, [0.0, 0.0, -1.0], block_type);
        // Right face (+X)
        Self::add_quad_face(vertices, indices, b2, b1, t1, t2, [1.0, 0.0, 0.0], block_type);
        // Left face (-X)
        Self::add_quad_face(vertices, indices, b0, b3, t3, t0, [-1.0, 0.0, 0.0], block_type);
        // Bottom face (-Y)
        Self::add_quad_face(vertices, indices, b3, b2, b1, b0, [0.0, -1.0, 0.0], block_type);

        // Add flame on top of the torch
        let flame_type = 25.0;  // Special block type for bright flame
        let flame_hw = hw * 1.2;  // Slightly wider than stick
        let flame_height = 0.15;  // Flame height

        // Flame bottom (at top of stick)
        let f_b0 = [top_cx - flame_hw, top_y, top_cz - flame_hw];
        let f_b1 = [top_cx + flame_hw, top_y, top_cz - flame_hw];
        let f_b2 = [top_cx + flame_hw, top_y, top_cz + flame_hw];
        let f_b3 = [top_cx - flame_hw, top_y, top_cz + flame_hw];

        // Flame top (slightly higher, with additional tilt for animation feel)
        let flame_top_y = top_y + flame_height;
        let f_t0 = [top_cx - flame_hw * 0.5, flame_top_y, top_cz - flame_hw * 0.5];
        let f_t1 = [top_cx + flame_hw * 0.5, flame_top_y, top_cz - flame_hw * 0.5];
        let f_t2 = [top_cx + flame_hw * 0.5, flame_top_y, top_cz + flame_hw * 0.5];
        let f_t3 = [top_cx - flame_hw * 0.5, flame_top_y, top_cz + flame_hw * 0.5];

        // Add flame faces (tapered shape)
        Self::add_quad_face(vertices, indices, f_b3, f_b2, f_t2, f_t3, [0.0, 0.0, 1.0], flame_type);
        Self::add_quad_face(vertices, indices, f_b1, f_b0, f_t0, f_t1, [0.0, 0.0, -1.0], flame_type);
        Self::add_quad_face(vertices, indices, f_b2, f_b1, f_t1, f_t2, [1.0, 0.0, 0.0], flame_type);
        Self::add_quad_face(vertices, indices, f_b0, f_b3, f_t3, f_t0, [-1.0, 0.0, 0.0], flame_type);
        Self::add_quad_face(vertices, indices, f_t0, f_t1, f_t2, f_t3, [0.0, 1.0, 0.0], flame_type);
        Self::add_quad_face(vertices, indices, f_b3, f_b2, f_b1, f_b0, [0.0, -1.0, 0.0], flame_type);
    }

    // Add a single quad face
    fn add_quad_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], v3: [f32; 3],
        normal: [f32; 3],
        block_type: f32,
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
                damage: 0.0,
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
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch {
                        continue;  // Torches are rendered separately with special geometry
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
                        Self::add_greedy_face_horizontal(
                            trans_vertices, trans_indices,
                            world_x as f32, y as f32, world_z as f32,
                            width as f32, depth as f32,
                            face, block_type, water_depth,
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
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch {
                        continue;  // Torches are rendered separately with special geometry
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
                    if block_type == BlockType::Air || block_type == BlockType::Barrier || block_type == BlockType::Torch {
                        continue;  // Torches are rendered separately with special geometry
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

    fn create_held_item_vertices(_camera: &Camera, opt_block_type: Option<BlockType>, progress: f32) -> Vec<Vertex> {
        let block_type_f = opt_block_type.map_or(6.0, Self::block_type_to_float);
        let size = 0.4;

        let face_verts = [
            TOP_FACE_VERTICES,
            BOTTOM_FACE_VERTICES,
            RIGHT_FACE_VERTICES,
            LEFT_FACE_VERTICES,
            FRONT_FACE_VERTICES,
            BACK_FACE_VERTICES,
        ];

        let mut verts: Vec<Vertex> = vec![];

        // Generate item vertices if a block is selected
        if opt_block_type.is_some() {
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
        } else {
            // Add dummy vertices for the item cube if no block is selected
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
                new_v.block_type = 6.0;
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
        matches!(block_type, BlockType::Water)
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

            // Block type as f32 for shader
            let block_type_f = bt as u32 as f32;

            // Generate a full cube with all 6 faces
            // Bottom face (Y-)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x, y, z + 1.0], [x + 1.0, y, z + 1.0], [x + 1.0, y, z], [x, y, z],
                [0.0, -1.0, 0.0], block_type_f,
            );
            // Top face (Y+)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x, y + 1.0, z], [x + 1.0, y + 1.0, z], [x + 1.0, y + 1.0, z + 1.0], [x, y + 1.0, z + 1.0],
                [0.0, 1.0, 0.0], block_type_f,
            );
            // Front face (Z-)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x, y, z], [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z],
                [0.0, 0.0, -1.0], block_type_f,
            );
            // Back face (Z+)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x + 1.0, y, z + 1.0], [x, y, z + 1.0], [x, y + 1.0, z + 1.0], [x + 1.0, y + 1.0, z + 1.0],
                [0.0, 0.0, 1.0], block_type_f,
            );
            // Left face (X-)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x, y, z + 1.0], [x, y, z], [x, y + 1.0, z], [x, y + 1.0, z + 1.0],
                [-1.0, 0.0, 0.0], block_type_f,
            );
            // Right face (X+)
            Self::add_preview_face(
                &mut vertices, &mut indices,
                [x + 1.0, y, z], [x + 1.0, y, z + 1.0], [x + 1.0, y + 1.0, z + 1.0], [x + 1.0, y + 1.0, z],
                [1.0, 0.0, 0.0], block_type_f,
            );

            self.preview_index_count = indices.len() as u32;
            self.preview_visible = true;

            self.queue.write_buffer(&self.preview_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.preview_index_buffer, 0, bytemuck::cast_slice(&indices));
        } else {
            self.preview_visible = false;
        }
    }

    // Add a preview face with semi-transparent flag (damage = -1.0)
    fn add_preview_face(
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], v3: [f32; 3],
        normal: [f32; 3],
        block_type: f32,
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
                damage: -1.0,  // Flag for semi-transparent preview
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

            let block_type_f = item.block_type as u32 as f32;

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
                    normal, block_type_f,
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