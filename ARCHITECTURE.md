# UI architecture

The dependency direction is deliberately one-way:

```text
App
↓
(component layer is future work)
↓
ui-runtime (retained tree + layout)
├─ ui-text (font shaping/metrics)
└─ ui-core DisplayList (ephemeral frame output)
   ↓
ui-renderer
↓
wgpu
↓
Metal / D3D12 / Vulkan
```

## Phase 0 decisions

- **Desktop-first, GPU-first:** the first platform path is a native `winit` window and a `wgpu` surface.
- **Hybrid retained/immediate:** components may retain state, but each frame is submitted as a small, explicit display list.
- **Text and accessibility are runtime concerns:** text and semantic output stay separate from paint commands; the renderer consumes only the display list.
- **HiDPI is explicit:** all public geometry is logical-point geometry; the renderer owns logical-to-physical conversion and clipping.
- **No browser clone:** layout and component semantics stay above the renderer; there is no CSS box-model compatibility layer.
- **Component isolation:** components publish drawing commands and never import `wgpu` or `winit`.
- **Renderer isolation:** the renderer consumes generic drawing commands and has no knowledge of business or component semantics.

## Module seams

`ui-core` owns the display-list seam. Paint commands are stored in submission order. Clip and transform push/pop commands scope later paint commands; the renderer interprets those stacks without reordering paint.

`ui-renderer` is the `wgpu` adapter. Its shape, image, and text pipelines are separate. It batches only adjacent commands with the same pipeline, clip, and (for images) texture, preserving z order. Each frame writes one contiguous vertex stream and draws individual batch ranges. `RendererStats` exposes CPU preparation time, draw calls, batches, vertices, glyph-cache deltas, and atlas occupancy.

`ui-text` owns system font discovery, cosmic-text shaping/layout/fallback, and swash rasterization. It returns logical glyph positions with fractional origins. Glyphs are cached in an R8 atlas; rasterization keys include scale and subpixel bins. `ui-renderer` samples the atlas through its dedicated text pipeline. Editing, caret, selection, IME, color emoji rendering, and GPU timing queries remain out of scope.

`ui-window` is a platform adapter. It owns `winit` lifecycle events and exposes window metrics without making the rest of the application depend on `winit` event details.

## Phase 2 and Phase 3 foundation

- `examples/basic` remains the primitive baseline.
- `examples/diagnostic` covers 10–18 px text, regular/medium/bold/monospace, Vietnamese samples, alpha levels, thin strokes, rounded corners, nested clips, image filtering, and a `UI_DPI_SIM=1|1.5|2` scale override.
- `examples/stress` renders an offscreen frame with at least 10,000 shapes and 5,000 text runs, then repeats unchanged frames and asserts no glyph rerasterization.

## Remaining runtime work

Visual QA should cover more operating-system font sets and fractional monitor scales. Rich-text spans, color emoji, full screen-reader UX certification, and GPU timestamp queries remain separate follow-on work. Rounded corners currently use tessellated arcs plus MSAA, not analytic coverage; line widths and HiDPI need visual calibration on multiple GPUs.

## Phase 3.5 — Rendering hardening

- `Transform` is a 2D affine 2x3 matrix with translation, scale, rotation and shear constructors. Nested transforms are local-to-parent and compose child-local before parent-world.
- Axis-aligned clips use the scissor fast path. A rotated/sheared rectangular clip is intersected as a convex polygon and paint triangles are clipped with interpolated UV/color attributes; the scissor is its conservative bounds. Rounded/path clips remain out of scope.
- `ui-text` owns stable `TextRunId` lifecycle (`update_run`, `remove_run`, color updates). Glyph keys include font/glyph/size/DPI/subpixel state. Atlas entries use frame pinning and LRU; freed rectangles are reused and coalesced. When the atlas is full, newly rasterized glyphs are deferred and the atlas grows at the next frame boundary, so a glyph already referenced in the current frame is never overwritten. The default maximum is 4096²; an atlas too small for that limit can still defer glyphs indefinitely.
- Atlas upload regions are copied into wgpu only when dirty; large dirty sets collapse into one full-atlas transfer. Metrics report frame CPU preparation, CPU queue-submit duration, draw calls, batches, vertices, upload bytes, glyph cache deltas, atlas occupancy, free rectangles and fragmentation. Queue-submit duration is not GPU execution time. Allocation count is `None` unless the host installs an instrumented allocator.

## Phase 4/5 — Retained tree and layout

`ui-runtime` owns monotonic `NodeId`s, parent/child relationships, per-node layout/paint/text/interaction state, a type-erased `StateStore`, and dirty flags for layout, paint, text, hit testing and accessibility. Paint/layout/text mutations invalidate only the affected node/subtree and required measurement ancestors; hover marks only paint. Measurement caches are keyed by constraints, and arrangement skips clean subtrees whose rect did not change.

Layout is a logical-point pipeline: measure intrinsic text/children, arrange Row/Column/Stack with padding/gap/alignment, min/max constraints, single-pass flex grow and basic absolute offsets, then paint a new ephemeral `DisplayList`. The display list is never used as retained storage; renderer remains unaware of NodeId or interaction/component semantics.

Current layout limits: dimensions are Auto or fixed points (no percentage), text wrapping is caller-provided through measured run metrics, flex surplus is not redistributed after a sibling hits max size, and absolute positioning has offsets but no anchors/insets.

## Phase 6/7 — Interaction and focus runtime

`ui-runtime` now owns a separate interaction layer for hit testing, event routing, pointer state, and focus. Each node has a `HitTestState` with local bounds, an affine transform, an optional local clip, visibility, pointer-event policy, and z order. Hit testing walks the retained tree in reverse paint order, composes nested transforms, applies nested clips, and returns the complete root-to-target path.

Pointer events use capture/target/bubble dispatch with `stop_propagation`, `stop_immediate_propagation`, and `prevent_default`. `PointerState` tracks hover paths, press targets, capture, and drag threshold state. Hover paths are diffed so enter/leave and paint invalidation are limited to changed nodes. Click and double-click require a matching press/release target and movement within the drag threshold. Wheel routing consumes remaining delta through the nearest scrollable ancestors.

`FocusManager` owns the focused and previous nodes, deterministic tabindex/document-order traversal, nested focus scopes with optional trapping and restoration, initial focus, and keyboard Tab/Shift+Tab behavior. Keyboard events route from the focused node through ancestors; global shortcut hooks are separate from component listeners. Components are still out of scope: these APIs are primitives for future Button, Menu, Dialog, TextInput, Tree, and DataGrid implementations.

## Phase 8 — Scroll and virtualization

`ui-runtime` now exposes `ScrollState` for logical offset, velocity, viewport, and content size. Scroll deltas return consumed and remaining components, so nested scroll views consume inner content first and bubble the remainder outward. `fling` and `tick` provide a small platform-neutral inertia primitive; touchpad/platform adapters remain outside the runtime. `ScrollbarGeometry` calculates a proportional thumb without introducing scrollbar styling.

Scroll offsets are applied as a content translation during hit testing and paint, while the scroll node's viewport is clipped. Scrolling invalidates only the scroll node's paint and hit-test state; it does not force a full remeasure or recursively dirty every child. Scroll content size is explicit so callers can use measured child content or a virtual provider.

`VirtualViewport` supports a fixed extent fast path and estimated extents with sparse measured overrides. A Fenwick correction tree keeps item offsets and total extent logarithmic, and measuring an item before the current anchor adjusts the scroll offset by the measured delta. `VirtualList` is the list-shaped alias; `VirtualGrid` virtualizes rows and columns and returns only viewport/overscan cells, so a million-row model does not become a million retained UI nodes.

## Phase 9 — Text editing runtime (V1)

`ui-runtime::TextBuffer` stores UTF-8 in a `String` while exposing `TextPosition` as a grapheme-cluster index. Consumers receive owned text/slices rather than references into the storage, so storage can change without changing the editing API. Ranges are ordered independently of `Selection` direction; `anchor` and `head` remain the source of selection direction. `Selection::click` accepts a logical position resolved by `ui-text`, then applies single, double, triple, or shift-click behavior. Editing and deletion clamp positions to grapheme boundaries; undo stores internal byte splice ranges to reverse edits that merge graphemes at insertion seams.

`TextEditor` applies platform-neutral `EditCommand`s. `Clipboard` is injected by the caller. `UndoManager` stores inserted/deleted operations and selection snapshots, coalesces adjacent typing, clears redo on a new edit, and keeps IME commits as ordinary undoable operations. `ImeState` holds transient preedit separately from committed buffer contents; cancellation drops preedit and preserves selection. Paste normalizes CRLF and CR to LF.

`ui-window::PlatformImeEvent` maps winit IME events into a platform-neutral event shape and converts preedit byte cursor offsets to grapheme offsets. `PlatformTextInput` keeps key/modifier mapping at the platform seam. Hosts gate input through `FocusManager`, provide a `Clipboard`, request IME enable/disable, and dispatch normalized input to `TextEditor`.

`ui-text` exposes logical-position hit testing, caret rectangles, selection spans, and visual line metrics from cosmic-text layout. `examples/text_editing_demo` connects mouse selection, keyboard commands, `FocusManager`, OS clipboard with a local fallback, undo/redo, IME events, caret blink/paint, scrolling, and debug state. Vertical movement uses shaped geometry and preferred X; Home/End resolve visual wrapped lines. The release benchmark covers 10k-line shape/layout, unchanged frames, single-line edits, newline insertion, multiline paste, and a 100k-line viewport.

The demo uses arboard for the OS clipboard and falls back to an in-memory clipboard with an explicit error message if the native backend is unavailable. Visual QA could not run because the current window surface reported `Occluded`; the headless smoke and logical geometry tests passed. Renderer remains unaware of editor state.

## Phase 9.5 — Incremental text layout

The text path now follows this sequence:

```text
TextBuffer
    ↓ logical edits
DirtyTextRange + DirtyLineRange
    ↓ line splice
TextDocumentLayout
    ↓ visible line range + overscan
per-line TextRunCache / shaped buffers
    ↓ visible runs
DisplayList
```

`TextBuffer` emits edit invalidations for insert, delete, multiline paste, IME commit, undo, and redo. A line splice carries the old start/count and grapheme lengths of the replacement lines, including unchanged prefix/suffix text within affected lines. The text layout cache applies that splice while retaining the shaped runs for unaffected lines. Callers must apply these invalidations before asking for layout; unchanged cached lines are trusted and their source text is not fetched again.

`TextDocumentLayout` retains per-line run identity, shaping style/constraint keys, metrics, grapheme start, position, and LRU use. `layout_visible_lines` reads/shapes only the requested visible range plus overscan. A configurable cache limit evicts shaped runs outside that range; evicted lines retain logical metadata and are shaped again when requested. Text content is owned once by the active shaped run rather than copied into a second line-cache string. Its memory estimate includes line metadata, text bytes, and glyph-position storage, but excludes cosmic-text's private buffers and allocator overhead.

Revisions are separated into document, per-line, shaping style, and layout constraints. Color changes update the cached run color without shaping. Wrap-width changes reshape only visible lines under the new constraint; rasterization remains deferred to `prepare` and uses the independent glyph-atlas cache. Viewport movement requests visible lines and shapes only cache misses. Hit testing, caret geometry, selection spans, and visual Home/End query cached line runs.

Invariants:

- editing state remains the source of truth for grapheme positions, selection, IME, and undo/redo;
- a logical edit invalidates only its affected line splice; later line positions may be recomputed without reshaping their text;
- unchanged cached lines do not fetch source text or recompute line metrics;
- viewport changes do not invalidate text, and hit tests do not shape text;
- document layout cache and glyph atlas cache have separate ownership and eviction.

The buffer still rebuilds its grapheme and line-start indexes on edit, and layout adjusts the position suffix when line count or height changes. Initial shaping of 10k separate logical-line runs is slower than shaping one multiline run; the viewport path keeps startup work proportional to visible lines. Benchmark measurements and memory estimates are recorded in `crates/ui-runtime/benches/text_editing.rs` and should be rerun on the target host before making performance comparisons.

## Phase 10 — Accessibility semantic runtime

The retained layout/interaction tree, paint output, and accessibility output are separate projections:

```text
UiTree (NodeId, layout, interaction, state)
├── Paint → DisplayList → Renderer
└── Semantics → SemanticTree → AccessibilityBackend → platform adapter
```

`SemanticTree` uses a persistent `NodeId ↔ AccessibilityId` mapping and caches semantic subtrees. Runtime nodes opt into `AccessibilitySemantics`; decorative nodes flatten their semantic descendants, and a hidden node excludes its whole subtree. Accessible names resolve in this order: explicit label, semantic text content, then `labelled_by` text. Debug identifiers and placeholders are not naming fallbacks. Child ordering follows the runtime tree.

Node state/layout changes set `DirtyFlags::ACCESSIBILITY`; focus changes dirty the old/new focus paths, and scroll changes dirty affected bounds. Hover remains paint-only. `SemanticTree::update` reuses clean subtrees and returns deterministic `SemanticUpdate { added, changed, removed, focus_changed }` records; unchanged frames produce an empty diff. Runtime bounds are clipped world-space logical points, composed from node transforms and scroll offsets. `ui-window` converts them to platform physical pixels using the current scale factor.

Platform actions enter as `AccessibilityActionRequest` and are routed back through `UiTree`: Focus uses `FocusManager`, Press emits the existing Click event path, and text value/selection actions call `TextEditor` editing and grapheme-selection APIs. Other role-specific actions dispatch `EventKind::AccessibilityAction` to application listeners. Editor owners that mutate a `TextEditor` held in `StateStore` directly call `UiTree::invalidate_accessibility` before the next semantic update.

`ui-runtime` defines `AccessibilityBackend`; `HeadlessAccessibilityBackend` is the deterministic test/mock implementation. `ui-window` owns the AccessKit 0.25 / `accesskit_winit` 0.34 adapter. Its callbacks queue actions for the UI thread, and the host calls `UiWindow::process_accessibility_event` before handling each winit event, submits semantic deltas, then drains and routes actions. AccessKit receives a synthetic native Window root; runtime semantic identities remain independent. Text input maps grapheme selection to an adapter-owned TextRun node. AccessKit-dependent types do not enter `ui-runtime`.

`examples/accessibility_demo` prints the semantic tree and exercises unchanged diffs, Press routing, text selection/replacement, hidden subtree omission, and a dialog reveal using only the headless backend. `crates/ui-runtime/benches/accessibility.rs` measures initial construction, unchanged frames, a single-node state update, focus movement, and diff generation over 10,000 runtime nodes with a smaller semantic subset. Native adapter compilation and headless tests verify the bridge contract; actual screen-reader behavior still requires manual OS/accessibility-tool QA.

## Phase 11 — Behavior primitives

`ui-runtime::primitives` contains style-free state and geometry models. `Pressable` registers button semantics and focus policy; pointer release, `BehaviorCommand::Activate`, and accessibility Press all enter the existing Click dispatch path. Disabled state blocks activation and is reflected in accessibility. Focus-visible is derived from the most recent input modality. `FocusScope` remains owned by `FocusManager`; layers open/close scopes and restore the prior focus. Disabled ancestors remove their descendants from focus traversal.

`LayerStack` orders active overlay hosts by `z_layer` and open sequence. Runtime paint and hit testing place layer hosts above ordinary siblings and order overlays consistently. Modal layers can trap focus and block outside pointer input. Outside pointer down, Escape, and focus outside enqueue one dismiss request for the topmost layer; the application handles that hook by closing the layer. Click-through is an explicit non-modal policy.

A portal node is physically reparented under a layer host for paint placement. `PortalRelationship` retains its logical owner. Pointer/keyboard events bubble through that owner, focus containment considers both physical host and logical owner, and `SemanticTree` projects the portal as a child of the owner. Portal bounds continue to use physical transforms/clips. Closing the layer also hides portal descendants from paint, hit testing, and semantics.

`place_popover` computes anchor-relative side/alignment/offset and then flips and shifts within a logical-point viewport; it has no style or renderer dependency. `TooltipController` accepts caller-provided time and trigger state so delays are deterministic in tests; `UiTree::tick_tooltip` can derive hover/focus triggers. Tooltip nodes are hidden from paint, hit testing, and accessibility until opened, and the trigger's `described_by` relationship is resolved through the existing semantic tree.

`MenuModel<K>` keeps stable item keys, skips disabled items, supports arrow/Home/End navigation, activation, Escape, and timed typeahead. `UiTree::install_menu` adds Menu/MenuItem semantics and focus policies; `link_submenu` is the nested-menu seam. `SelectionModel<K>` stores stable keys rather than row positions, supports single/multiple/toggle/range/clear/select-all-with-policy, and can publish selected state through `UiTree::apply_selection_state`. `Resizable` clamps pointer/keyboard changes to min/max, captures pointer during drag, exposes Slider Increment/Decrement semantics, and offers a reset hook used on unprevented double-click.

`BehaviorCommand` is platform-neutral and travels through the runtime event path. `ui-window::PlatformBehaviorInput` maps winit keys to Activate/Cancel/navigation/typeahead commands; primitives do not inspect platform keys. `examples/primitives_demo -- --headless-smoke` exercises the models and prints focus, layer, pointer-capture, menu and selection state. The release benchmark `crates/ui-runtime/benches/behavior_primitives.rs` covers 10k pressables, nested hit testing, popover cycles, 1k-item menu navigation, 100k stable selection keys, and resize streams.

Invariants:

- behavior != style; a primitive is not a component;
- platform key mapping != primitive behavior;
- accessibility action == the same semantic behavior path as pointer/keyboard activation;
- paint placement follows the physical tree, while portal event and semantic ownership follow the logical owner;
- overlay dismissal is ordered top-down and an outside pointer event dismisses at most one layer;
- `DisplayList` remains ephemeral and the renderer has no primitive knowledge.
