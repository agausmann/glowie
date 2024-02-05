use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use wgpu::RenderPipelineDescriptor;

use crate::GraphicsContext;

const STORAGE_DIMENSION: wgpu::TextureDimension = wgpu::TextureDimension::D2;
const STORAGE_VIEW_DIMENSION: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2;
const STORAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

const MAX_LINES: usize = 65536;

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Config {
    chunks: [Chunk4; 64],
    window_size: [f32; 2],
    line_radius: f32,
    decay: f32,
    sigma: f32,
    intensity: f32,
    total_time: f32,
    _pad: [u8; 4],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window_size: [360.0, 360.0],
            line_radius: 5.0,
            decay: 1.0 - 1e-3,
            sigma: 2e-3,
            intensity: 1e-5,
            total_time: 0.0,
            chunks: std::array::from_fn(|_| Chunk4::default()),
            _pad: [0; 4],
        }
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Chunk4 {
    // 2xu16
    offset_size: [u32; 4],
}

impl Default for Chunk4 {
    fn default() -> Self {
        Self {
            offset_size: [0; 4],
        }
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Line {
    // 2x16snorm
    start: u32,
    // 2x16snorm
    v: u32,
    time: f32,
}

fn pack16snorm(e: f32) -> u16 {
    (0.5 + 32767.0 * e.clamp(-1.0, 1.0)).floor() as i16 as u16
}

fn pack2x16snorm(e: [f32; 2]) -> u32 {
    (pack16snorm(e[0]) as u32) | ((pack16snorm(e[1]) as u32) << 16)
}

fn pack2xu16(e: [u16; 2]) -> u32 {
    (e[0] as u32) | ((e[1] as u32) << 16)
}

impl Default for Line {
    fn default() -> Self {
        Self {
            start: 0,
            v: 0,
            time: 0.0,
        }
    }
}

#[allow(dead_code)]
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
    chunk_lines: Vec<Vec<Line>>,
    lines: Vec<Line>,
    samples: Vec<[f32; 2]>,
    line_buffer: wgpu::Buffer,
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

        let lines = vec![];
        let samples = vec![[0.0; 2]];
        let chunk_lines = vec![Vec::new(); 256];

        let line_buffer = gfx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scope.line_buffer"),
            size: (MAX_LINES * std::mem::size_of::<Line>())
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
                    resource: line_buffer.as_entire_binding(),
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
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
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
            lines,
            chunk_lines,
            samples,
            line_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            size_dependent,
            pipeline,
        }
    }

    pub fn extend(&mut self, frames: impl IntoIterator<Item = [f32; 2]>) {
        self.samples.extend(frames);
    }

    fn generate_chunks(&mut self) {
        // generate lines from samples, and assign lines to chunks.
        let mut batch_size = 0;
        let mut line_buffer_size = 0;
        for seg in self.samples.windows(2) {
            // TODO: more efficient chunk iteration

            let start = Vec2::from(seg[0]);
            let end = Vec2::from(seg[1]);

            let line_data = Line {
                start: pack2x16snorm(start.into()),
                v: pack2x16snorm((end - start).into()),
                time: batch_size as f32,
            };

            for chunk_y in 0..16 {
                for chunk_x in 0..16 {
                    let i_chunk = 16 * chunk_y + chunk_x;

                    let chunk_center =
                        Vec2::new((chunk_x as f32 - 7.5) / 8.0, (chunk_y as f32 - 7.5) / 8.0);

                    let u = chunk_center - start;
                    let v = end - start;

                    let mut disp = u;
                    if v.dot(v) != 0.0 {
                        let proj_position = u.dot(v) / v.dot(v);
                        let proj = v * proj_position.clamp(0.0, 1.0);
                        disp -= proj;
                    }

                    // TODO vary threshold based on config.sigma
                    if 8.0 * disp.length() < 1.0 {
                        self.chunk_lines[i_chunk].push(line_data);
                        line_buffer_size += 1;
                    }
                }
            }
            batch_size += 1;

            if line_buffer_size > MAX_LINES - 256 {
                // don't risk trying to add another segment.
                break;
            }
        }

        // write chunk offset/size data
        let mut offset = 0;
        for i_chunk in 0..256 {
            let size: u16 = self.chunk_lines[i_chunk].len().try_into().unwrap();
            self.config.chunks[i_chunk >> 2].offset_size[i_chunk & 3] = pack2xu16([offset, size]);
            offset += size;
        }

        // flatten line buffers
        self.lines.clear();
        self.lines
            .extend(self.chunk_lines.iter_mut().flat_map(|v| v.drain(..)));

        // remove processed samples from buffer
        self.samples.copy_within(batch_size - 1.., 0);
        self.samples.truncate(self.samples.len() - batch_size + 1);

        // finalize
        self.config.total_time = batch_size as f32;
    }

    pub fn draw(
        &mut self,
        frame_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
    ) {
        self.generate_chunks();
        queue.write_buffer(&self.config_buffer, 0, bytemuck::bytes_of(&self.config));
        queue.write_buffer(&self.line_buffer, 0, bytemuck::cast_slice(&self.lines));

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
            render_pass.draw(0..4, 0..1);
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
