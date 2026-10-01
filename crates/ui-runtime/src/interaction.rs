use std::{collections::HashMap, time::Duration};

use ui_core::{Point, Rect, Transform};

use crate::{DirtyFlags, NodeId, RuntimeError, UiTree};

const DRAG_THRESHOLD: f32 = 4.0;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerEvents {
    #[default]
    Auto,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTestState {
    pub bounds: Rect,
    pub transform: Transform,
    pub clip: Option<Rect>,
    pub pointer_events: PointerEvents,
    pub visible: bool,
    pub z_order: i32,
}

impl Default for HitTestState {
    fn default() -> Self {
        Self {
            bounds: Rect::ZERO,
            transform: Transform::IDENTITY,
            clip: None,
            pointer_events: PointerEvents::Auto,
            visible: true,
            z_order: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FocusPolicy {
    pub focusable: bool,
    pub tab_index: i32,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub command: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerEvent {
    pub position: Point,
    pub button: Option<PointerButton>,
    pub buttons: u16,
    pub modifiers: Modifiers,
    pub click_count: u8,
    pub timestamp: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelEvent {
    pub position: Point,
    pub delta: Point,
    pub remaining_delta: Point,
    pub modifiers: Modifiers,
    pub timestamp: Duration,
}

impl WheelEvent {
    pub fn new(position: Point, delta: Point, modifiers: Modifiers, timestamp: Duration) -> Self {
        Self {
            position,
            delta,
            remaining_delta: delta,
            modifiers,
            timestamp,
        }
    }

    pub fn consume(&mut self, delta: Point) {
        self.remaining_delta = Point::new(
            consume_component(self.remaining_delta.x, delta.x),
            consume_component(self.remaining_delta.y, delta.y),
        );
    }
}

fn consume_component(remaining: f32, requested: f32) -> f32 {
    if remaining.signum() != requested.signum() {
        return remaining;
    }
    let amount = requested.abs().min(remaining.abs());
    remaining - remaining.signum() * amount
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    Escape,
    Character(char),
    Other(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
    pub repeat: bool,
    pub timestamp: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    PointerMove,
    PointerDown,
    PointerUp,
    PointerEnter,
    PointerLeave,
    Click,
    DoubleClick,
    Wheel,
    KeyDown,
    KeyUp,
    WindowFocus,
    WindowBlur,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    PointerMove(PointerEvent),
    PointerDown(PointerEvent),
    PointerUp(PointerEvent),
    PointerEnter(PointerEvent),
    PointerLeave(PointerEvent),
    Click(PointerEvent),
    DoubleClick(PointerEvent),
    Wheel(WheelEvent),
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    WindowFocus,
    WindowBlur,
}

impl EventKind {
    pub const fn event_type(&self) -> EventType {
        match self {
            Self::PointerMove(_) => EventType::PointerMove,
            Self::PointerDown(_) => EventType::PointerDown,
            Self::PointerUp(_) => EventType::PointerUp,
            Self::PointerEnter(_) => EventType::PointerEnter,
            Self::PointerLeave(_) => EventType::PointerLeave,
            Self::Click(_) => EventType::Click,
            Self::DoubleClick(_) => EventType::DoubleClick,
            Self::Wheel(_) => EventType::Wheel,
            Self::KeyDown(_) => EventType::KeyDown,
            Self::KeyUp(_) => EventType::KeyUp,
            Self::WindowFocus => EventType::WindowFocus,
            Self::WindowBlur => EventType::WindowBlur,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    Target,
    Bubble,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenerPhase {
    Capture,
    Bubble,
}

pub struct Event {
    kind: EventKind,
    target: Option<NodeId>,
    current_target: Option<NodeId>,
    phase: EventPhase,
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_propagation_stopped: bool,
}

impl Event {
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            target: None,
            current_target: None,
            phase: EventPhase::Global,
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
        }
    }

    pub const fn kind(&self) -> &EventKind {
        &self.kind
    }

    pub const fn event_type(&self) -> EventType {
        self.kind.event_type()
    }

    pub const fn target(&self) -> Option<NodeId> {
        self.target
    }

    pub const fn current_target(&self) -> Option<NodeId> {
        self.current_target
    }

    pub const fn phase(&self) -> EventPhase {
        self.phase
    }

    pub const fn default_prevented(&self) -> bool {
        self.default_prevented
    }

    pub const fn propagation_stopped(&self) -> bool {
        self.propagation_stopped
    }

    pub fn prevent_default(&mut self) {
        self.default_prevented = true;
    }

    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    pub fn stop_immediate_propagation(&mut self) {
        self.immediate_propagation_stopped = true;
        self.propagation_stopped = true;
    }

    pub fn consume_wheel(&mut self, delta: Point) {
        if let EventKind::Wheel(wheel) = &mut self.kind {
            wheel.consume(delta);
        }
    }

    pub const fn wheel_remaining(&self) -> Option<Point> {
        match &self.kind {
            EventKind::Wheel(wheel) => Some(wheel.remaining_delta),
            _ => None,
        }
    }

    fn set_dispatch_state(&mut self, target: NodeId, current_target: NodeId, phase: EventPhase) {
        self.target = Some(target);
        self.current_target = Some(current_target);
        self.phase = phase;
        self.immediate_propagation_stopped = false;
    }
}

pub type EventHandler = Box<dyn FnMut(&mut Event)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventListenerId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    CommandOrControl(char),
    Escape,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerState {
    pub position: Point,
    pub hovered_path: Vec<NodeId>,
    pub pressed_target: Option<NodeId>,
    pub captured_target: Option<NodeId>,
    pub dragging: bool,
}

#[derive(Clone, Debug)]
pub struct FocusScope {
    node: NodeId,
    trap_focus: bool,
    restore_focus: bool,
    initial_focus: Option<NodeId>,
    previous_focus: Option<NodeId>,
}

impl FocusScope {
    pub const fn node(&self) -> NodeId {
        self.node
    }

    pub const fn traps_focus(&self) -> bool {
        self.trap_focus
    }

    pub const fn initial_focus(&self) -> Option<NodeId> {
        self.initial_focus
    }
}

#[derive(Clone, Debug, Default)]
pub struct FocusManager {
    focused: Option<NodeId>,
    previous: Option<NodeId>,
    scopes: Vec<FocusScope>,
    traversal_order: Vec<NodeId>,
}

impl FocusManager {
    pub const fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    pub const fn previous(&self) -> Option<NodeId> {
        self.previous
    }

    pub fn scopes(&self) -> &[FocusScope] {
        &self.scopes
    }

    pub fn traversal_order(&self) -> &[NodeId] {
        &self.traversal_order
    }
}

struct EventListener {
    id: EventListenerId,
    event_type: EventType,
    phase: ListenerPhase,
    handler: EventHandler,
}

#[derive(Default)]
pub(crate) struct InteractionRuntime {
    pointer: PointerState,
    focus: FocusManager,
    next_listener_id: u64,
    listeners: HashMap<NodeId, Vec<EventListener>>,
    shortcuts: Vec<(Shortcut, EventHandler)>,
    press_position: Option<Point>,
    press_button: Option<PointerButton>,
    press_timestamp: Duration,
    press_cancelled: bool,
    last_click: Option<(NodeId, Duration)>,
}

impl InteractionRuntime {
    pub(crate) fn remove_node(&mut self, node: NodeId) {
        self.listeners.remove(&node);
        self.pointer.hovered_path.retain(|id| *id != node);
        if self.pointer.pressed_target == Some(node) {
            self.pointer.pressed_target = None;
        }
        if self.pointer.captured_target == Some(node) {
            self.pointer.captured_target = None;
        }
        if self.focus.focused == Some(node) {
            self.focus.previous = self.focus.focused;
            self.focus.focused = None;
        }
        if self.focus.previous == Some(node) {
            self.focus.previous = None;
        }
        self.focus.traversal_order.retain(|id| *id != node);
        self.focus.scopes.retain(|scope| scope.node != node);
    }
}

impl UiTree {
    pub fn set_hit_test_state(
        &mut self,
        node: NodeId,
        state: HitTestState,
    ) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        target.hit_test = state;
        target.hit_test_overridden = true;
        target.dirty.insert(DirtyFlags::HIT_TEST);
        Ok(())
    }

    pub fn hit_test_state(&self, node: NodeId) -> Option<HitTestState> {
        self.nodes.get(&node).map(|target| target.hit_test)
    }

    pub(crate) fn update_default_hit_test_state(&mut self, node: NodeId, rect: Rect) {
        let Some(target) = self.nodes.get(&node) else {
            return;
        };
        if target.hit_test_overridden {
            return;
        }
        let parent_min = target
            .parent
            .and_then(|parent| self.nodes.get(&parent))
            .and_then(|parent| parent.layout_rect())
            .map(|parent| parent.min)
            .unwrap_or(Point::ZERO);
        if let Some(target) = self.nodes.get_mut(&node) {
            target.hit_test.bounds = Rect::from_min_size(Point::ZERO, rect.size());
            target.hit_test.transform =
                Transform::translation(rect.min.x - parent_min.x, rect.min.y - parent_min.y);
            target.dirty.remove(DirtyFlags::HIT_TEST);
        }
    }

    pub fn hit_test(&self, root: NodeId, position: Point) -> Result<Vec<NodeId>, RuntimeError> {
        self.nodes
            .get(&root)
            .ok_or(RuntimeError::UnknownNode(root))?;
        Ok(self
            .hit_test_node(root, position, Transform::IDENTITY)
            .unwrap_or_default())
    }

    fn hit_test_node(
        &self,
        node: NodeId,
        position: Point,
        parent_transform: Transform,
    ) -> Option<Vec<NodeId>> {
        let target = self.nodes.get(&node)?;
        if !target.hit_test.visible {
            return None;
        }
        let world_transform = target.hit_test.transform.then(parent_transform);
        let inverse = world_transform.inverse()?;
        let local_position = inverse.transform_point(position);
        if target
            .hit_test
            .clip
            .is_some_and(|clip| !clip.contains(local_position))
        {
            return None;
        }
        let mut children = target.children.clone();
        children.sort_by_key(|child| self.nodes[child].hit_test.z_order);
        for child in children.into_iter().rev() {
            if let Some(mut path) = self.hit_test_node(child, position, world_transform) {
                path.insert(0, node);
                return Some(path);
            }
        }
        if target.hit_test.pointer_events == PointerEvents::Auto
            && target.hit_test.bounds.contains(local_position)
        {
            return Some(vec![node]);
        }
        None
    }

    pub fn pointer_state(&self) -> &PointerState {
        &self.interaction_runtime.pointer
    }

    pub fn capture_pointer(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.interaction_runtime.pointer.captured_target = Some(node);
        Ok(())
    }

    pub fn release_pointer(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if self.interaction_runtime.pointer.captured_target == Some(node) {
            self.interaction_runtime.pointer.captured_target = None;
        }
        Ok(())
    }

    pub fn pointer_move(
        &mut self,
        root: NodeId,
        pointer: PointerEvent,
    ) -> Result<(), RuntimeError> {
        self.update_hover_path(root, pointer.position, pointer)?;
        if let Some(start) = self.interaction_runtime.press_position
            && distance(start, pointer.position) > DRAG_THRESHOLD
        {
            self.interaction_runtime.pointer.dragging = true;
            self.interaction_runtime.press_cancelled = true;
        }
        let target = self
            .interaction_runtime
            .pointer
            .captured_target
            .or_else(|| {
                self.interaction_runtime
                    .pointer
                    .hovered_path
                    .last()
                    .copied()
            });
        if let Some(target) = target {
            self.dispatch_to_target(target, Event::new(EventKind::PointerMove(pointer)));
        }
        Ok(())
    }

    pub fn pointer_down(
        &mut self,
        root: NodeId,
        pointer: PointerEvent,
    ) -> Result<(), RuntimeError> {
        self.update_hover_path(root, pointer.position, pointer)?;
        let target = self
            .interaction_runtime
            .pointer
            .captured_target
            .or_else(|| {
                self.interaction_runtime
                    .pointer
                    .hovered_path
                    .last()
                    .copied()
            });
        self.interaction_runtime.pointer.pressed_target = target;
        self.interaction_runtime.pointer.dragging = false;
        self.interaction_runtime.press_position = Some(pointer.position);
        self.interaction_runtime.press_button = pointer.button;
        self.interaction_runtime.press_timestamp = pointer.timestamp;
        self.interaction_runtime.press_cancelled = false;
        if let Some(target) = target {
            self.set_pressed(target, true);
            if self
                .nodes
                .get(&target)
                .is_some_and(|node| node.focus_policy.focusable)
            {
                let _ = self.request_focus(target)?;
            }
            self.dispatch_to_target(target, Event::new(EventKind::PointerDown(pointer)));
        }
        Ok(())
    }

    pub fn pointer_up(&mut self, root: NodeId, pointer: PointerEvent) -> Result<(), RuntimeError> {
        self.update_hover_path(root, pointer.position, pointer)?;
        let pressed = self.interaction_runtime.pointer.pressed_target;
        let target = self
            .interaction_runtime
            .pointer
            .captured_target
            .or_else(|| {
                self.interaction_runtime
                    .pointer
                    .hovered_path
                    .last()
                    .copied()
            });
        if let Some(target) = target {
            self.dispatch_to_target(target, Event::new(EventKind::PointerUp(pointer)));
        }
        let is_click = pressed.is_some_and(|pressed| {
            Some(pressed) == target
                && self.interaction_runtime.press_button == pointer.button
                && !self.interaction_runtime.press_cancelled
                && self
                    .interaction_runtime
                    .press_position
                    .is_some_and(|start| distance(start, pointer.position) <= DRAG_THRESHOLD)
        });
        if is_click {
            let pressed = pressed.unwrap();
            let click_count = self.next_click_count(pressed, pointer.timestamp);
            let mut click = pointer;
            click.click_count = click_count;
            self.dispatch_to_target(pressed, Event::new(EventKind::Click(click)));
            if click_count == 2 {
                self.dispatch_to_target(pressed, Event::new(EventKind::DoubleClick(click)));
            }
        }
        if let Some(pressed) = pressed {
            self.set_pressed(pressed, false);
        }
        self.interaction_runtime.pointer.pressed_target = None;
        self.interaction_runtime.pointer.captured_target = None;
        self.interaction_runtime.pointer.dragging = false;
        self.interaction_runtime.press_position = None;
        self.interaction_runtime.press_button = None;
        self.interaction_runtime.press_cancelled = false;
        Ok(())
    }

    fn next_click_count(&mut self, target: NodeId, timestamp: Duration) -> u8 {
        let count = match self.interaction_runtime.last_click {
            Some((last_target, last_timestamp))
                if last_target == target
                    && timestamp.saturating_sub(last_timestamp) <= DOUBLE_CLICK_INTERVAL =>
            {
                2
            }
            _ => 1,
        };
        self.interaction_runtime.last_click = Some((target, timestamp));
        count
    }

    fn update_hover_path(
        &mut self,
        root: NodeId,
        position: Point,
        pointer: PointerEvent,
    ) -> Result<(), RuntimeError> {
        let new_path = self.hit_test(root, position)?;
        let old_path = self.interaction_runtime.pointer.hovered_path.clone();
        self.interaction_runtime.pointer.position = position;
        let common = old_path
            .iter()
            .zip(new_path.iter())
            .take_while(|(old, new)| old == new)
            .count();
        for node in old_path[common..].iter().rev().copied() {
            self.set_hovered(node, false);
            self.dispatch_at_target(node, Event::new(EventKind::PointerLeave(pointer)));
        }
        for node in new_path[common..].iter().copied() {
            self.set_hovered(node, true);
            self.dispatch_at_target(node, Event::new(EventKind::PointerEnter(pointer)));
        }
        self.interaction_runtime.pointer.hovered_path = new_path;
        Ok(())
    }

    fn set_hovered(&mut self, node: NodeId, hovered: bool) {
        if let Some(target) = self.nodes.get_mut(&node)
            && target.interaction.hovered != hovered
        {
            target.interaction.hovered = hovered;
            target.dirty.insert(DirtyFlags::PAINT);
        }
    }

    fn set_pressed(&mut self, node: NodeId, pressed: bool) {
        if let Some(target) = self.nodes.get_mut(&node)
            && target.interaction.pressed != pressed
        {
            target.interaction.pressed = pressed;
            target.dirty.insert(DirtyFlags::PAINT);
        }
    }

    pub fn wheel(&mut self, root: NodeId, wheel: WheelEvent) -> Result<(), RuntimeError> {
        let wheel = WheelEvent {
            remaining_delta: wheel.delta,
            ..wheel
        };
        let path = self.hit_test(root, wheel.position)?;
        let scroll_targets = path
            .iter()
            .rev()
            .copied()
            .filter(|node| self.nodes[node].scrollable)
            .collect::<Vec<_>>();
        if scroll_targets.is_empty() {
            if let Some(target) = path.last().copied() {
                self.dispatch_to_target(target, Event::new(EventKind::Wheel(wheel)));
            }
            return Ok(());
        }
        let mut remaining = wheel.delta;
        for target in scroll_targets {
            let event_wheel = WheelEvent {
                remaining_delta: remaining,
                ..wheel
            };
            let event = self.dispatch_to_target(target, Event::new(EventKind::Wheel(event_wheel)));
            remaining = event.wheel_remaining().unwrap_or(Point::ZERO);
            if remaining == Point::ZERO || event.default_prevented() {
                break;
            }
        }
        Ok(())
    }

    pub fn set_scrollable(&mut self, node: NodeId, scrollable: bool) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        target.scrollable = scrollable;
        Ok(())
    }

    pub fn add_event_listener(
        &mut self,
        node: NodeId,
        event_type: EventType,
        phase: ListenerPhase,
        handler: impl FnMut(&mut Event) + 'static,
    ) -> Result<EventListenerId, RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let id = EventListenerId(self.interaction_runtime.next_listener_id);
        self.interaction_runtime.next_listener_id += 1;
        self.interaction_runtime
            .listeners
            .entry(node)
            .or_default()
            .push(EventListener {
                id,
                event_type,
                phase,
                handler: Box::new(handler),
            });
        Ok(id)
    }

    pub fn remove_event_listener(&mut self, node: NodeId, id: EventListenerId) {
        if let Some(listeners) = self.interaction_runtime.listeners.get_mut(&node) {
            listeners.retain(|listener| listener.id != id);
        }
    }

    pub fn dispatch(&mut self, root: NodeId, event: Event) -> Result<Event, RuntimeError> {
        self.nodes
            .get(&root)
            .ok_or(RuntimeError::UnknownNode(root))?;
        match event.kind.event_type() {
            EventType::KeyDown | EventType::KeyUp => self.dispatch_key(root, event),
            _ => Ok(self.dispatch_to_target(root, event)),
        }
    }

    pub fn dispatch_to_node(
        &mut self,
        target: NodeId,
        event: Event,
    ) -> Result<Event, RuntimeError> {
        self.nodes
            .get(&target)
            .ok_or(RuntimeError::UnknownNode(target))?;
        Ok(self.dispatch_to_target(target, event))
    }

    pub fn window_focus(&mut self, root: NodeId) -> Result<Event, RuntimeError> {
        self.dispatch(root, Event::new(EventKind::WindowFocus))
    }

    pub fn window_blur(&mut self, root: NodeId) -> Result<Event, RuntimeError> {
        self.dispatch(root, Event::new(EventKind::WindowBlur))
    }

    fn dispatch_key(&mut self, root: NodeId, mut event: Event) -> Result<Event, RuntimeError> {
        let target = self.interaction_runtime.focus.focused.unwrap_or(root);
        self.dispatch_event_along_path(target, &mut event);
        if event.event_type() == EventType::KeyDown {
            self.dispatch_shortcuts(&mut event);
            if !event.default_prevented() {
                let tab = matches!(
                    event.kind(),
                    EventKind::KeyDown(KeyEvent { key: Key::Tab, .. })
                );
                if tab {
                    let shift = matches!(
                        event.kind(),
                        EventKind::KeyDown(KeyEvent {
                            modifiers: Modifiers { shift: true, .. },
                            ..
                        })
                    );
                    let _ = self.traverse_focus(shift)?;
                }
            }
        }
        Ok(event)
    }

    fn dispatch_shortcuts(&mut self, event: &mut Event) {
        let shortcut = match event.kind() {
            EventKind::KeyDown(key) if key.key == Key::Escape => Some(Shortcut::Escape),
            EventKind::KeyDown(key) => match key.key {
                Key::Character(character) if key.modifiers.command || key.modifiers.control => {
                    Some(Shortcut::CommandOrControl(character.to_ascii_lowercase()))
                }
                _ => None,
            },
            _ => None,
        };
        let Some(shortcut) = shortcut else {
            return;
        };
        for (registered, handler) in &mut self.interaction_runtime.shortcuts {
            if *registered == shortcut {
                event.phase = EventPhase::Global;
                event.current_target = None;
                handler(event);
                if event.immediate_propagation_stopped {
                    break;
                }
            }
        }
    }

    pub fn add_global_shortcut(
        &mut self,
        shortcut: Shortcut,
        handler: impl FnMut(&mut Event) + 'static,
    ) {
        self.interaction_runtime
            .shortcuts
            .push((shortcut, Box::new(handler)));
    }

    fn dispatch_to_target(&mut self, target: NodeId, mut event: Event) -> Event {
        self.dispatch_event_along_path(target, &mut event);
        event
    }

    fn dispatch_at_target(&mut self, target: NodeId, mut event: Event) {
        event.set_dispatch_state(target, target, EventPhase::Target);
        self.invoke_listeners(target, &mut event, ListenerPhase::Capture);
        if !event.immediate_propagation_stopped {
            self.invoke_listeners(target, &mut event, ListenerPhase::Bubble);
        }
    }

    fn dispatch_event_along_path(&mut self, target: NodeId, event: &mut Event) {
        let path = self.path_to_root(target);
        for node in path.iter().take(path.len().saturating_sub(1)).copied() {
            event.set_dispatch_state(target, node, EventPhase::Capture);
            self.invoke_listeners(node, event, ListenerPhase::Capture);
            if event.propagation_stopped {
                return;
            }
        }
        event.set_dispatch_state(target, target, EventPhase::Target);
        self.invoke_listeners(target, event, ListenerPhase::Capture);
        if !event.immediate_propagation_stopped {
            self.invoke_listeners(target, event, ListenerPhase::Bubble);
        }
        if event.propagation_stopped {
            return;
        }
        for node in path.iter().rev().skip(1).copied() {
            event.set_dispatch_state(target, node, EventPhase::Bubble);
            self.invoke_listeners(node, event, ListenerPhase::Bubble);
            if event.propagation_stopped {
                return;
            }
        }
    }

    fn path_to_root(&self, target: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut current = Some(target);
        while let Some(node) = current {
            path.push(node);
            current = self.nodes.get(&node).and_then(|target| target.parent);
        }
        path.reverse();
        path
    }

    fn invoke_listeners(&mut self, node: NodeId, event: &mut Event, phase: ListenerPhase) {
        let Some(listeners) = self.interaction_runtime.listeners.get_mut(&node) else {
            return;
        };
        for listener in listeners.iter_mut() {
            if listener.event_type == event.event_type() && listener.phase == phase {
                (listener.handler)(event);
                if event.immediate_propagation_stopped {
                    break;
                }
            }
        }
    }

    pub fn focus_manager(&self) -> &FocusManager {
        &self.interaction_runtime.focus
    }

    pub fn set_focus_policy(
        &mut self,
        node: NodeId,
        policy: FocusPolicy,
    ) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        target.focus_policy = policy;
        target.dirty.insert(DirtyFlags::ACCESSIBILITY);
        self.rebuild_focus_order();
        Ok(())
    }

    pub fn request_focus(&mut self, node: NodeId) -> Result<bool, RuntimeError> {
        let target = self
            .nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if !target.focus_policy.focusable || target.focus_policy.disabled {
            return Ok(false);
        }
        if let Some(scope) = self.interaction_runtime.focus.scopes.last()
            && scope.trap_focus
            && !self.is_descendant(node, scope.node)
        {
            return Ok(false);
        }
        self.set_focused(node);
        Ok(true)
    }

    fn set_focused(&mut self, node: NodeId) {
        let previous = self.interaction_runtime.focus.focused;
        if previous == Some(node) {
            return;
        }
        if let Some(previous) = previous {
            self.set_focused_state(previous, false);
        }
        self.interaction_runtime.focus.previous = previous;
        self.interaction_runtime.focus.focused = Some(node);
        self.set_focused_state(node, true);
    }

    fn set_focused_state(&mut self, node: NodeId, focused: bool) {
        if let Some(target) = self.nodes.get_mut(&node)
            && target.interaction.focused != focused
        {
            target.interaction.focused = focused;
            target.dirty.insert(DirtyFlags::PAINT);
        }
    }

    pub fn clear_focus(&mut self) {
        if let Some(previous) = self.interaction_runtime.focus.focused.take() {
            self.interaction_runtime.focus.previous = Some(previous);
            self.set_focused_state(previous, false);
        }
    }

    pub fn restore_focus(&mut self) -> Result<bool, RuntimeError> {
        let previous = self.interaction_runtime.focus.previous;
        let Some(previous) = previous else {
            return Ok(false);
        };
        self.request_focus(previous)
    }

    pub fn push_focus_scope(
        &mut self,
        node: NodeId,
        trap_focus: bool,
        restore_focus: bool,
        initial_focus: Option<NodeId>,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if let Some(initial) = initial_focus {
            self.nodes
                .get(&initial)
                .ok_or(RuntimeError::UnknownNode(initial))?;
            if !self.is_descendant(initial, node) {
                return Err(RuntimeError::FocusTargetOutsideScope);
            }
        }
        let scope = FocusScope {
            node,
            trap_focus,
            restore_focus,
            initial_focus,
            previous_focus: self.interaction_runtime.focus.focused,
        };
        self.interaction_runtime.focus.scopes.push(scope);
        let focus_target = initial_focus.or_else(|| {
            self.interaction_runtime
                .focus
                .traversal_order
                .iter()
                .copied()
                .find(|candidate| self.is_descendant(*candidate, node))
        });
        if let Some(focus_target) = focus_target {
            let _ = self.request_focus(focus_target)?;
        }
        Ok(())
    }

    pub fn pop_focus_scope(&mut self, node: NodeId) -> Result<bool, RuntimeError> {
        let Some(index) = self
            .interaction_runtime
            .focus
            .scopes
            .iter()
            .rposition(|scope| scope.node == node)
        else {
            return Ok(false);
        };
        let scope = self.interaction_runtime.focus.scopes.remove(index);
        if scope.restore_focus {
            if let Some(previous) = scope.previous_focus {
                let _ = self.request_focus(previous)?;
            } else {
                self.clear_focus();
            }
        }
        Ok(true)
    }

    fn traverse_focus(&mut self, backwards: bool) -> Result<bool, RuntimeError> {
        self.rebuild_focus_order();
        let order = if let Some(scope) = self.interaction_runtime.focus.scopes.last() {
            if scope.trap_focus {
                self.interaction_runtime
                    .focus
                    .traversal_order
                    .iter()
                    .copied()
                    .filter(|node| self.is_descendant(*node, scope.node))
                    .collect::<Vec<_>>()
            } else {
                self.interaction_runtime.focus.traversal_order.clone()
            }
        } else {
            self.interaction_runtime.focus.traversal_order.clone()
        };
        if order.is_empty() {
            return Ok(false);
        }
        let current_index = self
            .interaction_runtime
            .focus
            .focused
            .and_then(|focused| order.iter().position(|node| *node == focused));
        let next_index = match (current_index, backwards) {
            (Some(index), false) => (index + 1) % order.len(),
            (Some(index), true) => (index + order.len() - 1) % order.len(),
            (None, false) => 0,
            (None, true) => order.len() - 1,
        };
        self.request_focus(order[next_index])
    }

    fn rebuild_focus_order(&mut self) {
        let mut document_order = Vec::new();
        let mut roots = self
            .nodes
            .values()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id)
            .collect::<Vec<_>>();
        roots.sort();
        for root in roots {
            self.collect_document_order(root, &mut document_order);
        }
        let mut focusable = document_order
            .into_iter()
            .enumerate()
            .filter_map(|(order, node)| {
                let policy = self.nodes.get(&node)?.focus_policy;
                (policy.focusable && !policy.disabled && policy.tab_index >= 0).then_some((
                    node,
                    policy.tab_index,
                    order,
                ))
            })
            .collect::<Vec<_>>();
        focusable.sort_by_key(|(_, tab_index, order)| {
            let group = if *tab_index > 0 { 0 } else { 1 };
            (group, *tab_index, *order)
        });
        self.interaction_runtime.focus.traversal_order =
            focusable.into_iter().map(|(node, _, _)| node).collect();
    }

    fn collect_document_order(&self, node: NodeId, order: &mut Vec<NodeId>) {
        order.push(node);
        if let Some(target) = self.nodes.get(&node) {
            for child in &target.children {
                self.collect_document_order(*child, order);
            }
        }
    }

    fn is_descendant(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.nodes.get(&candidate).and_then(|target| target.parent);
        }
        false
    }
}

fn distance(first: Point, second: Point) -> f32 {
    let delta = second - first;
    (delta.x * delta.x + delta.y * delta.y).sqrt()
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::{Dimension, LayoutStyle, PaintState};
    use ui_core::Size;

    fn tree_with_root() -> (UiTree, NodeId) {
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
        (tree, root)
    }

    fn hit_state(transform: Transform, z_order: i32) -> HitTestState {
        HitTestState {
            bounds: Rect::from_min_size(Point::ZERO, Size::new(40.0, 40.0)),
            transform,
            z_order,
            ..HitTestState::default()
        }
    }

    #[test]
    fn hit_testing_returns_nested_path_with_transform_clip_and_z_order() {
        let (mut tree, root) = tree_with_root();
        let parent = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(Some(parent), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let overlap = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(
            root,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(100.0, 100.0)),
                ..HitTestState::default()
            },
        )
        .unwrap();
        tree.set_hit_test_state(
            parent,
            HitTestState {
                clip: Some(Rect::from_min_size(Point::ZERO, Size::new(30.0, 30.0))),
                transform: Transform::translation(10.0, 10.0),
                ..hit_state(Transform::translation(10.0, 10.0), 0)
            },
        )
        .unwrap();
        tree.set_hit_test_state(child, hit_state(Transform::translation(8.0, 8.0), 0))
            .unwrap();
        tree.set_hit_test_state(overlap, hit_state(Transform::translation(50.0, 10.0), 2))
            .unwrap();

        assert_eq!(
            tree.hit_test(root, Point::new(25.0, 25.0)).unwrap(),
            vec![root, parent, child]
        );
        assert_eq!(
            tree.hit_test(root, Point::new(55.0, 25.0)).unwrap(),
            vec![root, overlap]
        );
        tree.set_hit_test_state(
            overlap,
            HitTestState {
                pointer_events: PointerEvents::None,
                ..hit_state(Transform::translation(50.0, 10.0), 2)
            },
        )
        .unwrap();
        assert_eq!(
            tree.hit_test(root, Point::new(55.0, 25.0)).unwrap(),
            vec![root]
        );
    }

    #[test]
    fn event_dispatch_runs_capture_target_bubble_and_honors_prevent_default() {
        let (mut tree, root) = tree_with_root();
        let parent = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(Some(parent), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let add = |tree: &mut UiTree,
                   node: NodeId,
                   phase: ListenerPhase,
                   label: &'static str,
                   calls: Rc<RefCell<Vec<&'static str>>>| {
            tree.add_event_listener(node, EventType::PointerDown, phase, move |event| {
                calls.borrow_mut().push(label);
                if label == "target" {
                    event.prevent_default();
                }
            })
            .unwrap();
        };
        add(
            &mut tree,
            root,
            ListenerPhase::Capture,
            "root-capture",
            calls.clone(),
        );
        add(
            &mut tree,
            parent,
            ListenerPhase::Capture,
            "parent-capture",
            calls.clone(),
        );
        add(
            &mut tree,
            child,
            ListenerPhase::Bubble,
            "target",
            calls.clone(),
        );
        add(
            &mut tree,
            root,
            ListenerPhase::Bubble,
            "root-bubble",
            calls.clone(),
        );

        let event = tree
            .dispatch_to_node(
                child,
                Event::new(EventKind::PointerDown(PointerEvent::default())),
            )
            .unwrap();
        assert_eq!(
            *calls.borrow(),
            vec!["root-capture", "parent-capture", "target", "root-bubble"]
        );
        assert!(event.default_prevented());
    }

    #[test]
    fn pointer_capture_and_drag_threshold_gate_clicks() {
        let (mut tree, root) = tree_with_root();
        let child = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(
            root,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(100.0, 100.0)),
                ..HitTestState::default()
            },
        )
        .unwrap();
        tree.set_hit_test_state(child, hit_state(Transform::IDENTITY, 0))
            .unwrap();
        let pointer_up_targets = Rc::new(RefCell::new(Vec::new()));
        let clicks = Rc::new(RefCell::new(Vec::new()));
        let up_targets = pointer_up_targets.clone();
        tree.add_event_listener(
            child,
            EventType::PointerUp,
            ListenerPhase::Bubble,
            move |event| {
                up_targets.borrow_mut().push(event.current_target());
            },
        )
        .unwrap();
        let click_counts = clicks.clone();
        tree.add_event_listener(
            child,
            EventType::Click,
            ListenerPhase::Bubble,
            move |event| {
                if let EventKind::Click(pointer) = event.kind() {
                    click_counts.borrow_mut().push(pointer.click_count);
                }
            },
        )
        .unwrap();

        let down = PointerEvent {
            position: Point::new(10.0, 10.0),
            button: Some(PointerButton::Primary),
            timestamp: Duration::from_millis(1),
            ..PointerEvent::default()
        };
        tree.pointer_down(root, down).unwrap();
        tree.capture_pointer(child).unwrap();
        tree.pointer_move(
            root,
            PointerEvent {
                position: Point::new(30.0, 30.0),
                timestamp: Duration::from_millis(10),
                ..down
            },
        )
        .unwrap();
        tree.pointer_up(
            root,
            PointerEvent {
                position: Point::new(30.0, 30.0),
                timestamp: Duration::from_millis(20),
                ..down
            },
        )
        .unwrap();
        assert_eq!(*pointer_up_targets.borrow(), vec![Some(child)]);
        assert!(clicks.borrow().is_empty());

        for timestamp in [30, 100] {
            tree.pointer_down(
                root,
                PointerEvent {
                    timestamp: Duration::from_millis(timestamp),
                    ..down
                },
            )
            .unwrap();
            tree.pointer_up(
                root,
                PointerEvent {
                    timestamp: Duration::from_millis(timestamp + 1),
                    ..down
                },
            )
            .unwrap();
        }
        assert_eq!(*clicks.borrow(), vec![1, 2]);
    }

    #[test]
    fn wheel_routes_remaining_delta_from_inner_to_outer_scrollable_node() {
        let (mut tree, root) = tree_with_root();
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
        tree.set_scrollable(outer, true).unwrap();
        tree.set_scrollable(inner, true).unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let inner_calls = calls.clone();
        tree.add_event_listener(
            inner,
            EventType::Wheel,
            ListenerPhase::Bubble,
            move |event| {
                inner_calls
                    .borrow_mut()
                    .push(("inner", event.wheel_remaining()));
                event.consume_wheel(Point::new(3.0, 0.0));
            },
        )
        .unwrap();
        let outer_calls = calls.clone();
        tree.add_event_listener(
            outer,
            EventType::Wheel,
            ListenerPhase::Bubble,
            move |event| {
                outer_calls
                    .borrow_mut()
                    .push(("outer", event.wheel_remaining()));
                event.consume_wheel(Point::new(100.0, 0.0));
            },
        )
        .unwrap();
        tree.wheel(
            root,
            WheelEvent::new(
                Point::new(5.0, 5.0),
                Point::new(10.0, 0.0),
                Modifiers::default(),
                Duration::from_millis(1),
            ),
        )
        .unwrap();
        assert_eq!(calls.borrow()[0].0, "inner");
        assert_eq!(calls.borrow()[1].0, "outer");
        assert_eq!(calls.borrow()[1].1, Some(Point::new(7.0, 0.0)));
    }

    #[test]
    fn focus_tab_order_scopes_and_global_shortcuts_are_deterministic() {
        let (mut tree, root) = tree_with_root();
        let first = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let dialog = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let second = tree
            .create_node(Some(dialog), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let third = tree
            .create_node(Some(dialog), LayoutStyle::default(), PaintState::default())
            .unwrap();
        for node in [first, second, third] {
            tree.set_focus_policy(
                node,
                FocusPolicy {
                    focusable: true,
                    ..FocusPolicy::default()
                },
            )
            .unwrap();
        }
        tree.request_focus(first).unwrap();
        tree.dispatch(
            root,
            Event::new(EventKind::KeyDown(KeyEvent {
                key: Key::Tab,
                modifiers: Modifiers::default(),
                repeat: false,
                timestamp: Duration::from_millis(1),
            })),
        )
        .unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(second));
        tree.dispatch(
            root,
            Event::new(EventKind::KeyDown(KeyEvent {
                key: Key::Tab,
                modifiers: Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                repeat: false,
                timestamp: Duration::from_millis(2),
            })),
        )
        .unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(first));

        tree.push_focus_scope(dialog, true, true, Some(second))
            .unwrap();
        assert!(!tree.request_focus(first).unwrap());
        tree.pop_focus_scope(dialog).unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(first));

        let shortcut_hits = Rc::new(RefCell::new(0));
        let hits = shortcut_hits.clone();
        tree.add_global_shortcut(Shortcut::CommandOrControl('p'), move |_| {
            *hits.borrow_mut() += 1;
        });
        tree.dispatch(
            root,
            Event::new(EventKind::KeyDown(KeyEvent {
                key: Key::Character('P'),
                modifiers: Modifiers {
                    command: true,
                    ..Modifiers::default()
                },
                repeat: false,
                timestamp: Duration::from_millis(3),
            })),
        )
        .unwrap();
        assert_eq!(*shortcut_hits.borrow(), 1);
    }
}
