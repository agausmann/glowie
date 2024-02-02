use bytemuck::{Pod, Zeroable};
use wgpu::RenderPipelineDescriptor;

use crate::GraphicsContext;

const STORAGE_DIMENSION: wgpu::TextureDimension = wgpu::TextureDimension::D2;
const STORAGE_VIEW_DIMENSION: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2;
const STORAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

const MAX_SAMPLES: usize = 4096;

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Config {
    window_size: [f32; 2],
    sample_count: u32,
    line_radius: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window_size: [720.0, 720.0],
            sample_count: 0,
            line_radius: 5.0,
        }
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Sample {
    xy: [f32; 2],
}

struct SizeDependent {
    a: wgpu::Texture,
    b: wgpu::Texture,
    a_view: wgpu::TextureView,
    b_view: wgpu::TextureView,
    front: wgpu::BindGroup,
    back: wgpu::BindGroup,
}

impl SizeDependent {
    fn new(gfx: &GraphicsContext, texture_bind_group_layout: &wgpu::BindGroupLayout) -> Self {
        let window_size = gfx.window.inner_size();
        let texture_descriptor = wgpu::TextureDescriptor {
            label: Some("Scope.texture_descriptor"),
            size: wgpu::Extent3d {
                width: window_size.width,
                height: window_size.height,
                ..Default::default()
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: STORAGE_DIMENSION,
            format: STORAGE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        };

        let a = gfx.device.create_texture(&texture_descriptor);
        let b = gfx.device.create_texture(&texture_descriptor);

        let a_view = a.create_view(&Default::default());
        let b_view = b.create_view(&Default::default());

        // Render from A to B
        let front = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scope.front"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&b_view),
                },
            ],
        });

        // Render from B to A
        let back = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scope.front"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&a_view),
                },
            ],
        });

        Self {
            a,
            b,
            a_view,
            b_view,
            front,
            back,
        }
    }
}

pub struct Scope {
    gfx: GraphicsContext,
    config: Config,
    config_buffer: wgpu::Buffer,
    sample_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    size_dependent: SizeDependent,
    pipeline: wgpu::RenderPipeline,
}

impl Scope {
    pub fn new(gfx: GraphicsContext) -> Self {
        let config = Config::default();
        let config_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scope.config_buffer"),
            size: std::mem::size_of::<Config>().try_into().unwrap(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let sample_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scope.sample_buffer"),
            size: (MAX_SAMPLES * std::mem::size_of::<Sample>())
                .try_into()
                .unwrap(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let uniform_bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Scope.uniform_bind_group_layout"),
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
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let uniform_bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scope.config_bind_group"),
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sample_buffer.as_entire_binding(),
                },
            ],
        });

        let texture_bind_group_layout =
            gfx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Scope.texture_bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::ReadOnly,
                                format: STORAGE_FORMAT,
                                view_dimension: STORAGE_VIEW_DIMENSION,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: STORAGE_FORMAT,
                                view_dimension: STORAGE_VIEW_DIMENSION,
                            },
                            count: None,
                        },
                    ],
                });

        let size_dependent = SizeDependent::new(&gfx, &texture_bind_group_layout);

        let shader_module = gfx
            .device
            .create_shader_module(wgpu::include_wgsl!("scope.wgsl"));

        let pipeline_layout = gfx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Scope.pipeline_layout"),
                bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = gfx
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("Scope.pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader_module,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: gfx.surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::default(),
                    })],
                }),
                multiview: None,
            });

        Self {
            gfx: gfx.clone(),
            config,
            config_buffer,
            sample_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            size_dependent,
            pipeline,
        }
    }

    pub fn draw(
        &mut self,
        frame_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
    ) {
        queue.write_buffer(&self.config_buffer, 0, bytemuck::bytes_of(&self.config));

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scope.render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations::default(),
                })],
                ..Default::default()
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.size_dependent.front, &[]);
            render_pass.draw(0..3, 0..1);
        }

        std::mem::swap(
            &mut self.size_dependent.front,
            &mut self.size_dependent.back,
        );
    }

    pub fn window_resized(&mut self) {
        self.size_dependent = SizeDependent::new(&self.gfx, &self.texture_bind_group_layout);

        let size = self.gfx.window.inner_size();
        self.config.window_size = [size.width as f32, size.height as f32];
    }
}