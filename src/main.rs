mod scope;

use anyhow::{ensure, Context};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ChannelCount, SampleRate, SupportedBufferSize};
use pollster::block_on;
use scope::Scope;
use std::path::PathBuf;
use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Command {
    Play(PlayArgs),
}

#[derive(Debug, Clone, clap::Parser)]
struct PlayArgs {
    path: PathBuf,
}

pub type GraphicsContext = Arc<GraphicsContextInner>;

pub struct GraphicsContextInner {
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_caps: wgpu::SurfaceCapabilities,
    pub surface_format: wgpu::TextureFormat,
    pub window: Arc<Window>,
}

impl GraphicsContextInner {
    async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(Arc::clone(&window))
            .context("failed to create surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("failed to create adapter")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await?;
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        Ok(Self {
            surface,
            adapter,
            device,
            queue,
            surface_caps,
            surface_format,
            window,
        })
    }
}

struct App {
    gfx: GraphicsContext,
    scope: Scope,
}

impl App {
    async fn new(window: Window) -> anyhow::Result<Self> {
        let gfx = Arc::new(GraphicsContextInner::new(Arc::new(window)).await?);
        let scope = Scope::new(Arc::clone(&gfx));

        Ok(Self { gfx, scope })
    }

    fn update(&mut self) {}

    fn redraw(&mut self) -> anyhow::Result<()> {
        let frame = loop {
            match self.gfx.surface.get_current_texture() {
                Ok(frame) => break frame,
                Err(wgpu::SurfaceError::Lost) => {
                    self.reconfigure();
                }
                Err(wgpu::SurfaceError::Timeout) | Err(wgpu::SurfaceError::Outdated) => {
                    return Ok(());
                }
                Err(err) => {
                    return Err(err.into());
                }
            }
        };

        let frame_view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gfx.device.create_command_encoder(&Default::default());

        self.scope.draw(&frame_view, &mut encoder, &self.gfx.queue);

        self.gfx.queue.submit([encoder.finish()]);
        frame.present();

        Ok(())
    }

    fn window_resized(&mut self) {
        self.scope.window_resized();
        self.reconfigure();
    }

    fn reconfigure(&self) {
        let size = self.gfx.window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.gfx.surface_format,
            width: size.width,
            height: size.height,
            present_mode: self.gfx.surface_caps.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: self.gfx.surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        self.gfx.surface.configure(&self.gfx.device, &config);
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    // Open audio file
    let args = Args::parse();
    let mut source = match args.command {
        Command::Play(play_args) => {
            let file = audrey::open(play_args.path)?;

            file
        }
    };
    let descr = source.description();
    ensure!(
        descr.channel_count() == 2,
        "audio channels must be equal to 2 (stereo)"
    );

    let target_channels = ChannelCount::try_from(descr.channel_count()).unwrap();
    let target_rate = SampleRate(descr.sample_rate());

    // Setup audio output
    let host = cpal::default_host();
    let output_device = host
        .default_output_device()
        .context("no default output device")?;
    let output_config = output_device
        .supported_output_configs()?
        .filter(|cfg| cfg.channels() == target_channels)
        .filter_map(|cfg| cfg.try_with_sample_rate(target_rate))
        .max_by_key(|config| {
            // Priorities:
            // - Floating-point input
            // - Maximum precision
            // - Maximum buffer size
            (
                config.sample_format().is_float(),
                config.sample_format().sample_size(),
                match *config.buffer_size() {
                    SupportedBufferSize::Range { max, .. } => max,
                    _ => 0,
                },
            )
        })
        .context("no device configuration matches the given sample rate and channel count")?;

    let output_stream = output_device.build_output_stream::<f32, _, _>(
        &output_config.config(),
        move |output_data, _output_info| {
            output_data.fill_with(|| {
                source
                    .samples::<f32>()
                    .next()
                    .expect("end of audio")
                    .expect("read error")
            });
            // TODO pass to graphics
        },
        |stream_error| {
            eprintln!("stream error: {:?}", stream_error);
        },
        None,
    )?;
    output_stream.play()?;

    // Setup graphics loop
    // TODO account for sample rate in graphics
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_inner_size(LogicalSize::new(720, 720))
        .with_title("Glowie")
        .with_decorations(false)
        .build(&event_loop)?;

    let mut app = block_on(App::new(window))?;
    app.reconfigure();

    event_loop.set_control_flow(ControlFlow::Poll);

    event_loop.run(move |event, elwt| match event {
        Event::AboutToWait => {
            app.update();
            app.redraw().unwrap();
        }
        Event::WindowEvent { event, .. } => match event {
            WindowEvent::CloseRequested => {
                elwt.exit();
            }
            WindowEvent::Resized(..) | WindowEvent::ScaleFactorChanged { .. } => {
                app.window_resized();
            }
            _ => {}
        },
        _ => {}
    })?;

    Ok(())
}
