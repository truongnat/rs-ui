use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use ui_core::{Point, Rect, Transform};

use crate::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityRole, AccessibilitySemantics,
    BehaviorCommand, DirtyFlags, LayerDismissRequest, LayerId, LayerSpec, LayerStack, MenuModel,
    MenuOutcome, NodeId, PopoverConfig, PopoverPlacement, PortalRelationship, Pressable,
    PressableState, Resizable, ResizeAxis, ResizeConfig, RuntimeError, TooltipController,
    TooltipDelays, TooltipTriggers, UiTree, place_popover,
};

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
    AccessibilityAction,
    BehaviorCommand,
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
    AccessibilityAction(crate::AccessibilityAction),
    BehaviorCommand(BehaviorCommand),
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
            Self::AccessibilityAction(_) => EventType::AccessibilityAction,
            Self::BehaviorCommand(_) => EventType::BehaviorCommand,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputModality {
    Pointer,
    #[default]
    Keyboard,
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

#[derive(Clone, Debug)]
pub struct FocusManager {
    focused: Option<NodeId>,
    previous: Option<NodeId>,
    scopes: Vec<FocusScope>,
    traversal_order: Vec<NodeId>,
    order_dirty: bool,
}

impl Default for FocusManager {
    fn default() -> Self {
        Self {
            focused: None,
            previous: None,
            scopes: Vec::new(),
            traversal_order: Vec::new(),
            order_dirty: true,
        }
    }
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
    pressables: HashMap<NodeId, PressableState>,
    input_modality: InputModality,
    layers: LayerStack,
    portals: HashMap<NodeId, PortalRelationship>,
    dismiss_requests: VecDeque<LayerDismissRequest>,
    submenu_links: HashMap<NodeId, NodeId>,
    submenu_parent: HashMap<NodeId, NodeId>,
    resizables: HashMap<NodeId, Resizable>,
    tooltips: HashMap<NodeId, TooltipController>,
}

impl InteractionRuntime {
    pub(crate) fn mark_focus_order_dirty(&mut self) {
        self.focus.order_dirty = true;
    }

    pub(crate) fn remove_node(&mut self, node: NodeId) {
        self.listeners.remove(&node);
        self.pressables.remove(&node);
        self.resizables.remove(&node);
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
        self.portals
            .retain(|portal, relation| *portal != node && relation.logical_owner != node);
        let removed_layers = self
            .layers
            .entries()
            .iter()
            .filter(|entry| entry.spec.node == node || entry.spec.focus_scope == Some(node))
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        for layer in removed_layers {
            self.layers.close(layer);
        }
        self.dismiss_requests.retain(|request| request.node != node);
        self.submenu_links
            .retain(|parent, submenu| *parent != node && *submenu != node);
        self.submenu_parent
            .retain(|menu, item| *menu != node && *item != node);
        self.resizables.remove(&node);
        self.tooltips.remove(&node);
        self.focus.order_dirty = true;
    }
}

impl UiTree {
    pub(crate) fn accessibility_press(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if self.pressable_is_disabled(node)
            || target
                .accessibility
                .as_ref()
                .is_some_and(|semantics| semantics.state.disabled == Some(true))
        {
            return Ok(());
        }
        self.dispatch_to_target(
            node,
            Event::new(EventKind::Click(PointerEvent {
                button: Some(PointerButton::Primary),
                click_count: 1,
                ..PointerEvent::default()
            })),
        );
        Ok(())
    }

    pub(crate) fn accessibility_action_event(
        &mut self,
        node: NodeId,
        action: crate::AccessibilityAction,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.dispatch_to_target(node, Event::new(EventKind::AccessibilityAction(action)));
        Ok(())
    }

    pub fn layer_stack(&self) -> &LayerStack {
        &self.interaction_runtime.layers
    }

    pub fn take_layer_dismiss_requests(&mut self) -> Vec<LayerDismissRequest> {
        self.interaction_runtime
            .dismiss_requests
            .drain(..)
            .collect()
    }

    pub fn open_layer(&mut self, spec: LayerSpec) -> Result<LayerId, RuntimeError> {
        self.nodes
            .get(&spec.node)
            .ok_or(RuntimeError::UnknownNode(spec.node))?;
        if let Some(scope) = spec.focus_scope {
            self.nodes
                .get(&scope)
                .ok_or(RuntimeError::UnknownNode(scope))?;
        }
        if let Some(initial) = spec.initial_focus {
            self.nodes
                .get(&initial)
                .ok_or(RuntimeError::UnknownNode(initial))?;
            let scope = spec.focus_scope.unwrap_or(spec.node);
            if !self.is_descendant(initial, scope) {
                return Err(RuntimeError::FocusTargetOutsideScope);
            }
        }
        let id = self.interaction_runtime.layers.open(spec);
        if let Some(node) = self.nodes.get_mut(&spec.node) {
            node.hit_test.visible = true;
            node.dirty.insert(DirtyFlags::HIT_TEST);
            node.dirty.insert(DirtyFlags::PAINT);
        }
        if let Some(scope) = spec
            .focus_scope
            .or((spec.modal || spec.trap_focus).then_some(spec.node))
        {
            self.rebuild_focus_order();
            self.push_focus_scope(
                scope,
                spec.modal || spec.trap_focus,
                true,
                spec.initial_focus,
            )?;
        }
        self.set_portal_layer_visibility(id, true);
        if let Some(semantics) = self
            .nodes
            .get_mut(&spec.node)
            .and_then(|node| node.accessibility.as_mut())
        {
            semantics.hidden = false;
        }
        self.invalidate_accessibility_subtree(spec.node);
        self.invalidate_accessibility_chain(spec.node);
        Ok(id)
    }

    pub fn close_layer(&mut self, id: LayerId) -> Result<bool, RuntimeError> {
        let Some(entry) = self.interaction_runtime.layers.close(id) else {
            return Ok(false);
        };
        if let Some(scope) = entry
            .spec
            .focus_scope
            .or((entry.spec.modal || entry.spec.trap_focus).then_some(entry.spec.node))
        {
            self.pop_focus_scope(scope)?;
        }
        if let Some(semantics) = self
            .nodes
            .get_mut(&entry.spec.node)
            .and_then(|node| node.accessibility.as_mut())
        {
            semantics.hidden = true;
        }
        if let Some(node) = self.nodes.get_mut(&entry.spec.node) {
            node.hit_test.visible = false;
            node.dirty.insert(DirtyFlags::HIT_TEST);
            node.dirty.insert(DirtyFlags::PAINT);
        }
        self.set_portal_layer_visibility(id, false);
        if let Some(menu) = self.state.get_mut::<MenuModel<NodeId>>(entry.spec.node) {
            menu.close();
        }
        self.invalidate_accessibility_subtree(entry.spec.node);
        self.invalidate_accessibility_chain(entry.spec.node);
        Ok(true)
    }

    pub fn register_portal(
        &mut self,
        node: NodeId,
        logical_owner: NodeId,
        layer: LayerId,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.nodes
            .get(&logical_owner)
            .ok_or(RuntimeError::UnknownNode(logical_owner))?;
        if self.interaction_runtime.layers.get(layer).is_none() {
            return Err(RuntimeError::UnknownNode(node));
        }
        if node == logical_owner || self.is_descendant(logical_owner, node) {
            return Err(RuntimeError::CannotParentToDescendant);
        }
        let host = self
            .interaction_runtime
            .layers
            .get(layer)
            .unwrap()
            .spec
            .node;
        if host == node || self.is_descendant(host, node) {
            return Err(RuntimeError::CannotParentToDescendant);
        }
        if self.nodes.get(&node).and_then(|node| node.parent) != Some(host) {
            self.reparent(node, Some(host))?;
        }
        self.interaction_runtime.portals.insert(
            node,
            PortalRelationship {
                logical_owner,
                layer,
            },
        );
        self.invalidate_accessibility_subtree(node);
        self.invalidate_accessibility_chain(logical_owner);
        Ok(())
    }

    pub fn portal_relationship(&self, node: NodeId) -> Option<PortalRelationship> {
        self.interaction_runtime.portals.get(&node).copied()
    }

    fn set_portal_layer_visibility(&mut self, layer: LayerId, visible: bool) {
        let portals = self
            .interaction_runtime
            .portals
            .iter()
            .filter_map(|(node, relationship)| (relationship.layer == layer).then_some(*node))
            .collect::<Vec<_>>();
        for portal in portals {
            if let Some(node) = self.nodes.get_mut(&portal) {
                node.hit_test.visible = visible;
                node.dirty.insert(DirtyFlags::HIT_TEST);
                node.dirty.insert(DirtyFlags::PAINT);
            }
            self.invalidate_accessibility_subtree(portal);
            self.invalidate_accessibility_chain(portal);
        }
    }

    pub(crate) fn semantic_children(&self, node: NodeId) -> Vec<NodeId> {
        let mut children = self.nodes.get(&node).map_or_else(Vec::new, |n| {
            n.children
                .iter()
                .copied()
                .filter(|child| {
                    self.interaction_runtime
                        .portals
                        .get(child)
                        .is_none_or(|portal| portal.logical_owner == node)
                })
                .collect()
        });
        let portal_children = self
            .interaction_runtime
            .portals
            .iter()
            .filter_map(|(portal, relation)| {
                (relation.logical_owner == node && !children.contains(portal)).then_some(*portal)
            })
            .collect::<Vec<_>>();
        children.extend(portal_children);
        children
    }

    pub fn install_menu(
        &mut self,
        root: NodeId,
        items: Vec<(NodeId, String, bool)>,
        typeahead_timeout: Duration,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&root)
            .ok_or(RuntimeError::UnknownNode(root))?;
        let model_items = items
            .iter()
            .map(|(node, label, disabled)| {
                self.nodes
                    .get(node)
                    .ok_or(RuntimeError::UnknownNode(*node))?;
                Ok(crate::MenuItem {
                    key: *node,
                    label: label.clone(),
                    disabled: *disabled,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        self.state
            .insert(root, MenuModel::new(model_items, typeahead_timeout));
        let mut root_semantics = AccessibilitySemantics::new(AccessibilityRole::Menu);
        root_semantics.state.expanded = Some(false);
        self.set_accessibility_semantics(root, root_semantics)?;
        for (node, label, disabled) in items {
            self.set_focus_policy(
                node,
                FocusPolicy {
                    focusable: true,
                    disabled,
                    ..FocusPolicy::default()
                },
            )?;
            let mut semantics = AccessibilitySemantics::new(AccessibilityRole::MenuItem);
            semantics.label = Some(label);
            semantics.actions = vec![
                AccessibilityActionKind::Press,
                AccessibilityActionKind::Focus,
            ];
            semantics.state.disabled = Some(disabled);
            self.set_accessibility_semantics(node, semantics)?;
        }
        Ok(())
    }

    pub fn menu_command(
        &mut self,
        root: NodeId,
        command: BehaviorCommand,
        now: Duration,
    ) -> Result<MenuOutcome<NodeId>, RuntimeError> {
        let outcome = self
            .state
            .get_mut::<MenuModel<NodeId>>(root)
            .ok_or(RuntimeError::UnknownNode(root))?
            .command(command, now);
        match outcome {
            MenuOutcome::Moved(Some(index)) => {
                if let Some(target) = self
                    .state
                    .get::<MenuModel<NodeId>>(root)
                    .and_then(|menu| menu.active_key().copied())
                {
                    let _ = self.request_focus(target)?;
                }
                let _ = index;
            }
            MenuOutcome::Activate(target) => {
                self.accessibility_press(target)?;
                self.state
                    .get_mut::<MenuModel<NodeId>>(root)
                    .expect("menu remains registered")
                    .close();
                if let Some(semantics) = self
                    .nodes
                    .get_mut(&root)
                    .and_then(|node| node.accessibility.as_mut())
                {
                    semantics.state.expanded = Some(false);
                }
                self.invalidate_accessibility_chain(root);
            }
            MenuOutcome::Close => {
                if let Some(semantics) = self
                    .nodes
                    .get_mut(&root)
                    .and_then(|node| node.accessibility.as_mut())
                {
                    semantics.state.expanded = Some(false);
                }
                self.invalidate_accessibility_chain(root);
            }
            MenuOutcome::Ignored | MenuOutcome::Moved(None) => {}
        }
        Ok(outcome)
    }

    pub fn open_menu(&mut self, root: NodeId) -> Result<(), RuntimeError> {
        self.state
            .get_mut::<MenuModel<NodeId>>(root)
            .ok_or(RuntimeError::UnknownNode(root))?
            .open();
        if let Some(semantics) = self
            .nodes
            .get_mut(&root)
            .and_then(|node| node.accessibility.as_mut())
        {
            semantics.state.expanded = Some(true);
        }
        if let Some(target) = self
            .state
            .get::<MenuModel<NodeId>>(root)
            .and_then(|menu| menu.active_key().copied())
        {
            let _ = self.request_focus(target)?;
        }
        self.invalidate_accessibility_chain(root);
        Ok(())
    }

    pub fn link_submenu(
        &mut self,
        parent_item: NodeId,
        submenu_root: NodeId,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&parent_item)
            .ok_or(RuntimeError::UnknownNode(parent_item))?;
        self.state
            .get::<MenuModel<NodeId>>(submenu_root)
            .ok_or(RuntimeError::UnknownNode(submenu_root))?;
        self.interaction_runtime
            .submenu_links
            .insert(parent_item, submenu_root);
        self.interaction_runtime
            .submenu_parent
            .insert(submenu_root, parent_item);
        Ok(())
    }

    pub fn resize_command(
        &mut self,
        node: NodeId,
        command: BehaviorCommand,
    ) -> Result<bool, RuntimeError> {
        let Some(resizable) = self.interaction_runtime.resizables.get_mut(&node) else {
            return Err(RuntimeError::UnknownNode(node));
        };
        let changed = resizable.keyboard(command);
        if changed {
            self.invalidate_resizable(node)?;
        }
        Ok(changed)
    }

    fn invalidate_resizable(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        self.invalidate(node, DirtyFlags::PAINT)?;
        let value = self
            .interaction_runtime
            .resizables
            .get(&node)
            .map(|resizable| resizable.value.to_string());
        if let Some(semantics) = self
            .nodes
            .get_mut(&node)
            .and_then(|node| node.accessibility.as_mut())
        {
            semantics.value = value;
        }
        self.invalidate_accessibility_node(node);
        Ok(())
    }

    pub fn register_tooltip(
        &mut self,
        trigger: NodeId,
        tooltip: NodeId,
        delays: TooltipDelays,
        text: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&trigger)
            .ok_or(RuntimeError::UnknownNode(trigger))?;
        self.nodes
            .get(&tooltip)
            .ok_or(RuntimeError::UnknownNode(tooltip))?;
        self.interaction_runtime
            .tooltips
            .insert(trigger, TooltipController::new(delays));
        self.set_accessibility_text_content(tooltip, text)?;
        let mut tooltip_semantics = AccessibilitySemantics::new(AccessibilityRole::Tooltip);
        tooltip_semantics.hidden = true;
        self.set_accessibility_semantics(tooltip, tooltip_semantics)?;
        if let Some(node) = self.nodes.get_mut(&tooltip) {
            node.hit_test.visible = false;
            node.dirty.insert(DirtyFlags::HIT_TEST);
            node.dirty.insert(DirtyFlags::PAINT);
        }
        let mut trigger_semantics = self.nodes[&trigger]
            .accessibility
            .clone()
            .unwrap_or_else(|| AccessibilitySemantics::new(AccessibilityRole::Button));
        trigger_semantics.described_by.push(tooltip);
        self.set_accessibility_semantics(trigger, trigger_semantics)
    }

    pub fn update_tooltip(
        &mut self,
        trigger: NodeId,
        now: Duration,
        triggers: TooltipTriggers,
    ) -> Result<bool, RuntimeError> {
        let Some(controller) = self.interaction_runtime.tooltips.get_mut(&trigger) else {
            return Err(RuntimeError::UnknownNode(trigger));
        };
        let open = controller.update(now, triggers);
        let tooltip = self
            .nodes
            .get(&trigger)
            .and_then(|node| node.accessibility.as_ref())
            .and_then(|semantics| semantics.described_by.first().copied())
            .ok_or(RuntimeError::UnknownNode(trigger))?;
        let mut semantics = self.nodes[&tooltip]
            .accessibility
            .clone()
            .ok_or(RuntimeError::UnknownNode(tooltip))?;
        if semantics.hidden == open {
            semantics.hidden = !open;
            self.set_accessibility_semantics(tooltip, semantics)?;
            if let Some(node) = self.nodes.get_mut(&tooltip) {
                node.hit_test.visible = open;
                node.dirty.insert(DirtyFlags::HIT_TEST);
                node.dirty.insert(DirtyFlags::PAINT);
            }
        }
        Ok(open)
    }

    pub fn tick_tooltip(&mut self, trigger: NodeId, now: Duration) -> Result<bool, RuntimeError> {
        let tooltip = self
            .nodes
            .get(&trigger)
            .ok_or(RuntimeError::UnknownNode(trigger))?
            .accessibility
            .as_ref()
            .and_then(|semantics| semantics.described_by.first().copied())
            .ok_or(RuntimeError::UnknownNode(trigger))?;
        let trigger_node = &self.nodes[&trigger];
        let tooltip_node = self
            .nodes
            .get(&tooltip)
            .ok_or(RuntimeError::UnknownNode(tooltip))?;
        let triggers = TooltipTriggers {
            pointer_over_trigger: trigger_node.interaction.hovered,
            pointer_over_tooltip: tooltip_node.interaction.hovered,
            trigger_focused: self.focus_manager().focused() == Some(trigger),
        };
        self.update_tooltip(trigger, now, triggers)
    }

    pub fn position_popover(
        &self,
        anchor: NodeId,
        size: (f32, f32),
        viewport: Rect,
        config: PopoverConfig,
    ) -> Result<PopoverPlacement, RuntimeError> {
        let bounds = self.world_bounds(anchor)?;
        Ok(place_popover(bounds, size, viewport, config))
    }

    pub fn world_bounds(&self, node: NodeId) -> Result<Rect, RuntimeError> {
        let mut path = Vec::new();
        let mut current = Some(node);
        while let Some(id) = current {
            let target = self.nodes.get(&id).ok_or(RuntimeError::UnknownNode(id))?;
            path.push(id);
            current = target.parent;
        }
        path.reverse();
        let mut world = Transform::IDENTITY;
        for (index, id) in path.iter().copied().enumerate() {
            if index > 0
                && let Some(offset) = self.scroll_transform(path[index - 1])
            {
                world = Transform::translation(-offset.x, -offset.y).then(world);
            }
            world = self.nodes[&id].hit_test.transform.then(world);
        }
        let target = &self.nodes[&node];
        Ok(world.transform_rect(target.hit_test.bounds))
    }

    pub fn dispatch_behavior_command(
        &mut self,
        root: NodeId,
        command: BehaviorCommand,
        timestamp: Duration,
    ) -> Result<Event, RuntimeError> {
        self.nodes
            .get(&root)
            .ok_or(RuntimeError::UnknownNode(root))?;
        self.interaction_runtime.input_modality = InputModality::Keyboard;
        let target = self
            .interaction_runtime
            .focus
            .focused
            .filter(|node| *node == root || self.is_descendant(*node, root))
            .unwrap_or(root);
        let mut event =
            self.dispatch_to_target(target, Event::new(EventKind::BehaviorCommand(command)));
        if event.default_prevented() {
            return Ok(event);
        }
        if command == BehaviorCommand::Cancel
            && let Some(request) = self.interaction_runtime.layers.escape()
        {
            self.interaction_runtime.dismiss_requests.push_back(request);
            event.prevent_default();
            return Ok(event);
        }
        let menu_root = self
            .path_to_root(target)
            .into_iter()
            .rev()
            .find(|node| self.state.get::<MenuModel<NodeId>>(*node).is_some());
        if let Some(menu_root) = menu_root {
            if command == BehaviorCommand::MoveRight
                && let Some(submenu) = self.interaction_runtime.submenu_links.get(&target).copied()
            {
                self.open_menu(submenu)?;
                return Ok(event);
            }
            if command == BehaviorCommand::MoveLeft
                && let Some(parent_item) = self
                    .interaction_runtime
                    .submenu_parent
                    .get(&menu_root)
                    .copied()
            {
                self.menu_command(menu_root, BehaviorCommand::Cancel, timestamp)?;
                let _ = self.request_focus(parent_item)?;
                return Ok(event);
            }
            let _ = self.menu_command(menu_root, command, timestamp)?;
            return Ok(event);
        }
        match command {
            BehaviorCommand::Activate => {
                if self.interaction_runtime.resizables.contains_key(&target) {
                    let _ = self.resize_command(target, command)?;
                    event.prevent_default();
                } else if self
                    .state
                    .get::<Pressable>(target)
                    .is_some_and(|pressable| !pressable.disabled())
                {
                    self.accessibility_press(target)?;
                    event.prevent_default();
                }
            }
            BehaviorCommand::MoveNext => {
                if self.interaction_runtime.resizables.contains_key(&target) {
                    let _ = self.resize_command(target, command)?;
                } else {
                    let _ = self.traverse_focus(false)?;
                }
            }
            BehaviorCommand::MovePrevious => {
                if self.interaction_runtime.resizables.contains_key(&target) {
                    let _ = self.resize_command(target, command)?;
                } else {
                    let _ = self.traverse_focus_in(root, true, None)?;
                }
            }
            BehaviorCommand::MoveFirst | BehaviorCommand::MoveLast
                if self.interaction_runtime.resizables.contains_key(&target) =>
            {
                let _ = self.resize_command(target, command)?;
            }
            BehaviorCommand::MoveFirst => {
                self.traverse_focus_in(root, false, Some(false))?;
            }
            BehaviorCommand::MoveLast => {
                self.traverse_focus_in(root, false, Some(true))?;
            }
            BehaviorCommand::MoveLeft
            | BehaviorCommand::MoveRight
            | BehaviorCommand::MoveUp
            | BehaviorCommand::MoveDown
                if self.interaction_runtime.resizables.contains_key(&target) =>
            {
                let _ = self.resize_command(target, command)?;
            }
            BehaviorCommand::Increment => {
                let _ = self.adjust_resizable(target, 1.0)?;
            }
            BehaviorCommand::Decrement => {
                let _ = self.adjust_resizable(target, -1.0)?;
            }
            BehaviorCommand::Cancel => self.cancel_active_resize(target),
            BehaviorCommand::MoveRight => {
                if let Some(submenu) = self.interaction_runtime.submenu_links.get(&target).copied()
                {
                    self.open_menu(submenu)?;
                }
            }
            _ => {}
        }
        Ok(event)
    }

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
        self.invalidate_accessibility_subtree(node);
        self.invalidate_accessibility_chain(node);
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
        let child_transform = if let Some(offset) = self.scroll_transform(node) {
            if !target.hit_test.bounds.contains(local_position) {
                return None;
            }
            Transform::translation(-offset.x, -offset.y).then(world_transform)
        } else {
            world_transform
        };
        let mut children = target.children.clone();
        children.sort_by_key(|child| {
            self.interaction_runtime
                .layers
                .layer_for_node(*child)
                .map(|entry| (1_u8, entry.spec.z_layer, entry.id.get()))
                .unwrap_or((0, self.nodes[child].hit_test.z_order, child.get()))
        });
        for child in children.into_iter().rev() {
            if let Some(mut path) = self.hit_test_node(child, position, child_transform) {
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
        self.interaction_runtime.input_modality = InputModality::Pointer;
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
            if self.interaction_runtime.resizables.contains_key(&target) {
                let _ = self.update_resize(target, pointer.position)?;
            }
            self.dispatch_to_target(target, Event::new(EventKind::PointerMove(pointer)));
        }
        Ok(())
    }

    pub fn pointer_down(
        &mut self,
        root: NodeId,
        pointer: PointerEvent,
    ) -> Result<(), RuntimeError> {
        self.interaction_runtime.input_modality = InputModality::Pointer;
        let captured = self.interaction_runtime.pointer.captured_target;
        let candidate = self.hit_test(root, pointer.position)?.last().copied();
        let candidate_inside_top = self
            .interaction_runtime
            .layers
            .topmost()
            .is_some_and(|entry| {
                candidate.is_some_and(|target| self.is_descendant(target, entry.spec.node))
            });
        let outside = if captured.is_some() {
            crate::OutsidePointerResult::default()
        } else {
            self.interaction_runtime
                .layers
                .outside_pointer_down(candidate, |_, _| candidate_inside_top)
        };
        if let Some(request) = outside.dismissed {
            self.interaction_runtime.dismiss_requests.push_back(request);
        }
        if outside.blocked {
            return Ok(());
        }
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
        if let Some(target) = target.filter(|target| !self.pressable_is_disabled(*target)) {
            if self.interaction_runtime.resizables.contains_key(&target) {
                self.capture_pointer(target)?;
                self.begin_resize(target, pointer.position)?;
            }
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
                && !self.pressable_is_disabled(pressed)
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
                let event =
                    self.dispatch_to_target(pressed, Event::new(EventKind::DoubleClick(click)));
                if !event.default_prevented()
                    && self.interaction_runtime.resizables.contains_key(&pressed)
                {
                    self.reset_resizable(pressed)?;
                }
            }
        }
        if let Some(target) = target
            && self.interaction_runtime.resizables.contains_key(&target)
        {
            self.end_resize(target)?;
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
            if remaining != Point::ZERO && self.scroll_state(target).is_some() {
                remaining = self.scroll_by(target, remaining)?.remaining;
            }
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
        self.invalidate_accessibility_subtree(node);
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

    pub fn register_pressable(
        &mut self,
        node: NodeId,
        label: Option<String>,
        disabled: bool,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.state.insert(node, Pressable::new(disabled));
        self.interaction_runtime.pressables.insert(
            node,
            PressableState {
                disabled,
                ..PressableState::default()
            },
        );
        let current = self.nodes[&node].focus_policy;
        self.set_focus_policy(
            node,
            FocusPolicy {
                focusable: true,
                disabled,
                ..current
            },
        )?;
        let mut semantics = self.nodes[&node]
            .accessibility
            .clone()
            .unwrap_or_else(|| AccessibilitySemantics::new(AccessibilityRole::Button));
        if semantics.label.is_none() {
            semantics.label = label;
        }
        semantics.state.disabled = Some(disabled);
        if !semantics.actions.contains(&AccessibilityActionKind::Press) {
            semantics.actions.push(AccessibilityActionKind::Press);
        }
        self.set_accessibility_semantics(node, semantics)
            .and_then(|()| self.invalidate(node, DirtyFlags::PAINT))
    }

    pub fn set_pressable_disabled(
        &mut self,
        node: NodeId,
        disabled: bool,
    ) -> Result<(), RuntimeError> {
        if self.state.get::<Pressable>(node).is_none() {
            return Err(RuntimeError::UnknownNode(node));
        }
        self.state.insert(node, Pressable::new(disabled));
        if let Some(state) = self.interaction_runtime.pressables.get_mut(&node) {
            state.disabled = disabled;
        }
        let policy = self.nodes[&node].focus_policy;
        self.set_focus_policy(node, FocusPolicy { disabled, ..policy })?;
        if let Some(semantics) = self
            .nodes
            .get_mut(&node)
            .and_then(|node| node.accessibility.as_mut())
        {
            semantics.state.disabled = Some(disabled);
        }
        self.invalidate(node, DirtyFlags::PAINT)?;
        self.invalidate_accessibility_node(node);
        Ok(())
    }

    pub fn pressable_state(&self, node: NodeId) -> Option<PressableState> {
        self.state.get::<Pressable>(node)?;
        let interaction = self.nodes.get(&node)?.interaction;
        Some(PressableState {
            hovered: interaction.hovered,
            active: interaction.pressed,
            focused: interaction.focused,
            focus_visible: interaction.focused
                && self.interaction_runtime.input_modality == InputModality::Keyboard,
            disabled: self.pressable_is_disabled(node),
        })
    }

    pub fn register_resizable(
        &mut self,
        node: NodeId,
        config: ResizeConfig,
        initial_value: f32,
        label: Option<String>,
    ) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if !config.min.is_finite()
            || !config.max.is_finite()
            || !config.step.is_finite()
            || !config.reset.is_finite()
            || !initial_value.is_finite()
            || config.min > config.max
            || config.step < 0.0
            || !(config.min..=config.max).contains(&config.reset)
        {
            return Err(RuntimeError::InvalidResizeConfig);
        }
        let value = initial_value.clamp(config.min, config.max);
        self.interaction_runtime
            .resizables
            .insert(node, Resizable::new(config, value, label));
        let current = self.nodes[&node].focus_policy;
        self.set_focus_policy(
            node,
            FocusPolicy {
                focusable: true,
                ..current
            },
        )?;
        self.update_resizable_semantics(node)?;
        Ok(())
    }

    pub fn begin_resize(&mut self, node: NodeId, pointer: Point) -> Result<(), RuntimeError> {
        let resizable = self
            .interaction_runtime
            .resizables
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        resizable.drag_start_pointer = Some(pointer);
        resizable.drag_start_value = resizable.value;
        Ok(())
    }

    pub fn update_resize(&mut self, node: NodeId, pointer: Point) -> Result<f32, RuntimeError> {
        let resizable = self
            .interaction_runtime
            .resizables
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let Some(start) = resizable.drag_start_pointer else {
            return Ok(resizable.value);
        };
        let delta = match resizable.config.axis {
            ResizeAxis::Horizontal => pointer.x - start.x,
            ResizeAxis::Vertical => pointer.y - start.y,
        };
        let value =
            (resizable.drag_start_value + delta).clamp(resizable.config.min, resizable.config.max);
        if value != resizable.value {
            resizable.value = value;
            self.invalidate(node, DirtyFlags::LAYOUT)?;
            self.invalidate(node, DirtyFlags::PAINT)?;
            self.update_resizable_semantics(node)?;
        }
        Ok(value)
    }

    pub fn end_resize(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let resizable = self
            .interaction_runtime
            .resizables
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        resizable.drag_start_pointer = None;
        Ok(())
    }

    pub fn resizable_value(&self, node: NodeId) -> Option<f32> {
        self.interaction_runtime
            .resizables
            .get(&node)
            .map(|resizable| resizable.value)
    }

    pub fn reset_resizable(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let resizable = self
            .interaction_runtime
            .resizables
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let value = resizable.config.reset;
        resizable.drag_start_pointer = None;
        if value != resizable.value {
            resizable.value = value;
            self.invalidate(node, DirtyFlags::LAYOUT)?;
            self.invalidate(node, DirtyFlags::PAINT)?;
            self.update_resizable_semantics(node)?;
        }
        Ok(())
    }

    pub fn window_focus(&mut self, root: NodeId) -> Result<Event, RuntimeError> {
        self.dispatch(root, Event::new(EventKind::WindowFocus))
    }

    pub fn window_blur(&mut self, root: NodeId) -> Result<Event, RuntimeError> {
        self.dispatch(root, Event::new(EventKind::WindowBlur))
    }

    fn dispatch_key(&mut self, root: NodeId, mut event: Event) -> Result<Event, RuntimeError> {
        self.interaction_runtime.input_modality = InputModality::Keyboard;
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
        if matches!(event.kind, EventKind::Click(_))
            && self
                .interaction_runtime
                .pressables
                .get(&target)
                .is_some_and(|pressable| pressable.disabled)
        {
            event.prevent_default();
            return event;
        }
        let accessibility_action = match &event.kind {
            EventKind::AccessibilityAction(action) => Some(action.clone()),
            _ => None,
        };
        self.dispatch_event_along_path(target, &mut event);
        if !event.default_prevented() {
            match accessibility_action {
                Some(AccessibilityAction::Press) => {
                    let click = self.dispatch_to_target(
                        target,
                        Event::new(EventKind::Click(PointerEvent {
                            button: Some(PointerButton::Primary),
                            click_count: 1,
                            ..PointerEvent::default()
                        })),
                    );
                    if click.default_prevented() {
                        event.prevent_default();
                    }
                }
                Some(AccessibilityAction::Increment) => {
                    let _ = self.adjust_resizable(target, 1.0);
                }
                Some(AccessibilityAction::Decrement) => {
                    let _ = self.adjust_resizable(target, -1.0);
                }
                _ => {}
            }
        }
        event
    }

    fn update_resizable_semantics(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let resizable = self
            .interaction_runtime
            .resizables
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let value = resizable.value.to_string();
        let label = resizable.label.clone();
        let mut semantics = self.nodes[&node]
            .accessibility
            .clone()
            .unwrap_or_else(|| AccessibilitySemantics::new(AccessibilityRole::Separator));
        if semantics.label.is_none() {
            semantics.label = label;
        }
        semantics.value = Some(value);
        for action in [
            AccessibilityActionKind::Increment,
            AccessibilityActionKind::Decrement,
        ] {
            if !semantics.actions.contains(&action) {
                semantics.actions.push(action);
            }
        }
        self.set_accessibility_semantics(node, semantics)
    }

    pub(crate) fn adjust_resizable(
        &mut self,
        node: NodeId,
        direction: f32,
    ) -> Result<bool, RuntimeError> {
        let Some(resizable) = self.interaction_runtime.resizables.get_mut(&node) else {
            return Ok(false);
        };
        let value = (resizable.value + direction * resizable.config.step)
            .clamp(resizable.config.min, resizable.config.max);
        if value == resizable.value {
            return Ok(false);
        }
        resizable.value = value;
        self.invalidate(node, DirtyFlags::LAYOUT)?;
        self.invalidate(node, DirtyFlags::PAINT)?;
        self.update_resizable_semantics(node)?;
        Ok(true)
    }

    fn cancel_active_resize(&mut self, node: NodeId) {
        let Some(resizable) = self.interaction_runtime.resizables.get_mut(&node) else {
            return;
        };
        if resizable.drag_start_pointer.is_none() {
            return;
        }
        let value = resizable.drag_start_value;
        resizable.drag_start_pointer = None;
        if value != resizable.value {
            resizable.value = value;
            let _ = self.invalidate(node, DirtyFlags::LAYOUT);
            let _ = self.invalidate(node, DirtyFlags::PAINT);
            let _ = self.update_resizable_semantics(node);
        }
    }

    fn traverse_focus_in(
        &mut self,
        root: NodeId,
        backwards: bool,
        first_or_last: Option<bool>,
    ) -> Result<bool, RuntimeError> {
        if self.interaction_runtime.focus.order_dirty {
            self.rebuild_focus_order();
        }
        let trap_scope = self
            .interaction_runtime
            .focus
            .scopes
            .last()
            .filter(|scope| scope.trap_focus)
            .map(|scope| scope.node);
        let order = self
            .interaction_runtime
            .focus
            .traversal_order
            .iter()
            .copied()
            .filter(|node| {
                (*node == root || self.is_descendant(*node, root))
                    && trap_scope
                        .is_none_or(|scope| *node == scope || self.is_descendant(*node, scope))
            })
            .collect::<Vec<_>>();
        if order.is_empty() {
            return Ok(false);
        }
        let index = if let Some(last) = first_or_last {
            Some(if last { order.len() - 1 } else { 0 })
        } else {
            let current = self.interaction_runtime.focus.focused;
            let current_index =
                current.and_then(|focused| order.iter().position(|node| *node == focused));
            match (current_index, backwards) {
                (Some(index), false) => Some((index + 1) % order.len()),
                (Some(index), true) => Some((index + order.len() - 1) % order.len()),
                (None, false) => Some(0),
                (None, true) => Some(order.len() - 1),
            }
        };
        if let Some(index) = index {
            self.request_focus(order[index])
        } else {
            Ok(false)
        }
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
            current = self
                .interaction_runtime
                .portals
                .get(&node)
                .map(|portal| portal.logical_owner)
                .or_else(|| self.nodes.get(&node).and_then(|target| target.parent));
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
        self.invalidate_accessibility_chain(node);
        self.interaction_runtime.mark_focus_order_dirty();
        if policy.disabled
            && self
                .focus_manager()
                .focused()
                .is_some_and(|focused| self.is_descendant(focused, node))
        {
            self.clear_focus();
        }
        Ok(())
    }

    pub fn request_focus(&mut self, node: NodeId) -> Result<bool, RuntimeError> {
        let target = self
            .nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if !target.focus_policy.focusable || !self.can_focus_through_ancestors(node) {
            return Ok(false);
        }
        if let Some(scope) = self.interaction_runtime.focus.scopes.last()
            && scope.trap_focus
            && !self.is_descendant(node, scope.node)
        {
            return Ok(false);
        }
        if let Some(request) = self
            .interaction_runtime
            .layers
            .focus_outside(Some(node), |scope, target| {
                self.is_descendant(target, scope)
            })
        {
            self.interaction_runtime.dismiss_requests.push_back(request);
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
        let mut changed = false;
        if let Some(target) = self.nodes.get_mut(&node)
            && target.interaction.focused != focused
        {
            target.interaction.focused = focused;
            target.dirty.insert(DirtyFlags::PAINT);
            changed = true;
        }
        if changed {
            self.invalidate_accessibility_chain(node);
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
        if self.interaction_runtime.focus.order_dirty {
            self.rebuild_focus_order();
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
        if self.interaction_runtime.focus.order_dirty {
            self.rebuild_focus_order();
        }
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
                (policy.focusable
                    && !policy.disabled
                    && policy.tab_index >= 0
                    && self.can_focus_through_ancestors(node))
                .then_some((node, policy.tab_index, order))
            })
            .collect::<Vec<_>>();
        focusable.sort_by_key(|(_, tab_index, order)| {
            let group = if *tab_index > 0 { 0 } else { 1 };
            (group, *tab_index, *order)
        });
        self.interaction_runtime.focus.traversal_order =
            focusable.into_iter().map(|(node, _, _)| node).collect();
        self.interaction_runtime.focus.order_dirty = false;
    }

    fn collect_document_order(&self, node: NodeId, order: &mut Vec<NodeId>) {
        order.push(node);
        for child in self.semantic_children(node) {
            self.collect_document_order(child, order);
        }
    }

    fn is_descendant(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut pending = vec![node];
        let mut visited = std::collections::HashSet::new();
        while let Some(candidate) = pending.pop() {
            if candidate == ancestor {
                return true;
            }
            if !visited.insert(candidate) {
                continue;
            }
            if let Some(parent) = self.nodes.get(&candidate).and_then(|target| target.parent) {
                pending.push(parent);
            }
            if let Some(owner) = self
                .interaction_runtime
                .portals
                .get(&candidate)
                .map(|portal| portal.logical_owner)
            {
                pending.push(owner);
            }
        }
        false
    }

    fn can_focus_through_ancestors(&self, node: NodeId) -> bool {
        let mut pending = vec![node];
        let mut visited = std::collections::HashSet::new();
        while let Some(candidate) = pending.pop() {
            let Some(target) = self.nodes.get(&candidate) else {
                return false;
            };
            if target.focus_policy.disabled {
                return false;
            }
            if !visited.insert(candidate) {
                continue;
            }
            if let Some(parent) = target.parent {
                pending.push(parent);
            }
            if let Some(owner) = self
                .interaction_runtime
                .portals
                .get(&candidate)
                .map(|portal| portal.logical_owner)
            {
                pending.push(owner);
            }
        }
        true
    }

    fn pressable_is_disabled(&self, node: NodeId) -> bool {
        self.state
            .get::<Pressable>(node)
            .is_some_and(|pressable| pressable.disabled())
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
    use crate::{Dimension, LayoutStyle, OutsidePointerPolicy, PaintState};
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

    #[test]
    fn pressable_pointer_keyboard_and_accessibility_share_click_dispatch() {
        use crate::{AccessibilityAction, AccessibilityActionRequest, SemanticTree};

        let (mut tree, root) = tree_with_root();
        let button = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(root, hit_state(Transform::IDENTITY, 0))
            .unwrap();
        tree.set_hit_test_state(button, hit_state(Transform::IDENTITY, 0))
            .unwrap();
        tree.register_pressable(button, Some("Run".to_owned()), false)
            .unwrap();
        let activations = Rc::new(RefCell::new(0));
        let observed = activations.clone();
        tree.add_event_listener(button, EventType::Click, ListenerPhase::Bubble, move |_| {
            *observed.borrow_mut() += 1;
        })
        .unwrap();

        let pointer = PointerEvent {
            position: Point::new(10.0, 10.0),
            button: Some(PointerButton::Primary),
            timestamp: Duration::from_millis(10),
            ..PointerEvent::default()
        };
        tree.pointer_down(root, pointer).unwrap();
        assert!(tree.pressable_state(button).unwrap().active);
        tree.pointer_up(
            root,
            PointerEvent {
                timestamp: Duration::from_millis(20),
                ..pointer
            },
        )
        .unwrap();
        assert_eq!(*activations.borrow(), 1);

        tree.request_focus(button).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::from_millis(30))
            .unwrap();
        assert_eq!(*activations.borrow(), 2);
        assert!(tree.pressable_state(button).unwrap().focus_visible);

        let mut semantics = SemanticTree::default();
        let _ = semantics.update(&mut tree, root).unwrap();
        let id = semantics.accessibility_id(button).unwrap();
        semantics
            .route_action(
                &mut tree,
                AccessibilityActionRequest {
                    target: id,
                    action: AccessibilityAction::Press,
                },
            )
            .unwrap();
        assert_eq!(*activations.borrow(), 3);

        tree.set_pressable_disabled(button, true).unwrap();
        tree.pointer_down(root, pointer).unwrap();
        tree.pointer_up(root, pointer).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::ZERO)
            .unwrap();
        assert_eq!(*activations.borrow(), 3);
        let update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(update.changed.len(), 1);
        assert_eq!(update.changed[0].state.disabled, Some(true));
        assert!(
            !update.changed[0]
                .actions
                .contains(&AccessibilityActionKind::Press)
        );
    }

    #[test]
    fn nested_pressable_stop_propagation_activates_only_inner_listener() {
        let (mut tree, root) = tree_with_root();
        let outer = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let inner = tree
            .create_node(Some(outer), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.register_pressable(outer, None, false).unwrap();
        tree.register_pressable(inner, None, false).unwrap();
        let outer_hits = Rc::new(RefCell::new(0));
        let inner_hits = Rc::new(RefCell::new(0));
        let outer_seen = outer_hits.clone();
        let inner_seen = inner_hits.clone();
        tree.add_event_listener(outer, EventType::Click, ListenerPhase::Bubble, move |_| {
            *outer_seen.borrow_mut() += 1;
        })
        .unwrap();
        tree.add_event_listener(
            inner,
            EventType::Click,
            ListenerPhase::Bubble,
            move |event| {
                *inner_seen.borrow_mut() += 1;
                event.stop_propagation();
            },
        )
        .unwrap();
        tree.request_focus(inner).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::ZERO)
            .unwrap();
        assert_eq!(*inner_hits.borrow(), 1);
        assert_eq!(*outer_hits.borrow(), 0);
    }

    #[test]
    fn nested_focus_scopes_restore_focus_and_reject_disabled_subtrees() {
        let (mut tree, root) = tree_with_root();
        let outside = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let outer = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let outer_focus = tree
            .create_node(Some(outer), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let inner = tree
            .create_node(Some(outer), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let inner_focus = tree
            .create_node(Some(inner), LayoutStyle::default(), PaintState::default())
            .unwrap();
        for node in [outside, outer_focus, inner_focus] {
            tree.set_focus_policy(
                node,
                FocusPolicy {
                    focusable: true,
                    ..FocusPolicy::default()
                },
            )
            .unwrap();
        }
        tree.request_focus(outside).unwrap();
        tree.push_focus_scope(outer, true, true, Some(outer_focus))
            .unwrap();
        tree.push_focus_scope(inner, true, true, Some(inner_focus))
            .unwrap();
        assert!(!tree.request_focus(outside).unwrap());
        assert_eq!(tree.focus_manager().focused(), Some(inner_focus));
        tree.pop_focus_scope(inner).unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(outer_focus));
        tree.pop_focus_scope(outer).unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(outside));

        tree.set_focus_policy(
            outer,
            FocusPolicy {
                disabled: true,
                ..FocusPolicy::default()
            },
        )
        .unwrap();
        assert!(!tree.request_focus(outer_focus).unwrap());
    }

    #[test]
    fn overlay_blocks_outside_pointer_and_escape_dismisses_top_layer() {
        let (mut tree, root) = tree_with_root();
        let lower = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let upper = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(root, hit_state(Transform::IDENTITY, 0))
            .unwrap();
        tree.set_hit_test_state(lower, hit_state(Transform::translation(10.0, 10.0), 1))
            .unwrap();
        tree.set_hit_test_state(upper, hit_state(Transform::translation(50.0, 50.0), 2))
            .unwrap();
        tree.open_layer(LayerSpec {
            node: lower,
            z_layer: 1,
            outside_pointer: OutsidePointerPolicy::DismissAndContinue,
            dismiss_on_escape: true,
            ..LayerSpec::default()
        })
        .unwrap();
        tree.open_layer(LayerSpec {
            node: upper,
            z_layer: 2,
            modal: true,
            blocks_pointer: true,
            outside_pointer: OutsidePointerPolicy::DismissAndBlock,
            dismiss_on_escape: true,
            ..LayerSpec::default()
        })
        .unwrap();
        let hits = Rc::new(RefCell::new(0));
        let observed = hits.clone();
        tree.add_event_listener(
            root,
            EventType::PointerDown,
            ListenerPhase::Bubble,
            move |_| {
                *observed.borrow_mut() += 1;
            },
        )
        .unwrap();
        tree.pointer_down(
            root,
            PointerEvent {
                position: Point::new(20.0, 20.0),
                button: Some(PointerButton::Primary),
                ..PointerEvent::default()
            },
        )
        .unwrap();
        assert_eq!(*hits.borrow(), 0);
        let outside = tree.take_layer_dismiss_requests();
        assert_eq!(outside.len(), 1);
        assert_eq!(outside[0].node, upper);
        tree.dispatch_behavior_command(root, BehaviorCommand::Cancel, Duration::ZERO)
            .unwrap();
        let escape = tree.take_layer_dismiss_requests();
        assert_eq!(escape.len(), 1);
        assert_eq!(escape[0].node, upper);
    }

    #[test]
    fn resize_uses_pointer_capture_clamps_bounds_and_double_click_resets() {
        let (mut tree, root) = tree_with_root();
        let handle = tree
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
        tree.set_hit_test_state(handle, hit_state(Transform::IDENTITY, 1))
            .unwrap();
        tree.register_resizable(
            handle,
            ResizeConfig {
                axis: crate::ResizeAxis::Horizontal,
                min: 20.0,
                max: 40.0,
                step: 5.0,
                reset: 30.0,
            },
            30.0,
            Some("Sidebar width".to_owned()),
        )
        .unwrap();
        let pointer = PointerEvent {
            position: Point::new(10.0, 10.0),
            button: Some(PointerButton::Primary),
            timestamp: Duration::from_millis(1),
            ..PointerEvent::default()
        };
        tree.pointer_down(root, pointer).unwrap();
        assert_eq!(tree.pointer_state().captured_target, Some(handle));
        tree.pointer_move(
            root,
            PointerEvent {
                position: Point::new(200.0, 10.0),
                ..pointer
            },
        )
        .unwrap();
        assert_eq!(tree.resizable_value(handle), Some(40.0));
        tree.pointer_up(
            root,
            PointerEvent {
                position: Point::new(200.0, 10.0),
                timestamp: Duration::from_millis(20),
                ..pointer
            },
        )
        .unwrap();
        assert_eq!(tree.pointer_state().captured_target, None);

        for time in [Duration::from_millis(100), Duration::from_millis(200)] {
            tree.pointer_down(
                root,
                PointerEvent {
                    timestamp: time,
                    ..pointer
                },
            )
            .unwrap();
            tree.pointer_up(
                root,
                PointerEvent {
                    timestamp: time + Duration::from_millis(5),
                    ..pointer
                },
            )
            .unwrap();
        }
        assert_eq!(tree.resizable_value(handle), Some(30.0));
    }

    #[test]
    fn portal_reparents_for_paint_but_routes_events_and_semantics_to_owner() {
        use crate::{AccessibilityRole, AccessibilitySemantics, SemanticTree};

        let (mut tree, root) = tree_with_root();
        let owner = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let overlay_host = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let portal = tree
            .create_node(Some(owner), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_semantics(
            owner,
            AccessibilitySemantics::new(AccessibilityRole::Group),
        )
        .unwrap();
        tree.set_accessibility_semantics(
            overlay_host,
            AccessibilitySemantics::new(AccessibilityRole::Group),
        )
        .unwrap();
        tree.set_accessibility_semantics(
            portal,
            AccessibilitySemantics::new(AccessibilityRole::Tooltip),
        )
        .unwrap();
        let layer = tree
            .open_layer(LayerSpec {
                node: overlay_host,
                ..LayerSpec::default()
            })
            .unwrap();
        tree.register_portal(portal, owner, layer).unwrap();
        assert_eq!(tree.node(portal).unwrap().parent(), Some(overlay_host));

        let calls = Rc::new(RefCell::new(0));
        let observed = calls.clone();
        tree.add_event_listener(
            owner,
            EventType::Click,
            ListenerPhase::Bubble,
            move |event| {
                assert_eq!(event.target(), Some(portal));
                *observed.borrow_mut() += 1;
            },
        )
        .unwrap();
        tree.dispatch_to_node(
            portal,
            Event::new(EventKind::Click(PointerEvent::default())),
        )
        .unwrap();
        assert_eq!(*calls.borrow(), 1);

        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let owner_id = semantics.accessibility_id(owner).unwrap();
        let portal_id = semantics.accessibility_id(portal).unwrap();
        assert!(semantics.nodes()[&owner_id].children.contains(&portal_id));
        assert!(
            !semantics.nodes()[&semantics.accessibility_id(overlay_host).unwrap()]
                .children
                .contains(&portal_id)
        );
    }

    #[test]
    fn tooltip_relation_and_resizable_accessibility_changes_are_semantic_diffs() {
        use crate::{
            AccessibilityAction, AccessibilityActionRequest, AccessibilityRole, SemanticTree,
            TooltipDelays,
        };

        let (mut tree, root) = tree_with_root();
        let trigger = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let tooltip = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let splitter = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.register_pressable(trigger, Some("Help".to_owned()), false)
            .unwrap();
        tree.register_tooltip(
            trigger,
            tooltip,
            TooltipDelays {
                open: Duration::ZERO,
                close: Duration::from_millis(10),
            },
            "Help details",
        )
        .unwrap();
        tree.register_resizable(
            splitter,
            ResizeConfig {
                axis: crate::ResizeAxis::Horizontal,
                min: 80.0,
                max: 300.0,
                step: 10.0,
                reset: 120.0,
            },
            120.0,
            Some("Panel size".to_owned()),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        let initial = semantics.update(&mut tree, root).unwrap();
        let trigger_id = semantics.accessibility_id(trigger).unwrap();
        assert!(semantics.nodes()[&trigger_id].described_by.is_empty());
        assert!(semantics.accessibility_id(tooltip).is_none());

        tree.update_tooltip(
            trigger,
            Duration::ZERO,
            TooltipTriggers {
                trigger_focused: true,
                ..TooltipTriggers::default()
            },
        )
        .unwrap();
        let tooltip_update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(tooltip_update.added.len(), 1);
        assert_eq!(tooltip_update.added[0].role, AccessibilityRole::Tooltip);
        let tooltip_id = semantics.accessibility_id(tooltip).unwrap();
        assert_eq!(semantics.nodes()[&trigger_id].described_by, [tooltip_id]);

        let splitter_id = semantics.accessibility_id(splitter).unwrap();
        semantics
            .route_action(
                &mut tree,
                AccessibilityActionRequest {
                    target: splitter_id,
                    action: AccessibilityAction::Increment,
                },
            )
            .unwrap();
        assert_eq!(tree.resizable_value(splitter), Some(130.0));
        let resize_update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(resize_update.changed.len(), 1);
        assert_eq!(resize_update.changed[0].value.as_deref(), Some("130"));
        assert!(initial.added.len() >= 2);
    }
}
