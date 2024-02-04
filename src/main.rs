mod scope;

use anyhow::{ensure, Context};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ChannelCount, SampleRate, SupportedBufferSize};
use pollster::block_on;
use scope::Scope;
use std::iter::repeat;
use std::path::PathBuf;
use std::sync::Arc;
use thingbuf::ThingBuf;
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoopBuilder};
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
    sample_buf: Arc<ThingBuf<[f32; 2]>>,
    scope: Scope,
}

impl App {
    async fn new(window: Window, sample_buf: Arc<ThingBuf<[f32; 2]>>) -> anyhow::Result<Self> {
        let gfx = Arc::new(GraphicsContextInner::new(Arc::new(window)).await?);
        let scope = Scope::new(Arc::clone(&gfx));

        Ok(Self {
            gfx,
            sample_buf,
            scope,
        })
    }

    fn update(&mut self) {
        while let Some(frame) = self.sample_buf.pop() {
            self.scope.push(frame);
        }
    }

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

enum AppEvent {
    Overrun,
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

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build()?;
    let sample_buf: Arc<ThingBuf<[f32; 2]>> = Arc::new(ThingBuf::new(4096));

    let audio_buf = Arc::clone(&sample_buf);
    let audio_events = event_loop.create_proxy();
    let output_stream = output_device.build_output_stream::<f32, _, _>(
        &output_config.config(),
        move |output_data, _output_info| {
            let in_frames = source
                .frames::<[f32; 2]>()
                .map(|result| result.expect("read error"))
                .chain(repeat([0.0; 2]));
            let out_frames = output_data.chunks_mut(2);

            let mut overrun = false;
            for (i, (in_frame, out_frame)) in in_frames.zip(out_frames).enumerate() {
                out_frame.copy_from_slice(&in_frame);

                if i % 4 == 0 {
                    if audio_buf.push(in_frame).is_err() {
                        overrun = true;
                    }
                }
            }
            if overrun {
                let _ = audio_events.send_event(AppEvent::Overrun);
            }
        },
        |stream_error| {
            eprintln!("stream error: {:?}", stream_error);
        },
        None,
    )?;
    output_stream.play()?;

    // Setup graphics loop
    // TODO account for sample rate in graphics
    let window = WindowBuilder::new()
        .with_inner_size(LogicalSize::new(360, 360))
        .with_title("Glowie")
        .with_decorations(false)
        .build(&event_loop)?;

    let mut app = block_on(App::new(window, sample_buf))?;
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
        Event::UserEvent(app_event) => match app_event {
            AppEvent::Overrun => {
                eprintln!("OVERRUN from audio thread");
            }
        },
        _ => {}
    })?;

    Ok(())
}
