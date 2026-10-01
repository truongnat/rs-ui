use std::time::Instant;

use ui_core::{Color, DisplayListBuilder, Point, Rect, ScaleFactor, Size};
use ui_renderer::{RenderFrame, RendererOptions, UiRenderer, Viewport};
use ui_text::{FontWeight, TextStyle, TextSystem};

const SHAPE_COUNT: usize = 10_000;
const TEXT_RUN_COUNT: usize = 5_000;
const REPEATED_FRAMES: usize = 5;
const TARGET_SIZE: [u32; 2] = [1280, 1024];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut list = DisplayListBuilder::new();
    for index in 0..SHAPE_COUNT {
        let x = (index % 100) as f32 * 8.0;
        let y = (index / 100) as f32 * 8.0;
        list.fill_rect(
            Rect::from_min_size(Point::new(x, y), Size::new(6.0, 6.0)),
            Color::WHITE,
        );
    }
    let shape_build_time = started.elapsed();

    let mut text = TextSystem::new();
    let style = TextStyle {
        weight: FontWeight::Medium,
        ..TextStyle::default()
    };
    let shape_started = Instant::now();
    for index in 0..TEXT_RUN_COUNT {
        let run = text.shape(&format!("Run {index}: Tiếng Việt"), style, None);
        list.text(run, Point::new(0.25, (index % 900) as f32 + 0.5));
    }
    let shaping_time = shape_started.elapsed();
    let display_list = list.build();

    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("stress-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stress-target"),
        size: wgpu::Extent3d {
            width: TARGET_SIZE[0],
            height: TARGET_SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = UiRenderer::new(&device, &queue, format, RendererOptions::default())?;
    renderer.resize(&device, TARGET_SIZE);
    let viewport = Viewport::new(TARGET_SIZE, ScaleFactor::new(1.0));

    let first_frame_started = Instant::now();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    renderer.render(
        RenderFrame {
            device: &device,
            queue: &queue,
            encoder: &mut encoder,
            target: &target_view,
            viewport,
        },
        &display_list,
        Some(&mut text),
    );
    let submit_started = Instant::now();
    queue.submit(Some(encoder.finish()));
    renderer.record_queue_submit_time(submit_started.elapsed());
    let first_frame_elapsed = first_frame_started.elapsed();
    let first_stats = renderer.stats();

    let repeated_started = Instant::now();
    for _ in 0..REPEATED_FRAMES {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.render(
            RenderFrame {
                device: &device,
                queue: &queue,
                encoder: &mut encoder,
                target: &target_view,
                viewport,
            },
            &display_list,
            Some(&mut text),
        );
        let submit_started = Instant::now();
        queue.submit(Some(encoder.finish()));
        renderer.record_queue_submit_time(submit_started.elapsed());
    }
    let repeated_elapsed = repeated_started.elapsed();
    let repeated_stats = renderer.stats();
    let cache = text.stats();
    let cache_hit_rate = cache.hits as f64 / (cache.hits + cache.misses).max(1) as f64;

    println!(
        "shapes={SHAPE_COUNT} text_runs={TEXT_RUN_COUNT} commands={} shape_build={shape_build_time:?} shape_and_text_build={shaping_time:?}",
        display_list.commands().len()
    );
    println!(
        "first_frame_total={first_frame_elapsed:?} prepare={:?} submit_cpu={:?} uploads={}B glyph_uploads={}B draws={} batches={} vertices={} rasterized={}",
        first_stats.frame_prepare_time,
        first_stats.queue_submit_time,
        first_stats.texture_upload_bytes,
        first_stats.glyph_upload_bytes,
        first_stats.draw_calls,
        first_stats.batches,
        first_stats.vertices,
        first_stats.glyphs_rasterized
    );
    println!(
        "repeat_frames={REPEATED_FRAMES} total={repeated_elapsed:?} last_prepare={:?} last_submit_cpu={:?} uploads={}B draws={} batches={} vertices={} rasterized_delta={}",
        repeated_stats.frame_prepare_time,
        repeated_stats.queue_submit_time,
        repeated_stats.texture_upload_bytes,
        repeated_stats.draw_calls,
        repeated_stats.batches,
        repeated_stats.vertices,
        repeated_stats.glyphs_rasterized
    );
    println!(
        "glyph_cache_hits={} misses={} hit_rate={:.3}% evictions={} atlas_used={}/{} px fragmentation={}‰",
        cache.hits,
        cache.misses,
        cache_hit_rate * 100.0,
        cache.evictions,
        cache.atlas_used_pixels,
        cache.atlas_capacity_pixels,
        cache.fragmentation_per_mille
    );
    assert_eq!(
        repeated_stats.glyphs_rasterized, 0,
        "unchanged text must not rerasterize glyphs on repeated frames"
    );
    assert_eq!(
        first_stats.draw_calls, 2,
        "adjacent shapes and text should batch"
    );
    assert_eq!(first_stats.batches, 2);
    assert_eq!(first_stats.vertices, repeated_stats.vertices);
    assert_eq!(repeated_stats.glyph_upload_bytes, 0);
    Ok(())
}
