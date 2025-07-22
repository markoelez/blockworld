use wgpu::util::DeviceExt;
use winit::window::Window;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use image::GenericImageView;
use cgmath::SquareMatrix;
use std::time::Instant;
use cgmath::{Matrix4, Deg, Vector3, Vector4, InnerSpace, EuclideanSpace, Matrix};

use crate::camera::Camera;
use crate::world::{World, BlockType};
use crate::ui::{Inventory, UIRenderer};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
    normal: [f32; 3],
    block_type: f32, // 0=grass, 1=dirt, 2=stone, 3=wood, 4=leaves
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

struct ChunkMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inverse_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    _pad: f32,
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
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
    depth_texture: wgpu::TextureView,
    chunk_meshes: HashMap<(i32, i32), ChunkMesh>, // Chunk position -> mesh
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
    held_item_vertex_buffer: wgpu::Buffer,
    held_item_index_buffer: wgpu::Buffer,
    held_item_index_count: u32,
    last_render: Instant,
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
            ],
            label: Some("texture_bind_group_layout"),
        });
        
        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
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
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x3,
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
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
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
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 5]>() as wgpu::BufferAddress,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x3,
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
                module: &shader,
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
                    format: config.format,
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
        
        // Create texture views
        let grass_view = grass_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let grass_top_view = grass_top_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let dirt_view = dirt_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let stone_view = stone_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let wood_view = wood_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let leaves_view = leaves_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let water_view = water_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
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
        let ui_renderer = UIRenderer::new(&device, &queue, &config, &texture_bind_group, &texture_bind_group_layout);
        
        let mut held_indices: Vec<u16> = vec![];
        let face_indices = &[0u16, 1, 2, 2, 3, 0];
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

        let held_item_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Held Item Vertex Buffer"),
            size: (48 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let held_item_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Held Item Index Buffer"),
            contents: bytemuck::cast_slice(&held_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let arm_swing_progress = 0.0;
        let last_render = Instant::now();

        Self {
            surface,
            device,
            queue,
            config,
            render_pipeline,
            outline_pipeline,
            sky_pipeline,
            cloud_pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group,
            depth_texture,
            chunk_meshes: HashMap::new(),
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
            held_item_vertex_buffer,
            held_item_index_buffer,
            held_item_index_count,
            last_render,
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
    
    pub fn render(&mut self, camera: &Camera, world: &mut World, inventory: &Inventory, targeted_block: Option<(i32, i32, i32)>) {
        let now = Instant::now();
        let dt = (now - self.last_render).as_secs_f32();
        self.last_render = now;
        self.arm_swing_progress = (self.arm_swing_progress - dt * 4.0).max(0.0);

        // Update chunk meshes for loaded chunks
        self.update_chunk_meshes(world);
        
        let output = self.surface.get_current_texture().unwrap();
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let view_proj = camera.view_proj;
        let inverse_view_proj = view_proj.invert().unwrap();
        
        let uniform = Uniform {
            view_proj: view_proj.into(),
            inverse_view_proj: inverse_view_proj.into(),
            camera_pos: [camera.position.x, camera.position.y, camera.position.z],
            _pad: 0.0,
        };
        
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
        
        // Update outline buffers if a block is targeted
        if let Some((block_x, block_y, block_z)) = targeted_block {
            self.update_outline_buffers(block_x, block_y, block_z);
        }
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
                    view: &self.depth_texture,
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
            
            // Render terrain blocks
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            
            // Render opaque chunks
            for chunk_mesh in self.chunk_meshes_opaque.values() {
                if chunk_mesh.index_count > 0 {
                    render_pass.set_vertex_buffer(0, chunk_mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(chunk_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..chunk_mesh.index_count, 0, 0..1);
                }
            }

            // Render transparent chunks
            render_pass.set_pipeline(&self.transparent_pipeline);
            for chunk_mesh in self.chunk_meshes_transparent.values() {
                if chunk_mesh.index_count > 0 {
                    render_pass.set_vertex_buffer(0, chunk_mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(chunk_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..chunk_mesh.index_count, 0, 0..1);
                }
            }
            
            // Render 3D clouds (Minecraft-style volumetric clouds)
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
                render_pass.draw_indexed(0..72, 0, 0..1); // 12 edges * 6 indices each (2 triangles)
            }

            if let Some(block_type) = inventory.get_selected_block() {
                let vertices = Self::create_held_item_vertices(camera, block_type, self.arm_swing_progress);
                self.queue.write_buffer(&self.held_item_vertex_buffer, 0, bytemuck::cast_slice(&vertices));

                render_pass.set_pipeline(if Self::is_transparent(block_type) { &self.transparent_pipeline } else { &self.render_pipeline });
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.held_item_vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.held_item_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.held_item_index_count, 0, 0..1);
            }
        }
        
        // Render UI in a separate pass (no depth testing, renders on top)
        {
            let mut ui_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Load existing content (don't clear)
                        store: true,
                    },
                })],
                depth_stencil_attachment: None, // No depth testing for UI
            });
            
            self.ui_renderer.update_inventory_selection(&self.device, &self.queue, inventory);
            self.ui_renderer.render(&mut ui_render_pass, &self.texture_bind_group);
        }
        
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
    
    fn update_chunk_meshes(&mut self, world: &mut World) {
        let loaded_chunks: HashMap<(i32, i32), ()> = world.get_loaded_chunks()
            .map(|chunk| ((chunk.position.x, chunk.position.z), ()))
            .collect();
        
        self.chunk_meshes_opaque.retain(|key, _| loaded_chunks.contains_key(key));
        self.chunk_meshes_transparent.retain(|key, _| loaded_chunks.contains_key(key));
        
        let dirty_chunks: Vec<(i32, i32)> = world.get_loaded_chunks()
            .filter(|chunk| chunk.dirty || !chunk.mesh_generated)
            .map(|chunk| (chunk.position.x, chunk.position.z))
            .collect();
        
        for (chunk_x, chunk_z) in &dirty_chunks {
            let chunk_key = (*chunk_x, *chunk_z);
            if let Some(chunk) = world.chunks.get(&chunk_key) {
                let (opaque_vertices, opaque_indices, trans_vertices, trans_indices) = Self::create_vertices_for_chunk(world, chunk);
                
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
        }
        
        for (chunk_x, chunk_z) in &dirty_chunks {
            let chunk_key = (*chunk_x, *chunk_z);
            if let Some(chunk) = world.chunks.get_mut(&chunk_key) {
                chunk.dirty = false;
                chunk.mesh_generated = true;
            }
        }
    }
    
    fn create_vertices_for_chunk(world: &World, chunk: &crate::world::Chunk) -> (Vec<Vertex>, Vec<u16>, Vec<Vertex>, Vec<u16>) {
        let mut opaque_vertices = Vec::new();
        let mut opaque_indices = Vec::new();
        let mut trans_vertices = Vec::new();
        let mut trans_indices = Vec::new();
        
        for x in 0..World::CHUNK_SIZE {
            for y in 0..World::CHUNK_HEIGHT {
                for z in 0..World::CHUNK_SIZE {
                    let block_type = chunk.blocks[x][y][z];
                    if block_type == BlockType::Air || block_type == BlockType::Barrier {
                        continue;
                    }
                    
                    let world_x = chunk.position.x * World::CHUNK_SIZE as i32 + x as i32;
                    let world_z = chunk.position.z * World::CHUNK_SIZE as i32 + z as i32;
                    
                    let pos = Vector3::new(world_x as f32, y as f32, world_z as f32);
                    
                    let (vertices, indices) = if Self::is_transparent(block_type) {
                        (&mut trans_vertices, &mut trans_indices)
                    } else {
                        (&mut opaque_vertices, &mut opaque_indices)
                    };
                    
                    // Only render faces that are exposed (face culling optimization)
                    if Self::is_face_exposed(world, world_x, y as i32 + 1, world_z, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Top, block_type);
                    }
                    if Self::is_face_exposed(world, world_x, y as i32 - 1, world_z, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Bottom, block_type);
                    }
                    if Self::is_face_exposed(world, world_x + 1, y as i32, world_z, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Right, block_type);
                    }
                    if Self::is_face_exposed(world, world_x - 1, y as i32, world_z, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Left, block_type);
                    }
                    if Self::is_face_exposed(world, world_x, y as i32, world_z + 1, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Front, block_type);
                    }
                    if Self::is_face_exposed(world, world_x, y as i32, world_z - 1, block_type) {
                        Self::add_face(vertices, indices, pos, Face::Back, block_type);
                    }
                }
            }
        }
        
        (opaque_vertices, opaque_indices, trans_vertices, trans_indices)
    }

    fn create_held_item_vertices(camera: &Camera, block_type: BlockType, progress: f32) -> Vec<Vertex> {
        let block_type_f = Self::block_type_to_float(block_type);
        let size = 0.4;

        let face_verts = [
            TOP_FACE_VERTICES,
            BOTTOM_FACE_VERTICES,
            RIGHT_FACE_VERTICES,
            LEFT_FACE_VERTICES,
            FRONT_FACE_VERTICES,
            BACK_FACE_VERTICES,
        ];

        let mut item_verts: Vec<Vertex> = vec![];
        for &face_vert in face_verts.iter() {
            for v in face_vert.iter() {
                let mut new_v = *v;
                new_v.position[0] = (new_v.position[0] - 0.5) * size;
                new_v.position[1] = (new_v.position[1] - 0.5) * size;
                new_v.position[2] = (new_v.position[2] - 0.5) * size;
                new_v.block_type = block_type_f;
                item_verts.push(new_v);
            }
        }

        let mut arm_verts: Vec<Vertex> = vec![];
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
                arm_verts.push(new_v);
            }
        }

        let mut verts = vec![];
        verts.extend(item_verts);
        verts.extend(arm_verts);

        let front = camera.get_look_direction();
        let up = Vector3::unit_y();
        let right = front.cross(up).normalize();
        let true_up = right.cross(front).normalize();

        let item_offset = right * 0.5 - true_up * 0.8 + front * 0.2;
        let item_pos = camera.position + item_offset;

        let angle = -progress.powi(2) * 80.0;
        let rotation = Matrix4::from_axis_angle(right, Deg(angle));

        let tilt_rotation = Matrix4::from_axis_angle(right, Deg(-30.0)) * Matrix4::from_axis_angle(front, Deg(20.0));

        let model = Matrix4::from_translation(item_pos.to_vec()) * rotation * tilt_rotation;
        let normal_mat = model.invert().unwrap().transpose();

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
                block == BlockType::Air
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
            _ => 0.0,
        }
    }
    
    fn add_face(vertices: &mut Vec<Vertex>, indices: &mut Vec<u16>, pos: Vector3<f32>, face: Face, block_type: BlockType) {
        let base_index = vertices.len() as u16;
        let block_type_f = Self::block_type_to_float(block_type);
        
        let (face_vertices, face_indices) = match face {
            Face::Top => (TOP_FACE_VERTICES, TOP_FACE_INDICES),
            Face::Bottom => (BOTTOM_FACE_VERTICES, BOTTOM_FACE_INDICES),
            Face::Right => (RIGHT_FACE_VERTICES, RIGHT_FACE_INDICES),
            Face::Left => (LEFT_FACE_VERTICES, LEFT_FACE_INDICES),
            Face::Front => (FRONT_FACE_VERTICES, FRONT_FACE_INDICES),
            Face::Back => (BACK_FACE_VERTICES, BACK_FACE_INDICES),
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
            });
        }
        
        for i in face_indices {
            indices.push(base_index + i);
        }
    }

    fn is_transparent(block_type: BlockType) -> bool {
        matches!(block_type, BlockType::Water)
    }
}

#[derive(Copy, Clone)]
enum Face {
    Top, Bottom, Right, Left, Front, Back,
}

const TOP_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, 1.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, 1.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, 1.0, 0.0], block_type: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, 1.0, 0.0], block_type: 0.0 },
];

const BOTTOM_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, -1.0, 0.0], block_type: 0.0 },
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, -1.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, -1.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, -1.0, 0.0], block_type: 0.0 },
];

const RIGHT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [0.0, 1.0], normal: [1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 0.0], normal: [1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [1.0, 0.0, 0.0], block_type: 0.0 },
];

const LEFT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [1.0, 1.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 0.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0 },
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [-1.0, 0.0, 0.0], block_type: 0.0 },
];

const FRONT_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0], normal: [0.0, 0.0, 1.0], block_type: 0.0 },
    Vertex { position: [0.0, 1.0, 1.0], tex_coords: [0.0, 0.0], normal: [0.0, 0.0, 1.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 1.0], tex_coords: [1.0, 0.0], normal: [0.0, 0.0, 1.0], block_type: 0.0 },
    Vertex { position: [1.0, 0.0, 1.0], tex_coords: [1.0, 1.0], normal: [0.0, 0.0, 1.0], block_type: 0.0 },
];

const BACK_FACE_VERTICES: &[Vertex] = &[
    Vertex { position: [0.0, 0.0, 0.0], tex_coords: [1.0, 1.0], normal: [0.0, 0.0, -1.0], block_type: 0.0 },
    Vertex { position: [1.0, 0.0, 0.0], tex_coords: [0.0, 1.0], normal: [0.0, 0.0, -1.0], block_type: 0.0 },
    Vertex { position: [1.0, 1.0, 0.0], tex_coords: [0.0, 0.0], normal: [0.0, 0.0, -1.0], block_type: 0.0 },
    Vertex { position: [0.0, 1.0, 0.0], tex_coords: [1.0, 0.0], normal: [0.0, 0.0, -1.0], block_type: 0.0 },
];

const TOP_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
const BOTTOM_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
const RIGHT_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
const LEFT_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
const FRONT_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];
const BACK_FACE_INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];