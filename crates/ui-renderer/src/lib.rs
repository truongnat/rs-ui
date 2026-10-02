//! `wgpu` adapter for the renderer-independent `ui-core` display list.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use std::time::{Duration, Instant};
mod pipelines;
use ui_core::{
    Color, DisplayCommand, DisplayList, ImageId, Point, Radius, Rect, ScaleFactor, Stroke,
    Transform,
};
use ui_text::{PreparedText, TextSystem};

const ROUND_SEGMENTS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct RendererOptions {
    /// `1` disables MSAA; `4` is the default for clean primitive edges.
    pub msaa_samples: u32,
    /// Glyph coverage sampling, independent of image filtering.
    pub text_filter: wgpu::FilterMode,
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            msaa_samples: 4,
            text_filter: wgpu::FilterMode::Linear,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub physical_size: [u32; 2],
    pub scale_factor: ScaleFactor,
}

pub struct RenderFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub viewport: Viewport,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RendererStats {
    pub frame_prepare_time: Duration,
    pub draw_calls: u64,
    pub batches: u64,
    pub vertices: u64,
    pub glyph_cache_hits: u64,
    pub glyph_cache_misses: u64,
    pub glyph_cache_evictions: u64,
    pub glyph_cache_hit_rate: f32,
    pub glyphs_rasterized: u64,
    pub atlas_used_pixels: u64,
    pub atlas_capacity_pixels: u64,
    pub atlas_free_rect_count: u64,
    pub atlas_fragmentation_per_mille: u64,
    pub texture_upload_bytes: u64,
    pub glyph_upload_bytes: u64,
    /// Queue submission call duration, recorded by the app after `Queue::submit`.
    pub queue_submit_time: Option<Duration>,
    /// `None` when the application allocator does not expose allocation counters.
    pub allocation_count: Option<u64>,
}

impl Viewport {
    pub const fn new(physical_size: [u32; 2], scale_factor: ScaleFactor) -> Self {
        Self {
            physical_size,
            scale_factor,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ImageData<'a> {
    pub size: [u32; 2],
    /// Bytes are straight-alpha sRGB RGBA8. `upload_image` converts them at the
    /// texture sampling boundary by using an sRGB texture format.
    pub rgba8: &'a [u8],
}

#[derive(Debug)]
pub enum RendererError {
    InvalidMsaaSamples(u32),
    InvalidImageData { expected: usize, actual: usize },
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMsaaSamples(value) => {
                write!(formatter, "unsupported MSAA sample count: {value}")
            }
            Self::InvalidImageData { expected, actual } => write!(
                formatter,
                "image byte count mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for RendererError {}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

fn create_mask_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    label: &str,
    size: u32,
) -> ImageTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    ImageTexture {
        _texture: texture,
        bind_group,
    }
}

fn rect_vertices(rect: Rect, color: Color, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(6);
    push_rect(&mut vertices, rect, color, viewport);
    vertices
}

fn rounded_vertices(rect: Rect, radius: Radius, color: Color, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(ROUND_SEGMENTS * 4 * 3);
    push_rounded_rect(&mut vertices, rect, radius, color, viewport);
    vertices
}

fn border_vertices(rect: Rect, radius: Radius, stroke: Stroke, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(ROUND_SEGMENTS * 4 * 6);
    push_border(&mut vertices, rect, radius, stroke, viewport);
    vertices
}

fn line_vertices(from: Point, to: Point, stroke: Stroke, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(6);
    push_line(&mut vertices, from, to, stroke, viewport);
    vertices
}

fn image_vertices(rect: Rect, tint: Color, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(6);
    push_image(&mut vertices, rect, tint, viewport);
    vertices
}

fn text_vertices(text: &PreparedText, transform: Transform, viewport: Viewport) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(text.glyphs.len() * 6);
    for glyph in &text.glyphs {
        let rect = Rect::from_min_size(glyph.origin, glyph.size);
        let points = [
            rect.min,
            Point::new(rect.max.x, rect.min.y),
            rect.max,
            Point::new(rect.min.x, rect.max.y),
        ];
        let [u0, v0] = glyph.uv_min;
        let [u1, v1] = glyph.uv_max;
        let color = glyph.color.premultiplied_linear();
        vertices.extend_from_slice(&[
            vertex_with_color(points[0], color, viewport, [u0, v0]),
            vertex_with_color(points[1], color, viewport, [u1, v0]),
            vertex_with_color(points[2], color, viewport, [u1, v1]),
            vertex_with_color(points[0], color, viewport, [u0, v0]),
            vertex_with_color(points[2], color, viewport, [u1, v1]),
            vertex_with_color(points[3], color, viewport, [u0, v1]),
        ]);
    }
    transform_vertices(&mut vertices, transform, viewport);
    vertices
}

fn transform_vertices(vertices: &mut [Vertex], transform: Transform, viewport: Viewport) {
    if transform == Transform::IDENTITY {
        return;
    }
    for vertex in vertices {
        let point = transform.transform_point(Point::new(
            (vertex.position[0] + 1.0) * viewport.physical_size[0] as f32
                / (2.0 * viewport.scale_factor.get()),
            (1.0 - vertex.position[1]) * viewport.physical_size[1] as f32
                / (2.0 * viewport.scale_factor.get()),
        ));
        vertex.position = [
            point.x * viewport.scale_factor.get() / viewport.physical_size[0].max(1) as f32 * 2.0
                - 1.0,
            1.0 - point.y * viewport.scale_factor.get() / viewport.physical_size[1].max(1) as f32
                * 2.0,
        ];
    }
}

fn vertex_with_color(point: Point, color: [f32; 4], viewport: Viewport, uv: [f32; 2]) -> Vertex {
    let width = viewport.physical_size[0].max(1) as f32;
    let height = viewport.physical_size[1].max(1) as f32;
    let scale = viewport.scale_factor.get();
    Vertex {
        position: [
            point.x * scale / width * 2.0 - 1.0,
            1.0 - point.y * scale / height * 2.0,
        ],
        uv,
        color,
    }
}

struct ImageTexture {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

pub struct UiRenderer {
    shape_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    text_sampler: wgpu::Sampler,
    white: ImageTexture,
    images: HashMap<ImageId, ImageTexture>,
    text_atlas: ImageTexture,
    text_atlas_size: [u32; 2],
    uploaded_text_generation: u64,
    uploaded_text_system: Option<usize>,
    pending_texture_upload_bytes: u64,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    msaa_samples: u32,
    msaa_texture: Option<wgpu::Texture>,
    msaa_view: Option<wgpu::TextureView>,
    format: wgpu::TextureFormat,
    stats: RendererStats,
}

impl UiRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        options: RendererOptions,
    ) -> Result<Self, RendererError> {
        if !matches!(options.msaa_samples, 1 | 4) {
            return Err(RendererError::InvalidMsaaSamples(options.msaa_samples));
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui-renderer-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-renderer-image-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-renderer-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shape_pipeline = pipelines::shape::build(
            device,
            &pipeline_layout,
            &shader,
            format,
            options.msaa_samples,
        );
        let image_pipeline = pipelines::image::build(
            device,
            &pipeline_layout,
            &shader,
            format,
            options.msaa_samples,
        );
        let text_pipeline = pipelines::text::build(
            device,
            &pipeline_layout,
            &shader,
            format,
            options.msaa_samples,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui-renderer-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let text_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui-renderer-glyph-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: options.text_filter,
            min_filter: options.text_filter,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let white = create_texture(
            device,
            &bind_group_layout,
            &sampler,
            ImageId(0),
            [1, 1],
            "ui-renderer-white",
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let vertex_capacity = 1024;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-renderer-vertices"),
            size: (vertex_capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let text_atlas = create_mask_texture(
            device,
            &bind_group_layout,
            &text_sampler,
            "ui-renderer-glyph-atlas",
            1024,
        );
        Ok(Self {
            shape_pipeline,
            image_pipeline,
            text_pipeline,
            bind_group_layout,
            sampler,
            text_sampler,
            white,
            images: HashMap::new(),
            text_atlas,
            text_atlas_size: [1024, 1024],
            uploaded_text_generation: 0,
            uploaded_text_system: None,
            pending_texture_upload_bytes: 0,
            vertex_buffer,
            vertex_capacity,
            msaa_samples: options.msaa_samples,
            msaa_texture: None,
            msaa_view: None,
            format,
            stats: RendererStats::default(),
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, physical_size: [u32; 2]) {
        if self.msaa_samples == 1 || physical_size[0] == 0 || physical_size[1] == 0 {
            self.msaa_texture = None;
            self.msaa_view = None;
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui-renderer-msaa"),
            size: wgpu::Extent3d {
                width: physical_size[0],
                height: physical_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: self.msaa_samples,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.msaa_texture = Some(texture);
        self.msaa_view = Some(view);
    }

    pub fn upload_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: ImageId,
        data: ImageData<'_>,
    ) -> Result<(), RendererError> {
        let expected = data.size[0] as usize * data.size[1] as usize * 4;
        if data.rgba8.len() != expected {
            return Err(RendererError::InvalidImageData {
                expected,
                actual: data.rgba8.len(),
            });
        }
        let texture = create_texture(
            device,
            &self.bind_group_layout,
            &self.sampler,
            id,
            data.size,
            "ui-renderer-image",
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data.rgba8,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(data.size[0] * 4),
                rows_per_image: Some(data.size[1]),
            },
            wgpu::Extent3d {
                width: data.size[0],
                height: data.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.pending_texture_upload_bytes += expected as u64;
        self.images.insert(id, texture);
        Ok(())
    }

    pub fn render(
        &mut self,
        frame: RenderFrame<'_>,
        list: &DisplayList,
        mut text: Option<&mut TextSystem>,
    ) {
        let RenderFrame {
            device,
            queue,
            encoder,
            target,
            viewport,
        } = frame;
        let prepare_started = Instant::now();
        if viewport.physical_size[0] == 0 || viewport.physical_size[1] == 0 {
            return;
        }
        let mut batches: Vec<PreparedBatch> = Vec::new();
        if let Some(text_system) = text.as_deref_mut() {
            text_system.begin_frame();
        }
        let viewport_clip = ClipState {
            id: 0,
            scissor: [0, 0, viewport.physical_size[0], viewport.physical_size[1]],
            polygon: vec![[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]],
            geometry_clip: false,
        };
        let mut clips = vec![viewport_clip];
        let mut transforms = vec![Transform::IDENTITY];
        let mut next_clip_id = 1;
        let before_text_stats = text.as_ref().map(|system| system.stats());
        for command in list.commands() {
            match command {
                DisplayCommand::PushClip(rect) => {
                    let transform = transforms.last().copied().unwrap_or(Transform::IDENTITY);
                    let transformed = transform.transform_rect(*rect);
                    let scissor = physical_clip(
                        transformed,
                        viewport.scale_factor,
                        clips.last().unwrap().scissor,
                        viewport.physical_size,
                    );
                    let corners = rect_corners(*rect)
                        .map(|point| transform.transform_point(point))
                        .map(|point| point_to_ndc(point, viewport));
                    let polygon =
                        intersect_convex_polygons(&clips.last().unwrap().polygon, &corners);
                    let geometry_clip =
                        clips.last().unwrap().geometry_clip || !transform.is_axis_aligned();
                    clips.push(ClipState {
                        id: next_clip_id,
                        scissor,
                        polygon,
                        geometry_clip,
                    });
                    next_clip_id += 1;
                }
                DisplayCommand::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                DisplayCommand::PushTransform(transform) => {
                    let current = transforms.last().copied().unwrap_or(Transform::IDENTITY);
                    transforms.push(transform.then(current));
                }
                DisplayCommand::PopTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                }
                paint => {
                    let current_transform =
                        transforms.last().copied().unwrap_or(Transform::IDENTITY);
                    let mut vertex_transform = current_transform;
                    let clip = clips.last().unwrap();
                    let (kind, vertices) = match paint {
                        DisplayCommand::FillRect { rect, color } => {
                            (BatchKind::Shape, rect_vertices(*rect, *color, viewport))
                        }
                        DisplayCommand::FillRoundedRect {
                            rect,
                            radius,
                            color,
                        } => (
                            BatchKind::Shape,
                            rounded_vertices(*rect, *radius, *color, viewport),
                        ),
                        DisplayCommand::StrokeRoundedRect {
                            rect,
                            radius,
                            stroke,
                        } => (
                            BatchKind::Shape,
                            border_vertices(*rect, *radius, *stroke, viewport),
                        ),
                        DisplayCommand::Line { from, to, stroke } => (
                            BatchKind::Shape,
                            line_vertices(*from, *to, *stroke, viewport),
                        ),
                        DisplayCommand::Image { rect, image, tint } => (
                            BatchKind::Image(*image),
                            image_vertices(*rect, *tint, viewport),
                        ),
                        DisplayCommand::Text { run, origin } => {
                            let Some(text_system) = text.as_deref_mut() else {
                                continue;
                            };
                            // Include translations in the raster key, rather than
                            // moving an already positioned bitmap by a fractional pixel.
                            let text_origin = if current_transform.matrix[0][0] == 1.0
                                && current_transform.matrix[0][1] == 0.0
                                && current_transform.matrix[1][0] == 0.0
                                && current_transform.matrix[1][1] == 1.0
                            {
                                vertex_transform = Transform::IDENTITY;
                                current_transform.transform_point(*origin)
                            } else {
                                *origin
                            };
                            let prepared = match text_system.prepare(
                                *run,
                                text_origin,
                                viewport.scale_factor.get(),
                            ) {
                                Ok(prepared) => prepared,
                                Err(_) => continue,
                            };
                            (
                                BatchKind::Text,
                                text_vertices(&prepared, Transform::IDENTITY, viewport),
                            )
                        }
                        DisplayCommand::PushClip(_)
                        | DisplayCommand::PopClip
                        | DisplayCommand::PushTransform(_)
                        | DisplayCommand::PopTransform => continue,
                    };
                    if vertices.is_empty() {
                        continue;
                    }
                    let mut vertices = vertices;
                    transform_vertices(&mut vertices, vertex_transform, viewport);
                    if clip.geometry_clip {
                        vertices = clip_triangles_to_polygon(&vertices, &clip.polygon);
                    }
                    if vertices.is_empty() {
                        continue;
                    }
                    if let Some(last) = batches
                        .last_mut()
                        .filter(|batch| batch.kind == kind && batch.clip_id == clip.id)
                    {
                        last.vertices.extend(vertices);
                    } else {
                        batches.push(PreparedBatch {
                            kind,
                            clip: clip.scissor,
                            clip_id: clip.id,
                            vertices,
                            vertex_range: 0..0,
                        });
                    }
                }
            }
        }
        let mut glyph_upload_bytes = 0;
        if let Some(text_system) = text.as_deref_mut() {
            let identity = std::ptr::from_ref(text_system).addr();
            let atlas_size = text_system.atlas().size();
            let atlas_generation = text_system.atlas().generation();
            if atlas_size != self.text_atlas_size {
                self.text_atlas = create_mask_texture(
                    device,
                    &self.bind_group_layout,
                    &self.text_sampler,
                    "ui-renderer-glyph-atlas",
                    atlas_size[0],
                );
                self.text_atlas_size = atlas_size;
            }
            let full_upload = self.uploaded_text_system != Some(identity)
                || self.uploaded_text_generation != atlas_generation;
            let mut regions = if full_upload {
                vec![[0, 0, atlas_size[0], atlas_size[1]]]
            } else {
                text_system.take_dirty_regions()
            };
            let dirty_bytes = regions
                .iter()
                .map(|rect| u64::from(rect[2]) * u64::from(rect[3]))
                .sum::<u64>();
            let atlas_bytes = u64::from(atlas_size[0]) * u64::from(atlas_size[1]);
            if !full_upload && (regions.len() > 64 || dirty_bytes * 4 > atlas_bytes) {
                regions.clear();
                regions.push([0, 0, atlas_size[0], atlas_size[1]]);
                text_system.take_dirty_regions();
            }
            for [x, y, width, height] in regions {
                let pixels = text_system.atlas().copy_region([x, y, width, height]);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.text_atlas._texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width),
                        rows_per_image: Some(height),
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                glyph_upload_bytes += u64::from(width) * u64::from(height);
            }
            if full_upload {
                text_system.take_dirty_regions();
            }
            self.uploaded_text_generation = atlas_generation;
            self.uploaded_text_system = Some(identity);
        }
        let after_text_stats = text.as_ref().map(|system| system.stats());
        let total_vertices = batches
            .iter()
            .map(|batch| batch.vertices.len())
            .sum::<usize>();
        self.ensure_vertex_capacity(device, total_vertices);
        let mut vertex_data = Vec::with_capacity(total_vertices);
        let mut vertex_offset = 0u32;
        for batch in &mut batches {
            let start = vertex_offset;
            vertex_offset += batch.vertices.len() as u32;
            batch.vertex_range = start..vertex_offset;
            vertex_data.extend_from_slice(&batch.vertices);
        }
        if !vertex_data.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertex_data));
        }
        self.stats = RendererStats {
            frame_prepare_time: prepare_started.elapsed(),
            batches: batches.len() as u64,
            draw_calls: batches.len() as u64,
            vertices: total_vertices as u64,
            texture_upload_bytes: std::mem::take(&mut self.pending_texture_upload_bytes)
                + glyph_upload_bytes,
            glyph_upload_bytes,
            ..RendererStats::default()
        };
        if let (Some(before), Some(after)) = (before_text_stats, after_text_stats) {
            self.stats.glyph_cache_hits = after.hits - before.hits;
            self.stats.glyph_cache_misses = after.misses - before.misses;
            self.stats.glyph_cache_evictions = after.evictions - before.evictions;
            let lookups = self.stats.glyph_cache_hits + self.stats.glyph_cache_misses;
            self.stats.glyph_cache_hit_rate = if lookups == 0 {
                0.0
            } else {
                self.stats.glyph_cache_hits as f32 / lookups as f32
            };
            self.stats.glyphs_rasterized = after.rasterized - before.rasterized;
            self.stats.atlas_used_pixels = after.atlas_used_pixels;
            self.stats.atlas_capacity_pixels = after.atlas_capacity_pixels;
            self.stats.atlas_free_rect_count = after.free_rect_count;
            self.stats.atlas_fragmentation_per_mille = after.fragmentation_per_mille;
        }
        let color_attachment = wgpu::RenderPassColorAttachment {
            view: self.msaa_view.as_ref().unwrap_or(target),
            depth_slice: None,
            resolve_target: self.msaa_view.as_ref().map(|_| target),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.055,
                    g: 0.063,
                    b: 0.078,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui-renderer-pass"),
            color_attachments: &[Some(color_attachment)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        for batch in &batches {
            let [x, y, width, height] = batch.clip;
            pass.set_scissor_rect(x, y, width, height);
            let pipeline = match batch.kind {
                BatchKind::Shape => &self.shape_pipeline,
                BatchKind::Image(_) => &self.image_pipeline,
                BatchKind::Text => &self.text_pipeline,
            };
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            let binding = match batch.kind {
                BatchKind::Shape => &self.white,
                BatchKind::Image(image) => self.images.get(&image).unwrap_or(&self.white),
                BatchKind::Text => &self.text_atlas,
            };
            pass.set_bind_group(0, &binding.bind_group, &[]);
            pass.draw(batch.vertex_range.clone(), 0..1);
        }
    }

    pub fn stats(&self) -> RendererStats {
        self.stats
    }

    pub fn record_queue_submit_time(&mut self, duration: Duration) {
        self.stats.queue_submit_time = Some(duration);
    }

    fn ensure_vertex_capacity(&mut self, device: &wgpu::Device, required: usize) {
        if required <= self.vertex_capacity {
            return;
        }
        self.vertex_capacity = required.next_power_of_two();
        self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-renderer-vertices"),
            size: (self.vertex_capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatchKind {
    Shape,
    Image(ImageId),
    Text,
}

struct ClipState {
    id: u64,
    scissor: [u32; 4],
    polygon: Vec<[f32; 2]>,
    geometry_clip: bool,
}

struct PreparedBatch {
    kind: BatchKind,
    clip: [u32; 4],
    clip_id: u64,
    vertices: Vec<Vertex>,
    vertex_range: std::ops::Range<u32>,
}

fn create_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    id: ImageId,
    size: [u32; 2],
    label: &str,
) -> ImageTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("ui-renderer-image-{}", id.0)),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    ImageTexture {
        _texture: texture,
        bind_group,
    }
}

fn rect_corners(rect: Rect) -> [Point; 4] {
    [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ]
}

fn point_to_ndc(point: Point, viewport: Viewport) -> [f32; 2] {
    [
        point.x * viewport.scale_factor.get() / viewport.physical_size[0].max(1) as f32 * 2.0 - 1.0,
        1.0 - point.y * viewport.scale_factor.get() / viewport.physical_size[1].max(1) as f32 * 2.0,
    ]
}

fn cross(lhs: [f32; 2], rhs: [f32; 2]) -> f32 {
    lhs[0] * rhs[1] - lhs[1] * rhs[0]
}

fn polygon_area(polygon: &[[f32; 2]]) -> f32 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f32>()
        * 0.5
}

fn intersect_convex_polygons(subject: &[[f32; 2]], clip: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if subject.is_empty() || clip.len() < 3 {
        return Vec::new();
    }
    let winding = polygon_area(clip).signum();
    let mut output = subject.to_vec();
    for (edge_start, edge_end) in clip
        .iter()
        .zip(clip.iter().cycle().skip(1))
        .take(clip.len())
    {
        let input = std::mem::take(&mut output);
        if input.is_empty() {
            break;
        }
        let mut previous = *input.last().unwrap();
        let mut previous_distance = cross(
            [edge_end[0] - edge_start[0], edge_end[1] - edge_start[1]],
            [previous[0] - edge_start[0], previous[1] - edge_start[1]],
        ) * winding;
        for current in input {
            let current_distance = cross(
                [edge_end[0] - edge_start[0], edge_end[1] - edge_start[1]],
                [current[0] - edge_start[0], current[1] - edge_start[1]],
            ) * winding;
            let previous_inside = previous_distance >= -1e-6;
            let current_inside = current_distance >= -1e-6;
            if previous_inside != current_inside {
                let t = previous_distance / (previous_distance - current_distance);
                output.push([
                    previous[0] + (current[0] - previous[0]) * t,
                    previous[1] + (current[1] - previous[1]) * t,
                ]);
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_distance = current_distance;
        }
    }
    output
}

fn clip_triangles_to_polygon(vertices: &[Vertex], clip: &[[f32; 2]]) -> Vec<Vertex> {
    let mut result = Vec::with_capacity(vertices.len());
    for triangle in vertices.chunks_exact(3) {
        let mut polygon = triangle.to_vec();
        let winding = polygon_area(clip).signum();
        for (edge_start, edge_end) in clip
            .iter()
            .zip(clip.iter().cycle().skip(1))
            .take(clip.len())
        {
            let input = std::mem::take(&mut polygon);
            if input.is_empty() {
                break;
            }
            let distance = |vertex: &Vertex| {
                cross(
                    [edge_end[0] - edge_start[0], edge_end[1] - edge_start[1]],
                    [
                        vertex.position[0] - edge_start[0],
                        vertex.position[1] - edge_start[1],
                    ],
                ) * winding
            };
            let mut previous = *input.last().unwrap();
            let mut previous_distance = distance(&previous);
            for current in input {
                let current_distance = distance(&current);
                let previous_inside = previous_distance >= -1e-6;
                let current_inside = current_distance >= -1e-6;
                if previous_inside != current_inside {
                    let t = previous_distance / (previous_distance - current_distance);
                    polygon.push(interpolate_vertex(previous, current, t));
                }
                if current_inside {
                    polygon.push(current);
                }
                previous = current;
                previous_distance = current_distance;
            }
        }
        if polygon.len() >= 3 {
            for index in 1..polygon.len() - 1 {
                result.extend_from_slice(&[polygon[0], polygon[index], polygon[index + 1]]);
            }
        }
    }
    result
}

fn interpolate_vertex(from: Vertex, to: Vertex, amount: f32) -> Vertex {
    let mix = |a: f32, b: f32| a + (b - a) * amount;
    Vertex {
        position: [
            mix(from.position[0], to.position[0]),
            mix(from.position[1], to.position[1]),
        ],
        uv: [mix(from.uv[0], to.uv[0]), mix(from.uv[1], to.uv[1])],
        color: std::array::from_fn(|index| mix(from.color[index], to.color[index])),
    }
}

fn physical_clip(rect: Rect, scale: ScaleFactor, parent: [u32; 4], size: [u32; 2]) -> [u32; 4] {
    let scale = scale.get();
    let x0 = (rect.min.x * scale).floor().max(0.0) as u32;
    let y0 = (rect.min.y * scale).floor().max(0.0) as u32;
    let x1 = (rect.max.x * scale).ceil().max(0.0) as u32;
    let y1 = (rect.max.y * scale).ceil().max(0.0) as u32;
    let x0 = x0.min(size[0]).max(parent[0]);
    let y0 = y0.min(size[1]).max(parent[1]);
    let x1 = x1
        .min(size[0])
        .max(x0)
        .min(parent[0].saturating_add(parent[2]));
    let y1 = y1
        .min(size[1])
        .max(y0)
        .min(parent[1].saturating_add(parent[3]));
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}

fn vertex(point: Point, color: Color, viewport: Viewport, uv: [f32; 2]) -> Vertex {
    let width = viewport.physical_size[0].max(1) as f32;
    let height = viewport.physical_size[1].max(1) as f32;
    let physical = [
        point.x * viewport.scale_factor.get(),
        point.y * viewport.scale_factor.get(),
    ];
    Vertex {
        position: [
            physical[0] / width * 2.0 - 1.0,
            1.0 - physical[1] / height * 2.0,
        ],
        uv,
        color: color.premultiplied_linear(),
    }
}

fn push_rect(vertices: &mut Vec<Vertex>, rect: Rect, color: Color, viewport: Viewport) {
    let rect = rect.snap_to_physical(viewport.scale_factor);
    let p = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    vertices.extend_from_slice(&[
        vertex(p[0], color, viewport, [0.0, 0.0]),
        vertex(p[1], color, viewport, [1.0, 0.0]),
        vertex(p[2], color, viewport, [1.0, 1.0]),
        vertex(p[0], color, viewport, [0.0, 0.0]),
        vertex(p[2], color, viewport, [1.0, 1.0]),
        vertex(p[3], color, viewport, [0.0, 1.0]),
    ]);
}

fn rounded_outline(rect: Rect, radius: Radius) -> Vec<Point> {
    let radius = radius.clamp_to(rect);
    let corners = [
        (
            Point::new(rect.max.x - radius.top_right, rect.min.y + radius.top_right),
            radius.top_right,
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            Point::new(
                rect.max.x - radius.bottom_right,
                rect.max.y - radius.bottom_right,
            ),
            radius.bottom_right,
            0.0,
        ),
        (
            Point::new(
                rect.min.x + radius.bottom_left,
                rect.max.y - radius.bottom_left,
            ),
            radius.bottom_left,
            std::f32::consts::FRAC_PI_2,
        ),
        (
            Point::new(rect.min.x + radius.top_left, rect.min.y + radius.top_left),
            radius.top_left,
            std::f32::consts::PI,
        ),
    ];
    let mut points = Vec::with_capacity(ROUND_SEGMENTS * 4 + 1);
    for (center, radius, start) in corners {
        if radius == 0.0 {
            points.push(Point::new(
                center.x + start.cos() * radius,
                center.y + start.sin() * radius,
            ));
            continue;
        }
        for step in 0..=ROUND_SEGMENTS {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / ROUND_SEGMENTS as f32;
            points.push(Point::new(
                center.x + angle.cos() * radius,
                center.y + angle.sin() * radius,
            ));
        }
    }
    points
}

fn push_rounded_rect(
    vertices: &mut Vec<Vertex>,
    rect: Rect,
    radius: Radius,
    color: Color,
    viewport: Viewport,
) {
    let rect = rect.snap_to_physical(viewport.scale_factor);
    let outline = rounded_outline(rect, radius);
    let center = rect.center();
    for pair in outline.windows(2) {
        vertices.extend_from_slice(&[
            vertex(center, color, viewport, [0.5, 0.5]),
            vertex(pair[0], color, viewport, [0.0, 0.0]),
            vertex(pair[1], color, viewport, [1.0, 1.0]),
        ]);
    }
    if let (Some(first), Some(last)) = (outline.first(), outline.last()) {
        vertices.extend_from_slice(&[
            vertex(center, color, viewport, [0.5, 0.5]),
            vertex(*last, color, viewport, [1.0, 1.0]),
            vertex(*first, color, viewport, [0.0, 0.0]),
        ]);
    }
}

fn push_border(
    vertices: &mut Vec<Vertex>,
    rect: Rect,
    radius: Radius,
    stroke: Stroke,
    viewport: Viewport,
) {
    if stroke.width <= 0.0 {
        return;
    }
    let rect = rect.snap_to_physical(viewport.scale_factor);
    let outer = rounded_outline(rect, radius);
    let inner_rect = rect
        .inset(stroke.width)
        .intersect(rect)
        .unwrap_or(Rect::ZERO);
    let inner = rounded_outline(
        inner_rect,
        Radius {
            top_left: radius.top_left - stroke.width,
            top_right: radius.top_right - stroke.width,
            bottom_right: radius.bottom_right - stroke.width,
            bottom_left: radius.bottom_left - stroke.width,
        },
    );
    let count = outer.len().min(inner.len());
    for index in 0..count {
        let next = (index + 1) % count;
        vertices.extend_from_slice(&[
            vertex(outer[index], stroke.color, viewport, [0.0, 0.0]),
            vertex(outer[next], stroke.color, viewport, [1.0, 0.0]),
            vertex(inner[next], stroke.color, viewport, [1.0, 1.0]),
            vertex(outer[index], stroke.color, viewport, [0.0, 0.0]),
            vertex(inner[next], stroke.color, viewport, [1.0, 1.0]),
            vertex(inner[index], stroke.color, viewport, [0.0, 1.0]),
        ]);
    }
}

fn push_line(
    vertices: &mut Vec<Vertex>,
    from: Point,
    to: Point,
    stroke: Stroke,
    viewport: Viewport,
) {
    if stroke.width <= 0.0 {
        return;
    }
    let scale = viewport.scale_factor.get();
    let source_delta = to - from;
    let (from, to) = if source_delta.y.abs() < f32::EPSILON {
        let y = snap_line_center(from.y, stroke.width, scale);
        (
            Point::new((from.x * scale).round() / scale, y),
            Point::new((to.x * scale).round() / scale, y),
        )
    } else if source_delta.x.abs() < f32::EPSILON {
        let x = snap_line_center(from.x, stroke.width, scale);
        (
            Point::new(x, (from.y * scale).round() / scale),
            Point::new(x, (to.y * scale).round() / scale),
        )
    } else {
        (
            Point::new(
                (from.x * scale).round() / scale,
                (from.y * scale).round() / scale,
            ),
            Point::new(
                (to.x * scale).round() / scale,
                (to.y * scale).round() / scale,
            ),
        )
    };
    let delta = to - from;
    let length = (delta.x * delta.x + delta.y * delta.y).sqrt();
    if length == 0.0 {
        return;
    }
    let half = stroke.width * 0.5;
    let normal = Point::new(-delta.y / length * half, delta.x / length * half);
    let points = [from + normal, to + normal, to - normal, from - normal];
    vertices.extend_from_slice(&[
        vertex(points[0], stroke.color, viewport, [0.0, 0.0]),
        vertex(points[1], stroke.color, viewport, [1.0, 0.0]),
        vertex(points[2], stroke.color, viewport, [1.0, 1.0]),
        vertex(points[0], stroke.color, viewport, [0.0, 0.0]),
        vertex(points[2], stroke.color, viewport, [1.0, 1.0]),
        vertex(points[3], stroke.color, viewport, [0.0, 1.0]),
    ]);
}

fn snap_line_center(position: f32, width: f32, scale: f32) -> f32 {
    let physical_width = width * scale;
    let physical_position = position * scale;
    let rounded_width = physical_width.round();
    let aligned = if (physical_width - rounded_width).abs() < 0.001 {
        if (rounded_width as i32).rem_euclid(2) == 1 {
            (physical_position - 0.5).round() + 0.5
        } else {
            physical_position.round()
        }
    } else {
        (physical_position - 0.5).round() + 0.5
    };
    aligned / scale
}

fn push_image(vertices: &mut Vec<Vertex>, rect: Rect, tint: Color, viewport: Viewport) {
    let rect = rect.snap_to_physical(viewport.scale_factor);
    let p = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    vertices.extend_from_slice(&[
        vertex(p[0], tint, viewport, [0.0, 0.0]),
        vertex(p[1], tint, viewport, [1.0, 0.0]),
        vertex(p[2], tint, viewport, [1.0, 1.0]),
        vertex(p[0], tint, viewport, [0.0, 0.0]),
        vertex(p[2], tint, viewport, [1.0, 1.0]),
        vertex(p[3], tint, viewport, [0.0, 1.0]),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_line_center_alignment_accounts_for_physical_width() {
        assert_eq!(snap_line_center(20.0, 1.0, 1.0), 20.5);
        assert_eq!(snap_line_center(20.0, 2.0, 1.0), 20.0);
        assert_eq!(snap_line_center(20.0, 1.0, 2.0), 20.0);
        assert_eq!(snap_line_center(20.0, 0.5, 1.0), 20.5);
    }

    #[test]
    fn rotated_clip_intersects_geometry_instead_of_using_its_bounding_box() {
        let square = [[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];
        let triangle = [
            Vertex {
                position: [-1.0, 0.0],
                uv: [0.0, 0.0],
                color: [1.0; 4],
            },
            Vertex {
                position: [1.0, 0.0],
                uv: [1.0, 0.0],
                color: [1.0; 4],
            },
            Vertex {
                position: [0.0, -1.0],
                uv: [0.5, 1.0],
                color: [1.0; 4],
            },
        ];
        let clipped = clip_triangles_to_polygon(&triangle, &square);
        assert!(!clipped.is_empty());
        assert!(clipped.iter().all(|vertex| {
            vertex.position[0].abs() <= 0.5001 && vertex.position[1].abs() <= 0.5001
        }));
    }

    #[test]
    fn affine_vertices_preserve_fractional_position() {
        let viewport = Viewport::new([200, 100], ScaleFactor::new(2.0));
        let mut vertices = rect_vertices(
            Rect::from_min_size(Point::new(10.0, 5.0), ui_core::Size::new(10.0, 10.0)),
            Color::WHITE,
            viewport,
        );
        transform_vertices(&mut vertices, Transform::translation(0.25, 0.5), viewport);
        let logical_x = (vertices[0].position[0] + 1.0) * 200.0 / 4.0;
        let logical_y = (1.0 - vertices[0].position[1]) * 100.0 / 4.0;
        assert!((logical_x - 10.25).abs() < 0.0001);
        assert!((logical_y - 5.5).abs() < 0.0001);
    }

    #[test]
    fn fractional_clip_bounds_expand_outward_without_cutting_edge_pixels() {
        let clip = physical_clip(
            Rect::from_min_max(Point::new(10.2, 4.1), Point::new(20.1, 8.2)),
            ScaleFactor::new(1.5),
            [0, 0, 100, 100],
            [100, 100],
        );
        assert_eq!(clip, [15, 6, 16, 7]);
    }

    #[test]
    fn text_vertices_map_bitmap_pixels_and_texel_centres_one_to_one() {
        let mut text = TextSystem::new();
        let run = text.shape("Ag", ui_text::TextStyle::default(), None);
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let viewport = Viewport::new([2560, 1600], ScaleFactor::new(scale));
            let prepared = text.prepare(run, Point::new(20.37, 40.63), scale).unwrap();
            let vertices = text_vertices(&prepared, Transform::IDENTITY, viewport);
            for (quad, vertices) in prepared.glyphs.iter().zip(vertices.chunks_exact(6)) {
                let physical_x = (vertices[0].position[0] + 1.0) * 1280.0;
                let physical_y = (1.0 - vertices[0].position[1]) * 800.0;
                assert!((physical_x - quad.origin.x * scale).abs() < 0.001);
                assert!((physical_y - quad.origin.y * scale).abs() < 0.001);
                let width = quad.size.width * scale;
                let u_first_pixel =
                    quad.uv_min[0] + (quad.uv_max[0] - quad.uv_min[0]) * 0.5 / width;
                let atlas_x = quad.uv_min[0] * text.atlas().size()[0] as f32;
                assert!(
                    (u_first_pixel * text.atlas().size()[0] as f32 - atlas_x - 0.5).abs() < 0.001
                );
            }
        }
    }
}
