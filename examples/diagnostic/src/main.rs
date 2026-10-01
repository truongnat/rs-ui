use ui_core::{
    Color, DisplayList, DisplayListBuilder, ImageId, Point, Radius, Rect, ScaleFactor, Size, Srgb8,
    Stroke, Transform,
};
use ui_renderer::{ImageData, RenderFrame, RendererOptions, UiRenderer, Viewport};
use ui_text::{FontFamily, FontWeight, TextStyle, TextSystem};
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
struct Diagnostic {
    window: Option<UiWindow>,
    gpu: Option<GpuState>,
    list: DisplayList,
    text: TextSystem,
    stats_reported: bool,
}

impl Diagnostic {
    fn new() -> Self {
        let mut text = TextSystem::new();
        let mut list = DisplayListBuilder::new();
        let white = Color::from_srgba8(Srgb8 {
            r: 238,
            g: 241,
            b: 247,
            a: 255,
        });
        let divider = Color::from_srgba8(Srgb8 {
            r: 115,
            g: 128,
            b: 148,
            a: 255,
        });
        list.fill_rect(
            Rect::from_min_size(Point::ZERO, Size::new(1120.0, 900.0)),
            Color::from_srgba8(Srgb8 {
                r: 17,
                g: 19,
                b: 24,
                a: 255,
            }),
        );
        for (index, size) in (10..=18).enumerate() {
            let style = TextStyle {
                size_px: size as f32,
                line_height_px: size as f32 + 5.0,
                color: white,
                ..TextStyle::default()
            };
            let run = text.shape(
                &format!("{size}px  Tiếng Việt: Trường, Nguyễn, phở — crisp text"),
                style,
                None,
            );
            list.text(run, Point::new(32.0, 34.0 + index as f32 * 36.0));
        }
        for (index, (family, weight, label)) in [
            (FontFamily::Sans, FontWeight::Regular, "regular"),
            (FontFamily::Sans, FontWeight::Medium, "medium"),
            (FontFamily::Sans, FontWeight::Bold, "bold"),
            (FontFamily::Monospace, FontWeight::Regular, "monospace"),
        ]
        .into_iter()
        .enumerate()
        {
            let run = text.shape(
                label,
                TextStyle {
                    family,
                    weight,
                    size_px: 18.0,
                    line_height_px: 24.0,
                    color: white,
                },
                None,
            );
            list.text(run, Point::new(500.0 + index as f32 * 130.0, 40.0));
        }
        for (index, alpha) in [0.4, 0.6, 0.8, 1.0].into_iter().enumerate() {
            let run = text.shape(
                &format!("text alpha {}%", (alpha * 100.0) as u32),
                TextStyle {
                    color: Color::linear(0.92, 0.94, 1.0, alpha),
                    ..TextStyle::default()
                },
                None,
            );
            list.text(run, Point::new(32.0 + index as f32 * 230.0, 390.0));
        }
        for (index, width) in [0.5, 1.0, 1.5, 2.0].into_iter().enumerate() {
            let y = 450.0 + index as f32 * 38.0;
            list.line(
                Point::new(32.0, y),
                Point::new(440.0, y),
                Stroke::new(width, divider),
            );
        }
        list.fill_rounded_rect(
            Rect::from_min_size(Point::new(32.0, 620.0), Size::new(300.0, 150.0)),
            Radius::all(18.0),
            Color::from_srgba8(Srgb8 {
                r: 44,
                g: 55,
                b: 73,
                a: 255,
            }),
        );
        list.image(
            Rect::from_min_size(Point::new(370.0, 480.0), Size::new(128.0, 96.0)),
            ImageId(1),
            Color::WHITE,
        );
        list.push_clip(Rect::from_min_size(
            Point::new(365.0, 600.0),
            Size::new(680.0, 230.0),
        ));
        list.push_clip(Rect::from_min_size(
            Point::new(390.0, 625.0),
            Size::new(630.0, 150.0),
        ));
        list.push_transform(Transform::translation(15.25, 0.5));
        let nested = text.shape(
            "Fractional origin 15.25 / 0.5 — Tiếng Việt",
            TextStyle {
                size_px: 22.0,
                line_height_px: 28.0,
                color: white,
                ..TextStyle::default()
            },
            None,
        );
        list.text(nested, Point::new(385.0, 650.0));
        list.pop_transform().pop_clip().pop_clip();
        Self {
            window: None,
            gpu: None,
            list: list.build(),
            text,
            stats_reported: false,
        }
    }

    fn init_gpu(&mut self) {
        let window = self.window.as_ref().unwrap();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.window().clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("diagnostic-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .unwrap();
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
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut renderer =
            UiRenderer::new(&device, &queue, format, RendererOptions::default()).unwrap();
        let image = [
            255u8, 40, 30, 255, 30, 170, 255, 255, 40, 220, 80, 255, 255, 220, 40, 255,
        ];
        renderer
            .upload_image(
                &device,
                &queue,
                ImageId(1),
                ImageData {
                    size: [2, 2],
                    rgba8: &image,
                },
            )
            .unwrap();
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
        let (Some(window), Some(gpu)) = (self.window.as_ref(), self.gpu.as_mut()) else {
            return;
        };
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let metrics = window.metrics();
        let simulated_scale = std::env::var("UI_DPI_SIM")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .map(ScaleFactor::new)
            .unwrap_or(metrics.scale_factor);
        gpu.renderer.render(
            RenderFrame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut encoder,
                target: &view,
                viewport: Viewport::new(metrics.physical_size, simulated_scale),
            },
            &self.list,
            Some(&mut self.text),
        );
        let submit_started = std::time::Instant::now();
        gpu.queue.submit(Some(encoder.finish()));
        gpu.renderer
            .record_queue_submit_time(submit_started.elapsed());
        gpu.queue.present(frame);
        if !self.stats_reported {
            let stats = gpu.renderer.stats();
            println!(
                "prepare={:?} submit_cpu={:?} uploads={}B glyph_uploads={}B draws={} batches={} vertices={} glyph cache hit/miss={}/{} ({:.2}%) atlas={}/{} px",
                stats.frame_prepare_time,
                stats.queue_submit_time,
                stats.texture_upload_bytes,
                stats.glyph_upload_bytes,
                stats.draw_calls,
                stats.batches,
                stats.vertices,
                stats.glyph_cache_hits,
                stats.glyph_cache_misses,
                stats.glyph_cache_hit_rate * 100.0,
                stats.atlas_used_pixels,
                stats.atlas_capacity_pixels
            );
            self.stats_reported = true;
        }
    }
}

impl ApplicationHandler for Diagnostic {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.window = Some(
                UiWindow::open(
                    event_loop,
                    &WindowConfig {
                        title: "UI Text + Renderer Diagnostics".into(),
                        logical_size: Size::new(1120.0, 900.0),
                    },
                )
                .unwrap(),
            );
            self.init_gpu();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_some_and(|window| window.id() != id) {
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
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    EventLoop::new()?.run_app(&mut Diagnostic::new())?;
    Ok(())
}
