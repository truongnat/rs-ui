use std::time::Instant;

use ui_core::Size;
use ui_runtime::{
    AccessibilityRole, AccessibilitySemantics, Constraints, FocusPolicy, LayoutStyle, PaintState,
    SemanticTree, UiTree,
};

fn main() {
    let mut tree = UiTree::new();
    let root = tree
        .create_node(None, LayoutStyle::default(), PaintState::default())
        .unwrap();
    const RUNTIME_NODES: usize = 10_000;
    let mut meaningful = Vec::with_capacity(RUNTIME_NODES / 10);
    let mut focus_nodes = Vec::new();
    for index in 0..RUNTIME_NODES {
        let node = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        if index % 10 == 0 {
            let role = if index % 20 == 0 {
                AccessibilityRole::TreeItem
            } else {
                AccessibilityRole::Button
            };
            let mut semantics = AccessibilitySemantics::new(role);
            semantics.label = Some(format!("semantic item {index}"));
            tree.set_accessibility_semantics(node, semantics).unwrap();
            meaningful.push(node);
            if focus_nodes.len() < 2 {
                tree.set_focus_policy(
                    node,
                    FocusPolicy {
                        focusable: true,
                        ..FocusPolicy::default()
                    },
                )
                .unwrap();
                focus_nodes.push(node);
            }
        }
    }
    tree.set_accessibility_semantics(root, AccessibilitySemantics::new(AccessibilityRole::Window))
        .unwrap();
    tree.layout(root, Constraints::loose(Size::new(1_200.0, 800.0)))
        .unwrap();
    tree.request_focus(focus_nodes[0]).unwrap();
    let mut semantics = SemanticTree::default();

    let started = Instant::now();
    let initial = semantics.update(&mut tree, root).unwrap();
    let initial_elapsed = started.elapsed();
    println!(
        "initial_build_10k_runtime_nodes: {:.3} ms, exposed={}, added={}",
        initial_elapsed.as_secs_f64() * 1_000.0,
        semantics.nodes().len(),
        initial.added.len()
    );

    let started = Instant::now();
    for _ in 0..100 {
        let update = semantics.update(&mut tree, root).unwrap();
        assert!(update.is_empty());
    }
    println!(
        "unchanged_update_100_frames: {:.3} ms, visited_last={}, changes_last=0",
        started.elapsed().as_secs_f64() * 1_000.0,
        semantics.stats().visited_runtime_nodes
    );

    let state_node = meaningful[2];
    let mut state = AccessibilitySemantics::new(AccessibilityRole::TreeItem);
    state.label = Some("semantic item 20".to_owned());
    state.state.selected = Some(true);
    let started = Instant::now();
    tree.set_accessibility_semantics(state_node, state).unwrap();
    let state_update = semantics.update(&mut tree, root).unwrap();
    println!(
        "single_node_state_update: {:.3} ms, changed={}, visited={}",
        started.elapsed().as_secs_f64() * 1_000.0,
        state_update.changed.len(),
        semantics.stats().visited_runtime_nodes
    );

    let started = Instant::now();
    tree.request_focus(focus_nodes[1]).unwrap();
    let focus_update = semantics.update(&mut tree, root).unwrap();
    println!(
        "focus_change_and_diff: {:.3} ms, changed={}, focus_changed={}",
        started.elapsed().as_secs_f64() * 1_000.0,
        focus_update.changed.len(),
        focus_update.focus_changed.is_some()
    );
}
