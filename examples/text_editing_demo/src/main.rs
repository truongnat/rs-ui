use std::{
    ops::Range,
    time::{Duration, Instant},
};
use ui_core::{Color, DisplayListBuilder, Point, Rect, Size, Srgb8, Stroke};
use ui_renderer::{RenderFrame, RendererOptions, UiRenderer, Viewport};
use ui_runtime::{
    Clipboard, EditCommand, FocusPolicy, LayoutStyle, PaintState, Selection, TextEditor,
    TextPosition, TextRange, UiTree,
};
use ui_text::{
    DirtyLineRange as LayoutDirtyLineRange, FontWeight, TextDocumentLayout, TextDocumentLine,
    TextStyle, TextSystem,
};
use ui_window::{PlatformImeEvent, PlatformTextCommand, PlatformTextInput, UiWindow, WindowConfig};
use unicode_segmentation::UnicodeSegmentation;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::ModifiersState,
    window::WindowId,
};

const TEXT_ORIGIN: Point = Point::new(28.0, 116.0);
const VIEWPORT_MARGIN: f32 = 28.0;

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: UiRenderer,
}

#[derive(Default)]
struct DemoClipboard {
    system: Option<arboard::Clipboard>,
    local_copy: Option<String>,
    initialized: bool,
}
impl Clipboard for DemoClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.initialize();
        if let Some(clipboard) = self.system.as_mut() {
            clipboard.get_text().ok()
        } else {
            self.local_copy.clone()
        }
    }
    fn set_text(&mut self, text: &str) {
        self.initialize();
        self.local_copy = Some(text.to_owned());
        if let Some(clipboard) = self.system.as_mut()
            && let Err(error) = clipboard.set_text(text)
        {
            eprintln!("system clipboard write failed; using local clipboard: {error}");
            self.system = None;
        }
    }
}
impl DemoClipboard {
    fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        match arboard::Clipboard::new() {
            Ok(clipboard) => self.system = Some(clipboard),
            Err(error) => eprintln!("system clipboard unavailable; using local clipboard: {error}"),
        }
    }
    fn local() -> Self {
        Self {
            initialized: true,
            ..Self::default()
        }
    }
}

struct Demo {
    window: Option<UiWindow>,
    gpu: Option<Gpu>,
    editor: TextEditor,
    focus_tree: UiTree,
    editor_node: ui_runtime::NodeId,
    clipboard: DemoClipboard,
    text: TextSystem,
    document_layout: TextDocumentLayout,
    visible_lines: Vec<TextDocumentLine>,
    debug_run: ui_core::TextRunId,
    presentation: String,
    presentation_lines: Vec<Range<usize>>,
    was_composing: bool,
    modifiers: ModifiersState,
    cursor: Point,
    scroll: Point,
    dragging: bool,
    focused: bool,
    surface_occluded: bool,
    surface_issue_reported: bool,
    last_click: Instant,
    click_position: Point,
    click_count: u8,
    blink_started: Instant,
    caret_blink_visible: bool,
}

impl Demo {
    fn new(clipboard: DemoClipboard) -> Self {
        let initial = "Xin chào Việt Nam! Type, select, copy, undo, or use an IME.\n\nFamily emoji: 👨‍👩‍👧‍👦   Flag: 🇻🇳\nTry combining marks too: e\u{301} and ắ.\n\nUse Home/End, Ctrl/Cmd+A, arrows, Shift+arrows, double/triple click, or drag.";
        let mut editor = TextEditor::new(initial);
        editor.selection = Selection::caret(TextPosition::new(editor.buffer.len()));
        let presentation = editor.presentation_text();
        let presentation_lines = line_ranges(&presentation);
        let mut text = TextSystem::new();
        let mut document_layout =
            TextDocumentLayout::new(editor.buffer.line_grapheme_lengths(), 26.0);
        let visible_lines = document_layout
            .layout_visible_lines(
                &mut text,
                0..24.min(document_layout.line_count()),
                2,
                style(18.0, rgb(230, 235, 244)),
                Some(900.0),
                |index| editor.buffer.line_text(index).unwrap_or_default(),
            )
            .expect("initial document layout");
        let debug_run = text.shape("", style(12.0, rgb(145, 159, 180)), None);
        let mut focus_tree = UiTree::new();
        let editor_node = focus_tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .expect("editor focus node");
        focus_tree
            .set_focus_policy(
                editor_node,
                FocusPolicy {
                    focusable: true,
                    tab_index: 0,
                    disabled: false,
                },
            )
            .expect("editor focus policy");
        focus_tree
            .request_focus(editor_node)
            .expect("initial editor focus");
        Self {
            window: None,
            gpu: None,
            editor,
            focus_tree,
            editor_node,
            clipboard,
            text,
            document_layout,
            visible_lines,
            debug_run,
            presentation,
            presentation_lines,
            was_composing: false,
            modifiers: ModifiersState::empty(),
            cursor: Point::ZERO,
            scroll: Point::ZERO,
            dragging: false,
            focused: true,
            surface_occluded: false,
            surface_issue_reported: false,
            last_click: Instant::now() - Duration::from_secs(1),
            click_position: Point::ZERO,
            click_count: 0,
            blink_started: Instant::now(),
            caret_blink_visible: true,
        }
    }

    fn init_gpu(&mut self) {
        let window = self.window.as_ref().expect("window");
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.window().clone())
            .expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("text-editing-demo-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("GPU device");
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let metrics = window.metrics();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: metrics.physical_size[0].max(1),
            height: metrics.physical_size[1].max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut renderer =
            UiRenderer::new(&device, &queue, format, RendererOptions::default()).expect("renderer");
        renderer.resize(&device, [config.width, config.height]);
        self.gpu = Some(Gpu {
            surface,
            device,
            queue,
            config,
            renderer,
        });
    }

    fn update_text_runs(&mut self) {
        let next = self.editor.presentation_text();
        let composing = self.editor.ime.is_composing();
        let invalidations = self.editor.buffer.take_invalidations();
        if next != self.presentation {
            if !self.was_composing && !composing && !invalidations.is_empty() {
                for invalidation in invalidations {
                    self.document_layout
                        .apply_edit(
                            &mut self.text,
                            LayoutDirtyLineRange {
                                start: invalidation.lines.start,
                                removed: invalidation.lines.removed,
                                inserted: invalidation.lines.inserted,
                            },
                            &invalidation.inserted_line_grapheme_lengths,
                            26.0,
                        )
                        .expect("editor emitted a valid dirty line range");
                }
            } else if let Some((start, removed, inserted_lengths)) =
                changed_line_splice(&self.presentation, &next)
            {
                self.document_layout
                    .apply_edit(
                        &mut self.text,
                        LayoutDirtyLineRange {
                            start,
                            removed,
                            inserted: inserted_lengths.len(),
                        },
                        &inserted_lengths,
                        26.0,
                    )
                    .expect("presentation diff produced a valid dirty line range");
            }
            self.presentation = next;
            self.presentation_lines = line_ranges(&self.presentation);
        }
        self.was_composing = composing;
        self.refresh_visible_lines();
        self.update_debug_run();
    }

    fn update_debug_run(&mut self) {
        let debug = format!(
            "caret={}  selection={}:{}  composition={:?}  undo={} redo={}  focused={}  scroll=({:.0},{:.0})",
            self.editor.selection.head.grapheme_index(),
            self.editor.selection.anchor.grapheme_index(),
            self.editor.selection.head.grapheme_index(),
            self.editor.ime.composing_range,
            self.editor.undo.undo_depth(),
            self.editor.undo.redo_depth(),
            self.focused,
            self.scroll.x,
            self.scroll.y
        );
        self.text
            .update_run(
                self.debug_run,
                &debug,
                style(12.0, rgb(145, 159, 180)),
                None,
            )
            .expect("debug run");
    }

    fn refresh_visible_lines(&mut self) {
        let viewport = self.viewport();
        let visible = self
            .document_layout
            .visible_line_range(self.scroll.y, viewport.height());
        let content = &self.presentation;
        let ranges = &self.presentation_lines;
        let width = (viewport.width() - 24.0).max(100.0);
        self.visible_lines = self
            .document_layout
            .layout_visible_lines(
                &mut self.text,
                visible,
                2,
                style(18.0, rgb(230, 235, 244)),
                Some(width),
                |index| {
                    ranges
                        .get(index)
                        .map(|range| content[range.clone()].to_owned())
                        .unwrap_or_default()
                },
            )
            .expect("visible document layout");
    }

    fn viewport(&self) -> Rect {
        let size = self
            .window
            .as_ref()
            .map_or(Size::new(1120.0, 720.0), |w| w.metrics().logical_size);
        Rect::from_min_max(
            Point::new(VIEWPORT_MARGIN, TEXT_ORIGIN.y - 12.0),
            Point::new(
                (size.width - VIEWPORT_MARGIN).max(VIEWPORT_MARGIN),
                (size.height - 116.0).max(TEXT_ORIGIN.y),
            ),
        )
    }
    fn text_point(&self, point: Point) -> Point {
        Point::new(
            point.x - TEXT_ORIGIN.x + self.scroll.x,
            point.y - TEXT_ORIGIN.y + self.scroll.y,
        )
    }
    fn hit_position(&self, point: Point) -> TextPosition {
        TextPosition::new(
            self.document_layout
                .hit_test(&self.text, self.text_point(point))
                .unwrap_or(self.editor.buffer.len())
                .min(self.editor.buffer.len()),
        )
    }
    fn ensure_caret_visible(&mut self) {
        let caret_index = self.editor.selection.head.grapheme_index();
        let caret_line = self.editor.buffer.line_index(self.editor.selection.head);
        let content = &self.presentation;
        let ranges = &self.presentation_lines;
        let width = (self.viewport().width() - 24.0).max(100.0);
        let _ = self.document_layout.layout_visible_lines(
            &mut self.text,
            caret_line..caret_line.saturating_add(1),
            2,
            style(18.0, rgb(230, 235, 244)),
            Some(width),
            |index| {
                ranges
                    .get(index)
                    .map(|range| content[range.clone()].to_owned())
                    .unwrap_or_default()
            },
        );
        let Some(rect) = self
            .document_layout
            .caret_rect(&self.text, caret_index, 1.5)
        else {
            return;
        };
        let viewport = self.viewport();
        let point = Point::new(
            rect.min.x + TEXT_ORIGIN.x - self.scroll.x,
            rect.min.y + TEXT_ORIGIN.y - self.scroll.y,
        );
        let width = rect.width();
        if point.x < viewport.min.x {
            self.scroll.x = (self.scroll.x - (viewport.min.x - point.x)).max(0.0);
        }
        if point.x + width > viewport.max.x {
            self.scroll.x += point.x + width - viewport.max.x;
        }
        if point.y < viewport.min.y {
            self.scroll.y = (self.scroll.y - (viewport.min.y - point.y)).max(0.0);
        }
        if point.y + rect.height() > viewport.max.y {
            self.scroll.y += point.y + rect.height() - viewport.max.y;
        }
    }

    fn apply_command(&mut self, command: EditCommand) {
        let extend = self.modifiers.shift_key();
        let changes_text = matches!(
            &command,
            EditCommand::InsertText(_)
                | EditCommand::DeleteBackward
                | EditCommand::DeleteForward
                | EditCommand::DeleteWordBackward
                | EditCommand::DeleteWordForward
                | EditCommand::Cut
                | EditCommand::Paste
                | EditCommand::Undo
                | EditCommand::Redo
        );
        self.editor.execute_with_document_layout(
            command,
            &mut self.clipboard,
            &self.text,
            &self.document_layout,
            extend,
        );
        self.blink_started = Instant::now();
        self.caret_blink_visible = true;
        if changes_text {
            self.update_text_runs();
        } else {
            self.update_debug_run();
        }
        self.ensure_caret_visible();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn set_editor_focus(&mut self, focused: bool) {
        if focused {
            let _ = self.focus_tree.request_focus(self.editor_node);
        } else {
            self.focus_tree.clear_focus();
        }
        self.focused = self.focus_tree.focus_manager().focused() == Some(self.editor_node);
        if let Some(window) = &self.window {
            window.window().set_ime_allowed(self.focused);
        }
        if !self.focused {
            self.editor.ime_cancel();
            self.update_text_runs();
        }
        self.blink_started = Instant::now();
        self.caret_blink_visible = true;
    }

    fn key_input(&mut self, input: PlatformTextInput) {
        if !input.pressed || !self.focused {
            return;
        }
        let Some(command) = input.command else {
            if let Some(text) = input.text {
                self.apply_command(EditCommand::InsertText(text));
            }
            return;
        };
        let command = match command {
            PlatformTextCommand::SelectAll => EditCommand::SelectAll,
            PlatformTextCommand::Copy => EditCommand::Copy,
            PlatformTextCommand::Cut => EditCommand::Cut,
            PlatformTextCommand::Paste => EditCommand::Paste,
            PlatformTextCommand::Undo => EditCommand::Undo,
            PlatformTextCommand::Redo => EditCommand::Redo,
            PlatformTextCommand::DeleteBackward { word: true } => EditCommand::DeleteWordBackward,
            PlatformTextCommand::DeleteBackward { word: false } => EditCommand::DeleteBackward,
            PlatformTextCommand::DeleteForward { word: true } => EditCommand::DeleteWordForward,
            PlatformTextCommand::DeleteForward { word: false } => EditCommand::DeleteForward,
            PlatformTextCommand::MoveLeft {
                line: true,
                extend: true,
                ..
            } => EditCommand::ExtendLineStart,
            PlatformTextCommand::MoveLeft {
                line: true,
                extend: false,
                ..
            } => EditCommand::MoveLineStart,
            PlatformTextCommand::MoveRight {
                line: true,
                extend: true,
                ..
            } => EditCommand::ExtendLineEnd,
            PlatformTextCommand::MoveRight {
                line: true,
                extend: false,
                ..
            } => EditCommand::MoveLineEnd,
            PlatformTextCommand::MoveLeft {
                word: true,
                extend: true,
                ..
            } => EditCommand::ExtendWordLeft,
            PlatformTextCommand::MoveLeft {
                word: true,
                extend: false,
                ..
            } => EditCommand::MoveWordLeft,
            PlatformTextCommand::MoveRight {
                word: true,
                extend: true,
                ..
            } => EditCommand::ExtendWordRight,
            PlatformTextCommand::MoveRight {
                word: true,
                extend: false,
                ..
            } => EditCommand::MoveWordRight,
            PlatformTextCommand::MoveLeft { extend: true, .. } => EditCommand::ExtendLeft,
            PlatformTextCommand::MoveLeft { extend: false, .. } => EditCommand::MoveLeft,
            PlatformTextCommand::MoveRight { extend: true, .. } => EditCommand::ExtendRight,
            PlatformTextCommand::MoveRight { extend: false, .. } => EditCommand::MoveRight,
            PlatformTextCommand::MoveUp { .. } => EditCommand::MoveUp,
            PlatformTextCommand::MoveDown { .. } => EditCommand::MoveDown,
            PlatformTextCommand::LineStart {
                document: true,
                extend: true,
            } => EditCommand::ExtendDocumentStart,
            PlatformTextCommand::LineStart {
                document: true,
                extend: false,
            } => EditCommand::MoveDocumentStart,
            PlatformTextCommand::LineStart {
                document: false,
                extend: true,
            } => EditCommand::ExtendLineStart,
            PlatformTextCommand::LineStart {
                document: false,
                extend: false,
            } => EditCommand::MoveLineStart,
            PlatformTextCommand::LineEnd {
                document: true,
                extend: true,
            } => EditCommand::ExtendDocumentEnd,
            PlatformTextCommand::LineEnd {
                document: true,
                extend: false,
            } => EditCommand::MoveDocumentEnd,
            PlatformTextCommand::LineEnd {
                document: false,
                extend: true,
            } => EditCommand::ExtendLineEnd,
            PlatformTextCommand::LineEnd {
                document: false,
                extend: false,
            } => EditCommand::MoveLineEnd,
            PlatformTextCommand::InsertLineBreak => EditCommand::InsertText("\n".into()),
            PlatformTextCommand::CancelComposition => {
                self.editor.ime_cancel();
                self.update_text_runs();
                return;
            }
        };
        self.apply_command(command);
    }

    fn redraw(&mut self) {
        self.refresh_visible_lines();
        self.update_debug_run();
        if self.surface_occluded {
            return;
        }
        let viewport = self.viewport();
        let caret_index = self.visual_caret_index();
        if self.focused
            && let (Some(window), Some(caret)) = (
                self.window.as_ref(),
                self.document_layout
                    .caret_rect(&self.text, caret_index, 1.5),
            )
        {
            window.window().set_ime_cursor_area(
                LogicalPosition::new(
                    caret.min.x + TEXT_ORIGIN.x - self.scroll.x,
                    caret.min.y + TEXT_ORIGIN.y - self.scroll.y,
                ),
                LogicalSize::new(caret.width().max(1.0), caret.height().max(1.0)),
            );
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.surface_occluded = true;
                return;
            }
            issue => {
                if !self.surface_issue_reported {
                    eprintln!("text editing surface unavailable: {issue:?}");
                    self.surface_issue_reported = true;
                }
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("text-editing-demo-frame"),
            });
        let metrics = window.metrics();
        let mut list = DisplayListBuilder::new();
        list.fill_rect(
            Rect::from_min_size(Point::ZERO, metrics.logical_size),
            rgb(17, 19, 24),
        );
        list.fill_rect(
            Rect::from_min_max(
                Point::new(20.0, 22.0),
                Point::new(metrics.logical_size.width - 20.0, 82.0),
            ),
            rgb(25, 28, 35),
        );
        list.text(self.debug_run, Point::new(32.0, 46.0));
        list.fill_rect(viewport, rgb(21, 24, 30));
        list.push_clip(viewport);
        let origin = Point::new(TEXT_ORIGIN.x - self.scroll.x, TEXT_ORIGIN.y - self.scroll.y);
        let composing = self.editor.ime.is_composing();
        let line_range = self.visible_lines.first().map_or(0..0, |first| {
            first.logical_index
                ..self
                    .visible_lines
                    .last()
                    .map_or(first.logical_index, |last| last.logical_index + 1)
        });
        if !composing {
            for rect in self.document_layout.selection_rects_in_range(
                &self.text,
                self.editor.selection.range().start.grapheme_index(),
                self.editor.selection.range().end.grapheme_index(),
                line_range.clone(),
            ) {
                list.fill_rect(
                    Rect::from_min_size(
                        Point::new(rect.min.x + origin.x, rect.min.y + origin.y),
                        rect.size(),
                    ),
                    rgb(44, 89, 148),
                );
            }
        }
        if composing
            && let (Some(range), Some(cursor)) =
                (self.editor.ime.composing_range, self.editor.ime.cursor)
        {
            let base = range.start.grapheme_index();
            let selection_start = base + cursor.start.grapheme_index();
            let selection_end = base + cursor.end.grapheme_index();
            for rect in self.document_layout.selection_rects_in_range(
                &self.text,
                selection_start,
                selection_end,
                line_range.clone(),
            ) {
                list.fill_rect(
                    Rect::from_min_size(
                        Point::new(rect.min.x + origin.x, rect.min.y + origin.y),
                        rect.size(),
                    ),
                    rgb(95, 102, 120),
                );
            }
        }
        for line in &self.visible_lines {
            list.text(line.run, Point::new(origin.x, origin.y + line.top));
        }
        if composing && let Some(range) = self.editor.ime.composing_range {
            let start = range.start.grapheme_index();
            let end = start + self.editor.ime.preedit_text.graphemes(true).count();
            for rect in self.document_layout.selection_rects_in_range(
                &self.text,
                start,
                end,
                line_range.clone(),
            ) {
                let y = rect.max.y + origin.y - 2.0;
                list.line(
                    Point::new(rect.min.x + origin.x, y),
                    Point::new(rect.max.x + origin.x, y),
                    Stroke::new(1.5, rgb(233, 188, 94)),
                );
            }
        }
        if self.focused
            && self.caret_blink_visible
            && let Some(caret) = self
                .document_layout
                .caret_rect(&self.text, caret_index, 1.5)
        {
            list.fill_rect(
                Rect::from_min_size(
                    Point::new(caret.min.x + origin.x, caret.min.y + origin.y),
                    caret.size(),
                ),
                rgb(245, 247, 252),
            );
        }
        list.pop_clip();
        let display = list.build();
        gpu.renderer.render(
            RenderFrame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut encoder,
                target: &view,
                viewport: Viewport::new(metrics.physical_size, metrics.scale_factor),
            },
            &display,
            Some(&mut self.text),
        );
        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
    }

    fn visual_caret_index(&self) -> usize {
        if let Some(range) = self.editor.ime.composing_range {
            range.start.grapheme_index()
                + self.editor.ime.cursor.map_or_else(
                    || self.editor.ime.preedit_text.graphemes(true).count(),
                    |cursor| cursor.end.grapheme_index(),
                )
        } else {
            self.editor.selection.head.grapheme_index()
        }
    }
}

impl ApplicationHandler for Demo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        self.window = Some(
            UiWindow::open(
                event_loop,
                &WindowConfig {
                    title: "Text Editing Runtime Demo".into(),
                    logical_size: Size::new(1120.0, 720.0),
                },
            )
            .expect("window"),
        );
        self.init_gpu();
        if let Some(window) = &self.window {
            window.window().set_ime_allowed(true);
            window.request_redraw();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_some_and(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(window), Some(gpu)) = (self.window.as_mut(), self.gpu.as_mut()) {
                    window.resize(size);
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.renderer
                        .resize(&gpu.device, [gpu.config.width, gpu.config.height]);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &mut self.window {
                    window.refresh_metrics();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(focused) => {
                self.set_editor_focus(focused);
            }
            WindowEvent::Ime(event) if self.focused => {
                match PlatformImeEvent::from(event) {
                    PlatformImeEvent::Enabled => self.editor.ime_start(),
                    PlatformImeEvent::Preedit { text, cursor } => {
                        let cursor = cursor.map(|(a, b)| {
                            TextRange::new(TextPosition::new(a), TextPosition::new(b))
                        });
                        self.editor.ime_preedit(&text, cursor);
                    }
                    PlatformImeEvent::Commit(text) => self.editor.ime_commit(&text),
                    PlatformImeEvent::Disabled => self.editor.ime_cancel(),
                }
                self.update_text_runs();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.key_input(PlatformTextInput::from_winit(&event, self.modifiers))
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Point::new(
                    position.x as f32 / self.window.as_ref().unwrap().metrics().scale_factor.get(),
                    position.y as f32 / self.window.as_ref().unwrap().metrics().scale_factor.get(),
                );
                if self.dragging {
                    let p = self.hit_position(self.cursor);
                    self.editor.pointer_drag_to(p);
                    self.update_debug_run();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    let now = Instant::now();
                    if now.duration_since(self.last_click) < Duration::from_millis(500)
                        && (self.cursor.x - self.click_position.x).abs() < 4.0
                        && (self.cursor.y - self.click_position.y).abs() < 4.0
                    {
                        self.click_count = self.click_count % 3 + 1;
                    } else {
                        self.click_count = 1;
                    }
                    self.last_click = now;
                    self.click_position = self.cursor;
                    self.set_editor_focus(self.viewport().contains(self.cursor));
                    self.dragging = self.focused;
                    if self.focused {
                        let pos = self.hit_position(self.cursor);
                        self.editor.pointer_click(
                            pos,
                            self.click_count,
                            self.modifiers.shift_key(),
                        );
                        self.ensure_caret_visible();
                    }
                    self.blink_started = Instant::now();
                    self.caret_blink_visible = true;
                    self.update_debug_run();
                } else {
                    self.dragging = false;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 36.0, y * 36.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                self.scroll.y = (self.scroll.y - dy).max(0.0);
                self.scroll.x = (self.scroll.x - dx).max(0.0);
            }
            WindowEvent::Occluded(occluded) => {
                self.surface_occluded = occluded;
                if !occluded {
                    self.surface_issue_reported = false;
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            if self.focused {
                let elapsed = self.blink_started.elapsed();
                let phase = elapsed.as_millis() / 500;
                let visible = phase.is_multiple_of(2);
                if visible != self.caret_blink_visible {
                    self.caret_blink_visible = visible;
                    window.request_redraw();
                }
                let next_toggle = Duration::from_millis(((phase + 1) * 500) as u64);
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                    self.blink_started + next_toggle,
                ));
            } else {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            }
        }
    }
}

fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '\n' {
            ranges.push(start..index);
            start = index + character.len_utf8();
        }
    }
    ranges.push(start..text.len());
    ranges
}

fn changed_line_splice(old: &str, new: &str) -> Option<(usize, usize, Vec<usize>)> {
    let old_ranges = line_ranges(old);
    let new_ranges = line_ranges(new);
    let old_line = |index: usize| &old[old_ranges[index].clone()];
    let new_line = |index: usize| &new[new_ranges[index].clone()];
    let mut prefix = 0;
    while prefix < old_ranges.len().min(new_ranges.len()) && old_line(prefix) == new_line(prefix) {
        prefix += 1;
    }
    let mut old_end = old_ranges.len();
    let mut new_end = new_ranges.len();
    while old_end > prefix && new_end > prefix && old_line(old_end - 1) == new_line(new_end - 1) {
        old_end -= 1;
        new_end -= 1;
    }
    if old_end == prefix && new_end == prefix {
        return None;
    }
    let inserted_lengths = new_ranges[prefix..new_end]
        .iter()
        .map(|range| new[range.clone()].graphemes(true).count())
        .collect();
    Some((prefix, old_end - prefix, inserted_lengths))
}

fn style(size: f32, color: Color) -> TextStyle {
    TextStyle {
        size_px: size,
        line_height_px: size + 8.0,
        weight: FontWeight::Regular,
        color,
        ..TextStyle::default()
    }
}
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgba8(Srgb8 { r, g, b, a: 255 })
}
fn run_headless_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = Demo::new(DemoClipboard::local());
    demo.apply_command(EditCommand::InsertText(" Việt".into()));
    if !demo.editor.buffer.text().ends_with(" Việt") {
        return Err("typing smoke failed".into());
    }
    demo.apply_command(EditCommand::Undo);
    demo.apply_command(EditCommand::Redo);
    demo.editor.selection = Selection {
        anchor: TextPosition::new(0),
        head: TextPosition::new(3),
    };
    demo.apply_command(EditCommand::Copy);
    demo.apply_command(EditCommand::Cut);
    demo.apply_command(EditCommand::Paste);
    let before_ime = demo.editor.buffer.text().to_owned();
    demo.editor.ime_start();
    demo.editor.ime_preedit(
        "候補",
        Some(TextRange::new(TextPosition::new(1), TextPosition::new(2))),
    );
    if demo.editor.buffer.text() != before_ime || demo.editor.presentation_text() == before_ime {
        return Err("IME preedit isolation failed".into());
    }
    demo.editor.ime_commit("候補");
    demo.update_text_runs();
    if demo
        .document_layout
        .caret_rect(&demo.text, demo.editor.selection.head.grapheme_index(), 1.0)
        .is_none()
    {
        return Err("caret geometry smoke failed".into());
    }
    demo.editor.selection = Selection {
        anchor: TextPosition::new(0),
        head: TextPosition::new(demo.editor.buffer.len()),
    };
    if demo
        .document_layout
        .selection_rects(&demo.text, 0, demo.editor.buffer.len())
        .is_empty()
    {
        return Err("selection geometry smoke failed".into());
    }
    demo.editor.selection = Selection::caret(TextPosition::new(demo.editor.buffer.len()));
    demo.editor
        .replace_selection(&"\nscroll test".repeat(80), false);
    demo.update_text_runs();
    demo.ensure_caret_visible();
    if demo.scroll.y <= 0.0 {
        return Err("caret auto-scroll smoke failed".into());
    }
    println!(
        "text editing headless smoke: editing, undo/redo, clipboard, IME, caret/selection geometry, and caret auto-scroll passed"
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--headless-smoke") {
        return run_headless_smoke();
    }
    EventLoop::new()?.run_app(&mut Demo::new(DemoClipboard::default()))?;
    Ok(())
}
