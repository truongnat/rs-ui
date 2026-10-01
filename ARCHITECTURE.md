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
- **Text and accessibility are core concerns:** they will enter the runtime/display-list interfaces, not be bolted onto the renderer later.
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

Visual QA should cover more operating-system font sets and fractional monitor scales. Rich-text spans, color emoji, accessibility nodes, text editing, and GPU timestamp queries remain separate follow-on work. Rounded corners currently use tessellated arcs plus MSAA, not analytic coverage; line widths and HiDPI need visual calibration on multiple GPUs.

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
