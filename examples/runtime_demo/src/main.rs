use std::time::Instant;

use ui_core::{Color, Point, Radius, Size, Srgb8, Stroke};
use ui_renderer::{RenderFrame, RendererOptions, UiRenderer, Viewport};
use ui_runtime::{
    Align, Constraints, Dimension, Insets, LayoutMode, LayoutStyle, PaintState, UiTree,
};
use ui_text::{FontWeight, TextStyle, TextSystem};
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

struct RuntimeDemo {
    window: Option<UiWindow>,
    gpu: Option<GpuState>,
    tree: UiTree,
    text: TextSystem,
    root: ui_runtime::NodeId,
    last_size: Size,
    last_stats: Instant,
    stats_reported: bool,
    surface_issue_reported: bool,
    surface_occluded: bool,
}

impl RuntimeDemo {
    fn new() -> Self {
        let mut text = TextSystem::new();
        let mut tree = UiTree::new();
        let logical = Size::new(1120.0, 720.0);
        let root = tree
            .create_node(
                None,
                LayoutStyle {
                    mode: LayoutMode::Column,
                    width: Dimension::Points(logical.width),
                    height: Dimension::Points(logical.height),
                    align_x: Align::Stretch,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                paint(rgb(17, 19, 24), 0.0),
            )
            .unwrap();
        let toolbar = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    mode: LayoutMode::Row,
                    height: Dimension::Points(58.0),
                    padding: Insets {
                        left: 24.0,
                        right: 24.0,
                        top: 0.0,
                        bottom: 0.0,
                    },
                    align_y: Align::Center,
                    ..LayoutStyle::default()
                },
                paint(rgb(25, 28, 35), 0.0),
            )
            .unwrap();
        add_text(
            &mut tree,
            &mut text,
            toolbar,
            "Retained UI Runtime",
            20.0,
            FontWeight::Bold,
            rgb(235, 239, 247),
        );

        let body = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    mode: LayoutMode::Row,
                    flex_grow: 1.0,
                    gap: 1.0,
                    align_y: Align::Stretch,
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let sidebar = tree
            .create_node(
                Some(body),
                LayoutStyle {
                    mode: LayoutMode::Column,
                    width: Dimension::Points(248.0),
                    padding: Insets::all(20.0),
                    gap: 18.0,
                    align_x: Align::Start,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                paint(rgb(22, 25, 31), 0.0),
            )
            .unwrap();
        add_text(
            &mut tree,
            &mut text,
            sidebar,
            "WORKSPACE",
            12.0,
            FontWeight::Bold,
            rgb(122, 137, 159),
        );
        add_text(
            &mut tree,
            &mut text,
            sidebar,
            "Overview",
            15.0,
            FontWeight::Medium,
            rgb(228, 233, 242),
        );
        add_text(
            &mut tree,
            &mut text,
            sidebar,
            "Rendering Core",
            15.0,
            FontWeight::Regular,
            rgb(165, 176, 194),
        );
        add_text(
            &mut tree,
            &mut text,
            sidebar,
            "Layout Runtime",
            15.0,
            FontWeight::Regular,
            rgb(165, 176, 194),
        );
        add_text(
            &mut tree,
            &mut text,
            sidebar,
            "Diagnostics",
            15.0,
            FontWeight::Regular,
            rgb(165, 176, 194),
        );

        let main = tree
            .create_node(
                Some(body),
                LayoutStyle {
                    mode: LayoutMode::Column,
                    flex_grow: 1.0,
                    padding: Insets::all(28.0),
                    gap: 20.0,
                    align_x: Align::Stretch,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        add_text(
            &mut tree,
            &mut text,
            main,
            "Layout and paint stay separate",
            24.0,
            FontWeight::Bold,
            rgb(240, 243, 249),
        );
        add_text(
            &mut tree,
            &mut text,
            main,
            "Resize this window: the logical tree is measured and arranged again only when its constraints change.",
            15.0,
            FontWeight::Regular,
            rgb(160, 171, 190),
        );

        let cards = tree
            .create_node(
                Some(main),
                LayoutStyle {
                    mode: LayoutMode::Row,
                    gap: 14.0,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        for (title, subtitle, color) in [
            ("Identity", "Stable NodeId", rgb(42, 57, 77)),
            ("Layout", "Cached constraints", rgb(39, 63, 59)),
            ("Paint", "Fresh DisplayList", rgb(66, 53, 42)),
        ] {
            let card = tree
                .create_node(
                    Some(cards),
                    LayoutStyle {
                        mode: LayoutMode::Column,
                        width: Dimension::Points(220.0),
                        height: Dimension::Points(132.0),
                        padding: Insets::all(18.0),
                        gap: 12.0,
                        align_x: Align::Start,
                        align_y: Align::Start,
                        ..LayoutStyle::default()
                    },
                    paint(color, 10.0),
                )
                .unwrap();
            add_text(
                &mut tree,
                &mut text,
                card,
                title,
                17.0,
                FontWeight::Bold,
                rgb(237, 241, 247),
            );
            add_text(
                &mut tree,
                &mut text,
                card,
                subtitle,
                13.0,
                FontWeight::Regular,
                rgb(183, 194, 207),
            );
        }

        let footer = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    height: Dimension::Points(30.0),
                    padding: Insets {
                        left: 24.0,
                        top: 0.0,
                        right: 24.0,
                        bottom: 0.0,
                    },
                    align_y: Align::Center,
                    ..LayoutStyle::default()
                },
                paint(rgb(25, 28, 35), 0.0),
            )
            .unwrap();
        add_text(
            &mut tree,
            &mut text,
            footer,
            "Logical points · HiDPI-aware renderer · ephemeral display lists",
            12.0,
            FontWeight::Regular,
            rgb(127, 141, 162),
        );

        Self {
            window: None,
            gpu: None,
            tree,
            text,
            root,
            last_size: Size::ZERO,
            last_stats: Instant::now(),
            stats_reported: false,
            surface_issue_reported: false,
            surface_occluded: false,
        }
    }

    fn initialize_gpu(&mut self) {
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
            label: Some("runtime-demo-device"),
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
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
            issue => {
                if !self.surface_issue_reported {
                    eprintln!("surface frame unavailable: {issue:?}");
                    self.surface_issue_reported = true;
                }
                self.surface_occluded = matches!(issue, wgpu::CurrentSurfaceTexture::Occluded);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("runtime-demo-frame"),
            });
        let metrics = window.metrics();
        if metrics.logical_size != self.last_size {
            self.last_size = metrics.logical_size;
            let root_layout = LayoutStyle {
                mode: LayoutMode::Column,
                width: Dimension::Points(metrics.logical_size.width),
                height: Dimension::Points(metrics.logical_size.height),
                align_x: Align::Stretch,
                align_y: Align::Start,
                ..LayoutStyle::default()
            };
            self.tree.set_layout(self.root, root_layout).unwrap();
            self.tree
                .layout(self.root, Constraints::loose(metrics.logical_size))
                .unwrap();
        }
        let display_list = self.tree.paint(self.root).unwrap();
        gpu.renderer.render(
            RenderFrame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut encoder,
                target: &view,
                viewport: Viewport::new(metrics.physical_size, metrics.scale_factor),
            },
            &display_list,
            Some(&mut self.text),
        );
        let submit_started = Instant::now();
        gpu.queue.submit(Some(encoder.finish()));
        gpu.renderer
            .record_queue_submit_time(submit_started.elapsed());
        gpu.queue.present(frame);
        if !self.stats_reported || self.last_stats.elapsed().as_secs_f32() >= 1.0 {
            let stats = gpu.renderer.stats();
            let layout = self.tree.layout_stats();
            eprintln!(
                "layout measured={} laid_out={} measure_cache_hits={} layout_cache_hits={} | prepare={:?} submit_cpu={:?} draws={} batches={} vertices={} uploads={}B glyph_atlas={}/{} px",
                layout.measured_nodes,
                layout.laid_out_nodes,
                layout.measurement_cache_hits,
                layout.layout_cache_hits,
                stats.frame_prepare_time,
                stats.queue_submit_time,
                stats.draw_calls,
                stats.batches,
                stats.vertices,
                stats.texture_upload_bytes,
                stats.atlas_used_pixels,
                stats.atlas_capacity_pixels,
            );
            self.last_stats = Instant::now();
            self.stats_reported = true;
        }
    }
}

fn add_text(
    tree: &mut UiTree,
    text: &mut TextSystem,
    parent: ui_runtime::NodeId,
    value: &str,
    size: f32,
    weight: FontWeight,
    color: Color,
) {
    let run = text.shape(
        value,
        TextStyle {
            size_px: size,
            line_height_px: size + 5.0,
            weight,
            color,
            ..TextStyle::default()
        },
        None,
    );
    let metrics = text.run_metrics(run).unwrap();
    let node = tree
        .create_node(Some(parent), LayoutStyle::default(), PaintState::default())
        .unwrap();
    tree.set_text(node, run, metrics).unwrap();
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgba8(Srgb8 { r, g, b, a: 255 })
}

fn paint(background: Color, radius: f32) -> PaintState {
    PaintState {
        background: Some(background),
        radius: Radius::all(radius),
        border: Some(Stroke::new(1.0, rgb(53, 62, 77))),
    }
}

impl ApplicationHandler for RuntimeDemo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.window = Some(
                UiWindow::open(
                    event_loop,
                    &WindowConfig {
                        title: "UI Runtime Demo".to_owned(),
                        logical_size: Size::new(1120.0, 720.0),
                    },
                )
                .expect("window creation"),
            );
            self.initialize_gpu();
            self.window.as_ref().unwrap().request_redraw();
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
            WindowEvent::Occluded(occluded) => {
                self.surface_occluded = occluded;
                if !occluded && let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.surface_occluded
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--headless-smoke") {
        return run_headless_smoke();
    }
    EventLoop::new()?.run_app(&mut RuntimeDemo::new())?;
    Ok(())
}

fn run_headless_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = RuntimeDemo::new();
    let logical = Size::new(1120.0, 720.0);
    demo.tree.set_layout(
        demo.root,
        LayoutStyle {
            mode: LayoutMode::Column,
            width: Dimension::Points(logical.width),
            height: Dimension::Points(logical.height),
            align_x: Align::Stretch,
            align_y: Align::Start,
            ..LayoutStyle::default()
        },
    )?;
    demo.tree.layout(demo.root, Constraints::loose(logical))?;
    let display_list = demo.tree.paint(demo.root)?;

    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("runtime-demo-headless-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let size = [1120, 720];
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("runtime-demo-headless-target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = UiRenderer::new(&device, &queue, format, RendererOptions::default())?;
    renderer.resize(&device, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("runtime-demo-headless-frame"),
    });
    renderer.render(
        RenderFrame {
            device: &device,
            queue: &queue,
            encoder: &mut encoder,
            target: &target,
            viewport: Viewport::new(size, ui_core::ScaleFactor::new(1.0)),
        },
        &display_list,
        Some(&mut demo.text),
    );
    let submit_started = Instant::now();
    queue.submit(Some(encoder.finish()));
    renderer.record_queue_submit_time(submit_started.elapsed());
    let stats = renderer.stats();
    eprintln!(
        "headless runtime smoke: nodes={} commands={} prepare={:?} submit_cpu={:?} draws={} batches={} vertices={} glyphs={} uploads={}B",
        demo.tree.node_count(),
        display_list.commands().len(),
        stats.frame_prepare_time,
        stats.queue_submit_time,
        stats.draw_calls,
        stats.batches,
        stats.vertices,
        stats.glyphs_rasterized,
        stats.texture_upload_bytes,
    );
    demo.text.begin_frame();
    let incremental = demo.text.shape(
        "Incremental atlas update Ω",
        TextStyle {
            size_px: 22.0,
            line_height_px: 28.0,
            color: rgb(230, 235, 244),
            ..TextStyle::default()
        },
        None,
    );
    demo.text.prepare(incremental, Point::ZERO, 1.0)?;
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("runtime-demo-incremental-atlas-frame"),
    });
    renderer.render(
        RenderFrame {
            device: &device,
            queue: &queue,
            encoder: &mut encoder,
            target: &target,
            viewport: Viewport::new(size, ui_core::ScaleFactor::new(1.0)),
        },
        &display_list,
        Some(&mut demo.text),
    );
    let submit_started = Instant::now();
    queue.submit(Some(encoder.finish()));
    renderer.record_queue_submit_time(submit_started.elapsed());
    let incremental_stats = renderer.stats();
    eprintln!(
        "incremental atlas smoke: glyph_uploads={}B full_atlas={}B",
        incremental_stats.glyph_upload_bytes, incremental_stats.atlas_capacity_pixels,
    );
    assert!(incremental_stats.glyph_upload_bytes > 0);
    assert!(incremental_stats.glyph_upload_bytes < incremental_stats.atlas_capacity_pixels);
    Ok(())
}
