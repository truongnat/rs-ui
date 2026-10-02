use ui_core::Point;

use crate::{AccessibilityActionKind, AccessibilityRole, AccessibilitySemantics};

/// Host-independent commands for common focused-control behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BehaviorCommand {
    Activate,
    Cancel,
    MoveNext,
    MovePrevious,
    MoveFirst,
    MoveLast,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Increment,
    Decrement,
    Character(char),
}

/// State registered for a node that can be activated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pressable {
    pub disabled: bool,
}

impl Pressable {
    pub const fn new(disabled: bool) -> Self {
        Self { disabled }
    }

    pub const fn disabled(self) -> bool {
        self.disabled
    }

    pub fn semantics(&self) -> AccessibilitySemantics {
        let mut semantics = AccessibilitySemantics::new(AccessibilityRole::Button);
        semantics.actions = vec![AccessibilityActionKind::Press];
        semantics.state.disabled = Some(self.disabled);
        semantics
    }
}

/// Axis used to convert a pointer delta into a resize delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeAxis {
    Horizontal,
    Vertical,
}

/// Bounds and keyboard step for a resizable node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeConfig {
    pub axis: ResizeAxis,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub reset: f32,
}

/// Runtime state for a node registered as resizable.
#[derive(Clone, Debug, PartialEq)]
pub struct Resizable {
    pub(crate) config: ResizeConfig,
    pub(crate) value: f32,
    pub(crate) label: Option<String>,
    pub(crate) drag_start_pointer: Option<Point>,
    pub(crate) drag_start_value: f32,
}

impl Resizable {
    pub(crate) fn new(config: ResizeConfig, value: f32, label: Option<String>) -> Self {
        Self {
            config,
            value,
            label,
            drag_start_pointer: None,
            drag_start_value: value,
        }
    }

    pub(crate) fn keyboard(&mut self, command: BehaviorCommand) -> bool {
        let step = self.config.step;
        let next = match (self.config.axis, command) {
            (ResizeAxis::Horizontal, BehaviorCommand::MoveRight)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveDown)
            | (_, BehaviorCommand::MoveNext)
            | (_, BehaviorCommand::Increment)
            | (_, BehaviorCommand::Activate) => self.value + step,
            (ResizeAxis::Horizontal, BehaviorCommand::MoveLeft)
            | (ResizeAxis::Vertical, BehaviorCommand::MoveUp)
            | (_, BehaviorCommand::MovePrevious)
            | (_, BehaviorCommand::Decrement) => self.value - step,
            (_, BehaviorCommand::MoveFirst) => self.config.min,
            (_, BehaviorCommand::MoveLast) => self.config.max,
            _ => return false,
        };
        let value = next.clamp(self.config.min, self.config.max);
        if value == self.value {
            return false;
        }
        self.value = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use super::*;
    use crate::{
        AccessibilityAction, AccessibilityRole, AccessibilitySemantics, Event, EventKind,
        EventType, FocusPolicy, LayoutStyle, ListenerPhase, NodeId, PaintState, SemanticTree,
        UiTree,
    };

    fn tree_with_root() -> (UiTree, NodeId) {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .unwrap();
        (tree, root)
    }

    fn child(tree: &mut UiTree, parent: NodeId) -> NodeId {
        tree.create_node(Some(parent), LayoutStyle::default(), PaintState::default())
            .unwrap()
    }

    #[test]
    fn activate_dispatches_click_and_disabled_pressable_blocks_it() {
        let (mut tree, root) = tree_with_root();
        let enabled = child(&mut tree, root);
        let disabled = child(&mut tree, root);
        tree.register_pressable(enabled, Some("Run".into()), false)
            .unwrap();
        tree.register_pressable(disabled, Some("Stop".into()), true)
            .unwrap();
        let clicks = Rc::new(Cell::new(0));
        let click_count = Rc::clone(&clicks);
        tree.add_event_listener(
            enabled,
            EventType::Click,
            ListenerPhase::Bubble,
            move |_| {
                click_count.set(click_count.get() + 1);
            },
        )
        .unwrap();
        let click_count = Rc::clone(&clicks);
        tree.add_event_listener(
            disabled,
            EventType::Click,
            ListenerPhase::Bubble,
            move |_| {
                click_count.set(click_count.get() + 1);
            },
        )
        .unwrap();

        tree.request_focus(enabled).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::ZERO)
            .unwrap();
        let event = tree
            .dispatch_to_node(
                disabled,
                Event::new(EventKind::AccessibilityAction(AccessibilityAction::Press)),
            )
            .unwrap();

        assert!(event.default_prevented());
        assert_eq!(clicks.get(), 1);
        assert_eq!(
            tree.node(disabled)
                .unwrap()
                .accessibility
                .as_ref()
                .unwrap()
                .state
                .disabled,
            Some(true)
        );
    }

    #[test]
    fn focus_commands_skip_disabled_and_non_focusable_nodes() {
        let (mut tree, root) = tree_with_root();
        let first = child(&mut tree, root);
        let disabled = child(&mut tree, root);
        let non_focusable = child(&mut tree, root);
        let last = child(&mut tree, root);
        for (node, policy) in [
            (
                first,
                FocusPolicy {
                    focusable: true,
                    ..FocusPolicy::default()
                },
            ),
            (
                disabled,
                FocusPolicy {
                    focusable: true,
                    disabled: true,
                    ..FocusPolicy::default()
                },
            ),
            (non_focusable, FocusPolicy::default()),
            (
                last,
                FocusPolicy {
                    focusable: true,
                    ..FocusPolicy::default()
                },
            ),
        ] {
            tree.set_focus_policy(node, policy).unwrap();
        }
        tree.request_focus(first).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::MoveNext, Duration::ZERO)
            .unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(last));
        tree.dispatch_behavior_command(root, BehaviorCommand::MovePrevious, Duration::ZERO)
            .unwrap();
        assert_eq!(tree.focus_manager().focused(), Some(first));
    }

    fn resize_config(axis: ResizeAxis) -> ResizeConfig {
        ResizeConfig {
            axis,
            min: 10.0,
            max: 20.0,
            step: 2.0,
            reset: 12.0,
        }
    }

    #[test]
    fn resize_uses_configured_pointer_axis_and_clamps() {
        let (mut tree, root) = tree_with_root();
        let horizontal = child(&mut tree, root);
        tree.register_resizable(
            horizontal,
            resize_config(ResizeAxis::Horizontal),
            15.0,
            None,
        )
        .unwrap();
        tree.begin_resize(horizontal, Point::new(5.0, 5.0)).unwrap();
        assert_eq!(
            tree.update_resize(horizontal, Point::new(20.0, 500.0))
                .unwrap(),
            20.0
        );
        assert_eq!(
            tree.update_resize(horizontal, Point::new(0.0, 500.0))
                .unwrap(),
            10.0
        );

        let vertical = child(&mut tree, root);
        tree.register_resizable(vertical, resize_config(ResizeAxis::Vertical), 15.0, None)
            .unwrap();
        tree.begin_resize(vertical, Point::new(50.0, 5.0)).unwrap();
        assert_eq!(
            tree.update_resize(vertical, Point::new(500.0, 12.0))
                .unwrap(),
            20.0
        );
        tree.end_resize(vertical).unwrap();
    }

    #[test]
    fn resize_keyboard_updates_value_and_accessibility_semantics() {
        let (mut tree, root) = tree_with_root();
        let splitter = child(&mut tree, root);
        tree.register_resizable(
            splitter,
            resize_config(ResizeAxis::Horizontal),
            14.0,
            Some("Sidebar".into()),
        )
        .unwrap();
        tree.request_focus(splitter).unwrap();
        tree.dispatch_behavior_command(root, BehaviorCommand::Increment, Duration::ZERO)
            .unwrap();
        assert_eq!(tree.resizable_value(splitter), Some(16.0));
        tree.dispatch_behavior_command(root, BehaviorCommand::Decrement, Duration::ZERO)
            .unwrap();
        assert_eq!(tree.resizable_value(splitter), Some(14.0));
        tree.reset_resizable(splitter).unwrap();
        assert_eq!(tree.resizable_value(splitter), Some(12.0));
        let semantics = tree.node(splitter).unwrap().accessibility.as_ref().unwrap();
        assert_eq!(semantics.role, AccessibilityRole::Separator);
        assert_eq!(semantics.value.as_deref(), Some("12"));
        assert!(
            semantics
                .actions
                .contains(&crate::AccessibilityActionKind::Increment)
        );
        assert!(
            semantics
                .actions
                .contains(&crate::AccessibilityActionKind::Decrement)
        );
    }

    #[test]
    fn selection_helper_preserves_role_and_sets_selected_semantics() {
        let (mut tree, root) = tree_with_root();
        let item = child(&mut tree, root);
        tree.set_accessibility_semantics(
            item,
            AccessibilitySemantics::new(AccessibilityRole::TreeItem),
        )
        .unwrap();
        tree.apply_selection_state(item, true).unwrap();
        let semantics = tree.node(item).unwrap().accessibility.as_ref().unwrap();
        assert_eq!(semantics.role, AccessibilityRole::TreeItem);
        assert_eq!(semantics.state.selected, Some(true));
    }

    #[test]
    fn pressable_focus_and_accessibility_state_stay_aligned() {
        let (mut tree, root) = tree_with_root();
        let button = child(&mut tree, root);
        tree.register_pressable(button, Some("Run query".into()), false)
            .unwrap();
        tree.request_focus(button).unwrap();
        assert_eq!(
            tree.node(button)
                .unwrap()
                .accessibility
                .as_ref()
                .unwrap()
                .label
                .as_deref(),
            Some("Run query")
        );
        let mut semantic_tree = SemanticTree::default();
        semantic_tree.update(&mut tree, root).unwrap();
        let semantic = &semantic_tree.nodes()[&semantic_tree.accessibility_id(button).unwrap()];
        assert_eq!(semantic.state.focused, Some(true));
    }
}
