//! Text shaping and CPU glyph-atlas preparation for desktop UI rendering.

use std::collections::HashMap;
use std::mem::size_of;
use std::ops::Range;
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use swash::scale::image::Content as SwashContent;
use ui_core::{Color, Point, Rect, Size};
pub use ui_core::{DirtyLineRange, TextRunId};
use unicode_segmentation::UnicodeSegmentation;

const DEFAULT_ATLAS_SIZE: u32 = 1024;
const MAX_ATLAS_SIZE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Serif,
    Monospace,
    /// A host-provided or installed font family name.
    Named(&'static str),
}

impl FontFamily {
    /// Picks a platform UI family without coupling callers to font names.
    pub const fn system_ui() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Named("System Font")
        }
        #[cfg(target_os = "windows")]
        {
            Self::Named("Segoe UI")
        }
        #[cfg(target_os = "linux")]
        {
            Self::Named("Noto Sans")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Sans
        }
    }

    /// Picks a platform monospace family without coupling callers to font names.
    pub const fn system_monospace() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Named(".SF NS Mono")
        }
        #[cfg(target_os = "windows")]
        {
            Self::Named("Cascadia Mono")
        }
        #[cfg(target_os = "linux")]
        {
            Self::Named("Noto Sans Mono")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Monospace
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Regular,
    Medium,
    Bold,
    /// Requests a CSS-like numeric weight through fontdb's 1-1000 scale.
    Numeric(u16),
}

impl FontWeight {
    fn cosmic(self) -> Weight {
        match self {
            Self::Regular => Weight::NORMAL,
            Self::Medium => Weight::MEDIUM,
            Self::Bold => Weight::BOLD,
            Self::Numeric(weight) => Weight(weight),
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
pub struct LayoutRevision(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextDocumentLayoutStats {
    pub lines_invalidated: u64,
    pub lines_shaped: u64,
    pub cache_hits: u64,
    pub lines_materialized: u64,
    pub lines_evicted: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextLayoutRevisions {
    pub document: LayoutRevision,
    pub style: LayoutRevision,
    pub constraints: LayoutRevision,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDocumentLine {
    pub logical_index: usize,
    pub revision: LayoutRevision,
    pub run: TextRunId,
    pub grapheme_start: usize,
    pub top: f32,
    pub height: f32,
    pub metrics: TextMetrics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShapeKey {
    family: FontFamily,
    weight: FontWeight,
    size_px: u32,
    line_height_px: u32,
    width: Option<u32>,
}

impl ShapeKey {
    fn new(style: TextStyle, width: Option<f32>) -> Self {
        Self {
            family: style.family,
            weight: style.weight,
            size_px: style.size_px.to_bits(),
            line_height_px: style.line_height_px.to_bits(),
            width: width.map(f32::to_bits),
        }
    }
}

#[derive(Clone, Debug)]
struct CachedDocumentLine {
    revision: LayoutRevision,
    run: Option<TextRunId>,
    shape_key: Option<ShapeKey>,
    grapheme_len: usize,
    grapheme_start: usize,
    top: f32,
    height: f32,
    metrics: TextMetrics,
    color: Option<Color>,
    last_used: u64,
}

/// Per-logical-line shaping cache. Text content remains owned by TextSystem's shaped runs.
#[derive(Debug)]
pub struct TextDocumentLayout {
    lines: Vec<CachedDocumentLine>,
    max_cached_lines: Option<usize>,
    clock: u64,
    stats: TextDocumentLayoutStats,
    revisions: TextLayoutRevisions,
    last_shape_key: Option<ShapeKey>,
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
    /// Logical bitmap origin. Subpixel positioning is already baked into coverage.
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
    baseline: f32,
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
    shape_key: ShapeKey,
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
    shape_calls: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    AtlasFull,
    UnknownRun(TextRunId),
    InvalidDirtyLineRange,
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtlasFull => f.write_str("glyph atlas is full; create a larger text atlas"),
            Self::UnknownRun(id) => write!(f, "text run {} does not exist", id.0),
            Self::InvalidDirtyLineRange => {
                f.write_str("dirty line range does not match the document")
            }
        }
    }
}

impl std::error::Error for TextError {}

impl TextSystem {
    pub fn new() -> Self {
        Self::with_atlas_size(DEFAULT_ATLAS_SIZE)
    }

    /// Load host font bytes before shaping, retaining system Unicode fallbacks.
    pub fn with_fonts(font_data: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let mut text = Self::new();
        let (locale, mut db) = text.fonts.into_locale_and_db();
        for bytes in font_data {
            db.load_font_data(bytes);
        }
        text.fonts = FontSystem::new_with_locale_and_db(locale, db);
        text
    }

    pub fn with_atlas_size(size: u32) -> Self {
        Self::with_atlas_limits(size, MAX_ATLAS_SIZE)
    }

    pub fn with_atlas_limits(size: u32, max_size: u32) -> Self {
        let size = size.max(64);
        let max_size = max_size.max(size);
        // Preserve fontdb's system generic-family mapping; cosmic's defaults can
        // name an absent font and send even ASCII spaces through emoji fallback.
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // A named platform face keeps Sans/Monospace stable as weight changes.
        if let FontFamily::Named(name) = FontFamily::system_ui() {
            db.set_sans_serif_family(name);
        }
        if let FontFamily::Named(name) = FontFamily::system_monospace() {
            db.set_monospace_family(name);
        }
        let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
        let fonts = FontSystem::new_with_locale_and_db(locale, db);
        Self {
            fonts,
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
            shape_calls: 0,
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
        let shape_key = ShapeKey::new(style, width);
        let Some(existing) = self.runs.get(&id) else {
            return Err(TextError::UnknownRun(id));
        };
        if existing.text == text && existing.shape_key == shape_key {
            if existing.color != style.color {
                Arc::make_mut(self.runs.get_mut(&id).expect("run checked above")).color =
                    style.color;
            }
            return Ok(());
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
        self.shape_calls += 1;
        let metrics = Metrics::new(style.size_px, style.line_height_px.max(style.size_px));
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        buffer.set_size(width, None);
        let family = match style.family {
            FontFamily::Sans => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Named(name) => Family::Name(name),
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
                let y = run.line_y + glyph.y - glyph.font_size * glyph.y_offset;
                glyphs.push(GlyphPosition {
                    origin: Point::new(x, y),
                    baseline: run.line_y,
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
            shape_key: ShapeKey::new(style, width),
        }
    }

    pub fn shape_call_count(&self) -> u64 {
        self.shape_calls
    }

    fn estimated_run_bytes(&self, id: TextRunId) -> usize {
        self.runs.get(&id).map_or(0, |run| {
            size_of::<ShapedRun>() + run.text.len() + run.glyphs.len() * size_of::<GlyphPosition>()
        })
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
                // Snap the line baseline, preserving shaped mark offsets and X advances.
                ((origin.y + glyph.baseline) * scale_factor).round()
                    + (glyph.origin.y - glyph.baseline) * scale_factor,
            );
            let (key, physical_x, physical_y) = cosmic_text::CacheKey::new(
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
                // Swash already applies the CacheKey's fractional bin.
                origin: Point::new(
                    (physical_x + entry.left) as f32 / scale_factor,
                    (physical_y - entry.top) as f32 / scale_factor,
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
            fragmentation_per_mille: (1000 * (free_area - largest_free_area))
                .checked_div(free_area)
                .unwrap_or(0)
                .min(1000),
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
            if let Some(position) = self.allocate_rect(width + 2, height + 2) {
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
        let allocation = [x, y, width + 2, height + 2];
        // A reused allocation may contain old glyphs. Clear a full transparent
        // gutter on all sides so Linear sampling cannot bleed into its neighbour.
        for row in y..y + height + 2 {
            let start = (row * self.atlas.size[0] + x) as usize;
            self.atlas.pixels[start..start + width as usize + 2].fill(0);
        }
        let (x, y) = (x + 1, y + 1);
        for row in 0..height as usize {
            let source_start = row * width as usize;
            let target_start = (y as usize + row) * self.atlas.size[0] as usize + x as usize;
            let target = &mut self.atlas.pixels[target_start..target_start + width as usize];
            for (column, value) in target.iter_mut().enumerate() {
                *value = pixels.get(source_start + column).copied().unwrap_or(0);
            }
        }
        self.dirty_regions.push(allocation);
        Ok(AtlasEntry {
            rect: [x, y, width, height],
            allocation,
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

impl TextDocumentLayout {
    pub fn new(line_grapheme_lengths: impl IntoIterator<Item = usize>, line_height: f32) -> Self {
        let line_height = line_height.max(1.0);
        let lines = line_grapheme_lengths
            .into_iter()
            .map(|grapheme_len| CachedDocumentLine {
                run: None,
                revision: LayoutRevision::default(),
                shape_key: None,
                grapheme_len,
                grapheme_start: 0,
                top: 0.0,
                height: line_height,
                metrics: TextMetrics {
                    size: Size::new(0.0, line_height),
                    ..TextMetrics::default()
                },
                color: None,
                last_used: 0,
            })
            .collect::<Vec<_>>();
        let mut layout = Self {
            lines,
            max_cached_lines: None,
            clock: 0,
            stats: TextDocumentLayoutStats::default(),
            revisions: TextLayoutRevisions::default(),
            last_shape_key: None,
        };
        layout.rebuild_positions_from(0);
        layout
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn visible_line_range(&self, top: f32, height: f32) -> Range<usize> {
        if self.lines.is_empty() || height <= 0.0 {
            return 0..0;
        }
        let start = self.line_at_y(top).unwrap_or(0);
        let end = self
            .line_at_y(top + height)
            .map_or(self.lines.len(), |index| index + 1);
        start..end
    }

    pub fn revisions(&self) -> TextLayoutRevisions {
        self.revisions
    }

    pub fn stats(&self) -> TextDocumentLayoutStats {
        self.stats
    }

    pub fn take_stats(&mut self) -> TextDocumentLayoutStats {
        std::mem::take(&mut self.stats)
    }

    pub fn set_max_cached_lines(&mut self, maximum: Option<usize>) {
        self.max_cached_lines = maximum.map(|limit| limit.max(1));
    }

    /// Approximation excludes allocator metadata and cosmic-text's private buffer storage.
    pub fn estimated_cache_bytes(&self, text: &TextSystem) -> usize {
        self.lines.len() * size_of::<CachedDocumentLine>()
            + self
                .lines
                .iter()
                .filter_map(|line| line.run)
                .map(|run| text.estimated_run_bytes(run))
                .sum::<usize>()
    }

    pub fn apply_edit(
        &mut self,
        text: &mut TextSystem,
        dirty: DirtyLineRange,
        inserted_grapheme_lengths: &[usize],
        line_height: f32,
    ) -> Result<(), TextError> {
        if dirty.start > self.lines.len()
            || dirty.removed > self.lines.len() - dirty.start
            || dirty.inserted != inserted_grapheme_lengths.len()
        {
            return Err(TextError::InvalidDirtyLineRange);
        }
        let start = dirty.start;
        let end = start + dirty.removed;
        for line in &self.lines[start..end] {
            if let Some(run) = line.run {
                text.remove_run(run);
            }
        }
        let estimate = line_height.max(1.0);
        let next_document_revision = LayoutRevision(self.revisions.document.0.wrapping_add(1));
        let inserted = inserted_grapheme_lengths
            .iter()
            .map(|grapheme_len| CachedDocumentLine {
                run: None,
                revision: next_document_revision,
                shape_key: None,
                grapheme_len: *grapheme_len,
                grapheme_start: 0,
                top: 0.0,
                height: estimate,
                metrics: TextMetrics {
                    size: Size::new(0.0, estimate),
                    ..TextMetrics::default()
                },
                color: None,
                last_used: 0,
            })
            .collect::<Vec<_>>();
        self.lines.splice(start..end, inserted);
        self.stats.lines_invalidated += dirty.removed.max(inserted_grapheme_lengths.len()) as u64;
        self.revisions.document = next_document_revision;
        self.rebuild_positions_from(start);
        Ok(())
    }

    pub fn layout_visible_lines(
        &mut self,
        text: &mut TextSystem,
        visible: Range<usize>,
        overscan: usize,
        style: TextStyle,
        width: Option<f32>,
        mut line_text: impl FnMut(usize) -> String,
    ) -> Result<Vec<TextDocumentLine>, TextError> {
        let start = visible.start.min(self.lines.len()).saturating_sub(overscan);
        let end = visible
            .end
            .min(self.lines.len())
            .saturating_add(overscan)
            .min(self.lines.len());
        let shape_key = ShapeKey::new(style, width);
        let old_shape_key = self.last_shape_key;
        if let Some(previous) = old_shape_key {
            if previous.width != shape_key.width {
                self.revisions.constraints.0 = self.revisions.constraints.0.wrapping_add(1);
            } else if previous != shape_key {
                self.revisions.style.0 = self.revisions.style.0.wrapping_add(1);
            }
            if previous != shape_key {
                let estimate = style.line_height_px.max(1.0);
                for line in &mut self.lines {
                    line.height = estimate;
                    line.metrics.size.height = estimate;
                }
                self.rebuild_positions_from(0);
            }
        }
        self.last_shape_key = Some(shape_key);
        let mut position_dirty_from = None;
        let mut output = Vec::with_capacity(end.saturating_sub(start));
        for index in start..end {
            let line = &mut self.lines[index];
            let reusable = line.run.is_some() && line.shape_key == Some(shape_key);
            if line.run.is_some() && !reusable {
                self.stats.lines_invalidated += 1;
            }
            self.clock = self.clock.wrapping_add(1).max(1);
            line.last_used = self.clock;
            self.stats.lines_materialized += 1;
            if reusable {
                let run = line.run.expect("reusable line has a shaped run");
                if line.color != Some(style.color) {
                    text.set_run_color(run, style.color)?;
                    line.color = Some(style.color);
                }
                self.stats.cache_hits += 1;
            } else {
                let content = line_text(index);
                let grapheme_len = content.graphemes(true).count();
                if line.grapheme_len != grapheme_len {
                    line.grapheme_len = grapheme_len;
                    position_dirty_from =
                        Some(position_dirty_from.map_or(index, |old: usize| old.min(index)));
                }
                if let Some(run) = line.run {
                    let before = text.shape_call_count();
                    text.update_run(run, &content, style, width)?;
                    if text.shape_call_count() > before {
                        self.stats.lines_shaped += 1;
                        position_dirty_from =
                            Some(position_dirty_from.map_or(index, |old: usize| old.min(index)));
                    }
                    line.color = Some(style.color);
                } else {
                    let run = text.shape(&content, style, width);
                    line.run = Some(run);
                    self.stats.lines_shaped += 1;
                    position_dirty_from =
                        Some(position_dirty_from.map_or(index, |old: usize| old.min(index)));
                    line.color = Some(style.color);
                }
                let run = line.run.expect("line was shaped above");
                line.metrics = text.run_metrics(run).unwrap_or_default();
                line.height = line.metrics.size.height.max(style.line_height_px.max(1.0));
                line.shape_key = Some(shape_key);
            }
            output.push(TextDocumentLine {
                logical_index: index,
                revision: line.revision,
                run: line.run.expect("shaped or updated above"),
                grapheme_start: line.grapheme_start,
                top: line.top,
                height: line.height,
                metrics: line.metrics,
            });
        }
        if let Some(from) = position_dirty_from {
            self.rebuild_positions_from(from);
            for line in &mut output {
                line.top = self.lines[line.logical_index].top;
                line.height = self.lines[line.logical_index].height;
            }
        }
        self.evict_outside(text, start..end);
        Ok(output)
    }

    pub fn hit_test(&self, text: &TextSystem, point: Point) -> Option<usize> {
        let index = self.line_at_y(point.y)?;
        let line = self.lines.get(index)?;
        let run = line.run?;
        let local = text.point_to_position(run, Point::new(point.x, point.y - line.top))?;
        Some(self.grapheme_start(index) + local.min(line.grapheme_len))
    }

    pub fn position_to_point(&self, text: &TextSystem, position: usize) -> Option<Point> {
        let (index, local) = self.line_at_position(position)?;
        let line = &self.lines[index];
        let run = line.run?;
        let point = text.position_to_point(run, local)?;
        Some(Point::new(point.x, point.y + line.top))
    }

    pub fn caret_rect(&self, text: &TextSystem, position: usize, width: f32) -> Option<Rect> {
        let (index, local) = self.line_at_position(position)?;
        let line = &self.lines[index];
        let mut rect = text.caret_rect(line.run?, local, width)?;
        rect.min.y += line.top;
        rect.max.y += line.top;
        Some(rect)
    }

    pub fn visual_line_boundary(
        &self,
        text: &TextSystem,
        position: usize,
        end: bool,
    ) -> Option<usize> {
        let (index, local) = self.line_at_position(position)?;
        let line = &self.lines[index];
        let run = line.run?;
        let caret = text.position_to_point(run, local)?;
        let visual = text
            .line_metrics(run)?
            .into_iter()
            .find(|metric| caret.y >= metric.top && caret.y < metric.top + metric.height)
            .or_else(|| text.line_metrics(run)?.last().copied())?;
        Some(line.grapheme_start + if end { visual.end } else { visual.start })
    }

    pub fn selection_rects(&self, text: &TextSystem, start: usize, end: usize) -> Vec<Rect> {
        self.selection_rects_in_range(text, start, end, 0..self.lines.len())
    }

    pub fn selection_rects_in_range(
        &self,
        text: &TextSystem,
        start: usize,
        end: usize,
        line_range: Range<usize>,
    ) -> Vec<Rect> {
        let start = start.min(end);
        let end = start.max(end);
        let line_start = line_range.start.min(self.lines.len());
        let line_end = line_range.end.min(self.lines.len()).max(line_start);
        self.lines[line_start..line_end]
            .iter()
            .enumerate()
            .filter_map(|(offset, line)| {
                let index = line_start + offset;
                let run = line.run?;
                let line_start = self.grapheme_start(index);
                let local_start = start.saturating_sub(line_start).min(line.grapheme_len);
                let local_end = end.saturating_sub(line_start).min(line.grapheme_len);
                if local_start >= local_end {
                    return None;
                }
                Some(
                    text.selection_rects(run, local_start, local_end)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|mut rect| {
                            rect.min.y += line.top;
                            rect.max.y += line.top;
                            rect
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect()
    }

    fn line_at_y(&self, y: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let index = self
            .lines
            .partition_point(|line| line.top + line.height < y);
        Some(index.min(self.lines.len() - 1))
    }

    fn line_at_position(&self, position: usize) -> Option<(usize, usize)> {
        if self.lines.is_empty() {
            return None;
        }
        let index = self
            .lines
            .partition_point(|line| line.grapheme_start <= position)
            .saturating_sub(1);
        Some((
            index,
            position
                .saturating_sub(self.lines[index].grapheme_start)
                .min(self.lines[index].grapheme_len),
        ))
    }

    fn grapheme_start(&self, index: usize) -> usize {
        self.lines.get(index).map_or(0, |line| line.grapheme_start)
    }

    fn rebuild_positions_from(&mut self, from: usize) {
        if self.lines.is_empty() {
            return;
        }
        let from = from.min(self.lines.len() - 1);
        let mut top = if from == 0 {
            0.0
        } else {
            let previous = &self.lines[from - 1];
            previous.top + previous.height
        };
        let mut grapheme_start = if from == 0 {
            0
        } else {
            let previous = &self.lines[from - 1];
            previous.grapheme_start + previous.grapheme_len + 1
        };
        for line in &mut self.lines[from..] {
            line.top = top;
            line.grapheme_start = grapheme_start;
            top += line.height;
            grapheme_start += line.grapheme_len + 1;
        }
    }

    fn evict_outside(&mut self, text: &mut TextSystem, active: Range<usize>) {
        let Some(maximum) = self.max_cached_lines else {
            return;
        };
        let mut cached = self.lines.iter().filter(|line| line.run.is_some()).count();
        while cached > maximum {
            let candidate = self
                .lines
                .iter()
                .enumerate()
                .filter(|(index, line)| line.run.is_some() && !active.contains(index))
                .max_by_key(|(index, line)| {
                    (active.start.abs_diff(*index), u64::MAX - line.last_used)
                })
                .map(|(index, _)| index);
            let Some(index) = candidate else { break };
            if let Some(run) = self.lines[index].run.take() {
                text.remove_run(run);
                self.lines[index].shape_key = None;
                self.stats.lines_evicted += 1;
                cached -= 1;
            }
        }
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
    fn update_run_reuses_unchanged_shape_and_color_is_paint_only() {
        let mut text = TextSystem::new();
        let id = text.shape("stable", TextStyle::default(), None);
        let before = text.shape_call_count();
        text.update_run(id, "stable", TextStyle::default(), None)
            .unwrap();
        assert_eq!(text.shape_call_count(), before);
        let recolored = TextStyle {
            color: Color::from_srgba8(ui_core::Srgb8 {
                r: 200,
                g: 30,
                b: 60,
                a: 255,
            }),
            ..TextStyle::default()
        };
        text.update_run(id, "stable", recolored, None).unwrap();
        assert_eq!(text.shape_call_count(), before);
        assert!(
            text.prepare(id, Point::ZERO, 1.0)
                .unwrap()
                .glyphs
                .iter()
                .all(|g| g.color == recolored.color)
        );
    }

    #[test]
    fn high_dpi_aligns_bitmaps_without_rounding_shaped_layout() {
        let mut text = TextSystem::new();
        let id = text.shape("Scale", TextStyle::default(), None);
        let layout_before = text.position_to_point(id, 2).unwrap();
        let logical_origin = Point::new(0.25, 0.75);
        let at_one_x = text.prepare(id, logical_origin, 1.0).unwrap();
        let at_two_x = text.prepare(id, logical_origin, 2.0).unwrap();
        assert_eq!(at_one_x.glyphs[0].origin.x.fract(), 0.0);
        assert_eq!((at_two_x.glyphs[0].origin.x * 2.0).fract(), 0.0);
        assert!(text.stats().rasterized > at_one_x.glyphs.len() as u64);
        assert_eq!(text.position_to_point(id, 2), Some(layout_before));
    }

    #[test]
    fn welcome_ascii_uses_the_primary_sans_font_including_spaces() {
        let mut text = TextSystem::new();
        let primary = text
            .fonts
            .db()
            .query(&fontdb::Query {
                families: &[fontdb::Family::SansSerif],
                weight: fontdb::Weight::MEDIUM,
                ..fontdb::Query::default()
            })
            .expect("a system sans font must resolve");
        for value in [
            "DB Pro",
            "Open in editor",
            "New connection",
            "Orders by status",
        ] {
            let id = text.shape(
                value,
                TextStyle {
                    weight: FontWeight::Medium,
                    ..TextStyle::default()
                },
                None,
            );
            assert!(
                text.runs[&id]
                    .glyphs
                    .iter()
                    .all(|glyph| glyph.font_id == primary),
                "fallback in {value:?}"
            );
        }
    }

    #[test]
    fn host_font_bytes_can_be_selected_by_family_name() {
        let system = TextSystem::new();
        let id = system
            .fonts
            .db()
            .query(&fontdb::Query {
                families: &[fontdb::Family::SansSerif],
                ..Default::default()
            })
            .unwrap();
        let family = system.fonts.db().face(id).unwrap().families[0].0.clone();
        let bytes = system
            .fonts
            .db()
            .with_face_data(id, |bytes, _| bytes.to_vec())
            .unwrap();
        let mut text = TextSystem::with_fonts([bytes]);
        let system_faces = text
            .fonts
            .db()
            .faces()
            .filter(|face| !matches!(face.source, fontdb::Source::Binary(_)))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        for face in system_faces {
            text.fonts.db_mut().remove_face(face);
        }
        let family = Box::leak(family.into_boxed_str());
        let run = text.shape(
            "DB Pro",
            TextStyle {
                family: FontFamily::Named(family),
                ..Default::default()
            },
            None,
        );
        assert!(text.runs[&run].glyphs.iter().all(|glyph| matches!(
            text.fonts.db().face(glyph.font_id).unwrap().source,
            fontdb::Source::Binary(_)
        )));
        assert!(
            !text
                .prepare(run, Point::ZERO, 1.0)
                .unwrap()
                .glyphs
                .is_empty()
        );
    }

    #[test]
    fn reused_atlas_allocation_has_a_transparent_gutter_on_all_sides() {
        let mut text = TextSystem::with_atlas_size(64);
        text.atlas.pixels.fill(255);
        text.free_rects.push([10, 10, 12, 12]);
        let entry = text.insert_glyph(&[120; 12], 4, 3, 0, 3, 1).unwrap();
        assert_eq!(entry.rect, [11, 11, 4, 3]);
        assert_eq!(text.take_dirty_regions(), [entry.allocation]);
        for y in 10..15 {
            for x in 10..16 {
                let expected = if (11..15).contains(&x) && (11..14).contains(&y) {
                    120
                } else {
                    0
                };
                assert_eq!(text.atlas.pixels[(y * 64 + x) as usize], expected);
            }
        }
    }

    #[test]
    #[ignore = "prints system-dependent font and physical placement diagnostics"]
    fn text_quality_diagnostics() {
        let mut text = TextSystem::new();
        for family in [FontFamily::Sans, FontFamily::Monospace] {
            let id = text.shape(
                "DB Pro Open in editor New connection Orders by status",
                TextStyle {
                    family,
                    ..TextStyle::default()
                },
                None,
            );
            let run = Arc::clone(&text.runs[&id]);
            let mut fonts = run
                .glyphs
                .iter()
                .map(|glyph| glyph.font_id)
                .collect::<Vec<_>>();
            fonts.sort();
            fonts.dedup();
            for font in fonts {
                let face = text.fonts.db().face(font).unwrap();
                let source = match &face.source {
                    fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                        path.display().to_string()
                    }
                    fontdb::Source::Binary(_) => "memory".to_owned(),
                };
                eprintln!(
                    "[text-quality] family={family:?} actual={:?} file={source}",
                    face.families
                );
            }
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let origin = Point::new(20.37, 40.63);
                let prepared = text.prepare(id, origin, scale).unwrap();
                let glyph = &run.glyphs[0];
                let quad = prepared.glyphs[0];
                eprintln!(
                    "[text-quality] scale={scale} font_logical={} raster_physical={} glyph_logical={:?} glyph_physical={:?} baseline_physical={} quad_physical={:?} quad_logical={:?} size_logical={:?} framebuffer={}x{}",
                    glyph.font_size,
                    glyph.font_size * scale,
                    Point::new(origin.x + glyph.origin.x, origin.y + glyph.origin.y),
                    Point::new(
                        (origin.x + glyph.origin.x) * scale,
                        (origin.y + glyph.origin.y) * scale
                    ),
                    ((origin.y + glyph.baseline) * scale).round(),
                    Point::new(quad.origin.x * scale, quad.origin.y * scale),
                    quad.origin,
                    quad.size,
                    1280.0 * scale,
                    800.0 * scale
                );
            }
        }
    }

    #[test]
    #[ignore = "prints resolved faces for font family and numeric weight audit"]
    fn font_family_weight_diagnostics() {
        let mut text = TextSystem::new();
        let font_directory =
            std::env::var_os("RS_UI_TYPOGRAPHY_FONT_DIR").map(std::path::PathBuf::from);
        if let Some(directory) = &font_directory {
            for file in ["Inter.ttf", "NotoSans.ttf"] {
                let path = directory.join(file);
                if path.exists() {
                    text.fonts
                        .db_mut()
                        .load_font_data(std::fs::read(&path).expect("read audit font"));
                }
            }
        }

        let sample =
            "Sample E-Commerce (SQLite) main Tables Views Triggers Xe Lạc Hồng (PostgreSQL)";
        let families = [
            FontFamily::Sans,
            FontFamily::Named("Inter"),
            FontFamily::Named("Noto Sans"),
            FontFamily::Named("System Font"),
            FontFamily::Monospace,
            FontFamily::system_monospace(),
        ];
        let weights = [
            FontWeight::Regular,
            FontWeight::Numeric(450),
            FontWeight::Medium,
            FontWeight::Numeric(550),
            FontWeight::Numeric(600),
        ];

        for family in families {
            for weight in weights {
                let id = text.shape(
                    sample,
                    TextStyle {
                        family,
                        weight,
                        size_px: 13.0,
                        line_height_px: 18.0,
                        ..TextStyle::default()
                    },
                    None,
                );
                let run = &text.runs[&id];
                let mut used = Vec::new();
                for glyph in &run.glyphs {
                    if !used.contains(&glyph.font_id) {
                        used.push(glyph.font_id);
                    }
                }
                let chain = used
                    .iter()
                    .filter_map(|font_id| {
                        let face = text.fonts.db().face(*font_id)?;
                        let path = match &face.source {
                            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                                path.display().to_string()
                            }
                            fontdb::Source::Binary(_) => {
                                let filename = face.families.first().map_or("font", |(name, _)| {
                                    if name == "Inter" {
                                        "Inter.ttf"
                                    } else if name == "Noto Sans" {
                                        "NotoSans.ttf"
                                    } else {
                                        "font"
                                    }
                                });
                                font_directory.as_ref().map_or_else(
                                    || "memory".to_owned(),
                                    |directory| directory.join(filename).display().to_string(),
                                )
                            }
                        };
                        let glyph_weight = run
                            .glyphs
                            .iter()
                            .find(|glyph| glyph.font_id == *font_id)
                            .map_or(0, |glyph| glyph.font_weight.0);
                        Some(format!(
                            "{} face_style={:?} face_weight={} glyph_weight={} file={}",
                            face.families.first().map_or("<unnamed>", |(name, _)| name),
                            face.style,
                            face.weight.0,
                            glyph_weight,
                            path
                        ))
                    })
                    .collect::<Vec<_>>();
                let requested_weight = match weight {
                    FontWeight::Regular => 400,
                    FontWeight::Medium => 500,
                    FontWeight::Bold => 700,
                    FontWeight::Numeric(value) => value,
                };
                eprintln!(
                    "[font-audit] requested_family={family:?} requested_weight={requested_weight} resolved_chain=[{}]",
                    chain.join(" -> ")
                );
                text.remove_run(id);
            }
        }
    }

    #[test]
    #[ignore = "writes 1x, 1.5x and 2x CPU previews for the small-font family matrix"]
    fn typography_matrix_artifacts() {
        let font_dir = std::path::PathBuf::from(
            std::env::var_os("RS_UI_TYPOGRAPHY_FONT_DIR").expect("set audit font directory"),
        );
        let output_dir = std::path::PathBuf::from(
            std::env::var_os("RS_UI_TYPOGRAPHY_OUTPUT_DIR")
                .unwrap_or_else(|| "target/typography-matrix".into()),
        );
        std::fs::create_dir_all(&output_dir).expect("create output directory");

        let columns = [
            (
                "Current / Arial",
                FontFamily::Named("Arial"),
                FontWeight::Regular,
            ),
            ("Inter 400", FontFamily::Named("Inter"), FontWeight::Regular),
            ("Inter 500", FontFamily::Named("Inter"), FontWeight::Medium),
            (
                "Noto Sans 400",
                FontFamily::Named("Noto Sans"),
                FontWeight::Regular,
            ),
            (
                "Noto Sans 500",
                FontFamily::Named("Noto Sans"),
                FontWeight::Medium,
            ),
            (
                "System UI 400",
                FontFamily::Named("System Font"),
                FontWeight::Regular,
            ),
            (
                "System UI 500",
                FontFamily::Named("System Font"),
                FontWeight::Medium,
            ),
        ];
        let scale_cases = [("1x", 1.0f32), ("1_5x", 1.5), ("2x", 2.0)];

        for (scale_name, scale) in scale_cases {
            let mut canvas = TypographyCanvas::new(2350, 500, scale);
            let mut performance = Vec::new();
            for (column_index, (label, family, weight)) in columns.iter().copied().enumerate() {
                let mut text = TextSystem::new();
                for file in ["Inter.ttf", "NotoSans.ttf"] {
                    let path = font_dir.join(file);
                    text.fonts
                        .db_mut()
                        .load_font_data(std::fs::read(&path).expect("read comparison font"));
                }
                let required_families = ["Inter", "Noto Sans"];
                for required in required_families {
                    assert!(
                        text.fonts
                            .db()
                            .faces()
                            .any(|face| { face.families.iter().any(|(name, _)| name == required) }),
                        "comparison font {required} was not loaded"
                    );
                }

                let x = 80.0 + column_index as f32 * 322.0;
                draw_audit_label(
                    &mut canvas,
                    &mut text,
                    label,
                    TextStyle {
                        family,
                        weight: FontWeight::Medium,
                        size_px: 13.0,
                        line_height_px: 18.0,
                        color: Color::from_srgba8(ui_core::Srgb8 {
                            r: 55,
                            g: 65,
                            b: 81,
                            a: 255,
                        }),
                    },
                    x,
                    6.0,
                    scale,
                );

                for (row, size) in [12.0, 13.0, 14.0].into_iter().enumerate() {
                    draw_tree_cell(
                        &mut canvas,
                        &mut text,
                        family,
                        weight,
                        size,
                        x,
                        30.0 + row as f32 * 152.0,
                        scale,
                    );
                }
                let first_prepare = canvas.take_prepare_time();
                let before_repeat = text.stats();
                text.begin_frame();
                for (row, size) in [12.0, 13.0, 14.0].into_iter().enumerate() {
                    draw_tree_cell(
                        &mut canvas,
                        &mut text,
                        family,
                        weight,
                        size,
                        x,
                        30.0 + row as f32 * 152.0,
                        scale,
                    );
                }
                let repeat_prepare = canvas.take_prepare_time();
                let stats = text.stats();
                let repeat_hits = stats.hits - before_repeat.hits;
                let repeat_misses = stats.misses - before_repeat.misses;
                let repeat_requests = repeat_hits + repeat_misses;
                let repeat_hit_rate = if repeat_requests == 0 {
                    0.0
                } else {
                    repeat_hits as f64 / repeat_requests as f64 * 100.0
                };
                let requests = stats.hits + stats.misses;
                let hit_rate = if requests == 0 {
                    0.0
                } else {
                    stats.hits as f64 / requests as f64 * 100.0
                };
                performance.push(format!(
                    "{label}: first_prepare={:.3}ms repeat_prepare={:.3}ms cold_rasterized={} total_hits={} total_misses={} total_hit_rate={hit_rate:.1}% warm_hits={repeat_hits} warm_misses={repeat_misses} warm_hit_rate={repeat_hit_rate:.1}% rasterized={} evictions={} atlas={}x{} generation={} used_pixels={}",
                    first_prepare.as_secs_f64() * 1000.0,
                    repeat_prepare.as_secs_f64() * 1000.0,
                    before_repeat.rasterized,
                    stats.hits,
                    stats.misses,
                    stats.rasterized,
                    stats.evictions,
                    text.atlas().size()[0],
                    text.atlas().size()[1],
                    text.atlas().generation(),
                    stats.atlas_used_pixels
                ));
            }
            let (width, height) = (canvas.width, canvas.height);
            let pixels = canvas.into_rgb();
            let path = output_dir.join(format!("sidebar-typography-{scale_name}.ppm"));
            write_ppm(&path, width, height, &pixels);
            eprintln!(
                "[typography-matrix] scale={scale_name} file={}",
                path.display()
            );
            for line in performance {
                eprintln!("[typography-perf] scale={scale_name} {line}");
            }

            let mut selected_canvas = TypographyCanvas::new(1040, 180, scale);
            let selected_families = [
                ("Current / Arial", FontFamily::Named("Arial")),
                ("Inter", FontFamily::Named("Inter")),
                ("Noto Sans", FontFamily::Named("Noto Sans")),
                ("System UI", FontFamily::Named("System Font")),
            ];
            for (column, weight) in [500u16, 550, 600].into_iter().enumerate() {
                draw_audit_label(
                    &mut selected_canvas,
                    &mut TextSystem::new(),
                    &format!("Selected {weight}"),
                    TextStyle {
                        size_px: 12.0,
                        line_height_px: 16.0,
                        color: Color::from_srgba8(ui_core::Srgb8 {
                            r: 55,
                            g: 65,
                            b: 81,
                            a: 255,
                        }),
                        ..TextStyle::default()
                    },
                    260.0 + column as f32 * 250.0,
                    6.0,
                    scale,
                );
            }
            for (row, (label, family)) in selected_families.into_iter().enumerate() {
                let mut text = TextSystem::new();
                for file in ["Inter.ttf", "NotoSans.ttf"] {
                    text.fonts
                        .db_mut()
                        .load_font_data(std::fs::read(font_dir.join(file)).expect("read font"));
                }
                let y = 30.0 + row as f32 * 36.0;
                draw_audit_label(
                    &mut selected_canvas,
                    &mut text,
                    label,
                    TextStyle {
                        family,
                        size_px: 12.0,
                        line_height_px: 16.0,
                        color: Color::from_srgba8(ui_core::Srgb8 {
                            r: 55,
                            g: 65,
                            b: 81,
                            a: 255,
                        }),
                        ..TextStyle::default()
                    },
                    16.0,
                    y + 3.0,
                    scale,
                );
                for (column, weight) in [500u16, 550, 600].into_iter().enumerate() {
                    let x = 260.0 + column as f32 * 250.0;
                    selected_canvas.fill_rect(x, y, 220.0, 25.0, [220, 239, 255]);
                    let style = TextStyle {
                        family,
                        weight: FontWeight::Numeric(weight),
                        size_px: 13.0,
                        line_height_px: 18.0,
                        color: Color::from_srgba8(ui_core::Srgb8 {
                            r: 0,
                            g: 100,
                            b: 179,
                            a: 255,
                        }),
                    };
                    draw_audit_label(
                        &mut selected_canvas,
                        &mut text,
                        "Views",
                        style,
                        x + 8.0,
                        y + 3.0,
                        scale,
                    );
                    draw_right_count(
                        &mut selected_canvas,
                        &mut text,
                        "1",
                        style,
                        x,
                        y + 3.0,
                        220.0,
                        scale,
                    );
                }
            }
            let (width, height) = (selected_canvas.width, selected_canvas.height);
            let pixels = selected_canvas.into_rgb();
            let path = output_dir.join(format!("selected-weight-{scale_name}.ppm"));
            write_ppm(&path, width, height, &pixels);
            eprintln!("[typography-matrix] selected_weights={}", path.display());
        }
    }

    struct TypographyCanvas {
        width: u32,
        height: u32,
        scale: f32,
        pixels: Vec<u8>,
        prepare_time: std::time::Duration,
    }

    impl TypographyCanvas {
        fn new(width: u32, height: u32, scale: f32) -> Self {
            let physical_width = (width as f32 * scale).round() as u32;
            let physical_height = (height as f32 * scale).round() as u32;
            Self {
                width: physical_width,
                height: physical_height,
                scale,
                pixels: vec![255; (physical_width * physical_height * 3) as usize],
                prepare_time: std::time::Duration::ZERO,
            }
        }

        fn take_prepare_time(&mut self) -> std::time::Duration {
            std::mem::take(&mut self.prepare_time)
        }

        fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: [u8; 3]) {
            let left = (x * self.scale).floor().max(0.0) as u32;
            let top = (y * self.scale).floor().max(0.0) as u32;
            let right = ((x + width) * self.scale).ceil().min(self.width as f32) as u32;
            let bottom = ((y + height) * self.scale).ceil().min(self.height as f32) as u32;
            for py in top..bottom {
                for px in left..right {
                    let offset = ((py * self.width + px) * 3) as usize;
                    self.pixels[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }

        fn draw_text(&mut self, text: &mut TextSystem, run: TextRunId, x: f32, y: f32) {
            let started = std::time::Instant::now();
            let prepared = text.prepare(run, Point::new(x, y), self.scale).unwrap();
            self.prepare_time += started.elapsed();
            let atlas = text.atlas();
            let atlas_size = atlas.size();
            let atlas_pixels = atlas.pixels();
            for glyph in prepared.glyphs {
                if glyph.size.width <= 0.0 || glyph.size.height <= 0.0 {
                    continue;
                }
                let left = (glyph.origin.x * self.scale).floor() as i32;
                let top = (glyph.origin.y * self.scale).floor() as i32;
                let right = ((glyph.origin.x + glyph.size.width) * self.scale).ceil() as i32;
                let bottom = ((glyph.origin.y + glyph.size.height) * self.scale).ceil() as i32;
                for py in top.max(0)..bottom.min(self.height as i32) {
                    for px in left.max(0)..right.min(self.width as i32) {
                        let fx =
                            ((px as f32 + 0.5) / self.scale - glyph.origin.x) / glyph.size.width;
                        let fy =
                            ((py as f32 + 0.5) / self.scale - glyph.origin.y) / glyph.size.height;
                        let u = glyph.uv_min[0] + fx * (glyph.uv_max[0] - glyph.uv_min[0]);
                        let v = glyph.uv_min[1] + fy * (glyph.uv_max[1] - glyph.uv_min[1]);
                        let coverage = sample_linear_coverage(
                            atlas_pixels,
                            atlas_size[0],
                            atlas_size[1],
                            u,
                            v,
                        );
                        let color = glyph.color.linear_rgba();
                        let alpha = coverage * color.a;
                        let offset = ((py as u32 * self.width + px as u32) * 3) as usize;
                        for (channel, source) in [color.r, color.g, color.b].into_iter().enumerate()
                        {
                            let destination =
                                srgb_to_linear(self.pixels[offset + channel] as f32 / 255.0);
                            let linear = source.mul_add(alpha, destination * (1.0 - alpha));
                            self.pixels[offset + channel] =
                                (linear_to_srgb(linear) * 255.0 + 0.5) as u8;
                        }
                    }
                }
            }
            text.remove_run(run);
        }

        fn into_rgb(self) -> Vec<u8> {
            self.pixels
        }
    }

    fn draw_audit_label(
        canvas: &mut TypographyCanvas,
        text: &mut TextSystem,
        value: &str,
        style: TextStyle,
        x: f32,
        y: f32,
        _scale: f32,
    ) {
        let id = text.shape(value, style, None);
        canvas.draw_text(text, id, x, y);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tree_cell(
        canvas: &mut TypographyCanvas,
        text: &mut TextSystem,
        family: FontFamily,
        weight: FontWeight,
        size: f32,
        x: f32,
        y: f32,
        scale: f32,
    ) {
        let width = 310.0;
        let height = 148.0;
        canvas.fill_rect(x, y, width, height, [250, 251, 253]);
        let base = |color: [u8; 3], text_weight| TextStyle {
            family,
            weight: text_weight,
            size_px: size,
            line_height_px: size + 5.0,
            color: Color::from_srgba8(ui_core::Srgb8 {
                r: color[0],
                g: color[1],
                b: color[2],
                a: 255,
            }),
        };
        draw_audit_label(
            canvas,
            text,
            "Sample E-Commerce (SQLite)",
            base([38, 43, 51], weight),
            x + 10.0,
            y + 5.0,
            scale,
        );
        draw_audit_label(
            canvas,
            text,
            "main",
            base([53, 63, 75], weight),
            x + 20.0,
            y + 27.0,
            scale,
        );
        draw_audit_label(
            canvas,
            text,
            "Tables",
            base([45, 52, 62], weight),
            x + 38.0,
            y + 49.0,
            scale,
        );
        draw_right_count(
            canvas,
            text,
            "5",
            base([112, 120, 132], weight),
            x,
            y + 49.0,
            width,
            scale,
        );
        canvas.fill_rect(x + 28.0, y + 68.0, width - 38.0, 21.0, [220, 239, 255]);
        let selected = base([0, 100, 179], FontWeight::Numeric(550));
        draw_audit_label(canvas, text, "Views", selected, x + 38.0, y + 71.0, scale);
        draw_right_count(canvas, text, "1", selected, x, y + 71.0, width, scale);
        draw_audit_label(
            canvas,
            text,
            "Triggers",
            base([147, 153, 162], weight),
            x + 38.0,
            y + 93.0,
            scale,
        );
        draw_right_count(
            canvas,
            text,
            "1",
            base([147, 153, 162], weight),
            x,
            y + 93.0,
            width,
            scale,
        );
        draw_audit_label(
            canvas,
            text,
            "Xe Lạc Hồng (PostgreSQL)",
            base([93, 102, 114], weight),
            x + 10.0,
            y + 116.0,
            scale,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_right_count(
        canvas: &mut TypographyCanvas,
        text: &mut TextSystem,
        value: &str,
        style: TextStyle,
        x: f32,
        y: f32,
        width: f32,
        scale: f32,
    ) {
        let metrics = text.measure(value, style, None);
        draw_audit_label(
            canvas,
            text,
            value,
            style,
            x + width - metrics.size.width - 12.0,
            y,
            scale,
        );
    }

    fn sample_linear_coverage(pixels: &[u8], width: u32, height: u32, u: f32, v: f32) -> f32 {
        let x = u * width as f32 - 0.5;
        let y = v * height as f32 - 0.5;
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let tx = x - x.floor();
        let ty = y - y.floor();
        let sample = |sx: i32, sy: i32| {
            if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                0.0
            } else {
                pixels[(sy as u32 * width + sx as u32) as usize] as f32 / 255.0
            }
        };
        let top = sample(x0, y0).mul_add(1.0 - tx, sample(x0 + 1, y0) * tx);
        let bottom = sample(x0, y0 + 1).mul_add(1.0 - tx, sample(x0 + 1, y0 + 1) * tx);
        top.mul_add(1.0 - ty, bottom * ty)
    }

    fn srgb_to_linear(value: f32) -> f32 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn linear_to_srgb(value: f32) -> f32 {
        if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    }

    fn write_ppm(path: &std::path::Path, width: u32, height: u32, pixels: &[u8]) {
        use std::io::Write;
        let file = std::fs::File::create(path).expect("create PPM artifact");
        let mut writer = std::io::BufWriter::new(file);
        write!(writer, "P6\n{width} {height}\n255\n").expect("write PPM header");
        writer.write_all(pixels).expect("write PPM pixels");
    }

    #[test]
    fn glyph_quad_matches_swash_physical_bounds_at_fractional_origins() {
        let mut text = TextSystem::new();
        let id = text.shape("Ag Việt\na\u{301} g", TextStyle::default(), None);
        let run = Arc::clone(&text.runs[&id]);
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for origin in [Point::new(10.37, 5.63), Point::new(-2.87, -1.31)] {
                let prepared = text.prepare(id, origin, scale).unwrap();
                let mut quads = prepared.glyphs.iter();
                for glyph in &run.glyphs {
                    let (key, x, y) = cosmic_text::CacheKey::new(
                        glyph.font_id,
                        glyph.glyph_id,
                        glyph.font_size * scale,
                        (
                            (origin.x + glyph.origin.x) * scale,
                            ((origin.y + glyph.baseline) * scale).round()
                                + (glyph.origin.y - glyph.baseline) * scale,
                        ),
                        glyph.font_weight,
                        glyph.cache_key_flags,
                    );
                    let Some(image) = text.rasterizer.get_image(&mut text.fonts, key).as_ref()
                    else {
                        continue;
                    };
                    let quad = quads.next().unwrap();
                    let placement = image.placement;
                    assert!((quad.origin.x * scale - (x + placement.left) as f32).abs() < 0.001);
                    assert!((quad.origin.y * scale - (y - placement.top) as f32).abs() < 0.001);
                    // Swash can return width=1, height=0 for a space: no coverage.
                    if placement.width == 0 || placement.height == 0 {
                        assert_eq!(quad.size, Size::ZERO);
                        continue;
                    }
                    assert!((quad.size.width * scale - placement.width as f32).abs() < 0.001);
                    assert!((quad.size.height * scale - placement.height as f32).abs() < 0.001);
                }
                assert!(quads.next().is_none());
            }
        }
    }

    #[test]
    fn fractional_motion_within_a_cache_bin_does_not_shift_the_bitmap_again() {
        let mut text = TextSystem::new();
        let id = text.shape("DB Pro", TextStyle::default(), None);
        let first = text.prepare(id, Point::new(20.01, 40.01), 1.0).unwrap();
        let second = text.prepare(id, Point::new(20.02, 40.02), 1.0).unwrap();
        assert_eq!(first.glyphs, second.glyphs);
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

    #[test]
    fn unchanged_document_frames_and_hit_testing_do_not_reshape_lines() {
        let lines = (0..10_000).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let lengths = lines.iter().map(|line| line.graphemes(true).count());
        let mut document = TextDocumentLayout::new(lengths, 22.0);
        let mut text = TextSystem::new();
        let visible = 4_999..5_049;
        let first = document
            .layout_visible_lines(
                &mut text,
                visible.clone(),
                8,
                TextStyle::default(),
                None,
                |index| lines[index].clone(),
            )
            .unwrap();
        assert_eq!(first.len(), 66);
        assert_eq!(document.stats().lines_shaped, 66);
        let shape_count = text.shape_call_count();
        assert!(
            document
                .hit_test(&text, Point::new(24.0, first[10].top + 10.0))
                .is_some()
        );
        let mut source_reads = 0;
        for _ in 0..100 {
            document
                .layout_visible_lines(
                    &mut text,
                    visible.clone(),
                    8,
                    TextStyle::default(),
                    None,
                    |index| {
                        source_reads += 1;
                        lines[index].clone()
                    },
                )
                .unwrap();
        }
        assert_eq!(text.shape_call_count(), shape_count);
        assert_eq!(source_reads, 0);
        assert_eq!(document.stats().lines_shaped, 66);
        assert_eq!(document.stats().cache_hits, 66 * 100);
    }

    #[test]
    fn single_line_edit_and_newline_split_reuse_unaffected_runs() {
        let mut lines = vec![
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ];
        let mut document = TextDocumentLayout::new([1, 1, 1, 1], 20.0);
        let mut text = TextSystem::new();
        let before = document
            .layout_visible_lines(&mut text, 0..4, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();
        lines[2] = "C changed".into();
        document
            .apply_edit(
                &mut text,
                DirtyLineRange {
                    start: 2,
                    removed: 1,
                    inserted: 1,
                },
                &[9],
                20.0,
            )
            .unwrap();
        let shaped_before = text.shape_call_count();
        let edited = document
            .layout_visible_lines(&mut text, 0..4, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();
        assert_eq!(text.shape_call_count() - shaped_before, 1);
        assert_eq!(before[0].run, edited[0].run);
        assert_eq!(before[1].run, edited[1].run);
        assert_eq!(before[3].run, edited[3].run);

        lines.splice(2..3, ["C".to_owned(), "inserted".to_owned()]);
        document
            .apply_edit(
                &mut text,
                DirtyLineRange {
                    start: 2,
                    removed: 1,
                    inserted: 2,
                },
                &[1, 8],
                20.0,
            )
            .unwrap();
        let split = document
            .layout_visible_lines(&mut text, 0..5, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();
        assert_eq!(split[0].run, before[0].run);
        assert_eq!(split[1].run, before[1].run);
        assert_eq!(split[4].run, before[3].run);
        assert_eq!(document.line_count(), 5);
    }

    #[test]
    fn viewport_overscan_and_cache_eviction_limit_materialized_lines() {
        let mut document = TextDocumentLayout::new(std::iter::repeat_n(12, 100_000), 20.0);
        document.set_max_cached_lines(Some(64));
        let mut text = TextSystem::new();
        let visible = document
            .layout_visible_lines(
                &mut text,
                50_000..50_050,
                10,
                TextStyle::default(),
                None,
                |i| format!("row {i}"),
            )
            .unwrap();
        assert_eq!(visible.len(), 70);
        assert_eq!(document.stats().lines_shaped, 70);
        assert!(document.estimated_cache_bytes(&text) > 100_000 * size_of::<CachedDocumentLine>());
        let before = text.shape_call_count();
        document
            .layout_visible_lines(
                &mut text,
                99_900..99_950,
                10,
                TextStyle::default(),
                None,
                |i| format!("row {i}"),
            )
            .unwrap();
        assert_eq!(text.shape_call_count() - before, 70);
        assert!(document.stats().lines_evicted > 0);
        assert!(
            document
                .hit_test(&text, Point::new(4.0, visible[0].top + 2.0))
                .is_none()
        );
    }

    #[test]
    fn scrolling_into_cached_overlap_shapes_only_newly_visible_lines() {
        let mut document = TextDocumentLayout::new(std::iter::repeat_n(8, 100), 20.0);
        let mut text = TextSystem::new();
        document
            .layout_visible_lines(&mut text, 10..20, 2, TextStyle::default(), None, |i| {
                format!("row {i}")
            })
            .unwrap();
        let before = text.shape_call_count();
        let scrolled = document
            .layout_visible_lines(&mut text, 15..25, 2, TextStyle::default(), None, |i| {
                format!("row {i}")
            })
            .unwrap();
        assert_eq!(scrolled.len(), 14);
        assert_eq!(text.shape_call_count() - before, 5);
    }

    #[test]
    fn newline_merge_and_multiline_paste_preserve_unaffected_cached_runs() {
        let mut lines = vec![
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ];
        let mut document = TextDocumentLayout::new([1, 1, 1, 1], 20.0);
        let mut text = TextSystem::new();
        let original = document
            .layout_visible_lines(&mut text, 0..4, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();

        lines.splice(1..3, ["BC".to_owned()]);
        document
            .apply_edit(
                &mut text,
                DirtyLineRange {
                    start: 1,
                    removed: 2,
                    inserted: 1,
                },
                &[2],
                20.0,
            )
            .unwrap();
        let before = text.shape_call_count();
        let merged = document
            .layout_visible_lines(&mut text, 0..3, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();
        assert_eq!(text.shape_call_count() - before, 1);
        assert_eq!(merged[0].run, original[0].run);
        assert_eq!(merged[2].run, original[3].run);

        lines.splice(1..2, ["x".into(), "y".into(), "z".into()]);
        document
            .apply_edit(
                &mut text,
                DirtyLineRange {
                    start: 1,
                    removed: 1,
                    inserted: 3,
                },
                &[1, 1, 1],
                20.0,
            )
            .unwrap();
        let before = text.shape_call_count();
        let pasted = document
            .layout_visible_lines(&mut text, 0..5, 0, TextStyle::default(), None, |i| {
                lines[i].clone()
            })
            .unwrap();
        assert_eq!(text.shape_call_count() - before, 3);
        assert_eq!(pasted[0].run, original[0].run);
        assert_eq!(pasted[4].run, original[3].run);
    }

    #[test]
    fn wrap_width_change_shapes_visible_lines_but_does_not_rasterize() {
        let mut document = TextDocumentLayout::new([64, 64, 64], 20.0);
        let mut text = TextSystem::new();
        let content = "many words in a line that should wrap";
        document
            .layout_visible_lines(
                &mut text,
                0..3,
                0,
                TextStyle::default(),
                Some(100.0),
                |_| content.into(),
            )
            .unwrap();
        let before = text.shape_call_count();
        let rasterized = text.stats().rasterized;
        document
            .layout_visible_lines(
                &mut text,
                0..3,
                0,
                TextStyle::default(),
                Some(200.0),
                |_| content.into(),
            )
            .unwrap();
        assert_eq!(text.shape_call_count() - before, 3);
        assert_eq!(text.stats().rasterized, rasterized);
        assert_eq!(document.revisions().constraints, LayoutRevision(1));
    }

    #[test]
    fn document_layout_resolves_visual_line_boundaries_from_cached_runs() {
        let content = "one two three four five six seven eight nine ten";
        let mut document = TextDocumentLayout::new([content.graphemes(true).count()], 20.0);
        let mut text = TextSystem::new();
        let lines = document
            .layout_visible_lines(&mut text, 0..1, 0, TextStyle::default(), Some(80.0), |_| {
                content.into()
            })
            .unwrap();
        let metrics = text.line_metrics(lines[0].run).unwrap();
        assert!(metrics.len() > 1);
        let current = metrics[1].start + 1;
        assert_eq!(
            document.visual_line_boundary(&text, current, false),
            Some(metrics[1].start)
        );
        assert_eq!(
            document.visual_line_boundary(&text, current, true),
            Some(metrics[1].end)
        );
    }
}
