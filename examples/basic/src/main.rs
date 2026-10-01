use ui_core::{Color, DisplayList, DisplayListBuilder, Point, Radius, Rect, Size, Srgb8, Stroke};
use ui_renderer::{RenderFrame, RendererOptions, UiRenderer, Viewport};
use ui_window::{UiWindow, WindowConfig};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: UiRenderer,
}

struct BasicApp {
    window: Option<UiWindow>,
    gpu: Option<GpuState>,
    display_list: DisplayList,
}

impl BasicApp {
    fn new() -> Self {
        let mut list = DisplayListBuilder::new();
        let background = Color::from_srgba8(Srgb8 {
            r: 14,
            g: 16,
            b: 20,
            a: 255,
        });
        let panel = Color::from_srgba8(Srgb8 {
            r: 28,
            g: 32,
            b: 40,
            a: 255,
        });
        let panel_alt = Color::from_srgba8(Srgb8 {
            r: 35,
            g: 40,
            b: 50,
            a: 255,
        });
        let border = Color::from_srgba8(Srgb8 {
            r: 76,
            g: 86,
            b: 104,
            a: 255,
        });
        list.fill_rect(
            Rect::from_min_size(Point::ZERO, Size::new(1120.0, 720.0)),
            background,
        );
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(56.0, 48.0), Size::new(1008.0, 624.0)),
            Radius::all(18.0),
            panel,
        );
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(82.0, 80.0), Size::new(220.0, 560.0)),
            Radius::all(12.0),
            panel_alt,
        );
        list.stroke_rounded_rect(
            Rect::from_min_size(Point::new(56.0, 48.0), Size::new(1008.0, 624.0)),
            Radius::all(18.0),
            Stroke::new(1.0, border),
        );
        list.stroke_rounded_rect(
            Rect::from_min_size(Point::new(82.0, 80.0), Size::new(220.0, 560.0)),
            Radius::all(12.0),
            Stroke::new(1.0, border),
        );
        list.line(
            Point::new(338.0, 148.0),
            Point::new(1016.0, 148.0),
            Stroke::new(1.0, border),
        );
        list.push_clip(Rect::from_min_size(
            Point::new(338.0, 180.0),
            Size::new(678.0, 430.0),
        ));
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(370.0, 208.0), Size::new(300.0, 142.0)),
            Radius::all(10.0),
            Color::from_srgba8(Srgb8 {
                r: 46,
                g: 53,
                b: 66,
                a: 255,
            }),
        );
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(694.0, 208.0), Size::new(290.0, 142.0)),
            Radius::all(10.0),
            Color::from_srgba8(Srgb8 {
                r: 43,
                g: 60,
                b: 57,
                a: 255,
            }),
        );
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(370.0, 374.0), Size::new(614.0, 156.0)),
            Radius::all(10.0),
            Color::from_srgba8(Srgb8 {
                r: 37,
                g: 43,
                b: 54,
                a: 255,
            }),
        );
        list.pop_clip();
        Self {
            window: None,
            gpu: None,
            display_list: list.build(),
        }
    }

    fn initialize_gpu(&mut self) {
        let window = self.window.as_ref().expect("window is created before GPU");
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.window().clone())
            .expect("surface creation");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no compatible GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("basic-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("device creation");
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let metrics = window.metrics();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: metrics.physical_size[0].max(1),
            height: metrics.physical_size[1].max(1),
            present_mode: capabilities
                .present_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::PresentMode::Fifo)
                .unwrap_or(capabilities.present_modes[0]),
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut renderer = UiRenderer::new(&device, &queue, format, RendererOptions::default())
            .expect("renderer creation");
        renderer.resize(&device, [config.width, config.height]);
        self.gpu = Some(GpuState {
            surface,
            device,
            queue,
            config,
            renderer,
        });
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("basic-frame"),
            });
        let metrics = window.metrics();
        gpu.renderer.render(
            RenderFrame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut encoder,
                target: &view,
                viewport: Viewport::new(metrics.physical_size, metrics.scale_factor),
            },
            &self.display_list,
            None,
        );
        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
    }
}

impl ApplicationHandler for BasicApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.window = Some(
                UiWindow::open(event_loop, &WindowConfig::default()).expect("window creation"),
            );
            self.initialize_gpu();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_some_and(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(window), Some(gpu)) = (self.window.as_mut(), self.gpu.as_mut()) {
                    window.resize(size);
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.renderer
                        .resize(&gpu.device, [gpu.config.width, gpu.config.height]);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = self.window.as_mut() {
                    window.refresh_metrics();
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut BasicApp::new())?;
    Ok(())
}
