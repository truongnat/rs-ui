use std::{cell::Cell, rc::Rc, time::Duration};

use ui_core::{Point, Rect, Size, Transform};
use ui_runtime::{
    AccessibilityBackend, AccessibilityRole, AccessibilitySemantics, BehaviorCommand, EventType,
    FocusPolicy, HeadlessAccessibilityBackend, HitTestState, LayerSpec, LayoutStyle, ListenerPhase,
    OrderedSelectionModel, OutsidePointerPolicy, PaintState, PointerButton, PointerEvent,
    PopoverAlignment, PopoverConfig, PopoverSide, ResizeAxis, ResizeConfig, SemanticTree,
    TooltipDelays, TooltipTriggers, UiTree,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = UiTree::new();
    let root = make_node(&mut tree, None)?;
    tree.set_accessibility_semantics(
        root,
        named(AccessibilityRole::Window, "Behavior primitives"),
    )?;
    tree.set_hit_test_state(
        root,
        hit(
            Rect::from_min_size(Point::ZERO, Size::new(800.0, 600.0)),
            true,
        ),
    )?;

    let anchor = make_node(&mut tree, Some(root))?;
    set_bounds(&mut tree, anchor, 24.0, 24.0, 120.0, 40.0)?;
    tree.register_pressable(anchor, Some("Open actions".into()), false)?;

    let overlay = make_node(&mut tree, Some(root))?;
    set_bounds(&mut tree, overlay, 300.0, 80.0, 220.0, 180.0)?;
    tree.set_accessibility_semantics(overlay, named(AccessibilityRole::Group, "Actions popover"))?;
    tree.set_hit_test_state(
        overlay,
        HitTestState {
            visible: false,
            ..hit(
                Rect::from_min_size(Point::ZERO, Size::new(220.0, 180.0)),
                true,
            )
        },
    )?;

    let portal = make_node(&mut tree, Some(root))?;
    tree.set_accessibility_semantics(
        portal,
        named(AccessibilityRole::Group, "Portaled action details"),
    )?;

    let tooltip = make_node(&mut tree, Some(root))?;
    tree.register_tooltip(
        anchor,
        tooltip,
        TooltipDelays {
            open: Duration::from_millis(250),
            close: Duration::from_millis(100),
        },
        "Actions available for this region",
    )?;

    let dialog = make_node(&mut tree, Some(root))?;
    set_bounds(&mut tree, dialog, 200.0, 120.0, 360.0, 260.0)?;
    let confirm = make_node(&mut tree, Some(dialog))?;
    tree.register_pressable(confirm, Some("Confirm".into()), false)?;
    tree.set_hit_test_state(
        dialog,
        HitTestState {
            visible: false,
            ..hit(
                Rect::from_min_size(Point::ZERO, Size::new(360.0, 260.0)),
                true,
            )
        },
    )?;
    tree.set_accessibility_semantics(dialog, named(AccessibilityRole::Dialog, "Confirmation"))?;
    tree.set_accessibility_semantics(confirm, named(AccessibilityRole::Button, "Confirm"))?;

    let menu = make_node(&mut tree, Some(root))?;
    let open_item = make_node(&mut tree, Some(menu))?;
    let disabled_item = make_node(&mut tree, Some(menu))?;
    let close_item = make_node(&mut tree, Some(menu))?;
    tree.install_menu(
        menu,
        vec![
            (open_item, "Open".into(), false),
            (disabled_item, "Unavailable".into(), true),
            (close_item, "Close".into(), false),
        ],
        Duration::from_millis(700),
    )?;
    tree.set_hit_test_state(
        menu,
        HitTestState {
            visible: false,
            ..hit(
                Rect::from_min_size(Point::ZERO, Size::new(180.0, 130.0)),
                true,
            )
        },
    )?;

    let splitter = make_node(&mut tree, Some(root))?;
    set_bounds(&mut tree, splitter, 400.0, 20.0, 8.0, 500.0)?;
    tree.register_resizable(
        splitter,
        ResizeConfig {
            axis: ResizeAxis::Horizontal,
            min: 180.0,
            max: 520.0,
            step: 12.0,
            reset: 320.0,
        },
        320.0,
        Some("Sidebar width".into()),
    )?;

    let rows = (0..3)
        .map(|_| make_node(&mut tree, Some(root)))
        .collect::<Result<Vec<_>, _>>()?;
    for (index, row) in rows.iter().copied().enumerate() {
        tree.set_accessibility_semantics(
            row,
            named(AccessibilityRole::ListItem, &format!("Row {}", index + 1)),
        )?;
    }
    let mut selection = OrderedSelectionModel::multiple();
    let keys = ["stable-a", "stable-b", "stable-c"];
    selection.select(keys[0]);
    selection.select_range(&keys, keys[2], false);
    for (row, key) in rows.iter().copied().zip(keys) {
        tree.apply_selection_state(row, selection.is_selected(&key))?;
    }

    let activations = Rc::new(Cell::new(0));
    let observed = activations.clone();
    tree.add_event_listener(anchor, EventType::Click, ListenerPhase::Bubble, move |_| {
        observed.set(observed.get() + 1)
    })?;
    let pointer = PointerEvent {
        position: Point::new(60.0, 42.0),
        button: Some(PointerButton::Primary),
        timestamp: Duration::from_millis(1),
        ..PointerEvent::default()
    };
    tree.pointer_down(root, pointer)?;
    tree.pointer_up(
        root,
        PointerEvent {
            timestamp: Duration::from_millis(10),
            ..pointer
        },
    )?;
    tree.request_focus(anchor)?;
    tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::from_millis(20))?;

    let popover_layer = tree.open_layer(LayerSpec {
        node: overlay,
        z_layer: 20,
        outside_pointer: OutsidePointerPolicy::DismissAndContinue,
        dismiss_on_escape: true,
        ..LayerSpec::default()
    })?;
    tree.register_portal(portal, anchor, popover_layer)?;
    let placement = tree.position_popover(
        anchor,
        (220.0, 180.0),
        Rect::from_min_size(Point::ZERO, Size::new(800.0, 600.0)),
        PopoverConfig {
            preferred_side: PopoverSide::Bottom,
            alignment: PopoverAlignment::Start,
            offset: 8.0,
        },
    )?;
    tree.update_tooltip(
        anchor,
        Duration::from_millis(300),
        TooltipTriggers {
            trigger_focused: true,
            ..TooltipTriggers::default()
        },
    )?;
    tree.update_tooltip(
        anchor,
        Duration::from_millis(550),
        TooltipTriggers {
            trigger_focused: true,
            ..TooltipTriggers::default()
        },
    )?;

    tree.open_menu(menu)?;
    let menu_layer = tree.open_layer(LayerSpec {
        node: menu,
        z_layer: 30,
        focus_scope: Some(menu),
        outside_pointer: OutsidePointerPolicy::DismissAndBlock,
        dismiss_on_escape: true,
        ..LayerSpec::default()
    })?;
    tree.dispatch_behavior_command(root, BehaviorCommand::MoveNext, Duration::from_millis(400))?;
    let menu_active = tree.focus_manager().focused();
    tree.dispatch_behavior_command(root, BehaviorCommand::Cancel, Duration::from_millis(500))?;
    let dismissed = tree.take_layer_dismiss_requests();
    if dismissed.iter().any(|request| request.layer == menu_layer) {
        tree.close_layer(menu_layer)?;
    }
    let next_escape = tree.layer_stack().escape();

    let (mut second_tree, second_root) = modal_fixture()?;
    let dialog_id = second_root.1;
    let previous_focus = second_root.0;
    let layer = second_tree.open_layer(LayerSpec {
        node: dialog_id,
        z_layer: 100,
        modal: true,
        blocks_pointer: true,
        outside_pointer: OutsidePointerPolicy::DismissAndBlock,
        dismiss_on_escape: true,
        focus_scope: Some(dialog_id),
        initial_focus: Some(second_root.2),
        ..LayerSpec::default()
    })?;
    let modal_focus = second_tree.focus_manager().focused();
    second_tree.close_layer(layer)?;
    let restored_focus = second_tree.focus_manager().focused();

    let mut semantic_tree = SemanticTree::default();
    let update = semantic_tree.update(&mut tree, root)?;
    let mut backend = HeadlessAccessibilityBackend::default();
    backend.update(&update);
    println!(
        "pressable: pointer + keyboard activations={}, focus={:?}",
        activations.get(),
        tree.focus_manager().focused()
    );
    println!(
        "focus scopes: modal initial={modal_focus:?}, restored={restored_focus:?}, expected prior={previous_focus:?}"
    );
    println!(
        "layers: open={:?}, Escape requested={:?}",
        tree.layer_stack()
            .ordered()
            .iter()
            .map(|layer| layer.id.get())
            .collect::<Vec<_>>(),
        next_escape.map(|request| request.node.get())
    );
    println!(
        "popover: side={:?}, bounds={:?}; tooltip described_by={:?}",
        placement.side,
        placement.rect,
        semantic_tree
            .nodes()
            .get(&semantic_tree.accessibility_id(anchor).unwrap())
            .map(|node| &node.described_by)
    );
    println!(
        "menu: active focus={menu_active:?}; selection={:?}; pointer capture={:?}",
        selection.selected(),
        tree.pointer_state().captured_target
    );
    println!(
        "resize: value={}, semantic updates={}, rows={rows:?}, other nested layer={popover_layer:?}",
        tree.resizable_value(splitter).unwrap(),
        update.change_count()
    );
    println!("primitives_demo headless smoke: ok");
    Ok(())
}

fn modal_fixture() -> Result<
    (
        UiTree,
        (ui_runtime::NodeId, ui_runtime::NodeId, ui_runtime::NodeId),
    ),
    ui_runtime::RuntimeError,
> {
    let mut tree = UiTree::new();
    let root = make_node(&mut tree, None)?;
    let before = make_node(&mut tree, Some(root))?;
    let dialog = make_node(&mut tree, Some(root))?;
    let first = make_node(&mut tree, Some(dialog))?;
    tree.set_focus_policy(
        before,
        FocusPolicy {
            focusable: true,
            ..FocusPolicy::default()
        },
    )?;
    tree.set_focus_policy(
        first,
        FocusPolicy {
            focusable: true,
            ..FocusPolicy::default()
        },
    )?;
    tree.request_focus(before)?;
    Ok((tree, (before, dialog, first)))
}

fn make_node(
    tree: &mut UiTree,
    parent: Option<ui_runtime::NodeId>,
) -> Result<ui_runtime::NodeId, ui_runtime::RuntimeError> {
    tree.create_node(parent, LayoutStyle::default(), PaintState::default())
}

fn set_bounds(
    tree: &mut UiTree,
    node: ui_runtime::NodeId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<(), ui_runtime::RuntimeError> {
    tree.set_hit_test_state(
        node,
        HitTestState {
            bounds: Rect::from_min_size(Point::ZERO, Size::new(width, height)),
            transform: Transform::translation(x, y),
            ..HitTestState::default()
        },
    )
}

fn hit(bounds: Rect, visible: bool) -> HitTestState {
    HitTestState {
        bounds,
        visible,
        ..HitTestState::default()
    }
}

fn named(role: AccessibilityRole, label: &str) -> AccessibilitySemantics {
    let mut semantics = AccessibilitySemantics::new(role);
    semantics.label = Some(label.to_owned());
    semantics
}
