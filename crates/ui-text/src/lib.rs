//! Text shaping and CPU glyph-atlas preparation for desktop UI rendering.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use swash::scale::image::Content as SwashContent;
pub use ui_core::TextRunId;
use ui_core::{Color, Point, Rect, Size};
use unicode_segmentation::UnicodeSegmentation;

const DEFAULT_ATLAS_SIZE: u32 = 1024;
const MAX_ATLAS_SIZE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Serif,
    Monospace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Regular,
    Medium,
    Bold,
}

impl FontWeight {
    fn cosmic(self) -> Weight {
        match self {
            Self::Regular => Weight::NORMAL,
            Self::Medium => Weight::MEDIUM,
            Self::Bold => Weight::BOLD,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub family: FontFamily,
    pub weight: FontWeight,
    pub size_px: f32,
    pub line_height_px: f32,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::Sans,
            weight: FontWeight::Regular,
            size_px: 14.0,
            line_height_px: 20.0,
            color: Color::WHITE,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub size: Size,
    pub baseline: f32,
    pub glyph_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextLineMetrics {
    pub start: usize,
    pub end: usize,
    pub top: f32,
    pub baseline: f32,
    pub height: f32,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub rasterized: u64,
    pub atlas_used_pixels: u64,
    pub atlas_capacity_pixels: u64,
    pub evictions: u64,
    pub free_rect_count: u64,
    pub fragmentation_per_mille: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    /// Logical position, kept fractional through the GPU vertex stage.
    pub origin: Point,
    pub size: Size,
    /// Normalized coordinates into the single-channel glyph atlas.
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct PreparedText {
    pub id: TextRunId,
    pub metrics: TextMetrics,
    pub glyphs: Vec<GlyphQuad>,
}

#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    size: [u32; 2],
    pixels: Vec<u8>,
    generation: u64,
}

impl GlyphAtlas {
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn copy_region(&self, rect: [u32; 4]) -> Vec<u8> {
        let [x, y, width, height] = rect;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for row in y..y + height {
            let start = (row * self.size[0] + x) as usize;
            pixels.extend_from_slice(&self.pixels[start..start + width as usize]);
        }
        pixels
    }
}

#[derive(Clone, Copy, Debug)]
struct AtlasEntry {
    rect: [u32; 4],
    allocation: [u32; 4],
    left: i32,
    top: i32,
    last_used: u64,
    pinned_frame: u64,
}

#[derive(Debug)]
struct PendingGlyph {
    key: cosmic_text::CacheKey,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
}

#[derive(Clone, Debug)]
struct GlyphPosition {
    origin: Point,
    font_id: fontdb::ID,
    glyph_id: u16,
    font_size: f32,
    font_weight: fontdb::Weight,
    cache_key_flags: cosmic_text::CacheKeyFlags,
}

#[derive(Clone, Debug)]
struct ShapedRun {
    metrics: TextMetrics,
    glyphs: Vec<GlyphPosition>,
    color: Color,
    text: String,
    layout: Arc<Buffer>,
}

#[derive(Debug)]
pub struct TextSystem {
    fonts: FontSystem,
    rasterizer: SwashCache,
    runs: HashMap<TextRunId, Arc<ShapedRun>>,
    atlas: GlyphAtlas,
    glyphs: HashMap<cosmic_text::CacheKey, AtlasEntry>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_height: u32,
    max_atlas_size: u32,
    free_rects: Vec<[u32; 4]>,
    dirty_regions: Vec<[u32; 4]>,
    pending_glyphs: Vec<PendingGlyph>,
    grow_pending: bool,
    frame: u64,
    clock: u64,
    next_run_id: u64,
    stats: GlyphCacheStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    AtlasFull,
    UnknownRun(TextRunId),
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtlasFull => f.write_str("glyph atlas is full; create a larger text atlas"),
            Self::UnknownRun(id) => write!(f, "text run {} does not exist", id.0),
        }
    }
}

impl std::error::Error for TextError {}

impl TextSystem {
    pub fn new() -> Self {
        Self::with_atlas_size(DEFAULT_ATLAS_SIZE)
    }

    pub fn with_atlas_size(size: u32) -> Self {
        Self::with_atlas_limits(size, MAX_ATLAS_SIZE)
    }

    pub fn with_atlas_limits(size: u32, max_size: u32) -> Self {
        let size = size.max(64);
        let max_size = max_size.max(size);
        Self {
            fonts: FontSystem::new(),
            rasterizer: SwashCache::new(),
            runs: HashMap::new(),
            atlas: GlyphAtlas {
                size: [size, size],
                pixels: vec![0; (size * size) as usize],
                generation: 0,
            },
            glyphs: HashMap::new(),
            shelf_x: 1,
            shelf_y: 1,
            shelf_height: 0,
            max_atlas_size: max_size,
            free_rects: Vec::new(),
            dirty_regions: Vec::new(),
            pending_glyphs: Vec::new(),
            grow_pending: false,
            frame: 0,
            clock: 0,
            next_run_id: 1,
            stats: GlyphCacheStats {
                atlas_capacity_pixels: (size * size) as u64,
                ..GlyphCacheStats::default()
            },
        }
    }

    pub fn shape(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> TextRunId {
        let run = self.shape_run(text, style, width);
        let id = TextRunId(self.next_run_id);
        self.next_run_id += 1;
        self.runs.insert(id, Arc::new(run));
        id
    }

    pub fn update_run(
        &mut self,
        id: TextRunId,
        text: &str,
        style: TextStyle,
        width: Option<f32>,
    ) -> Result<(), TextError> {
        if !self.runs.contains_key(&id) {
            return Err(TextError::UnknownRun(id));
        }
        let run = self.shape_run(text, style, width);
        self.runs.insert(id, Arc::new(run));
        Ok(())
    }

    pub fn remove_run(&mut self, id: TextRunId) -> bool {
        self.runs.remove(&id).is_some()
    }

    pub fn run_metrics(&self, id: TextRunId) -> Option<TextMetrics> {
        self.runs.get(&id).map(|run| run.metrics)
    }

    pub fn set_run_color(&mut self, id: TextRunId, color: Color) -> Result<(), TextError> {
        let run = self.runs.get_mut(&id).ok_or(TextError::UnknownRun(id))?;
        Arc::make_mut(run).color = color;
        Ok(())
    }

    fn shape_run(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> ShapedRun {
        let metrics = Metrics::new(style.size_px, style.line_height_px.max(style.size_px));
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        buffer.set_size(width, None);
        let family = match style.family {
            FontFamily::Sans => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
        };
        let attrs = Attrs::new().family(family).weight(style.weight.cosmic());
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);

        let mut glyphs = Vec::new();
        let mut measured_width: f32 = 0.0;
        let mut measured_height: f32 = 0.0;
        let mut baseline: f32 = style.size_px;
        for run in buffer.layout_runs() {
            measured_width = measured_width.max(run.line_w);
            measured_height = measured_height.max(run.line_top + run.line_height);
            baseline = run.line_y;
            for glyph in run.glyphs {
                let x = glyph.x + glyph.font_size * glyph.x_offset;
                let y = run.line_y - glyph.font_size * glyph.y_offset;
                glyphs.push(GlyphPosition {
                    origin: Point::new(x, y),
                    font_id: glyph.font_id,
                    glyph_id: glyph.glyph_id,
                    font_size: glyph.font_size,
                    font_weight: glyph.font_weight,
                    cache_key_flags: glyph.cache_key_flags,
                });
            }
        }
        ShapedRun {
            metrics: TextMetrics {
                size: Size::new(measured_width, measured_height),
                baseline,
                glyph_count: glyphs.len(),
            },
            glyphs,
            color: style.color,
            text: text.to_owned(),
            layout: Arc::new(buffer),
        }
    }

    /// Resolve a grapheme-cluster index to logical-point coordinates from the shaped layout.
    pub fn position_to_point(&self, id: TextRunId, position: usize) -> Option<Point> {
        let run = self.runs.get(&id)?;
        let cursor = cursor_for_position(&run.text, position);
        let (x, y) = run.layout.cursor_position(&cursor)?;
        Some(Point::new(x, y))
    }

    /// Resolve logical-point coordinates to a grapheme-cluster index.
    pub fn point_to_position(&self, id: TextRunId, point: Point) -> Option<usize> {
        let run = self.runs.get(&id)?;
        let cursor = run.layout.hit(point.x, point.y)?;
        Some(position_for_cursor(&run.text, cursor))
    }

    /// Return one rectangle per visual selection span, including bidi and multiline spans.
    pub fn selection_rects(&self, id: TextRunId, start: usize, end: usize) -> Option<Vec<Rect>> {
        let run = self.runs.get(&id)?;
        let (start_pos, end_pos) = (start.min(end), start.max(end));
        let start = cursor_for_position(&run.text, start_pos);
        let end = cursor_for_position(&run.text, end_pos);
        Some(
            run.layout
                .layout_runs()
                .flat_map(|line| {
                    let y = line.line_top;
                    let height = line.line_height;
                    line.highlight(start, end).map(move |(x, width)| {
                        Rect::from_min_size(Point::new(x, y), Size::new(width, height))
                    })
                })
                .collect(),
        )
    }

    pub fn caret_rect(&self, id: TextRunId, position: usize, width: f32) -> Option<Rect> {
        let run = self.runs.get(&id)?;
        let cursor = cursor_for_position(&run.text, position);
        let (x, _) = run.layout.cursor_position(&cursor)?;
        let line = run
            .layout
            .layout_runs()
            .find(|line| line.line_i == cursor.line && line.cursor_position(&cursor).is_some())?;
        Some(Rect::from_min_size(
            Point::new(x, line.line_top),
            Size::new(width.max(0.0), line.line_height),
        ))
    }

    pub fn line_metrics(&self, id: TextRunId) -> Option<Vec<TextLineMetrics>> {
        let run = self.runs.get(&id)?;
        Some(
            run.layout
                .layout_runs()
                .map(|line| {
                    let base = run
                        .text
                        .split('\n')
                        .take(line.line_i)
                        .map(|s| s.graphemes(true).count() + 1)
                        .sum::<usize>();
                    let start_byte = line
                        .glyphs
                        .iter()
                        .map(|glyph| glyph.start)
                        .min()
                        .unwrap_or(0);
                    let end_byte = line
                        .glyphs
                        .iter()
                        .map(|glyph| glyph.end)
                        .max()
                        .unwrap_or(start_byte);
                    let text_line = line.text;
                    TextLineMetrics {
                        start: base
                            + text_line
                                .get(..start_byte)
                                .unwrap_or("")
                                .graphemes(true)
                                .count(),
                        end: base
                            + text_line
                                .get(..end_byte)
                                .unwrap_or(text_line)
                                .graphemes(true)
                                .count(),
                        top: line.line_top,
                        baseline: line.line_y,
                        height: line.line_height,
                        width: line.line_w,
                    }
                })
                .collect(),
        )
    }

    pub fn measure(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> TextMetrics {
        let id = self.shape(text, style, width);
        self.runs
            .remove(&id)
            .map_or(TextMetrics::default(), |run| run.metrics)
    }

    pub fn prepare(
        &mut self,
        id: TextRunId,
        origin: Point,
        scale_factor: f32,
    ) -> Result<PreparedText, TextError> {
        let scale_factor = scale_factor.max(0.01);
        let run = Arc::clone(self.runs.get(&id).ok_or(TextError::UnknownRun(id))?);
        let mut quads = Vec::with_capacity(run.glyphs.len());
        for glyph in &run.glyphs {
            let physical_origin = Point::new(
                (origin.x + glyph.origin.x) * scale_factor,
                (origin.y + glyph.origin.y) * scale_factor,
            );
            let (key, _, _) = cosmic_text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size * scale_factor,
                (physical_origin.x, physical_origin.y),
                glyph.font_weight,
                glyph.cache_key_flags,
            );
            let entry = if let Some(entry) = self.glyphs.get_mut(&key) {
                self.stats.hits += 1;
                if entry.pinned_frame != self.frame {
                    self.clock += 1;
                    entry.last_used = self.clock;
                    entry.pinned_frame = self.frame;
                }
                *entry
            } else {
                self.clock += 1;
                let clock = self.clock;
                self.stats.misses += 1;
                let Some(image) = self.rasterizer.get_image(&mut self.fonts, key).clone() else {
                    continue;
                };
                if image.placement.width + 2 >= self.max_atlas_size
                    || image.placement.height + 2 >= self.max_atlas_size
                {
                    return Err(TextError::AtlasFull);
                }
                let coverage = glyph_coverage(image.content, &image.data);
                let entry = match self.insert_glyph(
                    &coverage,
                    image.placement.width,
                    image.placement.height,
                    image.placement.left,
                    image.placement.top,
                    clock,
                ) {
                    Ok(entry) => entry,
                    Err(TextError::AtlasFull) => {
                        if !self.pending_glyphs.iter().any(|pending| pending.key == key) {
                            self.pending_glyphs.push(PendingGlyph {
                                key,
                                pixels: coverage,
                                width: image.placement.width,
                                height: image.placement.height,
                                left: image.placement.left,
                                top: image.placement.top,
                            });
                        }
                        self.grow_pending = true;
                        self.stats.rasterized += 1;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                self.glyphs.insert(key, entry);
                self.stats.rasterized += 1;
                entry
            };
            let [x, y, width, height] = entry.rect;
            quads.push(GlyphQuad {
                origin: Point::new(
                    origin.x + glyph.origin.x + entry.left as f32 / scale_factor,
                    origin.y + glyph.origin.y - entry.top as f32 / scale_factor,
                ),
                size: Size::new(width as f32 / scale_factor, height as f32 / scale_factor),
                uv_min: [
                    x as f32 / self.atlas.size[0] as f32,
                    y as f32 / self.atlas.size[1] as f32,
                ],
                uv_max: [
                    (x + width) as f32 / self.atlas.size[0] as f32,
                    (y + height) as f32 / self.atlas.size[1] as f32,
                ],
                color: run.color,
            });
        }
        Ok(PreparedText {
            id,
            metrics: run.metrics,
            glyphs: quads,
        })
    }

    /// Advances the pinning epoch; call once before preparing a display frame.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1).max(1);
        if self.grow_pending {
            self.grow_atlas();
            let pending = std::mem::take(&mut self.pending_glyphs);
            self.grow_pending = false;
            for glyph in pending {
                self.clock += 1;
                if let Ok(entry) = self.insert_glyph(
                    &glyph.pixels,
                    glyph.width,
                    glyph.height,
                    glyph.left,
                    glyph.top,
                    self.clock,
                ) {
                    self.glyphs.insert(glyph.key, entry);
                } else {
                    self.pending_glyphs.push(glyph);
                    self.grow_pending = true;
                }
            }
        }
    }

    pub fn take_dirty_regions(&mut self) -> Vec<[u32; 4]> {
        std::mem::take(&mut self.dirty_regions)
    }

    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    pub fn stats(&self) -> GlyphCacheStats {
        let free_area = self
            .free_rects
            .iter()
            .map(|rect| u64::from(rect[2]) * u64::from(rect[3]))
            .sum::<u64>();
        let largest_free_area = self
            .free_rects
            .iter()
            .map(|rect| u64::from(rect[2]) * u64::from(rect[3]))
            .max()
            .unwrap_or(0);
        GlyphCacheStats {
            atlas_used_pixels: self
                .glyphs
                .values()
                .map(|glyph| u64::from(glyph.rect[2] * glyph.rect[3]))
                .sum(),
            free_rect_count: self.free_rects.len() as u64,
            fragmentation_per_mille: if free_area == 0 {
                0
            } else {
                (1000 * (free_area - largest_free_area) / free_area).min(1000)
            },
            ..self.stats
        }
    }

    fn insert_glyph(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        last_used: u64,
    ) -> Result<AtlasEntry, TextError> {
        if width == 0 || height == 0 {
            return Ok(AtlasEntry {
                rect: [0, 0, 0, 0],
                allocation: [0, 0, 0, 0],
                left,
                top,
                last_used,
                pinned_frame: self.frame,
            });
        }
        if width + 2 >= self.atlas.size[0] || height + 2 >= self.atlas.size[1] {
            return Err(TextError::AtlasFull);
        }
        let (x, y) = loop {
            if let Some(position) = self.allocate_rect(width + 1, height + 1) {
                break position;
            }
            let evictable = self
                .glyphs
                .iter()
                .filter(|(_, entry)| entry.pinned_frame != self.frame)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(evictable) = evictable else {
                return Err(TextError::AtlasFull);
            };
            if let Some(entry) = self.glyphs.remove(&evictable) {
                if entry.rect[2] > 0 && entry.rect[3] > 0 {
                    self.free_rects.push(entry.allocation);
                    self.coalesce_free_rects();
                }
                self.stats.evictions += 1;
            }
        };
        for row in 0..height as usize {
            let source_start = row * width as usize;
            let target_start = (y as usize + row) * self.atlas.size[0] as usize + x as usize;
            let target = &mut self.atlas.pixels[target_start..target_start + width as usize];
            for (column, value) in target.iter_mut().enumerate() {
                *value = pixels.get(source_start + column).copied().unwrap_or(0);
            }
        }
        self.dirty_regions.push([x, y, width, height]);
        Ok(AtlasEntry {
            rect: [x, y, width, height],
            allocation: [x, y, width + 1, height + 1],
            left,
            top,
            last_used,
            pinned_frame: self.frame,
        })
    }

    fn allocate_rect(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if let Some((index, _)) = self
            .free_rects
            .iter()
            .enumerate()
            .filter(|(_, rect)| rect[2] >= width && rect[3] >= height)
            .min_by_key(|(_, rect)| rect[2] * rect[3] - width * height)
        {
            let [x, y, free_width, free_height] = self.free_rects.swap_remove(index);
            if free_width > width {
                self.free_rects
                    .push([x + width, y, free_width - width, height]);
            }
            if free_height > height {
                self.free_rects
                    .push([x, y + height, free_width, free_height - height]);
            }
            return Some((x, y));
        }
        if self.shelf_x + width > self.atlas.size[0] {
            let next_shelf_y = self.shelf_y + self.shelf_height + 1;
            if next_shelf_y + height > self.atlas.size[1] {
                return None;
            }
            self.shelf_x = 1;
            self.shelf_y = next_shelf_y;
            self.shelf_height = 0;
        } else if self.shelf_y + height > self.atlas.size[1] {
            return None;
        }
        let position = (self.shelf_x, self.shelf_y);
        self.shelf_x += width + 1;
        self.shelf_height = self.shelf_height.max(height);
        Some(position)
    }

    fn coalesce_free_rects(&mut self) {
        loop {
            let mut merged = false;
            'pairs: for left in 0..self.free_rects.len() {
                for right in left + 1..self.free_rects.len() {
                    let a = self.free_rects[left];
                    let b = self.free_rects[right];
                    let union = if a[1] == b[1] && a[3] == b[3] && a[0] + a[2] == b[0] {
                        Some([a[0], a[1], a[2] + b[2], a[3]])
                    } else if a[1] == b[1] && a[3] == b[3] && b[0] + b[2] == a[0] {
                        Some([b[0], b[1], a[2] + b[2], a[3]])
                    } else if a[0] == b[0] && a[2] == b[2] && a[1] + a[3] == b[1] {
                        Some([a[0], a[1], a[2], a[3] + b[3]])
                    } else if a[0] == b[0] && a[2] == b[2] && b[1] + b[3] == a[1] {
                        Some([b[0], b[1], a[2], a[3] + b[3]])
                    } else {
                        None
                    };
                    if let Some(union) = union {
                        self.free_rects[left] = union;
                        self.free_rects.swap_remove(right);
                        merged = true;
                        break 'pairs;
                    }
                }
            }
            if !merged {
                break;
            }
        }
    }

    fn grow_atlas(&mut self) {
        let old_size = self.atlas.size[0];
        let new_size = (old_size * 2).min(self.max_atlas_size);
        if new_size == old_size {
            return;
        }
        let mut pixels = vec![0; (new_size * new_size) as usize];
        for row in 0..old_size as usize {
            let old_start = row * old_size as usize;
            let new_start = row * new_size as usize;
            pixels[new_start..new_start + old_size as usize]
                .copy_from_slice(&self.atlas.pixels[old_start..old_start + old_size as usize]);
        }
        self.atlas = GlyphAtlas {
            size: [new_size, new_size],
            pixels,
            generation: self.atlas.generation + 1,
        };
        self.free_rects
            .push([old_size, 0, new_size - old_size, old_size]);
        self.free_rects
            .push([0, old_size, new_size, new_size - old_size]);
        self.stats.atlas_capacity_pixels = u64::from(new_size) * u64::from(new_size);
        self.dirty_regions.clear();
        self.dirty_regions.push([0, 0, new_size, new_size]);
    }
}

fn cursor_for_position(text: &str, position: usize) -> Cursor {
    let mut remaining = position;
    for (line, value) in text.split('\n').enumerate() {
        let count = value.graphemes(true).count();
        if remaining <= count {
            let byte = value
                .grapheme_indices(true)
                .nth(remaining)
                .map_or(value.len(), |(i, _)| i);
            return Cursor::new(line, byte);
        }
        remaining = remaining.saturating_sub(count + 1);
    }
    let line = text.split('\n').count().saturating_sub(1);
    Cursor::new(line, text.rsplit('\n').next().unwrap_or_default().len())
}

fn position_for_cursor(text: &str, cursor: Cursor) -> usize {
    let mut position = 0;
    for (line, value) in text.split('\n').enumerate() {
        if line == cursor.line {
            let byte = cursor.index.min(value.len());
            let byte = (0..=byte)
                .rev()
                .find(|index| value.is_char_boundary(*index))
                .unwrap_or(0);
            return position + value[..byte].graphemes(true).count();
        }
        position += value.graphemes(true).count() + 1;
    }
    text.graphemes(true).count()
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn glyph_coverage(content: SwashContent, pixels: &[u8]) -> Vec<u8> {
    match content {
        SwashContent::Mask => pixels.to_vec(),
        SwashContent::SubpixelMask => pixels
            .chunks_exact(4)
            .map(|rgba| ((u16::from(rgba[0]) + u16::from(rgba[1]) + u16::from(rgba[2])) / 3) as u8)
            .collect(),
        SwashContent::Color => pixels.chunks_exact(4).map(|rgba| rgba[3]).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_vietnamese_and_measures_nonzero_text() {
        let mut text = TextSystem::new();
        let metrics = text.measure("Tiếng Việt có dấu", TextStyle::default(), None);
        assert!(metrics.size.width > 0.0);
        assert!(metrics.glyph_count > 0);
    }

    #[test]
    fn repeated_prepare_hits_raster_cache() {
        let mut text = TextSystem::new();
        let id = text.shape("Cache", TextStyle::default(), None);
        let first = text.prepare(id, Point::new(0.25, 0.5), 1.0).unwrap();
        let rasterized = text.stats().rasterized;
        let second = text.prepare(id, Point::new(0.25, 0.5), 1.0).unwrap();
        assert_eq!(first.glyphs.len(), second.glyphs.len());
        assert_eq!(text.stats().rasterized, rasterized);
        assert!(text.stats().hits > 0);
    }

    #[test]
    fn high_dpi_uses_scaled_glyph_rasterization_without_rounding_layout_origin() {
        let mut text = TextSystem::new();
        let id = text.shape("Scale", TextStyle::default(), None);
        let logical_origin = Point::new(0.25, 0.75);
        let at_one_x = text.prepare(id, logical_origin, 1.0).unwrap();
        let at_two_x = text.prepare(id, logical_origin, 2.0).unwrap();
        assert_ne!(at_one_x.glyphs[0].origin.x.fract(), 0.0);
        assert_ne!(at_two_x.glyphs[0].origin.x.fract(), 0.0);
        assert!(text.stats().rasterized > at_one_x.glyphs.len() as u64);
    }

    #[test]
    fn swash_color_and_subpixel_images_reduce_to_coverage() {
        assert_eq!(glyph_coverage(SwashContent::Mask, &[9, 10]), [9, 10]);
        assert_eq!(
            glyph_coverage(SwashContent::SubpixelMask, &[0, 90, 180, 255]),
            [90]
        );
        assert_eq!(
            glyph_coverage(SwashContent::Color, &[30, 60, 90, 120]),
            [120]
        );
    }

    #[test]
    fn text_run_identity_survives_update_and_can_be_removed() {
        let mut text = TextSystem::new();
        let id = text.shape("small", TextStyle::default(), None);
        let old_width = text.run_metrics(id).unwrap().size.width;
        text.update_run(id, "a much wider run", TextStyle::default(), None)
            .unwrap();
        assert!(text.run_metrics(id).unwrap().size.width > old_width);
        assert!(text.remove_run(id));
        assert_eq!(text.run_metrics(id), None);
    }

    #[test]
    fn logical_grapheme_positions_resolve_to_geometry_and_back_across_lines() {
        let mut text = TextSystem::new();
        let value = "Xin chào\nTrường 🇻🇳";
        let id = text.shape(value, TextStyle::default(), None);
        let position = value.graphemes(true).count() - 2;
        for position in [
            0,
            2,
            5,
            8,
            10,
            13,
            value.graphemes(true).count() - 1,
            position,
        ] {
            let point = text.position_to_point(id, position).unwrap();
            assert_eq!(text.point_to_position(id, point), Some(position));
        }
        assert!(text.caret_rect(id, position, 1.0).unwrap().height() > 0.0);
        assert!(!text.selection_rects(id, 0, position).unwrap().is_empty());
        assert!(text.line_metrics(id).unwrap().len() >= 2);
    }

    #[test]
    fn atlas_growth_is_deferred_to_frame_boundary_and_reuses_pending_glyphs() {
        let mut text = TextSystem::with_atlas_size(64);
        let run = text.shape(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            TextStyle::default(),
            None,
        );
        text.begin_frame();
        text.prepare(run, Point::ZERO, 1.0).unwrap();
        assert_eq!(text.atlas().generation(), 0);
        assert!(text.stats().rasterized > 0);
        text.begin_frame();
        assert_eq!(text.atlas().generation(), 1);
        text.prepare(run, Point::ZERO, 1.0).unwrap();
        assert!(text.stats().hits > 0);
        assert!(text.take_dirty_regions().iter().any(|rect| rect[2] == 128));
    }

    #[test]
    fn atlas_evicts_only_entries_from_prior_frames() {
        let mut text = TextSystem::with_atlas_limits(64, 64);
        let first = text.shape(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            TextStyle::default(),
            None,
        );
        let second = text.shape(
            "!@#$%^&*()_+-=[]{};':,./<>?~`\\|",
            TextStyle::default(),
            None,
        );
        text.begin_frame();
        text.prepare(first, Point::ZERO, 1.0).unwrap();
        let evictions_during_first_frame = text.stats().evictions;
        text.prepare(second, Point::ZERO, 1.0).unwrap();
        assert_eq!(text.stats().evictions, evictions_during_first_frame);
        text.begin_frame();
        text.prepare(second, Point::ZERO, 1.0).unwrap();
        assert!(text.stats().evictions > evictions_during_first_frame);
    }
}
