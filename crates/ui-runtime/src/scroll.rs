use std::{collections::HashMap, time::Duration};

use ui_core::{Point, Rect, Size};

use crate::{DirtyFlags, NodeId, RuntimeError, UiTree};

const MIN_SCROLLBAR_THUMB: f32 = 18.0;
const INERTIA_FRICTION_PER_SECOND: f32 = 0.001;
const MIN_INERTIA_SPEED: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    pub offset: Point,
    pub velocity: Point,
    pub viewport: Size,
    pub content_size: Size,
}

impl ScrollState {
    pub const fn new(viewport: Size, content_size: Size) -> Self {
        Self {
            offset: Point::ZERO,
            velocity: Point::ZERO,
            viewport,
            content_size,
        }
    }

    pub fn max_offset(self) -> Point {
        Point::new(
            (self.content_size.width - self.viewport.width).max(0.0),
            (self.content_size.height - self.viewport.height).max(0.0),
        )
    }

    pub fn set_viewport(&mut self, viewport: Size) -> bool {
        let changed = self.viewport != viewport;
        self.viewport = viewport;
        self.clamp_offset() || changed
    }

    pub fn set_content_size(&mut self, content_size: Size) -> bool {
        let changed = self.content_size != content_size;
        self.content_size = content_size;
        self.clamp_offset() || changed
    }

    pub fn set_offset(&mut self, offset: Point) -> bool {
        let clamped = clamp_point(offset, self.max_offset());
        let changed = self.offset != clamped;
        self.offset = clamped;
        changed
    }

    pub fn scroll_by(&mut self, delta: Point) -> ScrollDelta {
        let previous = self.offset;
        self.set_offset(previous + delta);
        let consumed = self.offset - previous;
        ScrollDelta {
            consumed,
            remaining: delta - consumed,
            changed: consumed != Point::ZERO,
        }
    }

    pub fn fling(&mut self, velocity: Point) {
        self.velocity = velocity;
    }

    pub fn tick(&mut self, elapsed: Duration) -> bool {
        let seconds = elapsed.as_secs_f32();
        if seconds <= 0.0 || speed(self.velocity) < MIN_INERTIA_SPEED {
            self.velocity = Point::ZERO;
            return false;
        }
        let moved = self.scroll_by(Point::new(
            self.velocity.x * seconds,
            self.velocity.y * seconds,
        ));
        if moved.remaining.x != 0.0 {
            self.velocity.x = 0.0;
        }
        if moved.remaining.y != 0.0 {
            self.velocity.y = 0.0;
        }
        let decay = INERTIA_FRICTION_PER_SECOND.powf(seconds);
        self.velocity = Point::new(self.velocity.x * decay, self.velocity.y * decay);
        moved.changed || speed(self.velocity) >= MIN_INERTIA_SPEED
    }

    pub fn scrollbar_geometry(
        self,
        orientation: ScrollbarOrientation,
        track: Rect,
    ) -> Option<ScrollbarGeometry> {
        let (viewport_extent, content_extent, offset, track_extent) = match orientation {
            ScrollbarOrientation::Vertical => (
                self.viewport.height,
                self.content_size.height,
                self.offset.y,
                track.height(),
            ),
            ScrollbarOrientation::Horizontal => (
                self.viewport.width,
                self.content_size.width,
                self.offset.x,
                track.width(),
            ),
        };
        if viewport_extent <= 0.0 || content_extent <= viewport_extent || track_extent <= 0.0 {
            return None;
        }
        let thumb_extent = (track_extent * viewport_extent / content_extent)
            .clamp(MIN_SCROLLBAR_THUMB, track_extent);
        let travel = (track_extent - thumb_extent).max(0.0);
        let scrollable_extent = (content_extent - viewport_extent).max(0.0);
        let thumb_offset = if scrollable_extent > 0.0 {
            travel * offset.clamp(0.0, scrollable_extent) / scrollable_extent
        } else {
            0.0
        };
        let thumb = match orientation {
            ScrollbarOrientation::Vertical => Rect::from_min_max(
                Point::new(track.min.x, track.min.y + thumb_offset),
                Point::new(track.max.x, track.min.y + thumb_offset + thumb_extent),
            ),
            ScrollbarOrientation::Horizontal => Rect::from_min_max(
                Point::new(track.min.x + thumb_offset, track.min.y),
                Point::new(track.min.x + thumb_offset + thumb_extent, track.max.y),
            ),
        };
        Some(ScrollbarGeometry {
            track,
            thumb,
            orientation,
        })
    }

    fn clamp_offset(&mut self) -> bool {
        self.set_offset(self.offset)
    }
}

fn clamp_point(point: Point, max: Point) -> Point {
    Point::new(point.x.clamp(0.0, max.x), point.y.clamp(0.0, max.y))
}

fn speed(point: Point) -> f32 {
    (point.x * point.x + point.y * point.y).sqrt()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollDelta {
    pub consumed: Point,
    pub remaining: Point,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarOrientation {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarGeometry {
    pub track: Rect,
    pub thumb: Rect,
    pub orientation: ScrollbarOrientation,
}

#[derive(Default)]
pub(crate) struct ScrollRuntime {
    states: HashMap<NodeId, ScrollState>,
}

impl ScrollRuntime {
    pub(crate) fn remove_node(&mut self, node: NodeId) {
        self.states.remove(&node);
    }
}

impl UiTree {
    pub fn attach_scroll_view(
        &mut self,
        node: NodeId,
        viewport: Size,
        content_size: Size,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.interaction_runtime_scroll_set(node, ScrollState::new(viewport, content_size));
        if let Some(target) = self.nodes.get_mut(&node) {
            target.scrollable = true;
        }
        Ok(())
    }

    pub fn set_scroll_state(
        &mut self,
        node: NodeId,
        state: ScrollState,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.interaction_runtime_scroll_set(node, state);
        if let Some(target) = self.nodes.get_mut(&node) {
            target.scrollable = true;
        }
        Ok(())
    }

    pub fn scroll_state(&self, node: NodeId) -> Option<ScrollState> {
        self.interaction_runtime_scroll_get(node)
    }

    pub fn set_scroll_content_size(
        &mut self,
        node: NodeId,
        content_size: Size,
    ) -> Result<bool, RuntimeError> {
        let state = self
            .interaction_runtime_scroll_get_mut(node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let changed = state.set_content_size(content_size);
        if changed {
            self.invalidate_scroll_node(node);
        }
        Ok(changed)
    }

    pub fn scroll_by(&mut self, node: NodeId, delta: Point) -> Result<ScrollDelta, RuntimeError> {
        let state = self
            .interaction_runtime_scroll_get_mut(node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let result = state.scroll_by(delta);
        if result.changed {
            self.invalidate_scroll_node(node);
        }
        Ok(result)
    }

    pub fn fling_scroll(&mut self, node: NodeId, velocity: Point) -> Result<(), RuntimeError> {
        let state = self
            .interaction_runtime_scroll_get_mut(node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        state.fling(velocity);
        Ok(())
    }

    pub fn tick_scroll(&mut self, elapsed: Duration) -> bool {
        let nodes = self.interaction_runtime_scroll_nodes();
        let mut changed = false;
        for node in nodes {
            if let Some(state) = self.interaction_runtime_scroll_get_mut(node)
                && state.tick(elapsed)
            {
                changed = true;
                self.invalidate_scroll_node(node);
            }
        }
        changed
    }

    pub fn scrollbar_geometry(
        &self,
        node: NodeId,
        orientation: ScrollbarOrientation,
        track: Rect,
    ) -> Option<ScrollbarGeometry> {
        self.scroll_state(node)?
            .scrollbar_geometry(orientation, track)
    }

    pub(crate) fn update_scroll_viewport(&mut self, node: NodeId, viewport: Size) {
        if let Some(state) = self.interaction_runtime_scroll_get_mut(node) {
            state.set_viewport(viewport);
        }
    }

    pub(crate) fn scroll_transform(&self, node: NodeId) -> Option<Point> {
        self.scroll_state(node).map(|state| state.offset)
    }

    fn invalidate_scroll_node(&mut self, node: NodeId) {
        if let Some(target) = self.nodes.get_mut(&node) {
            target.dirty.insert(DirtyFlags::PAINT);
            target.dirty.insert(DirtyFlags::HIT_TEST);
        }
        self.invalidate_accessibility_subtree(node);
        self.invalidate_accessibility_chain(node);
    }

    fn interaction_runtime_scroll_set(&mut self, node: NodeId, state: ScrollState) {
        self.scroll_runtime.states.insert(node, state);
    }

    fn interaction_runtime_scroll_get(&self, node: NodeId) -> Option<ScrollState> {
        self.scroll_runtime.states.get(&node).copied()
    }

    fn interaction_runtime_scroll_get_mut(&mut self, node: NodeId) -> Option<&mut ScrollState> {
        self.scroll_runtime.states.get_mut(&node)
    }

    fn interaction_runtime_scroll_nodes(&self) -> Vec<NodeId> {
        self.scroll_runtime.states.keys().copied().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAlignment {
    Start,
    Center,
    End,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualViewportError {
    ItemOutOfBounds(usize),
    InvalidExtent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollAdjustment {
    pub offset_delta: f32,
    pub anchor_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleItem {
    pub index: usize,
    pub offset: f32,
    pub extent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum VirtualExtentMode {
    Fixed(f32),
    Estimated(f32),
}

pub struct VirtualViewport {
    pub first_visible: usize,
    pub last_visible: usize,
    pub overscan: usize,
    item_count: usize,
    viewport_extent: f32,
    scroll_offset: f32,
    mode: VirtualExtentMode,
    measured: HashMap<usize, f32>,
    corrections: FenwickTree,
}

impl VirtualViewport {
    pub fn new_fixed(
        item_count: usize,
        item_extent: f32,
        viewport_extent: f32,
        overscan: usize,
    ) -> Result<Self, VirtualViewportError> {
        validate_extent(item_extent)?;
        let mut viewport = Self {
            first_visible: 0,
            last_visible: 0,
            overscan,
            item_count,
            viewport_extent: viewport_extent.max(0.0),
            scroll_offset: 0.0,
            mode: VirtualExtentMode::Fixed(item_extent),
            measured: HashMap::new(),
            corrections: FenwickTree::new(item_count),
        };
        viewport.refresh_visible_range();
        Ok(viewport)
    }

    pub fn new_estimated(
        item_count: usize,
        estimated_extent: f32,
        viewport_extent: f32,
        overscan: usize,
    ) -> Result<Self, VirtualViewportError> {
        validate_extent(estimated_extent)?;
        let mut viewport = Self {
            first_visible: 0,
            last_visible: 0,
            overscan,
            item_count,
            viewport_extent: viewport_extent.max(0.0),
            scroll_offset: 0.0,
            mode: VirtualExtentMode::Estimated(estimated_extent),
            measured: HashMap::new(),
            corrections: FenwickTree::new(item_count),
        };
        viewport.refresh_visible_range();
        Ok(viewport)
    }

    pub const fn item_count(&self) -> usize {
        self.item_count
    }

    pub const fn viewport_extent(&self) -> f32 {
        self.viewport_extent
    }

    pub const fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }

    pub fn set_viewport_extent(&mut self, viewport_extent: f32) {
        self.viewport_extent = viewport_extent.max(0.0);
        self.scroll_offset = self.scroll_offset.min(self.max_scroll_offset());
        self.refresh_visible_range();
    }

    pub fn set_scroll_offset(&mut self, offset: f32) {
        self.scroll_offset = offset.clamp(0.0, self.max_scroll_offset());
        self.refresh_visible_range();
    }

    pub fn max_scroll_offset(&self) -> f32 {
        (self.total_extent() - self.viewport_extent).max(0.0)
    }

    pub fn total_extent(&self) -> f32 {
        let estimated = self.estimated_extent() * self.item_count as f32;
        estimated + self.corrections.sum(self.item_count)
    }

    pub fn estimated_extent(&self) -> f32 {
        match self.mode {
            VirtualExtentMode::Fixed(extent) | VirtualExtentMode::Estimated(extent) => extent,
        }
    }

    pub fn item_extent(&self, index: usize) -> Result<f32, VirtualViewportError> {
        if index >= self.item_count {
            return Err(VirtualViewportError::ItemOutOfBounds(index));
        }
        Ok(self
            .measured
            .get(&index)
            .copied()
            .unwrap_or_else(|| self.estimated_extent()))
    }

    pub fn item_offset(&self, index: usize) -> Result<f32, VirtualViewportError> {
        if index > self.item_count {
            return Err(VirtualViewportError::ItemOutOfBounds(index));
        }
        Ok(self.estimated_extent() * index as f32 + self.corrections.sum(index))
    }

    pub fn measure_item(
        &mut self,
        index: usize,
        extent: f32,
    ) -> Result<ScrollAdjustment, VirtualViewportError> {
        validate_extent(extent)?;
        let old_extent = self.item_extent(index)?;
        if matches!(self.mode, VirtualExtentMode::Fixed(_)) && old_extent != extent {
            return Err(VirtualViewportError::InvalidExtent);
        }
        let item_offset = self.item_offset(index)?;
        let old_scroll_offset = self.scroll_offset;
        self.measured.insert(index, extent);
        self.corrections.add(index, extent - old_extent);
        let mut offset_delta = 0.0;
        if item_offset < old_scroll_offset {
            self.scroll_offset =
                (old_scroll_offset + extent - old_extent).clamp(0.0, self.max_scroll_offset());
            offset_delta = self.scroll_offset - old_scroll_offset;
        }
        self.refresh_visible_range();
        Ok(ScrollAdjustment {
            offset_delta,
            anchor_index: index,
        })
    }

    pub fn scroll_to_index(
        &mut self,
        index: usize,
        alignment: ScrollAlignment,
    ) -> Result<(), VirtualViewportError> {
        let item_offset = self.item_offset(index)?;
        let extent = self.item_extent(index)?;
        let target = match alignment {
            ScrollAlignment::Start => item_offset,
            ScrollAlignment::Center => item_offset - (self.viewport_extent - extent) * 0.5,
            ScrollAlignment::End => item_offset - self.viewport_extent + extent,
            ScrollAlignment::Nearest => {
                if item_offset < self.scroll_offset {
                    item_offset
                } else if item_offset + extent > self.scroll_offset + self.viewport_extent {
                    item_offset + extent - self.viewport_extent
                } else {
                    self.scroll_offset
                }
            }
        };
        self.set_scroll_offset(target);
        Ok(())
    }

    pub fn visible_items(&self) -> Vec<VisibleItem> {
        (self.first_visible..self.last_visible)
            .filter_map(|index| {
                let offset = self.item_offset(index).ok()?;
                let extent = self.item_extent(index).ok()?;
                Some(VisibleItem {
                    index,
                    offset,
                    extent,
                })
            })
            .collect()
    }

    fn refresh_visible_range(&mut self) {
        if self.item_count == 0 || self.viewport_extent <= 0.0 {
            self.first_visible = 0;
            self.last_visible = 0;
            return;
        }
        let first = self.index_at_or_after(self.scroll_offset);
        let end_offset = self.scroll_offset + self.viewport_extent;
        let mut last = first;
        while last < self.item_count {
            let item_offset = self.item_offset(last).unwrap_or(self.total_extent());
            if item_offset >= end_offset && last > first {
                break;
            }
            last += 1;
        }
        self.first_visible = first.saturating_sub(self.overscan);
        self.last_visible = last.saturating_add(self.overscan).min(self.item_count);
    }

    fn index_at_or_after(&self, offset: f32) -> usize {
        let mut low = 0;
        let mut high = self.item_count;
        while low < high {
            let middle = low + (high - low) / 2;
            let end = self.item_offset(middle).unwrap_or(self.total_extent())
                + self.item_extent(middle).unwrap_or(self.estimated_extent());
            if end <= offset {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low.min(self.item_count)
    }
}

pub type VirtualList = VirtualViewport;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridCell {
    pub row: usize,
    pub column: usize,
    pub index: usize,
    pub rect: Rect,
}

pub struct VirtualGrid {
    pub rows: VirtualViewport,
    pub first_visible_column: usize,
    pub last_visible_column: usize,
    pub overscan: usize,
    column_count: usize,
    column_extent: f32,
    viewport_width: f32,
    horizontal_offset: f32,
}

impl VirtualGrid {
    pub fn new_fixed(
        row_count: usize,
        column_count: usize,
        row_extent: f32,
        column_extent: f32,
        viewport: Size,
        overscan: usize,
    ) -> Result<Self, VirtualViewportError> {
        validate_extent(column_extent)?;
        let rows = VirtualViewport::new_fixed(row_count, row_extent, viewport.height, overscan)?;
        let mut grid = Self {
            rows,
            first_visible_column: 0,
            last_visible_column: 0,
            overscan,
            column_count,
            column_extent,
            viewport_width: viewport.width.max(0.0),
            horizontal_offset: 0.0,
        };
        grid.refresh_columns();
        Ok(grid)
    }

    pub fn set_viewport(&mut self, viewport: Size) {
        self.rows.set_viewport_extent(viewport.height);
        self.viewport_width = viewport.width.max(0.0);
        self.horizontal_offset = self.horizontal_offset.min(self.max_horizontal_offset());
        self.refresh_columns();
    }

    pub fn set_scroll_offset(&mut self, offset: Point) {
        self.rows.set_scroll_offset(offset.y);
        self.horizontal_offset = offset.x.clamp(0.0, self.max_horizontal_offset());
        self.refresh_columns();
    }

    pub fn scroll_offset(&self) -> Point {
        Point::new(self.horizontal_offset, self.rows.scroll_offset())
    }

    pub fn total_content_size(&self) -> Size {
        Size::new(
            self.column_count as f32 * self.column_extent,
            self.rows.total_extent(),
        )
    }

    pub fn visible_cells(&self) -> Vec<GridCell> {
        let mut cells = Vec::new();
        for row in self.rows.visible_items() {
            for column in self.first_visible_column..self.last_visible_column {
                let x = column as f32 * self.column_extent - self.horizontal_offset;
                let y = row.offset - self.rows.scroll_offset();
                cells.push(GridCell {
                    row: row.index,
                    column,
                    index: row.index * self.column_count + column,
                    rect: Rect::from_min_size(
                        Point::new(x, y),
                        Size::new(self.column_extent, row.extent),
                    ),
                });
            }
        }
        cells
    }

    fn max_horizontal_offset(&self) -> f32 {
        (self.column_count as f32 * self.column_extent - self.viewport_width).max(0.0)
    }

    fn refresh_columns(&mut self) {
        if self.column_count == 0 || self.viewport_width <= 0.0 {
            self.first_visible_column = 0;
            self.last_visible_column = 0;
            return;
        }
        let first = (self.horizontal_offset / self.column_extent).floor() as usize;
        let mut last = first;
        let end = self.horizontal_offset + self.viewport_width;
        while last < self.column_count && last as f32 * self.column_extent < end {
            last += 1;
        }
        self.first_visible_column = first.saturating_sub(self.overscan);
        self.last_visible_column = last.saturating_add(self.overscan).min(self.column_count);
    }
}

fn validate_extent(extent: f32) -> Result<(), VirtualViewportError> {
    if extent.is_finite() && extent > 0.0 {
        Ok(())
    } else {
        Err(VirtualViewportError::InvalidExtent)
    }
}

struct FenwickTree {
    values: Vec<f32>,
}

impl FenwickTree {
    fn new(length: usize) -> Self {
        Self {
            values: vec![0.0; length + 1],
        }
    }

    fn add(&mut self, index: usize, delta: f32) {
        let mut cursor = index + 1;
        while cursor < self.values.len() {
            self.values[cursor] += delta;
            cursor += cursor & cursor.wrapping_neg();
        }
    }

    fn sum(&self, count: usize) -> f32 {
        let mut cursor = count.min(self.values.len() - 1);
        let mut sum = 0.0;
        while cursor > 0 {
            sum += self.values[cursor];
            cursor &= cursor - 1;
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dimension, HitTestState, LayoutStyle, Modifiers, PaintState, WheelEvent};

    #[test]
    fn scroll_state_clamps_and_returns_remaining_nested_delta() {
        let mut state = ScrollState::new(Size::new(100.0, 100.0), Size::new(100.0, 500.0));
        let consumed = state.scroll_by(Point::new(4.0, 450.0));
        assert_eq!(consumed.consumed, Point::new(0.0, 400.0));
        assert_eq!(consumed.remaining, Point::new(4.0, 50.0));
        assert_eq!(state.offset, Point::new(0.0, 400.0));
    }

    #[test]
    fn inertia_tick_moves_and_stops_at_scroll_boundary() {
        let mut state = ScrollState::new(Size::new(100.0, 100.0), Size::new(100.0, 1_000.0));
        state.fling(Point::new(0.0, 200.0));
        assert!(state.tick(Duration::from_millis(16)));
        assert!(state.offset.y > 0.0);
        state.set_offset(Point::new(0.0, 900.0));
        state.fling(Point::new(0.0, 500.0));
        state.tick(Duration::from_secs(1));
        assert_eq!(state.offset.y, 900.0);
        assert_eq!(state.velocity, Point::ZERO);
    }

    #[test]
    fn variable_virtual_viewport_keeps_anchor_when_previous_item_grows() {
        let mut viewport = VirtualViewport::new_estimated(1_000_000, 20.0, 100.0, 2).unwrap();
        viewport.set_scroll_offset(2_000.0);
        let before = viewport.scroll_offset();
        let adjustment = viewport.measure_item(20, 40.0).unwrap();
        assert_eq!(adjustment.offset_delta, 20.0);
        assert_eq!(viewport.scroll_offset(), before + 20.0);
        assert!(viewport.last_visible - viewport.first_visible < 20);
    }

    #[test]
    fn fixed_virtual_list_and_million_row_grid_only_materialize_viewport_items() {
        let list = VirtualViewport::new_fixed(1_000_000, 24.0, 240.0, 2).unwrap();
        assert!(list.visible_items().len() <= 14);
        let grid = VirtualGrid::new_fixed(1_000_000, 100, 24.0, 120.0, Size::new(600.0, 240.0), 1)
            .unwrap();
        assert!(grid.visible_cells().len() <= 14 * 7);
    }

    #[test]
    fn scrollbar_geometry_uses_content_ratio_and_minimum_thumb() {
        let mut state = ScrollState::new(Size::new(100.0, 100.0), Size::new(100.0, 1_000.0));
        state.set_offset(Point::new(0.0, 450.0));
        let geometry = state
            .scrollbar_geometry(
                ScrollbarOrientation::Vertical,
                Rect::from_min_size(Point::ZERO, Size::new(12.0, 100.0)),
            )
            .unwrap();
        assert_eq!(geometry.thumb.height(), MIN_SCROLLBAR_THUMB);
        assert!(geometry.thumb.min.y > geometry.track.min.y);
    }

    #[test]
    fn nested_scroll_consumes_inner_delta_then_bubbles_remaining_to_outer() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .unwrap();
        let outer = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let inner = tree
            .create_node(Some(outer), LayoutStyle::default(), PaintState::default())
            .unwrap();
        for node in [root, outer, inner] {
            tree.set_hit_test_state(
                node,
                HitTestState {
                    bounds: Rect::from_min_size(Point::ZERO, Size::new(100.0, 100.0)),
                    ..HitTestState::default()
                },
            )
            .unwrap();
        }
        tree.attach_scroll_view(outer, Size::new(100.0, 100.0), Size::new(100.0, 300.0))
            .unwrap();
        tree.attach_scroll_view(inner, Size::new(100.0, 100.0), Size::new(100.0, 200.0))
            .unwrap();

        tree.wheel(
            root,
            WheelEvent::new(
                Point::new(10.0, 10.0),
                Point::new(0.0, 40.0),
                Modifiers::default(),
                Duration::ZERO,
            ),
        )
        .unwrap();
        assert_eq!(tree.scroll_state(inner).unwrap().offset.y, 40.0);
        assert_eq!(tree.scroll_state(outer).unwrap().offset.y, 0.0);

        let mut inner_state = tree.scroll_state(inner).unwrap();
        inner_state.offset.y = 100.0;
        tree.set_scroll_state(inner, inner_state).unwrap();
        tree.wheel(
            root,
            WheelEvent::new(
                Point::new(10.0, 10.0),
                Point::new(0.0, 40.0),
                Modifiers::default(),
                Duration::ZERO,
            ),
        )
        .unwrap();
        assert_eq!(tree.scroll_state(inner).unwrap().offset.y, 100.0);
        assert_eq!(tree.scroll_state(outer).unwrap().offset.y, 40.0);
    }

    #[test]
    fn scroll_view_paint_emits_a_viewport_clip() {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(
                None,
                LayoutStyle {
                    width: Dimension::Points(100.0),
                    height: Dimension::Points(100.0),
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        let scroll = tree
            .create_node(
                Some(root),
                LayoutStyle {
                    width: Dimension::Points(100.0),
                    height: Dimension::Points(100.0),
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        tree.attach_scroll_view(scroll, Size::new(100.0, 100.0), Size::new(100.0, 300.0))
            .unwrap();
        tree.layout(root, crate::Constraints::loose(Size::new(100.0, 100.0)))
            .unwrap();
        let display_list = tree.paint(root).unwrap();
        assert!(
            display_list
                .commands()
                .iter()
                .any(|command| matches!(command, ui_core::DisplayCommand::PushClip(_)))
        );
    }
}
