use std::hint::black_box;
use std::time::Instant;
use ui_core::{Color, Point};
use ui_runtime::{TextBuffer, TextPosition, TextRange};
use ui_text::{TextStyle, TextSystem};

fn elapsed(label: &str, start: Instant) {
    println!("{label}: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
}

fn main() {
    let single = "a".repeat(10_000);
    let lines = (0..10_000)
        .map(|_| "line 0123456789")
        .collect::<Vec<_>>()
        .join("\n");

    let start = Instant::now();
    let buffer = TextBuffer::new(&single);
    black_box(buffer.len());
    elapsed("construct_10k_char", start);

    let start = Instant::now();
    let mut position = TextPosition::new(0);
    for _ in 0..10_000 {
        position = buffer.next(position);
    }
    black_box(position);
    elapsed("cursor_move_time_10k", start);

    let start = Instant::now();
    let mut edited = TextBuffer::new(&single);
    for i in 0..1_000 {
        let p = TextPosition::new(i % 10_000);
        edited.insert(p, "x");
        edited.delete(TextRange::new(p, TextPosition::new(p.grapheme_index() + 1)));
    }
    black_box(edited);
    elapsed("edit_time_1000_small_edits", start);

    let mut text = TextSystem::new();
    let style = TextStyle {
        color: Color::WHITE,
        ..TextStyle::default()
    };
    let start = Instant::now();
    let run = text.shape(&lines, style, None);
    elapsed("layout_time_shape_10k_lines", start);
    let mut reshaped_runs = 1;
    let start = Instant::now();
    for _ in 0..3 {
        text.update_run(run, &lines, style, None).unwrap();
        reshaped_runs += 1;
    }
    elapsed("layout_time_unchanged_text_3_full_reshapes", start);
    let start = Instant::now();
    black_box(text.position_to_point(run, 100_000));
    black_box(text.selection_rects(run, 100, 100_000));
    elapsed("selection_geometry_time_10k_lines", start);

    let start = Instant::now();
    for _ in 0..100 {
        black_box(text.point_to_position(run, Point::new(42.0, 100.0)));
    }
    elapsed("hit_test_100", start);
    println!("reshaped_runs: {reshaped_runs} full shaped runs; allocation counts unavailable");
}
