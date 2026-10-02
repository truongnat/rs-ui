use std::time::{Duration, Instant};

use ui_core::{Point, Rect, Size, Transform};
use ui_runtime::{
    AccessibilityRole, AccessibilitySemantics, BehaviorCommand, DirtyFlags, HitTestState,
    LayerSpec, LayoutStyle, MenuItem, MenuModel, OrderedSelectionModel, OutsidePointerPolicy,
    PaintState, ResizablePrimitive, ResizeAxis, ResizeConfig, SemanticTree, UiTree,
};

fn node(tree: &mut UiTree, parent: Option<ui_runtime::NodeId>) -> ui_runtime::NodeId {
    tree.create_node(parent, LayoutStyle::default(), PaintState::default())
        .unwrap()
}

fn timed(label: &str, operation: impl FnOnce() -> String) {
    let started = Instant::now();
    let details = operation();
    println!(
        "{label}: {:.3} ms {details}",
        started.elapsed().as_secs_f64() * 1_000.0
    );
}

fn main() {
    const NODE_COUNT: usize = 10_000;
    const OPERATIONS: usize = 1_000;

    let mut tree = UiTree::new();
    let root = node(&mut tree, None);
    let mut buttons = Vec::with_capacity(NODE_COUNT);
    for index in 0..NODE_COUNT {
        let button = node(&mut tree, Some(root));
        tree.register_pressable(button, Some(format!("Button {index}")), false)
            .unwrap();
        buttons.push(button);
    }
    tree.set_accessibility_semantics(root, AccessibilitySemantics::new(AccessibilityRole::Window))
        .unwrap();
    tree.request_focus(buttons[0]).unwrap();
    let mut semantics = SemanticTree::default();
    let initial = semantics.update(&mut tree, root).unwrap();
    let initial_semantics = initial.added.len();
    tree.paint(root).unwrap();

    timed("event_dispatch_10k_pressables_1k_activate", || {
        for index in 0..OPERATIONS {
            tree.request_focus(buttons[index % 2]).unwrap();
            tree.dispatch_behavior_command(root, BehaviorCommand::Activate, Duration::ZERO)
                .unwrap();
        }
        let diff = semantics.update(&mut tree, root).unwrap();
        format!("semantic_updates={}", diff.changed.len())
    });

    let started = Instant::now();
    for index in 0..OPERATIONS {
        tree.request_focus(buttons[index % 2]).unwrap();
    }
    let focus_diff = semantics.update(&mut tree, root).unwrap();
    println!(
        "focus_updates_1k: {:.3} ms semantic_updates={}",
        started.elapsed().as_secs_f64() * 1_000.0,
        focus_diff.changed.len()
    );

    timed("primitive_state_mutation_10k_pressables", || {
        let mut paint_invalidations = 0usize;
        for button in &buttons {
            tree.set_pressable_disabled(*button, true).unwrap();
            paint_invalidations += usize::from(
                tree.node(*button)
                    .unwrap()
                    .dirty_flags()
                    .contains(DirtyFlags::PAINT),
            );
        }
        let diff = semantics.update(&mut tree, root).unwrap();
        format!(
            "paint_invalidations={paint_invalidations} semantic_updates={}",
            diff.changed.len()
        )
    });

    let mut hit_tree = UiTree::new();
    let hit_root = node(&mut hit_tree, None);
    hit_tree
        .set_hit_test_state(
            hit_root,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(1_000.0, 1_000.0)),
                transform: Transform::IDENTITY,
                ..HitTestState::default()
            },
        )
        .unwrap();
    let mut deepest = hit_root;
    for depth in 0..32 {
        deepest = node(&mut hit_tree, Some(deepest));
        hit_tree
            .set_hit_test_state(
                deepest,
                HitTestState {
                    bounds: Rect::from_min_size(
                        Point::ZERO,
                        Size::new(900.0 - depth as f32, 900.0),
                    ),
                    ..HitTestState::default()
                },
            )
            .unwrap();
    }
    timed("nested_hit_test_depth_32_1k", || {
        let mut depth = 0;
        for _ in 0..OPERATIONS {
            depth = hit_tree
                .hit_test(hit_root, Point::new(10.0, 10.0))
                .unwrap()
                .len();
        }
        format!("last_path_depth={depth}")
    });

    let mut layer_tree = UiTree::new();
    let layer_root = node(&mut layer_tree, None);
    let panel = node(&mut layer_tree, Some(layer_root));
    timed("popover_layer_open_close_100", || {
        for _ in 0..100 {
            let id = layer_tree
                .open_layer(LayerSpec {
                    node: panel,
                    outside_pointer: OutsidePointerPolicy::DismissAndBlock,
                    ..LayerSpec::default()
                })
                .unwrap();
            layer_tree.close_layer(id).unwrap();
        }
        "cycles=100".to_owned()
    });

    let menu_items = (0..1_000)
        .map(|key| MenuItem {
            key,
            label: format!("Item {key}"),
            disabled: key % 17 == 0,
        })
        .collect();
    let mut menu = MenuModel::new(menu_items, Duration::from_millis(500));
    timed("menu_navigation_1k_items_1k_commands", || {
        menu.open();
        for index in 0..OPERATIONS {
            menu.command(
                if index % 2 == 0 {
                    BehaviorCommand::MoveNext
                } else {
                    BehaviorCommand::MovePrevious
                },
                Duration::from_millis(index as u64),
            );
        }
        format!("active={:?}", menu.active_key())
    });

    let keys = (0..100_000_u32).collect::<Vec<_>>();
    let mut selection = OrderedSelectionModel::multiple();
    timed("selection_stable_keys_100k", || {
        selection.select_all(&keys, |key| key % 3 != 0);
        for key in (0..10_000_u32).step_by(2) {
            selection.toggle(key);
        }
        format!("selected={}", selection.selected().len())
    });

    let mut resizable = ResizablePrimitive::new(
        ResizeConfig {
            axis: ResizeAxis::Horizontal,
            min: 80.0,
            max: 900.0,
            step: 8.0,
            reset: 320.0,
        },
        320.0,
    );
    timed("resize_pointer_stream_10k", || {
        let start = Point::new(0.0, 0.0);
        resizable.begin_pointer(start);
        let mut mutations = 0;
        for index in 0..10_000 {
            mutations += usize::from(resizable.pointer_move(Point::new(index as f32, 0.0)));
        }
        resizable.end_pointer();
        format!("state_mutations={mutations} value={:.1}", resizable.value())
    });

    println!("summary: initial_semantic_updates={initial_semantics}, pressable_nodes={NODE_COUNT}");
}
