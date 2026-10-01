use std::{cell::Cell, rc::Rc};

use ui_core::{Point, Rect, Size};
use ui_runtime::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityRole, AccessibilitySemantics,
    AccessibilityState, Constraints, Dimension, EventType, FocusPolicy,
    HeadlessAccessibilityBackend, HitTestState, LayoutStyle, ListenerPhase, PaintState,
    SemanticTree, TextEditor, UiTree,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut tree, root, button, text_input, dialog) = build_demo()?;
    let mut semantics = SemanticTree::default();
    let mut backend = HeadlessAccessibilityBackend::default();

    let initial = semantics.update_backend(&mut tree, root, &mut backend)?;
    println!("Initial semantic tree ({} nodes):", backend.nodes().len());
    print_semantic_tree(&semantics, initial.roots.first().copied(), 0);

    let unchanged = semantics.update_backend(&mut tree, root, &mut backend)?;
    println!(
        "Unchanged frame: {} semantic changes",
        unchanged.change_count()
    );

    let button_id = semantics
        .accessibility_id(button)
        .expect("button has semantics");
    let activation_count = Rc::new(Cell::new(0));
    let observed = activation_count.clone();
    tree.add_event_listener(button, EventType::Click, ListenerPhase::Bubble, move |_| {
        observed.set(observed.get() + 1);
    })?;
    semantics.route_action(
        &mut tree,
        AccessibilityActionRequest {
            target: button_id,
            action: AccessibilityAction::Press,
        },
    )?;
    println!(
        "Headless Press routed through Click: {}",
        activation_count.get()
    );

    let text_input_id = semantics
        .accessibility_id(text_input)
        .expect("text input has semantics");
    semantics.route_action(
        &mut tree,
        AccessibilityActionRequest {
            target: text_input_id,
            action: AccessibilityAction::SetSelection { anchor: 1, head: 4 },
        },
    )?;
    semantics.route_action(
        &mut tree,
        AccessibilityActionRequest {
            target: text_input_id,
            action: AccessibilityAction::ReplaceSelectedText("runtime".to_owned()),
        },
    )?;
    let edit = semantics.update_backend(&mut tree, root, &mut backend)?;
    println!(
        "Text edit localized diff: {} changed node(s)",
        edit.change_count()
    );

    let mut dialog_spec = AccessibilitySemantics::new(AccessibilityRole::Dialog);
    dialog_spec.label = Some("Fake dialog test".to_owned());
    dialog_spec.state.expanded = Some(true);
    tree.set_accessibility_semantics(dialog, dialog_spec)?;
    let dialog_update = semantics.update_backend(&mut tree, root, &mut backend)?;
    println!(
        "Dialog reveal diff: {} added node(s)",
        dialog_update.added.len()
    );
    print_semantic_tree(&semantics, dialog_update.roots.first().copied(), 0);
    Ok(())
}

fn build_demo() -> Result<
    (
        UiTree,
        ui_runtime::NodeId,
        ui_runtime::NodeId,
        ui_runtime::NodeId,
        ui_runtime::NodeId,
    ),
    ui_runtime::RuntimeError,
> {
    let mut tree = UiTree::new();
    let root = tree.create_node(
        None,
        LayoutStyle {
            width: Dimension::Points(800.0),
            height: Dimension::Points(600.0),
            ..LayoutStyle::default()
        },
        PaintState::default(),
    )?;
    let toolbar = tree.create_node(Some(root), LayoutStyle::default(), PaintState::default())?;
    let button = tree.create_node(Some(toolbar), LayoutStyle::default(), PaintState::default())?;
    let label = tree.create_node(Some(root), LayoutStyle::default(), PaintState::default())?;
    let text_input = tree.create_node(Some(root), LayoutStyle::default(), PaintState::default())?;
    let scroll = tree.create_node(Some(root), LayoutStyle::default(), PaintState::default())?;
    let tree_node =
        tree.create_node(Some(scroll), LayoutStyle::default(), PaintState::default())?;
    let first_item = tree.create_node(
        Some(tree_node),
        LayoutStyle::default(),
        PaintState::default(),
    )?;
    let second_item = tree.create_node(
        Some(tree_node),
        LayoutStyle::default(),
        PaintState::default(),
    )?;
    let dialog = tree.create_node(Some(root), LayoutStyle::default(), PaintState::default())?;

    tree.set_accessibility_semantics(
        root,
        named(AccessibilityRole::Window, "rs-ui accessibility demo"),
    )?;
    tree.set_accessibility_semantics(
        toolbar,
        AccessibilitySemantics::new(AccessibilityRole::Group),
    )?;
    tree.set_accessibility_semantics(button, named(AccessibilityRole::Button, "Run action"))?;
    tree.set_focus_policy(
        button,
        FocusPolicy {
            focusable: true,
            ..FocusPolicy::default()
        },
    )?;
    tree.set_accessibility_text_content(label, "Editor value")?;
    tree.set_accessibility_semantics(label, AccessibilitySemantics::new(AccessibilityRole::Text))?;
    let mut input = named(AccessibilityRole::TextInput, "Editor");
    input.labelled_by.push(label);
    input.state = AccessibilityState {
        read_only: Some(false),
        required: Some(true),
        ..AccessibilityState::default()
    };
    input.multiline = Some(true);
    tree.set_accessibility_semantics(text_input, input)?;
    tree.state_mut()
        .insert(text_input, TextEditor::new("Phase ten editor"));
    tree.set_focus_policy(
        text_input,
        FocusPolicy {
            focusable: true,
            tab_index: 1,
            ..FocusPolicy::default()
        },
    )?;
    tree.set_accessibility_semantics(scroll, named(AccessibilityRole::ScrollArea, "Navigation"))?;
    tree.attach_scroll_view(scroll, Size::new(220.0, 160.0), Size::new(220.0, 480.0))?;
    tree.set_accessibility_semantics(tree_node, named(AccessibilityRole::Tree, "Project tree"))?;
    let mut first = named(AccessibilityRole::TreeItem, "ui-runtime");
    first.state.selected = Some(true);
    first.state.expanded = Some(true);
    tree.set_accessibility_semantics(first_item, first)?;
    tree.set_accessibility_semantics(second_item, named(AccessibilityRole::TreeItem, "ui-window"))?;
    let mut hidden_dialog = named(AccessibilityRole::Dialog, "Fake dialog test");
    hidden_dialog.hidden = true;
    tree.set_accessibility_semantics(dialog, hidden_dialog)?;

    tree.set_hit_test_state(
        root,
        HitTestState {
            bounds: Rect::from_min_size(Point::ZERO, Size::new(800.0, 600.0)),
            ..HitTestState::default()
        },
    )?;
    tree.layout(root, Constraints::loose(Size::new(800.0, 600.0)))?;
    Ok((tree, root, button, text_input, dialog))
}

fn named(role: AccessibilityRole, label: &str) -> AccessibilitySemantics {
    let mut semantics = AccessibilitySemantics::new(role);
    semantics.label = Some(label.to_owned());
    semantics
}

fn print_semantic_tree(
    semantics: &SemanticTree,
    id: Option<ui_runtime::AccessibilityId>,
    depth: usize,
) {
    let Some(id) = id else { return };
    let Some(node) = semantics.nodes().get(&id) else {
        return;
    };
    println!(
        "{}{:?} #{} label={:?} value={:?} bounds={:?}",
        "  ".repeat(depth),
        node.role,
        id.get(),
        node.label,
        node.value,
        node.bounds
    );
    for child in &node.children {
        print_semantic_tree(semantics, Some(*child), depth + 1);
    }
}
