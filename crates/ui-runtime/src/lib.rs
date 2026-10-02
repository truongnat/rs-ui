//! Minimal retained tree and cached layout runtime.
//!
//! Nodes keep identity and state; `paint` always emits a fresh ephemeral
//! display list. Renderer and windowing details stay outside this crate.

use std::{any::Any, collections::HashMap};

use ui_core::{
    Color, DisplayList, DisplayListBuilder, Point, Radius, Rect, Size, Stroke, TextRunId,
};
use ui_text::TextMetrics;

mod accessibility;
mod behavior;
mod interaction;
mod scroll;
mod selection;
mod text_editing;

pub use accessibility::*;
pub use behavior::*;
pub use interaction::*;
pub use scroll::*;
pub use selection::*;
pub use text_editing::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u64);

impl NodeId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyFlags(u8);

impl DirtyFlags {
    pub const LAYOUT: Self = Self(1 << 0);
    pub const PAINT: Self = Self(1 << 1);
    pub const TEXT: Self = Self(1 << 2);
    pub const HIT_TEST: Self = Self(1 << 3);
    pub const ACCESSIBILITY: Self = Self(1 << 4);
    pub const ALL: Self = Self((1 << 5) - 1);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    pub const fn all(value: f32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    fn horizontal(self) -> f32 {
        self.left + self.right
    }

    fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dimension {
    #[default]
    Auto,
    Points(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    Row,
    Column,
    #[default]
    Stack,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Position {
    #[default]
    Flow,
    Absolute(Point),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutStyle {
    pub mode: LayoutMode,
    pub width: Dimension,
    pub height: Dimension,
    pub min_size: Size,
    pub max_size: Size,
    pub padding: Insets,
    pub gap: f32,
    pub align_x: Align,
    pub align_y: Align,
    pub flex_grow: f32,
    pub position: Position,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            mode: LayoutMode::Stack,
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_size: Size::ZERO,
            max_size: Size::new(f32::INFINITY, f32::INFINITY),
            padding: Insets::ZERO,
            gap: 0.0,
            align_x: Align::Stretch,
            align_y: Align::Stretch,
            flex_grow: 0.0,
            position: Position::Flow,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraints {
    pub min: Size,
    pub max: Size,
}

impl Constraints {
    pub const fn loose(max: Size) -> Self {
        Self {
            min: Size::ZERO,
            max,
        }
    }

    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min.width, self.max.width),
            size.height.clamp(self.min.height, self.max.height),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutStats {
    pub measured_nodes: u64,
    pub laid_out_nodes: u64,
    pub measurement_cache_hits: u64,
    pub layout_cache_hits: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractionState {
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaintState {
    pub background: Option<Color>,
    pub radius: Radius,
    pub border: Option<Stroke>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextContent {
    run: TextRunId,
    metrics: TextMetrics,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LayoutCache {
    measured_for: Option<Constraints>,
    measured: Size,
    rect: Rect,
    has_rect: bool,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self {
            measured_for: None,
            measured: Size::ZERO,
            rect: Rect::ZERO,
            has_rect: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    layout: LayoutStyle,
    paint: PaintState,
    text: Option<TextContent>,
    accessibility: Option<AccessibilitySemantics>,
    accessibility_text: Option<String>,
    interaction: InteractionState,
    pub(crate) hit_test: HitTestState,
    pub(crate) hit_test_overridden: bool,
    pub(crate) focus_policy: FocusPolicy,
    pub(crate) scrollable: bool,
    dirty: DirtyFlags,
    cache: LayoutCache,
}

impl Node {
    pub const fn id(&self) -> NodeId {
        self.id
    }

    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    pub fn layout(&self) -> &LayoutStyle {
        &self.layout
    }

    pub const fn paint_state(&self) -> PaintState {
        self.paint
    }

    pub const fn interaction(&self) -> InteractionState {
        self.interaction
    }

    pub const fn dirty_flags(&self) -> DirtyFlags {
        self.dirty
    }

    pub const fn layout_rect(&self) -> Option<Rect> {
        if self.cache.has_rect {
            Some(self.cache.rect)
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeError {
    UnknownNode(NodeId),
    CannotParentToDescendant,
    FocusTargetOutsideScope,
    InvalidResizeConfig,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "unknown UI node {}", id.get()),
            Self::CannotParentToDescendant => f.write_str("cannot parent a node to its descendant"),
            Self::FocusTargetOutsideScope => {
                f.write_str("focus target is outside the active focus scope")
            }
            Self::InvalidResizeConfig => f.write_str("invalid resize configuration"),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Default)]
pub struct StateStore {
    values: HashMap<NodeId, Box<dyn Any>>,
}

impl StateStore {
    pub fn get<T: 'static>(&self, node: NodeId) -> Option<&T> {
        self.values.get(&node)?.downcast_ref()
    }

    pub fn get_mut<T: 'static>(&mut self, node: NodeId) -> Option<&mut T> {
        self.values.get_mut(&node)?.downcast_mut()
    }

    pub fn insert<T: 'static>(&mut self, node: NodeId, value: T) -> Option<T> {
        self.values
            .insert(node, Box::new(value))
            .and_then(|previous| previous.downcast().ok().map(|value| *value))
    }

    pub fn remove(&mut self, node: NodeId) {
        self.values.remove(&node);
    }
}

pub struct UiTree {
    pub(crate) nodes: HashMap<NodeId, Node>,
    next_id: u64,
    state: StateStore,
    stats: LayoutStats,
    pub(crate) interaction_runtime: InteractionRuntime,
    pub(crate) scroll_runtime: ScrollRuntime,
}

impl UiTree {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: 1,
            state: StateStore::default(),
            stats: LayoutStats::default(),
            interaction_runtime: InteractionRuntime::default(),
            scroll_runtime: ScrollRuntime::default(),
        }
    }

    pub fn create_node(
        &mut self,
        parent: Option<NodeId>,
        layout: LayoutStyle,
        paint: PaintState,
    ) -> Result<NodeId, RuntimeError> {
        if let Some(parent_id) = parent {
            self.nodes
                .get(&parent_id)
                .ok_or(RuntimeError::UnknownNode(parent_id))?;
        }
        let id = NodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(
            id,
            Node {
                id,
                parent,
                children: Vec::new(),
                layout,
                paint,
                text: None,
                accessibility: None,
                accessibility_text: None,
                interaction: InteractionState::default(),
                hit_test: HitTestState::default(),
                hit_test_overridden: false,
                focus_policy: FocusPolicy::default(),
                scrollable: false,
                dirty: DirtyFlags::ALL,
                cache: LayoutCache::default(),
            },
        );
        if let Some(parent_id) = parent {
            self.nodes.get_mut(&parent_id).unwrap().children.push(id);
            self.invalidate_measure_chain(parent_id);
        }
        self.interaction_runtime.mark_focus_order_dirty();
        Ok(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn state(&self) -> &StateStore {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    pub fn set_layout(&mut self, id: NodeId, layout: LayoutStyle) -> Result<(), RuntimeError> {
        let node = self.nodes.get(&id).ok_or(RuntimeError::UnknownNode(id))?;
        if node.layout == layout {
            return Ok(());
        }
        self.nodes.get_mut(&id).unwrap().layout = layout;
        self.invalidate_subtree_layout(id);
        self.invalidate_measure_chain(id);
        Ok(())
    }

    pub fn set_paint(&mut self, id: NodeId, paint: PaintState) -> Result<(), RuntimeError> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or(RuntimeError::UnknownNode(id))?;
        if node.paint != paint {
            node.paint = paint;
            node.dirty.insert(DirtyFlags::PAINT);
        }
        Ok(())
    }

    pub fn set_text(
        &mut self,
        id: NodeId,
        run: TextRunId,
        metrics: TextMetrics,
    ) -> Result<(), RuntimeError> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or(RuntimeError::UnknownNode(id))?;
        let content = TextContent { run, metrics };
        if node.text != Some(content) {
            node.text = Some(content);
            node.dirty.insert(DirtyFlags::TEXT);
            node.dirty.insert(DirtyFlags::PAINT);
            node.cache.measured_for = None;
            self.invalidate_measure_chain(id);
        }
        Ok(())
    }

    pub fn clear_text(&mut self, id: NodeId) -> Result<(), RuntimeError> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or(RuntimeError::UnknownNode(id))?;
        if node.text.take().is_some() {
            node.dirty.insert(DirtyFlags::TEXT);
            node.dirty.insert(DirtyFlags::PAINT);
            node.cache.measured_for = None;
            self.invalidate_measure_chain(id);
        }
        Ok(())
    }

    pub fn set_interaction(
        &mut self,
        id: NodeId,
        interaction: InteractionState,
    ) -> Result<(), RuntimeError> {
        let request_focus = interaction.focused;
        let current_focus = self.focus_manager().focused();
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or(RuntimeError::UnknownNode(id))?;
        let interaction = InteractionState {
            focused: node.interaction.focused,
            ..interaction
        };
        if node.interaction != interaction {
            node.interaction = interaction;
            node.dirty.insert(DirtyFlags::PAINT);
        }
        if request_focus && current_focus != Some(id) {
            let _ = self.request_focus(id)?;
        } else if !request_focus && current_focus == Some(id) {
            self.clear_focus();
        }
        Ok(())
    }

    pub fn invalidate(&mut self, id: NodeId, flags: DirtyFlags) -> Result<(), RuntimeError> {
        self.nodes.get(&id).ok_or(RuntimeError::UnknownNode(id))?;
        if flags.contains(DirtyFlags::LAYOUT) {
            self.invalidate_subtree_layout(id);
            self.invalidate_measure_chain(id);
        } else if let Some(node) = self.nodes.get_mut(&id) {
            node.dirty.insert(flags);
        }
        Ok(())
    }

    pub fn reparent(&mut self, id: NodeId, new_parent: Option<NodeId>) -> Result<(), RuntimeError> {
        if !self.nodes.contains_key(&id) {
            return Err(RuntimeError::UnknownNode(id));
        }
        if let Some(parent) = new_parent {
            if !self.nodes.contains_key(&parent) {
                return Err(RuntimeError::UnknownNode(parent));
            }
            let mut current = Some(parent);
            while let Some(ancestor) = current {
                if ancestor == id {
                    return Err(RuntimeError::CannotParentToDescendant);
                }
                current = self.nodes[&ancestor].parent;
            }
        }
        let old_parent = self.nodes[&id].parent;
        if old_parent == new_parent {
            return Ok(());
        }
        if let Some(parent) = old_parent {
            self.nodes
                .get_mut(&parent)
                .unwrap()
                .children
                .retain(|child| *child != id);
            self.invalidate_measure_chain(parent);
        }
        self.nodes.get_mut(&id).unwrap().parent = new_parent;
        if let Some(parent) = new_parent {
            self.nodes.get_mut(&parent).unwrap().children.push(id);
            self.invalidate_measure_chain(parent);
        }
        self.invalidate_subtree_layout(id);
        self.interaction_runtime.mark_focus_order_dirty();
        Ok(())
    }

    pub fn remove_subtree(&mut self, id: NodeId) -> Result<(), RuntimeError> {
        let parent = self
            .nodes
            .get(&id)
            .ok_or(RuntimeError::UnknownNode(id))?
            .parent;
        if let Some(parent) = parent {
            self.nodes
                .get_mut(&parent)
                .unwrap()
                .children
                .retain(|child| *child != id);
            self.invalidate_measure_chain(parent);
        }
        let mut pending = vec![id];
        while let Some(next) = pending.pop() {
            if let Some(node) = self.nodes.remove(&next) {
                pending.extend(node.children);
                self.state.remove(next);
                self.interaction_runtime.remove_node(next);
                self.scroll_runtime.remove_node(next);
            }
        }
        Ok(())
    }

    pub fn layout(&mut self, root: NodeId, constraints: Constraints) -> Result<Rect, RuntimeError> {
        if !self.nodes.contains_key(&root) {
            return Err(RuntimeError::UnknownNode(root));
        }
        self.stats = LayoutStats::default();
        let measured = self.measure_node(root, constraints);
        let size = constraints.constrain(measured);
        let rect = Rect::from_min_size(Point::ZERO, size);
        self.arrange_node(root, rect);
        Ok(rect)
    }

    pub fn paint(&mut self, root: NodeId) -> Result<DisplayList, RuntimeError> {
        if !self.nodes.contains_key(&root) {
            return Err(RuntimeError::UnknownNode(root));
        }
        let mut builder = DisplayListBuilder::new();
        self.paint_node(root, &mut builder, Point::ZERO);
        Ok(builder.build())
    }

    pub const fn layout_stats(&self) -> LayoutStats {
        self.stats
    }

    fn invalidate_subtree_layout(&mut self, id: NodeId) {
        let children = if let Some(node) = self.nodes.get_mut(&id) {
            node.dirty.insert(DirtyFlags::LAYOUT);
            node.dirty.insert(DirtyFlags::HIT_TEST);
            node.dirty.insert(DirtyFlags::ACCESSIBILITY);
            node.children.clone()
        } else {
            return;
        };
        for child in children {
            self.invalidate_subtree_layout(child);
        }
    }

    fn invalidate_measure_chain(&mut self, id: NodeId) {
        let mut current = Some(id);
        while let Some(node_id) = current {
            let Some(node) = self.nodes.get_mut(&node_id) else {
                break;
            };
            node.cache.measured_for = None;
            node.dirty.insert(DirtyFlags::LAYOUT);
            node.dirty.insert(DirtyFlags::HIT_TEST);
            node.dirty.insert(DirtyFlags::ACCESSIBILITY);
            current = node.parent;
        }
    }

    fn measure_node(&mut self, id: NodeId, constraints: Constraints) -> Size {
        let Some(node) = self.nodes.get(&id) else {
            return Size::ZERO;
        };
        if node.cache.measured_for == Some(constraints) {
            self.stats.measurement_cache_hits += 1;
            return node.cache.measured;
        }
        let style = node.layout.clone();
        let children = node.children.clone();
        let text_size = node
            .text
            .map(|text| text.metrics.size)
            .unwrap_or(Size::ZERO);
        let measured_children = children
            .iter()
            .map(|child| {
                let child_size = self.measure_node(
                    *child,
                    Constraints::loose(Size::new(f32::INFINITY, f32::INFINITY)),
                );
                (*child, child_size)
            })
            .collect::<Vec<_>>();
        self.stats.measured_nodes += 1;

        let flow_sizes = measured_children
            .iter()
            .filter(|(child, _)| {
                self.nodes
                    .get(child)
                    .is_some_and(|node| node.layout.position == Position::Flow)
            })
            .map(|(_, size)| *size)
            .collect::<Vec<_>>();
        let (content_width, content_height) = match style.mode {
            LayoutMode::Row => (
                flow_sizes.iter().map(|size| size.width).sum::<f32>()
                    + style.gap.max(0.0) * flow_sizes.len().saturating_sub(1) as f32,
                flow_sizes
                    .iter()
                    .map(|size| size.height)
                    .fold(text_size.height, f32::max),
            ),
            LayoutMode::Column => (
                flow_sizes
                    .iter()
                    .map(|size| size.width)
                    .fold(text_size.width, f32::max),
                flow_sizes.iter().map(|size| size.height).sum::<f32>()
                    + style.gap.max(0.0) * flow_sizes.len().saturating_sub(1) as f32,
            ),
            LayoutMode::Stack => (
                flow_sizes
                    .iter()
                    .map(|size| size.width)
                    .fold(text_size.width, f32::max),
                flow_sizes
                    .iter()
                    .map(|size| size.height)
                    .fold(text_size.height, f32::max),
            ),
        };
        let natural = Size::new(
            resolve_dimension(style.width, content_width + style.padding.horizontal()),
            resolve_dimension(style.height, content_height + style.padding.vertical()),
        );
        let bounded = clamp_size(natural, style.min_size, style.max_size);
        let measured = constraints.constrain(bounded);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.cache.measured_for = Some(constraints);
            node.cache.measured = measured;
        }
        measured
    }

    fn arrange_node(&mut self, id: NodeId, rect: Rect) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        if !node.dirty.contains(DirtyFlags::LAYOUT)
            && node.cache.has_rect
            && node.cache.rect == rect
        {
            self.stats.layout_cache_hits += 1;
            return;
        }
        let style = node.layout.clone();
        let children = node.children.clone();
        let text_size = node
            .text
            .map(|text| text.metrics.size)
            .unwrap_or(Size::ZERO);
        let mut child_sizes = children
            .iter()
            .map(|child| (*child, self.nodes[child].cache.measured))
            .collect::<Vec<_>>();
        let content = Rect::from_min_max(
            Point::new(
                rect.min.x + style.padding.left,
                rect.min.y + style.padding.top,
            ),
            Point::new(
                (rect.max.x - style.padding.right).max(rect.min.x + style.padding.left),
                (rect.max.y - style.padding.bottom).max(rect.min.y + style.padding.top),
            ),
        );
        if let Some(node) = self.nodes.get_mut(&id) {
            node.cache.rect = rect;
            node.cache.has_rect = true;
            node.dirty.remove(DirtyFlags::LAYOUT);
        }
        self.update_default_hit_test_state(id, rect);
        self.update_scroll_viewport(id, rect.size());
        self.stats.laid_out_nodes += 1;

        let flow_count = children
            .iter()
            .filter(|child| self.nodes[child].layout.position == Position::Flow)
            .count();
        let is_row = style.mode == LayoutMode::Row;
        let is_stack = style.mode == LayoutMode::Stack;
        let main_available = if is_row {
            content.width()
        } else {
            content.height()
        };
        let gap_total = style.gap.max(0.0) * flow_count.saturating_sub(1) as f32;
        let current_main = child_sizes
            .iter()
            .filter(|(child, _)| self.nodes[child].layout.position == Position::Flow)
            .map(|(_, size)| if is_row { size.width } else { size.height })
            .sum::<f32>();
        let total_grow = children
            .iter()
            .filter(|child| self.nodes[child].layout.position == Position::Flow)
            .map(|child| self.nodes[child].layout.flex_grow.max(0.0))
            .sum::<f32>();
        let extra = (main_available - gap_total - current_main).max(0.0);
        let mut cursor = if is_row { content.min.x } else { content.min.y };
        for (child_id, measured) in child_sizes.drain(..) {
            let child = &self.nodes[&child_id];
            let child_style = child.layout.clone();
            let size = if let Position::Absolute(offset) = child_style.position {
                let child_size = clamp_size(measured, child_style.min_size, child_style.max_size);
                self.arrange_node(
                    child_id,
                    Rect::from_min_size(content.min + offset, child_size),
                );
                continue;
            } else {
                let grow = child_style.flex_grow.max(0.0);
                let mut size = measured;
                if is_stack {
                    if style.align_x == Align::Stretch && child_style.width == Dimension::Auto {
                        size.width = content.width();
                    }
                    if style.align_y == Align::Stretch && child_style.height == Dimension::Auto {
                        size.height = content.height();
                    }
                } else if total_grow > 0.0 && grow > 0.0 {
                    let main = if is_row { size.width } else { size.height };
                    let grown = main + extra * grow / total_grow;
                    if is_row {
                        size.width = clamp_dimension(
                            grown,
                            child_style.min_size.width,
                            child_style.max_size.width,
                        );
                    } else {
                        size.height = clamp_dimension(
                            grown,
                            child_style.min_size.height,
                            child_style.max_size.height,
                        );
                    }
                }
                if is_row
                    && style.align_y == Align::Stretch
                    && child_style.height == Dimension::Auto
                {
                    size.height = content.height();
                }
                if !is_row
                    && !is_stack
                    && style.align_x == Align::Stretch
                    && child_style.width == Dimension::Auto
                {
                    size.width = content.width();
                }
                size = clamp_size(size, child_style.min_size, child_style.max_size);
                size
            };
            let x = if is_row {
                cursor
            } else {
                aligned_start(content.min.x, content.width(), size.width, style.align_x)
            };
            let y = if is_row || is_stack {
                aligned_start(content.min.y, content.height(), size.height, style.align_y)
            } else {
                cursor
            };
            let child_rect = Rect::from_min_size(Point::new(x, y), size);
            self.arrange_node(child_id, child_rect);
            if !is_stack {
                cursor += if is_row { size.width } else { size.height } + style.gap.max(0.0);
            }
        }
        let _ = text_size;
    }

    fn paint_node(&mut self, id: NodeId, builder: &mut DisplayListBuilder, scroll: Point) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let rect = translate_rect(node.cache.rect, scroll);
        let paint = node.paint;
        let text = node.text;
        let children = node.children.clone();
        if let Some(color) = paint.background {
            builder.fill_rounded_rect(rect, paint.radius, color);
        }
        if let Some(stroke) = paint.border {
            builder.stroke_rounded_rect(rect, paint.radius, stroke);
        }
        if let Some(text) = text {
            builder.text(text.run, rect.min);
        }
        let child_scroll = if let Some(offset) = self.scroll_transform(id) {
            builder.push_clip(rect);
            scroll - offset
        } else {
            scroll
        };
        for child in children {
            self.paint_node(child, builder, child_scroll);
        }
        if self.scroll_transform(id).is_some() {
            builder.pop_clip();
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.dirty.remove(DirtyFlags::PAINT);
            node.dirty.remove(DirtyFlags::TEXT);
        }
    }
}

impl Default for UiTree {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_dimension(dimension: Dimension, intrinsic: f32) -> f32 {
    match dimension {
        Dimension::Auto => intrinsic,
        Dimension::Points(value) => value.max(0.0),
    }
}

fn clamp_dimension(value: f32, min: f32, max: f32) -> f32 {
    value.clamp(min.max(0.0), max.max(min.max(0.0)))
}

fn clamp_size(size: Size, min: Size, max: Size) -> Size {
    Size::new(
        clamp_dimension(size.width, min.width, max.width),
        clamp_dimension(size.height, min.height, max.height),
    )
}

fn aligned_start(origin: f32, available: f32, size: f32, align: Align) -> f32 {
    match align {
        Align::Start | Align::Stretch => origin,
        Align::Center => origin + (available - size) * 0.5,
        Align::End => origin + available - size,
    }
}

fn translate_rect(rect: Rect, offset: Point) -> Rect {
    Rect::from_min_max(rect.min + offset, rect.max + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> LayoutStyle {
        LayoutStyle {
            mode: LayoutMode::Row,
            align_y: Align::Start,
            ..LayoutStyle::default()
        }
    }

    fn fixed(width: f32, height: f32) -> LayoutStyle {
        LayoutStyle {
            width: Dimension::Points(width),
            height: Dimension::Points(height),
            ..LayoutStyle::default()
        }
    }

    #[test]
    fn nested_row_and_column_respect_gap_padding_and_alignment() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, row(), PaintState::default())
            .unwrap();
        let left = tree
            .create_node(Some(root), fixed(100.0, 80.0), PaintState::default())
            .unwrap();
        let column = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    mode: LayoutMode::Column,
                    gap: 4.0,
                    padding: Insets::all(3.0),
                    width: Dimension::Points(60.0),
                    height: Dimension::Points(80.0),
                    align_x: Align::Start,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let top = tree
            .create_node(Some(column), fixed(30.0, 20.0), PaintState::default())
            .unwrap();
        let bottom = tree
            .create_node(Some(column), fixed(30.0, 20.0), PaintState::default())
            .unwrap();
        tree.layout(root, Constraints::loose(Size::new(200.0, 100.0)))
            .unwrap();
        assert_eq!(
            tree.node(left).unwrap().layout_rect().unwrap().min,
            Point::ZERO
        );
        assert_eq!(
            tree.node(column).unwrap().layout_rect().unwrap().min,
            Point::new(100.0, 0.0)
        );
        assert_eq!(
            tree.node(top).unwrap().layout_rect().unwrap().min,
            Point::new(103.0, 3.0)
        );
        assert_eq!(
            tree.node(bottom).unwrap().layout_rect().unwrap().min,
            Point::new(103.0, 27.0)
        );
    }

    #[test]
    fn min_max_constraints_and_flex_grow_are_applied() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(
                None,
                LayoutStyle {
                    width: Dimension::Points(100.0),
                    height: Dimension::Points(30.0),
                    ..row()
                },
                PaintState::default(),
            )
            .unwrap();
        let first = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    width: Dimension::Points(20.0),
                    height: Dimension::Points(10.0),
                    flex_grow: 1.0,
                    min_size: Size::new(15.0, 0.0),
                    max_size: Size::new(100.0, 100.0),
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let second = tree
            .create_node(Some(root), fixed(20.0, 10.0), PaintState::default())
            .unwrap();
        tree.layout(root, Constraints::loose(Size::new(100.0, 30.0)))
            .unwrap();
        assert_eq!(
            tree.node(first).unwrap().layout_rect().unwrap().width(),
            80.0
        );
        assert_eq!(
            tree.node(second).unwrap().layout_rect().unwrap().min.x,
            80.0
        );
        tree.set_layout(
            first,
            LayoutStyle {
                max_size: Size::new(35.0, 100.0),
                ..tree.node(first).unwrap().layout.clone()
            },
        )
        .unwrap();
        tree.layout(root, Constraints::loose(Size::new(100.0, 30.0)))
            .unwrap();
        assert_eq!(
            tree.node(first).unwrap().layout_rect().unwrap().width(),
            35.0
        );
    }

    #[test]
    fn intrinsic_text_sizing_is_logical_and_display_list_is_ephemeral() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .unwrap();
        let text = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let metrics = TextMetrics {
            size: Size::new(84.5, 19.0),
            baseline: 14.0,
            glyph_count: 6,
        };
        tree.set_text(text, TextRunId(7), metrics).unwrap();
        let rect = tree
            .layout(root, Constraints::loose(Size::new(200.0, 100.0)))
            .unwrap();
        assert_eq!(rect.size(), metrics.size);
        let first = tree.paint(root).unwrap();
        let second = tree.paint(root).unwrap();
        assert_eq!(first, second);
        assert!(matches!(
            first.commands().last(),
            Some(ui_core::DisplayCommand::Text { .. })
        ));
    }

    #[test]
    fn layout_constraints_are_logical_and_independent_of_physical_dpi() {
        let logical_size = Size::new(640.0, 480.0);
        let physical_1x = [640.0_f32, 480.0];
        let physical_2x = [1280.0_f32, 960.0];
        let logical_1x = Size::new(physical_1x[0] / 1.0, physical_1x[1] / 1.0);
        let logical_2x = Size::new(physical_2x[0] / 2.0, physical_2x[1] / 2.0);
        assert_eq!(logical_1x, logical_size);
        assert_eq!(logical_2x, logical_size);

        let mut tree = UiTree::new();
        let root = tree
            .create_node(
                None,
                LayoutStyle {
                    width: Dimension::Points(logical_size.width),
                    height: Dimension::Points(logical_size.height),
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let at_1x = tree.layout(root, Constraints::loose(logical_1x)).unwrap();
        let at_2x = tree.layout(root, Constraints::loose(logical_2x)).unwrap();
        assert_eq!(at_1x, at_2x);
    }

    #[test]
    fn hover_invalidates_only_paint_and_measurement_reuses_clean_sibling() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, row(), PaintState::default())
            .unwrap();
        let first = tree
            .create_node(Some(root), fixed(20.0, 20.0), PaintState::default())
            .unwrap();
        let second = tree
            .create_node(Some(root), fixed(20.0, 20.0), PaintState::default())
            .unwrap();
        let constraints = Constraints::loose(Size::new(100.0, 30.0));
        tree.layout(root, constraints).unwrap();
        tree.layout(first, Constraints::loose(Size::new(20.0, 20.0)))
            .unwrap();
        tree.set_interaction(
            first,
            InteractionState {
                hovered: true,
                ..InteractionState::default()
            },
        )
        .unwrap();
        let dirty = tree.node(first).unwrap().dirty_flags();
        assert!(dirty.contains(DirtyFlags::PAINT));
        assert!(!dirty.contains(DirtyFlags::LAYOUT));
        tree.layout(root, constraints).unwrap();
        assert!(
            !tree
                .node(second)
                .unwrap()
                .dirty_flags()
                .contains(DirtyFlags::LAYOUT)
        );
        assert!(tree.layout_stats().measurement_cache_hits > 0);
    }

    #[test]
    fn reparent_rejects_cycles_and_subtree_removal_clears_state() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        assert_eq!(
            tree.reparent(root, Some(child)),
            Err(RuntimeError::CannotParentToDescendant)
        );
        tree.state_mut().insert(child, String::from("held"));
        tree.remove_subtree(child).unwrap();
        assert!(tree.node(child).is_none());
        assert!(tree.state().get::<String>(child).is_none());
    }

    #[test]
    fn layout_mutation_remeasures_ancestors_but_reuses_sibling_measurement() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, row(), PaintState::default())
            .unwrap();
        let first = tree
            .create_node(Some(root), fixed(20.0, 20.0), PaintState::default())
            .unwrap();
        let second = tree
            .create_node(Some(root), fixed(20.0, 20.0), PaintState::default())
            .unwrap();
        let constraints = Constraints::loose(Size::new(100.0, 40.0));
        tree.layout(root, constraints).unwrap();
        tree.set_layout(first, fixed(30.0, 20.0)).unwrap();
        tree.layout(root, constraints).unwrap();
        assert_eq!(tree.layout_stats().measured_nodes, 2);
        assert!(tree.layout_stats().measurement_cache_hits > 0);
        assert!(
            !tree
                .node(second)
                .unwrap()
                .dirty_flags()
                .contains(DirtyFlags::LAYOUT)
        );
    }

    #[test]
    fn stack_overlays_children_and_absolute_offset_is_relative_to_content_box() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(
                None,
                LayoutStyle {
                    width: Dimension::Points(100.0),
                    height: Dimension::Points(80.0),
                    mode: LayoutMode::Stack,
                    align_x: Align::Start,
                    align_y: Align::Start,
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let first = tree
            .create_node(Some(root), fixed(20.0, 20.0), PaintState::default())
            .unwrap();
        let second = tree
            .create_node(Some(root), fixed(30.0, 10.0), PaintState::default())
            .unwrap();
        let absolute = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    position: Position::Absolute(Point::new(40.0, 12.0)),
                    ..fixed(10.0, 10.0)
                },
                PaintState::default(),
            )
            .unwrap();
        tree.layout(root, Constraints::loose(Size::new(100.0, 80.0)))
            .unwrap();
        assert_eq!(
            tree.node(first).unwrap().layout_rect().unwrap().min,
            Point::ZERO
        );
        assert_eq!(
            tree.node(second).unwrap().layout_rect().unwrap().min,
            Point::ZERO
        );
        assert_eq!(
            tree.node(absolute).unwrap().layout_rect().unwrap().min,
            Point::new(40.0, 12.0)
        );
    }
}
