use std::time::Instant;
use ui_core::{Color, Point};
use ui_text::{DirtyLineRange, TextDocumentLayout, TextStyle, TextSystem};

fn report(label: &str, start: Instant) {
    println!("{label}: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
}

fn materialize(
    layout: &mut TextDocumentLayout,
    text: &mut TextSystem,
    lines: &[String],
    range: std::ops::Range<usize>,
    overscan: usize,
    style: TextStyle,
) -> usize {
    layout
        .layout_visible_lines(text, range, overscan, style, None, |index| {
            lines.get(index).cloned().unwrap_or_default()
        })
        .expect("document layout")
        .len()
}

fn main() {
    let style = TextStyle {
        color: Color::WHITE,
        ..TextStyle::default()
    };
    let mut text = TextSystem::new();
    let mut lines = (0..10_000)
        .map(|index| format!("line {index:05} contents to shape"))
        .collect::<Vec<_>>();
    let joined = lines.join("\n");
    let start = Instant::now();
    let baseline_run = text.shape(&joined, style, None);
    report("monolithic_initial_shape_10k_lines_baseline", start);
    text.remove_run(baseline_run);
    let lengths = lines
        .iter()
        .map(|line| line.chars().count())
        .collect::<Vec<_>>();
    let mut document = TextDocumentLayout::new(lengths, style.line_height_px);

    let start = Instant::now();
    materialize(&mut document, &mut text, &lines, 0..10_000, 0, style);
    report("initial_shape_10k_lines", start);
    println!(
        "initial_lines_shaped: {}",
        document.take_stats().lines_shaped
    );

    let shape_before = text.shape_call_count();
    let start = Instant::now();
    for _ in 0..100 {
        materialize(&mut document, &mut text, &lines, 0..10_000, 0, style);
    }
    report("unchanged_layout_10k_lines_100_frames", start);
    println!(
        "unchanged_frames_reshaped_lines: {}",
        text.shape_call_count() - shape_before
    );
    document.take_stats();

    let changed = 5_000;
    lines[changed].push('!');
    document
        .apply_edit(
            &mut text,
            DirtyLineRange {
                start: changed,
                removed: 1,
                inserted: 1,
            },
            &[lines[changed].chars().count()],
            style.line_height_px,
        )
        .unwrap();
    let shape_before = text.shape_call_count();
    let start = Instant::now();
    materialize(
        &mut document,
        &mut text,
        &lines,
        changed..changed + 1,
        0,
        style,
    );
    report("single_character_edit_layout", start);
    let stats = document.take_stats();
    println!(
        "single_character_edit: lines_invalidated={}, lines_reshaped={}",
        stats.lines_invalidated,
        text.shape_call_count() - shape_before
    );

    let first = lines.remove(changed);
    let split = first.len() / 2;
    lines.splice(
        changed..changed,
        [first[..split].to_owned(), first[split..].to_owned()],
    );
    document
        .apply_edit(
            &mut text,
            DirtyLineRange {
                start: changed,
                removed: 1,
                inserted: 2,
            },
            &[
                first[..split].chars().count(),
                first[split..].chars().count(),
            ],
            style.line_height_px,
        )
        .unwrap();
    let shape_before = text.shape_call_count();
    let start = Instant::now();
    materialize(
        &mut document,
        &mut text,
        &lines,
        changed..changed + 2,
        0,
        style,
    );
    report("newline_insert_layout", start);
    let stats = document.take_stats();
    println!(
        "newline_insert: lines_invalidated={}, lines_reshaped={}",
        stats.lines_invalidated,
        text.shape_call_count() - shape_before
    );

    let paste = (0..101)
        .map(|index| format!("pasted line {index}"))
        .collect::<Vec<_>>();
    lines.splice(changed..changed + 2, paste.clone());
    let paste_lengths = paste
        .iter()
        .map(|line| line.chars().count())
        .collect::<Vec<_>>();
    document
        .apply_edit(
            &mut text,
            DirtyLineRange {
                start: changed,
                removed: 2,
                inserted: paste.len(),
            },
            &paste_lengths,
            style.line_height_px,
        )
        .unwrap();
    let shape_before = text.shape_call_count();
    let start = Instant::now();
    materialize(
        &mut document,
        &mut text,
        &lines,
        changed..changed + paste.len(),
        0,
        style,
    );
    report("multiline_paste_100_layout", start);
    let stats = document.take_stats();
    println!(
        "multiline_paste: lines_invalidated={}, lines_reshaped={}",
        stats.lines_invalidated,
        text.shape_call_count() - shape_before
    );
    println!(
        "cache_memory_10k_lines_estimate: {} bytes",
        document.estimated_cache_bytes(&text)
    );

    let mut large_document =
        TextDocumentLayout::new(std::iter::repeat_n(18, 100_000), style.line_height_px);
    large_document.set_max_cached_lines(Some(256));
    let large_lines = (0..100_000)
        .map(|index| format!("large row {index}"))
        .collect::<Vec<_>>();
    let start = Instant::now();
    let materialized = materialize(
        &mut large_document,
        &mut text,
        &large_lines,
        50_000..50_050,
        10,
        style,
    );
    report("viewport_prepare_100k_lines", start);
    let stats = large_document.take_stats();
    println!(
        "viewport_100k: lines_materialized={materialized}, lines_shaped={}, cache_memory_estimate={} bytes",
        stats.lines_shaped,
        large_document.estimated_cache_bytes(&text)
    );

    let visible = large_document
        .layout_visible_lines(&mut text, 50_000..50_050, 10, style, None, |index| {
            large_lines[index].clone()
        })
        .unwrap();
    let shape_before = text.shape_call_count();
    for line in visible.iter().take(3) {
        std::hint::black_box(large_document.hit_test(&text, Point::new(8.0, line.top + 5.0)));
    }
    println!(
        "visible_hit_tests_reshaped_lines: {}",
        text.shape_call_count() - shape_before
    );
}
