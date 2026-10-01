use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use ui_core::{Point, Rect, Size, Transform};
use ui_runtime::{
    Dimension, Event, EventKind, FocusPolicy, HitTestState, Key, KeyEvent, LayoutStyle, Modifiers,
    PaintState, PointerEvent, UiTree,
};

const NODE_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 200;
const WARMUP_COUNT: usize = 20;

fn main() {
    let (mut tree, root, target) = build_tree();
    let hit_test = measure("pointer move hit-test", || {
        black_box(tree.hit_test(root, Point::new(NODE_COUNT as f32 - 0.5, 10.0)))
    });
    let click_dispatch = measure_batch("click dispatch", 1_000, || {
        black_box(tree.dispatch_to_node(
            target,
            Event::new(EventKind::PointerDown(PointerEvent::default())),
        ))
    });
    let focus_traversal = measure("focus traversal", || {
        black_box(tree.dispatch(
            root,
            Event::new(EventKind::KeyDown(KeyEvent {
                key: Key::Tab,
                modifiers: Modifiers::default(),
                repeat: false,
                timestamp: Duration::ZERO,
            })),
        ))
    });

    println!("nodes={NODE_COUNT} samples={SAMPLE_COUNT}");
    for sample in [hit_test, click_dispatch, focus_traversal] {
        println!(
            "{}: median={:.3}us p95={:.3}us min={:.3}us max={:.3}us",
            sample.name, sample.median_us, sample.p95_us, sample.min_us, sample.max_us,
        );
    }
}

struct BenchmarkSummary {
    name: &'static str,
    median_us: f64,
    p95_us: f64,
    min_us: f64,
    max_us: f64,
}

fn measure<T>(name: &'static str, mut operation: impl FnMut() -> T) -> BenchmarkSummary {
    for _ in 0..WARMUP_COUNT {
        black_box(operation());
    }
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        black_box(operation());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    BenchmarkSummary {
        name,
        median_us: elapsed_us(samples[SAMPLE_COUNT / 2]),
        p95_us: elapsed_us(samples[SAMPLE_COUNT * 95 / 100]),
        min_us: elapsed_us(samples[0]),
        max_us: elapsed_us(samples[SAMPLE_COUNT - 1]),
    }
}

fn measure_batch<T>(
    name: &'static str,
    repetitions: usize,
    mut operation: impl FnMut() -> T,
) -> BenchmarkSummary {
    for _ in 0..WARMUP_COUNT {
        for _ in 0..repetitions {
            black_box(operation());
        }
    }
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        for _ in 0..repetitions {
            black_box(operation());
        }
        samples.push(started.elapsed() / repetitions as u32);
    }
    samples.sort_unstable();
    BenchmarkSummary {
        name,
        median_us: elapsed_us(samples[SAMPLE_COUNT / 2]),
        p95_us: elapsed_us(samples[SAMPLE_COUNT * 95 / 100]),
        min_us: elapsed_us(samples[0]),
        max_us: elapsed_us(samples[SAMPLE_COUNT - 1]),
    }
}

fn elapsed_us(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn build_tree() -> (UiTree, ui_runtime::NodeId, ui_runtime::NodeId) {
    let mut tree = UiTree::new();
    let root = tree
        .create_node(
            None,
            LayoutStyle {
                width: Dimension::Points(NODE_COUNT as f32),
                height: Dimension::Points(20.0),
                ..LayoutStyle::default()
            },
            PaintState::default(),
        )
        .unwrap();
    tree.set_hit_test_state(
        root,
        HitTestState {
            bounds: Rect::from_min_size(Point::ZERO, Size::new(NODE_COUNT as f32, 20.0)),
            ..HitTestState::default()
        },
    )
    .unwrap();
    for index in 0..NODE_COUNT {
        let node = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(
            node,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(1.0, 20.0)),
                transform: Transform::translation(index as f32, 0.0),
                z_order: index as i32,
                ..HitTestState::default()
            },
        )
        .unwrap();
        tree.set_focus_policy(
            node,
            FocusPolicy {
                focusable: true,
                ..FocusPolicy::default()
            },
        )
        .unwrap();
        if index == 0 {
            tree.request_focus(node).unwrap();
        }
    }
    let target = tree
        .hit_test(root, Point::new(NODE_COUNT as f32 - 0.5, 10.0))
        .unwrap()
        .last()
        .copied()
        .unwrap();
    (tree, root, target)
}
