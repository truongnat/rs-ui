//! Style-free state machines and geometry for future components.

use std::collections::BTreeSet;
use std::time::Duration;

use ui_core::{Point, Rect};

use crate::{BehaviorCommand, NodeId, ResizeAxis, ResizeConfig};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressableState {
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
    pub focus_visible: bool,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(u64);

impl LayerId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutsidePointerPolicy {
    #[default]
    Ignore,
    DismissAndContinue,
    DismissAndBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerSpec {
    pub node: NodeId,
    pub z_layer: i32,
    pub modal: bool,
    pub blocks_pointer: bool,
    pub trap_focus: bool,
    pub outside_pointer: OutsidePointerPolicy,
    pub dismiss_on_escape: bool,
    pub focus_scope: Option<NodeId>,
    pub initial_focus: Option<NodeId>,
}

impl Default for LayerSpec {
    fn default() -> Self {
        Self {
            node: NodeId(0),
            z_layer: 0,
            modal: false,
            blocks_pointer: false,
            trap_focus: false,
            outside_pointer: OutsidePointerPolicy::Ignore,
            dismiss_on_escape: false,
            focus_scope: None,
            initial_focus: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerEntry {
    pub id: LayerId,
    pub spec: LayerSpec,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerDismissReason {
    OutsidePointerDown,
    Escape,
    FocusOutside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerDismissRequest {
    pub layer: LayerId,
    pub node: NodeId,
    pub reason: LayerDismissReason,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutsidePointerResult {
    pub dismissed: Option<LayerDismissRequest>,
    pub blocked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LayerStack {
    entries: Vec<LayerEntry>,
    next_id: u64,
    next_sequence: u64,
}

impl LayerStack {
    pub fn open(&mut self, spec: LayerSpec) -> LayerId {
        self.next_id += 1;
        self.next_sequence += 1;
        let id = LayerId(self.next_id);
        self.entries.push(LayerEntry {
            id,
            spec,
            sequence: self.next_sequence,
        });
        id
    }

    pub fn close(&mut self, id: LayerId) -> Option<LayerEntry> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        Some(self.entries.remove(index))
    }

    pub fn get(&self, id: LayerId) -> Option<&LayerEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn layer_for_node(&self, node: NodeId) -> Option<LayerEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.spec.node == node)
            .max_by_key(|entry| (entry.spec.z_layer, entry.sequence))
            .copied()
    }

    pub fn ordered(&self) -> Vec<LayerEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by_key(|entry| (entry.spec.z_layer, entry.sequence));
        entries
    }

    pub fn topmost(&self) -> Option<LayerEntry> {
        self.entries
            .iter()
            .max_by_key(|entry| (entry.spec.z_layer, entry.sequence))
            .copied()
    }

    pub fn entries(&self) -> &[LayerEntry] {
        &self.entries
    }

    pub fn outside_pointer_down(
        &mut self,
        target: Option<NodeId>,
        mut is_inside: impl FnMut(NodeId, NodeId) -> bool,
    ) -> OutsidePointerResult {
        let Some(entry) = self.topmost() else {
            return OutsidePointerResult::default();
        };
        if target.is_some_and(|target| is_inside(entry.spec.node, target)) {
            return OutsidePointerResult::default();
        }
        let dismissed = match entry.spec.outside_pointer {
            OutsidePointerPolicy::Ignore => None,
            OutsidePointerPolicy::DismissAndContinue | OutsidePointerPolicy::DismissAndBlock => {
                Some(LayerDismissRequest {
                    layer: entry.id,
                    node: entry.spec.node,
                    reason: LayerDismissReason::OutsidePointerDown,
                })
            }
        };
        OutsidePointerResult {
            dismissed,
            blocked: entry.spec.modal
                || entry.spec.blocks_pointer
                || entry.spec.outside_pointer == OutsidePointerPolicy::DismissAndBlock,
        }
    }

    pub fn escape(&self) -> Option<LayerDismissRequest> {
        let entry = self.topmost()?;
        entry.spec.dismiss_on_escape.then_some(LayerDismissRequest {
            layer: entry.id,
            node: entry.spec.node,
            reason: LayerDismissReason::Escape,
        })
    }

    pub fn focus_outside(
        &self,
        target: Option<NodeId>,
        mut is_inside: impl FnMut(NodeId, NodeId) -> bool,
    ) -> Option<LayerDismissRequest> {
        let entry = self.topmost()?;
        let scope = entry.spec.focus_scope?;
        if target.is_some_and(|target| is_inside(scope, target)) {
            return None;
        }
        Some(LayerDismissRequest {
            layer: entry.id,
            node: entry.spec.node,
            reason: LayerDismissReason::FocusOutside,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalRelationship {
    pub logical_owner: NodeId,
    pub layer: LayerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopoverAlignment {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverConfig {
    pub preferred_side: PopoverSide,
    pub alignment: PopoverAlignment,
    pub offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverPlacement {
    pub side: PopoverSide,
    pub rect: Rect,
}

pub fn place_popover(
    anchor: Rect,
    size: (f32, f32),
    viewport: Rect,
    config: PopoverConfig,
) -> PopoverPlacement {
    let (width, height) = (size.0.max(0.0), size.1.max(0.0));
    let candidates = [
        config.preferred_side,
        opposite(config.preferred_side),
        rotate_side(config.preferred_side, true),
        rotate_side(config.preferred_side, false),
    ];
    let fits = |side| {
        let rect = place_on_side(anchor, width, height, side, config);
        rect.min.x >= viewport.min.x
            && rect.min.y >= viewport.min.y
            && rect.max.x <= viewport.max.x
            && rect.max.y <= viewport.max.y
    };
    let side = candidates
        .iter()
        .copied()
        .find(|side| fits(*side))
        .unwrap_or_else(|| {
            if fits(config.preferred_side) {
                config.preferred_side
            } else if fits(opposite(config.preferred_side)) {
                opposite(config.preferred_side)
            } else {
                config.preferred_side
            }
        });
    let mut rect = place_on_side(anchor, width, height, side, config);
    let max_x = (viewport.max.x - width).max(viewport.min.x);
    let max_y = (viewport.max.y - height).max(viewport.min.y);
    let min_x = viewport.min.x.min(max_x);
    let min_y = viewport.min.y.min(max_y);
    rect = Rect::from_min_max(
        Point::new(
            rect.min.x.clamp(min_x, max_x),
            rect.min.y.clamp(min_y, max_y),
        ),
        Point::new(
            (rect.min.x.clamp(min_x, max_x) + width).min(viewport.max.x),
            (rect.min.y.clamp(min_y, max_y) + height).min(viewport.max.y),
        ),
    );
    PopoverPlacement { side, rect }
}

fn place_on_side(
    anchor: Rect,
    width: f32,
    height: f32,
    side: PopoverSide,
    config: PopoverConfig,
) -> Rect {
    let cross = |start: f32, end: f32, extent: f32| match config.alignment {
        PopoverAlignment::Start => start,
        PopoverAlignment::Center => (start + end - extent) / 2.0,
        PopoverAlignment::End => end - extent,
    };
    let (x, y) = match side {
        PopoverSide::Top => (
            cross(anchor.min.x, anchor.max.x, width),
            anchor.min.y - config.offset - height,
        ),
        PopoverSide::Bottom => (
            cross(anchor.min.x, anchor.max.x, width),
            anchor.max.y + config.offset,
        ),
        PopoverSide::Left => (
            anchor.min.x - config.offset - width,
            cross(anchor.min.y, anchor.max.y, height),
        ),
        PopoverSide::Right => (
            anchor.max.x + config.offset,
            cross(anchor.min.y, anchor.max.y, height),
        ),
    };
    Rect::from_min_max(Point::new(x, y), Point::new(x + width, y + height))
}

fn opposite(side: PopoverSide) -> PopoverSide {
    match side {
        PopoverSide::Top => PopoverSide::Bottom,
        PopoverSide::Bottom => PopoverSide::Top,
        PopoverSide::Left => PopoverSide::Right,
        PopoverSide::Right => PopoverSide::Left,
    }
}

fn rotate_side(side: PopoverSide, clockwise: bool) -> PopoverSide {
    match (side, clockwise) {
        (PopoverSide::Top, true) | (PopoverSide::Bottom, false) => PopoverSide::Right,
        (PopoverSide::Right, true) | (PopoverSide::Left, false) => PopoverSide::Bottom,
        (PopoverSide::Bottom, true) | (PopoverSide::Top, false) => PopoverSide::Left,
        (PopoverSide::Left, true) | (PopoverSide::Right, false) => PopoverSide::Top,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipDelays {
    pub open: Duration,
    pub close: Duration,
}

impl Default for TooltipDelays {
    fn default() -> Self {
        Self {
            open: Duration::from_millis(500),
            close: Duration::from_millis(120),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TooltipTriggers {
    pub pointer_over_trigger: bool,
    pub pointer_over_tooltip: bool,
    pub trigger_focused: bool,
}

#[derive(Clone, Debug)]
pub struct TooltipController {
    delays: TooltipDelays,
    opened: bool,
    deadline: Option<Duration>,
    opening: bool,
}

impl TooltipController {
    pub fn new(delays: TooltipDelays) -> Self {
        Self {
            delays,
            opened: false,
            deadline: None,
            opening: false,
        }
    }

    pub const fn is_open(&self) -> bool {
        self.opened
    }

    pub fn update(&mut self, now: Duration, triggers: TooltipTriggers) -> bool {
        let wanted = triggers.pointer_over_trigger
            || triggers.pointer_over_tooltip
            || triggers.trigger_focused;
        if wanted {
            if self.opened {
                self.deadline = None;
            } else {
                if !self.opening {
                    self.deadline = Some(now + self.delays.open);
                    self.opening = true;
                }
                if self.deadline.is_some_and(|deadline| now >= deadline) {
                    self.opened = true;
                    self.deadline = None;
                }
            }
        } else if self.opened {
            if self.deadline.is_none() {
                self.deadline = Some(now + self.delays.close);
            }
            if self.deadline.is_some_and(|deadline| now >= deadline) {
                self.opened = false;
                self.opening = false;
                self.deadline = None;
            }
        } else {
            self.opening = false;
            self.deadline = None;
        }
        self.opened
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem<K> {
    pub key: K,
    pub label: String,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct MenuModel<K> {
    items: Vec<MenuItem<K>>,
    active: Option<usize>,
    typeahead: String,
    typeahead_deadline: Option<Duration>,
    typeahead_timeout: Duration,
    open: bool,
}

impl<K: Clone + Eq> MenuModel<K> {
    pub fn new(items: Vec<MenuItem<K>>, typeahead_timeout: Duration) -> Self {
        Self {
            items,
            active: None,
            typeahead: String::new(),
            typeahead_deadline: None,
            typeahead_timeout,
            open: false,
        }
    }

    pub fn items(&self) -> &[MenuItem<K>] {
        &self.items
    }

    pub const fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active_key(&self) -> Option<&K> {
        self.items.get(self.active?).map(|item| &item.key)
    }

    pub const fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.active = self.next_enabled(None, false);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.active = None;
        self.typeahead.clear();
        self.typeahead_deadline = None;
    }

    pub fn command(&mut self, command: BehaviorCommand, now: Duration) -> MenuOutcome<K> {
        if !self.open {
            return MenuOutcome::Ignored;
        }
        match command {
            BehaviorCommand::MoveNext | BehaviorCommand::MoveDown => {
                self.active = self.next_enabled(self.active, false);
                MenuOutcome::Moved(self.active)
            }
            BehaviorCommand::MovePrevious | BehaviorCommand::MoveUp => {
                self.active = self.next_enabled(self.active, true);
                MenuOutcome::Moved(self.active)
            }
            BehaviorCommand::MoveFirst => {
                self.active = self.edge_enabled(false);
                MenuOutcome::Moved(self.active)
            }
            BehaviorCommand::MoveLast => {
                self.active = self.edge_enabled(true);
                MenuOutcome::Moved(self.active)
            }
            BehaviorCommand::Activate => self
                .active
                .and_then(|index| self.items.get(index))
                .filter(|item| !item.disabled)
                .map(|item| MenuOutcome::Activate(item.key.clone()))
                .unwrap_or(MenuOutcome::Ignored),
            BehaviorCommand::Cancel => {
                self.close();
                MenuOutcome::Close
            }
            BehaviorCommand::MoveLeft
            | BehaviorCommand::MoveRight
            | BehaviorCommand::Increment
            | BehaviorCommand::Decrement => MenuOutcome::Ignored,
            BehaviorCommand::Character(character) => self.type_character(character, now),
        }
    }

    pub fn set_items(&mut self, items: Vec<MenuItem<K>>) {
        self.items = items;
        if self
            .active
            .is_some_and(|index| self.items.get(index).is_none_or(|item| item.disabled))
        {
            self.active = self.next_enabled(None, false);
        }
    }

    fn type_character(&mut self, character: char, now: Duration) -> MenuOutcome<K> {
        if !character.is_control() {
            if self
                .typeahead_deadline
                .is_none_or(|deadline| now > deadline)
            {
                self.typeahead.clear();
            }
            self.typeahead.push(character.to_ascii_lowercase());
            self.typeahead_deadline = Some(now + self.typeahead_timeout);
        }
        let query = self.typeahead.as_str();
        let start = self.active.map_or(0, |index| index + 1) % self.items.len().max(1);
        let found = (0..self.items.len())
            .map(|offset| (start + offset) % self.items.len().max(1))
            .find(|index| {
                !self.items[*index].disabled
                    && self.items[*index]
                        .label
                        .to_ascii_lowercase()
                        .starts_with(query)
            });
        self.active = found;
        MenuOutcome::Moved(found)
    }

    fn next_enabled(&self, from: Option<usize>, backwards: bool) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let length = self.items.len();
        let start = from.map_or(if backwards { length - 1 } else { 0 }, |index| {
            if backwards {
                (index + length - 1) % length
            } else {
                (index + 1) % length
            }
        });
        (0..length)
            .map(|offset| {
                if backwards {
                    (start + length - offset) % length
                } else {
                    (start + offset) % length
                }
            })
            .find(|index| !self.items[*index].disabled)
    }

    fn edge_enabled(&self, last: bool) -> Option<usize> {
        let iter: Box<dyn Iterator<Item = usize>> = if last {
            Box::new((0..self.items.len()).rev())
        } else {
            Box::new(0..self.items.len())
        };
        iter.into_iter().find(|index| !self.items[*index].disabled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOutcome<K> {
    Ignored,
    Moved(Option<usize>),
    Activate(K),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizablePrimitive {
    config: ResizeConfig,
    value: f32,
    drag_origin: Option<(f32, f32)>,
}

impl ResizablePrimitive {
    pub fn new(config: ResizeConfig, value: f32) -> Self {
        let mut result = Self {
            config,
            value,
            drag_origin: None,
        };
        result.value = result.clamp(value);
        result
    }

    pub const fn value(self) -> f32 {
        self.value
    }

    pub const fn axis(self) -> ResizeAxis {
        self.config.axis
    }

    pub fn begin_pointer(&mut self, position: Point) {
        let coordinate = self.coordinate(position);
        self.drag_origin = Some((coordinate, self.value));
    }

    pub fn pointer_move(&mut self, position: Point) -> bool {
        let Some((origin, value)) = self.drag_origin else {
            return false;
        };
        self.set_value(value + self.coordinate(position) - origin)
    }

    pub fn end_pointer(&mut self) {
        self.drag_origin = None;
    }

    pub fn keyboard(&mut self, command: BehaviorCommand) -> bool {
        match (self.config.axis, command) {
            (ResizeAxis::Horizontal, BehaviorCommand::MoveRight)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveDown)
            | (_, BehaviorCommand::MoveNext)
            | (_, BehaviorCommand::Activate) => self.set_value(self.value + self.config.step),
            (ResizeAxis::Horizontal, BehaviorCommand::MoveLeft)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveUp)
            | (_, BehaviorCommand::MovePrevious) => self.set_value(self.value - self.config.step),
            (ResizeAxis::Horizontal, BehaviorCommand::MoveFirst)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveFirst) => self.set_value(self.config.min),
            (ResizeAxis::Horizontal, BehaviorCommand::MoveLast)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveLast) => self.set_value(self.config.max),
            _ => false,
        }
    }

    pub fn increment(&mut self) -> bool {
        self.set_value(self.value + self.config.step)
    }

    pub fn decrement(&mut self) -> bool {
        self.set_value(self.value - self.config.step)
    }

    pub fn reset(&mut self) -> bool {
        self.set_value(self.config.reset)
    }

    fn set_value(&mut self, value: f32) -> bool {
        let value = self.clamp(value);
        if self.value == value {
            return false;
        }
        self.value = value;
        true
    }

    fn clamp(self, value: f32) -> f32 {
        value.clamp(
            self.config.min.min(self.config.max),
            self.config.max.max(self.config.min),
        )
    }

    fn coordinate(self, position: Point) -> f32 {
        match self.config.axis {
            ResizeAxis::Horizontal => position.x,
            ResizeAxis::Vertical => position.y,
        }
    }
}

/// The policy used by `SelectionModel::select_all`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderedSelectionModel<K: Ord> {
    multiple: bool,
    selected: BTreeSet<K>,
    anchor: Option<K>,
}

impl<K: Ord + Clone> OrderedSelectionModel<K> {
    pub fn single() -> Self {
        Self::new(false)
    }

    pub fn multiple() -> Self {
        Self::new(true)
    }

    pub fn new(multiple: bool) -> Self {
        Self {
            multiple,
            selected: BTreeSet::new(),
            anchor: None,
        }
    }

    pub const fn allows_multiple(&self) -> bool {
        self.multiple
    }

    pub fn selected(&self) -> &BTreeSet<K> {
        &self.selected
    }

    pub fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    pub fn is_selected(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub fn clear(&mut self) -> bool {
        self.anchor = None;
        let changed = !self.selected.is_empty();
        self.selected.clear();
        changed
    }

    pub fn select(&mut self, key: K) -> bool {
        let before = self.selected.clone();
        if !self.multiple {
            self.selected.clear();
        }
        self.selected.insert(key.clone());
        self.anchor = Some(key);
        self.selected != before
    }

    pub fn toggle(&mut self, key: K) -> bool {
        if !self.multiple {
            if self.selected.contains(&key) {
                self.selected.clear();
                self.anchor = None;
                return true;
            }
            return self.select(key);
        }
        if !self.selected.remove(&key) {
            self.selected.insert(key.clone());
        }
        self.anchor = Some(key);
        true
    }

    pub fn select_range(&mut self, ordered_keys: &[K], target: K, additive: bool) -> bool {
        if !self.multiple {
            return self.select(target);
        }
        let anchor = self.anchor.clone().unwrap_or_else(|| target.clone());
        let Some(anchor_index) = ordered_keys.iter().position(|key| key == &anchor) else {
            return self.select(target);
        };
        let Some(target_index) = ordered_keys.iter().position(|key| key == &target) else {
            return false;
        };
        if !additive {
            self.selected.clear();
        }
        for key in &ordered_keys[anchor_index.min(target_index)..=anchor_index.max(target_index)] {
            self.selected.insert(key.clone());
        }
        self.anchor = Some(anchor);
        true
    }

    pub fn select_all(&mut self, keys: &[K], mut is_selectable: impl FnMut(&K) -> bool) -> bool {
        if !self.multiple {
            return false;
        }
        let before = self.selected.clone();
        for key in keys.iter().filter(|key| is_selectable(key)) {
            self.selected.insert(key.clone());
        }
        self.selected != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccessibilityActionKind, AccessibilityRole, Pressable};
    use ui_core::{Point, Rect};

    #[test]
    fn pressable_exposes_button_semantics_and_single_press_action() {
        let semantics = Pressable::new(true).semantics();
        assert_eq!(semantics.role, AccessibilityRole::Button);
        assert_eq!(semantics.actions, [AccessibilityActionKind::Press]);
        assert_eq!(semantics.state.disabled, Some(true));
    }

    #[test]
    fn layer_stack_orders_topmost_and_dismisses_only_the_top_outside_layer() {
        let mut layers = LayerStack::default();
        let lower = layers.open(LayerSpec {
            node: NodeId(1),
            z_layer: 1,
            outside_pointer: OutsidePointerPolicy::DismissAndContinue,
            ..LayerSpec::default()
        });
        let upper = layers.open(LayerSpec {
            node: NodeId(2),
            z_layer: 2,
            outside_pointer: OutsidePointerPolicy::DismissAndBlock,
            dismiss_on_escape: true,
            ..LayerSpec::default()
        });
        assert_eq!(layers.topmost().unwrap().id, upper);
        let result = layers.outside_pointer_down(None, |_, _| false);
        assert_eq!(result.dismissed.unwrap().layer, upper);
        assert!(result.blocked);
        assert_eq!(layers.escape().unwrap().layer, upper);
        assert!(layers.close(upper).is_some());
        assert_eq!(layers.topmost().unwrap().id, lower);
    }

    #[test]
    fn popover_flips_and_shifts_inside_viewport() {
        let viewport = Rect::from_min_max(Point::new(0.0, 0.0), Point::new(100.0, 80.0));
        let anchor = Rect::from_min_max(Point::new(42.0, 62.0), Point::new(58.0, 70.0));
        let placement = place_popover(
            anchor,
            (90.0, 30.0),
            viewport,
            PopoverConfig {
                preferred_side: PopoverSide::Bottom,
                alignment: PopoverAlignment::Center,
                offset: 4.0,
            },
        );
        assert_eq!(placement.side, PopoverSide::Top);
        assert!(placement.rect.min.x >= viewport.min.x);
        assert!(placement.rect.max.x <= viewport.max.x);
        assert!(placement.rect.max.y <= viewport.max.y);
    }

    #[test]
    fn tooltip_waits_for_delay_and_keeps_pointer_safe_transition() {
        let mut tooltip = TooltipController::new(TooltipDelays {
            open: Duration::from_millis(100),
            close: Duration::from_millis(50),
        });
        assert!(!tooltip.update(
            Duration::ZERO,
            TooltipTriggers {
                pointer_over_trigger: true,
                ..TooltipTriggers::default()
            }
        ));
        assert!(tooltip.update(
            Duration::from_millis(100),
            TooltipTriggers {
                pointer_over_trigger: true,
                ..TooltipTriggers::default()
            }
        ));
        assert!(tooltip.update(
            Duration::from_millis(110),
            TooltipTriggers {
                pointer_over_tooltip: true,
                ..TooltipTriggers::default()
            }
        ));
        assert!(tooltip.update(Duration::from_millis(170), TooltipTriggers::default()));
        assert!(!tooltip.update(Duration::from_millis(220), TooltipTriggers::default()));
    }

    #[test]
    fn menu_navigation_skips_disabled_and_typeahead_activates_stable_key() {
        let mut menu = MenuModel::new(
            vec![
                MenuItem {
                    key: 1,
                    label: "Alpha".into(),
                    disabled: false,
                },
                MenuItem {
                    key: 2,
                    label: "Bravo".into(),
                    disabled: true,
                },
                MenuItem {
                    key: 3,
                    label: "Beta".into(),
                    disabled: false,
                },
            ],
            Duration::from_millis(700),
        );
        menu.open();
        assert_eq!(menu.active_index(), Some(0));
        assert_eq!(
            menu.command(BehaviorCommand::MoveNext, Duration::ZERO),
            MenuOutcome::Moved(Some(2))
        );
        assert_eq!(
            menu.command(BehaviorCommand::MoveFirst, Duration::ZERO),
            MenuOutcome::Moved(Some(0))
        );
        assert_eq!(
            menu.command(BehaviorCommand::Character('b'), Duration::ZERO),
            MenuOutcome::Moved(Some(2))
        );
        assert_eq!(
            menu.command(BehaviorCommand::Activate, Duration::ZERO),
            MenuOutcome::Activate(3)
        );
    }

    #[test]
    fn resizable_clamps_pointer_and_keyboard_updates_and_resets() {
        let mut resize = ResizablePrimitive::new(
            ResizeConfig {
                axis: ResizeAxis::Horizontal,
                min: 100.0,
                max: 300.0,
                step: 10.0,
                reset: 180.0,
            },
            150.0,
        );
        resize.begin_pointer(Point::new(20.0, 0.0));
        assert!(resize.pointer_move(Point::new(400.0, 0.0)));
        resize.end_pointer();
        assert_eq!(resize.value(), 300.0);
        assert!(!resize.increment());
        assert!(resize.reset());
        assert!(resize.keyboard(BehaviorCommand::MoveNext));
        assert_eq!(resize.value(), 190.0);
    }

    #[test]
    fn selection_uses_stable_keys_for_single_multi_and_range() {
        let keys = ["a", "b", "c", "d"];
        let mut single = OrderedSelectionModel::single();
        single.select("b");
        single.select("d");
        assert_eq!(single.selected(), &BTreeSet::from(["d"]));

        let mut multi = OrderedSelectionModel::multiple();
        multi.select("b");
        multi.select_range(&keys, "d", false);
        assert_eq!(multi.selected(), &BTreeSet::from(["b", "c", "d"]));
        multi.toggle("c");
        assert!(!multi.is_selected(&"c"));
        multi.select_all(&keys, |_| true);
        assert_eq!(multi.selected().len(), 4);
        assert!(multi.clear());
    }
}
