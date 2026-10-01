//! AccessKit/winit platform bridge. Runtime semantics remain AccessKit-free.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
};

use accesskit::{
    Action as NativeAction, ActionData, ActionRequest, ActivationHandler, DeactivationHandler,
    Node as NativeNode, NodeId as NativeNodeId, Role as NativeRole,
    TextPosition as NativeTextPosition, TextSelection as NativeTextSelection, Toggled, TreeId,
    TreeInfo, TreeUpdate,
};
use accesskit_winit::Adapter;
use ui_core::{Rect, ScaleFactor};
use ui_runtime::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityActionRequest, AccessibilityBackend,
    AccessibilityId, AccessibilityNode, AccessibilityRole, SemanticUpdate,
};
use unicode_segmentation::UnicodeSegmentation;
use winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

const SYNTHETIC_ROOT: u64 = 0;
const TEXT_RUN_TAG: u64 = 1 << 63;

/// Winit-backed AccessKit bridge owned by `UiWindow`.
pub struct PlatformAccessibility {
    backend: AccessKitBackend,
}

impl PlatformAccessibility {
    pub(crate) fn new(event_loop: &ActiveEventLoop, window: &Window, scale_factor: f32) -> Self {
        Self {
            backend: AccessKitBackend::new(event_loop, window, scale_factor),
        }
    }

    pub fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.backend.process_event(window, event);
    }

    pub fn update(&mut self, update: &SemanticUpdate) {
        self.backend.update(update);
    }

    pub fn poll_actions(&mut self) -> Vec<AccessibilityActionRequest> {
        self.backend.poll_actions()
    }

    pub fn set_scale_factor(&mut self, scale: f32) {
        self.backend.set_scale_factor(scale);
    }
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    nodes: BTreeMap<AccessibilityId, AccessibilityNode>,
    roots: Vec<AccessibilityId>,
    focused: Option<AccessibilityId>,
    scale_factor: f32,
}

pub(crate) struct AccessKitBackend {
    adapter: Adapter,
    snapshot: Arc<RwLock<Snapshot>>,
    initialized: Arc<AtomicBool>,
    action_receiver: Receiver<AccessibilityActionRequest>,
}

impl AccessKitBackend {
    pub(crate) fn new(event_loop: &ActiveEventLoop, window: &Window, scale_factor: f32) -> Self {
        let snapshot = Arc::new(RwLock::new(Snapshot {
            scale_factor,
            ..Snapshot::default()
        }));
        let initialized = Arc::new(AtomicBool::new(false));
        let (action_sender, action_receiver) = mpsc::channel();
        let adapter = Adapter::with_direct_handlers(
            event_loop,
            window,
            SnapshotActivation {
                snapshot: snapshot.clone(),
                initialized: initialized.clone(),
            },
            QueuedActionHandler {
                sender: action_sender.clone(),
            },
            SnapshotDeactivation {
                initialized: initialized.clone(),
            },
        );
        Self {
            adapter,
            snapshot,
            initialized,
            action_receiver,
        }
    }

    pub(crate) fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.adapter.process_event(window, event);
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f32) {
        self.snapshot
            .write()
            .expect("AccessKit snapshot lock poisoned")
            .scale_factor = scale_factor;
    }
}

impl AccessibilityBackend for AccessKitBackend {
    fn update(&mut self, update: &SemanticUpdate) {
        if update.is_empty() {
            return;
        }
        {
            let mut snapshot = self
                .snapshot
                .write()
                .expect("AccessKit snapshot lock poisoned");
            for node in update.added.iter().chain(&update.changed) {
                snapshot.nodes.insert(node.id, node.clone());
            }
            for id in &update.removed {
                snapshot.nodes.remove(id);
            }
            snapshot.roots = update.roots.clone();
            snapshot.focused = update.focused;
        }
        let snapshot = self.snapshot.clone();
        let initialized = self.initialized.clone();
        let delta = update.clone();
        self.adapter.update_if_active(move || {
            let snapshot = snapshot.read().expect("AccessKit snapshot lock poisoned");
            if initialized.load(Ordering::Acquire) {
                delta_tree_update(&delta, &snapshot)
            } else {
                full_tree_update(&snapshot)
            }
        });
    }

    fn set_focus(&mut self, focus: Option<AccessibilityId>) {
        self.snapshot
            .write()
            .expect("AccessKit snapshot lock poisoned")
            .focused = focus;
    }

    fn poll_actions(&mut self) -> Vec<AccessibilityActionRequest> {
        self.action_receiver.try_iter().collect()
    }
}

struct SnapshotActivation {
    snapshot: Arc<RwLock<Snapshot>>,
    initialized: Arc<AtomicBool>,
}

impl ActivationHandler for SnapshotActivation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let snapshot = self
            .snapshot
            .read()
            .expect("AccessKit snapshot lock poisoned");
        self.initialized.store(true, Ordering::Release);
        Some(full_tree_update(&snapshot))
    }
}

struct QueuedActionHandler {
    sender: Sender<AccessibilityActionRequest>,
}

impl accesskit::ActionHandler for QueuedActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        let Some(request) = translate_action_request(request) else {
            return;
        };
        let _ = self.sender.send(request);
    }
}

struct SnapshotDeactivation {
    initialized: Arc<AtomicBool>,
}

impl DeactivationHandler for SnapshotDeactivation {
    fn deactivate_accessibility(&mut self) {
        self.initialized.store(false, Ordering::Release);
    }
}

fn full_tree_update(snapshot: &Snapshot) -> TreeUpdate {
    let mut nodes = Vec::with_capacity(snapshot.nodes.len() + 1);
    nodes.push((
        NativeNodeId(SYNTHETIC_ROOT),
        synthetic_root(&snapshot.roots),
    ));
    for semantic in snapshot.nodes.values() {
        nodes.extend(native_nodes(semantic, snapshot.scale_factor));
    }
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(NativeNodeId(SYNTHETIC_ROOT))),
        tree_id: TreeId::ROOT,
        focus: snapshot
            .focused
            .map(native_id)
            .unwrap_or(NativeNodeId(SYNTHETIC_ROOT)),
    }
}

fn delta_tree_update(update: &SemanticUpdate, snapshot: &Snapshot) -> TreeUpdate {
    let mut nodes = vec![(
        NativeNodeId(SYNTHETIC_ROOT),
        synthetic_root(&snapshot.roots),
    )];
    for semantic in update.added.iter().chain(&update.changed) {
        nodes.extend(native_nodes(semantic, snapshot.scale_factor));
    }
    TreeUpdate {
        nodes,
        tree: None,
        tree_id: TreeId::ROOT,
        focus: snapshot
            .focused
            .map(native_id)
            .unwrap_or(NativeNodeId(SYNTHETIC_ROOT)),
    }
}

fn synthetic_root(roots: &[AccessibilityId]) -> NativeNode {
    let mut root = NativeNode::new(NativeRole::Window);
    root.set_children(roots.iter().copied().map(native_id).collect::<Vec<_>>());
    root
}

fn native_nodes(
    semantic: &AccessibilityNode,
    scale_factor: f32,
) -> Vec<(NativeNodeId, NativeNode)> {
    let mut node = NativeNode::new(native_role(semantic.role, semantic.multiline));
    let mut children = semantic
        .children
        .iter()
        .copied()
        .map(native_id)
        .collect::<Vec<_>>();
    if semantic.role == AccessibilityRole::TextInput
        && let Some(value) = semantic.value.as_deref()
    {
        let run_id = text_run_id(semantic.id);
        let mut run = NativeNode::new(NativeRole::TextRun);
        run.set_value(value.to_owned());
        let lengths = value
            .graphemes(true)
            .map(|grapheme| u8::try_from(grapheme.len()).ok())
            .collect::<Option<Vec<_>>>();
        if let Some(lengths) = lengths {
            run.set_character_lengths(lengths);
        }
        if let Some(bounds) = native_bounds(semantic.bounds, scale_factor) {
            run.set_bounds(bounds);
        }
        children.insert(0, run_id);
        if let Some(selection) = semantic.text_selection {
            node.set_text_selection(Box::new(NativeTextSelection {
                anchor: NativeTextPosition {
                    node: run_id,
                    character_index: selection.anchor,
                },
                focus: NativeTextPosition {
                    node: run_id,
                    character_index: selection.head,
                },
            }));
        }
        let mut output = vec![(run_id, run)];
        node.set_children(children);
        apply_semantic_properties(&mut node, semantic, scale_factor);
        output.push((native_id(semantic.id), node));
        return output;
    }
    node.set_children(children);
    apply_semantic_properties(&mut node, semantic, scale_factor);
    vec![(native_id(semantic.id), node)]
}

fn apply_semantic_properties(
    node: &mut NativeNode,
    semantic: &AccessibilityNode,
    scale_factor: f32,
) {
    if semantic.role == AccessibilityRole::Text {
        if let Some(value) = semantic.value.as_ref().or(semantic.label.as_ref()) {
            node.set_value(value.clone());
        }
    } else {
        if let Some(label) = &semantic.label {
            node.set_label(label.clone());
        }
        if let Some(value) = &semantic.value {
            node.set_value(value.clone());
        }
    }
    if let Some(description) = &semantic.description {
        node.set_description(description.clone());
    }
    if let Some(bounds) = native_bounds(semantic.bounds, scale_factor) {
        node.set_bounds(bounds);
    }
    for action in &semantic.actions {
        node.add_action(native_action(*action));
    }
    if semantic.state.disabled == Some(true) {
        node.set_disabled();
    }
    if let Some(selected) = semantic.state.selected {
        node.set_selected(selected);
    }
    if let Some(toggled) = semantic.state.checked.or(semantic.state.pressed) {
        node.set_toggled(if toggled {
            Toggled::True
        } else {
            Toggled::False
        });
    }
    if let Some(expanded) = semantic.state.expanded {
        node.set_expanded(expanded);
    }
    if semantic.state.read_only == Some(true) {
        node.set_read_only();
    }
    if semantic.state.required == Some(true) {
        node.set_required();
    }
}

fn native_role(role: AccessibilityRole, multiline: Option<bool>) -> NativeRole {
    match role {
        AccessibilityRole::Window => NativeRole::Window,
        AccessibilityRole::Group => NativeRole::Group,
        AccessibilityRole::Button => NativeRole::Button,
        AccessibilityRole::Text => NativeRole::Label,
        AccessibilityRole::TextInput if multiline == Some(true) => NativeRole::MultilineTextInput,
        AccessibilityRole::TextInput => NativeRole::TextInput,
        AccessibilityRole::Checkbox => NativeRole::CheckBox,
        AccessibilityRole::Radio => NativeRole::RadioButton,
        AccessibilityRole::Switch => NativeRole::Switch,
        AccessibilityRole::Tab => NativeRole::Tab,
        AccessibilityRole::TabList => NativeRole::TabList,
        AccessibilityRole::Menu => NativeRole::Menu,
        AccessibilityRole::MenuItem => NativeRole::MenuItem,
        AccessibilityRole::Dialog => NativeRole::Dialog,
        AccessibilityRole::Tooltip => NativeRole::Tooltip,
        AccessibilityRole::Tree => NativeRole::Tree,
        AccessibilityRole::TreeItem => NativeRole::TreeItem,
        AccessibilityRole::Table => NativeRole::Table,
        AccessibilityRole::Row => NativeRole::Row,
        AccessibilityRole::Cell => NativeRole::Cell,
        AccessibilityRole::ColumnHeader => NativeRole::ColumnHeader,
        AccessibilityRole::RowHeader => NativeRole::RowHeader,
        AccessibilityRole::ScrollArea => NativeRole::ScrollView,
        AccessibilityRole::Separator => NativeRole::Splitter,
        AccessibilityRole::ProgressBar => NativeRole::ProgressIndicator,
        AccessibilityRole::Slider => NativeRole::Slider,
        AccessibilityRole::Link => NativeRole::Link,
        AccessibilityRole::Image => NativeRole::Image,
    }
}

fn native_action(action: AccessibilityActionKind) -> NativeAction {
    match action {
        AccessibilityActionKind::Focus => NativeAction::Focus,
        AccessibilityActionKind::Press => NativeAction::Click,
        AccessibilityActionKind::SetValue => NativeAction::SetValue,
        AccessibilityActionKind::ReplaceSelectedText => NativeAction::ReplaceSelectedText,
        AccessibilityActionKind::Increment => NativeAction::Increment,
        AccessibilityActionKind::Decrement => NativeAction::Decrement,
        AccessibilityActionKind::Expand => NativeAction::Expand,
        AccessibilityActionKind::Collapse => NativeAction::Collapse,
        AccessibilityActionKind::ScrollIntoView => NativeAction::ScrollIntoView,
        AccessibilityActionKind::SetSelection => NativeAction::SetTextSelection,
    }
}

fn native_bounds(bounds: Option<Rect>, scale_factor: f32) -> Option<accesskit::Rect> {
    let bounds = bounds?;
    let scale = f64::from(ScaleFactor::new(scale_factor).get());
    Some(accesskit::Rect::new(
        f64::from(bounds.min.x) * scale,
        f64::from(bounds.min.y) * scale,
        f64::from(bounds.max.x) * scale,
        f64::from(bounds.max.y) * scale,
    ))
}

fn native_id(id: AccessibilityId) -> NativeNodeId {
    NativeNodeId(id.get())
}

fn text_run_id(id: AccessibilityId) -> NativeNodeId {
    NativeNodeId(TEXT_RUN_TAG | id.get())
}

fn translate_action_request(request: ActionRequest) -> Option<AccessibilityActionRequest> {
    let raw_target = request.target_node.0;
    let target = if raw_target & TEXT_RUN_TAG != 0 {
        raw_target & !TEXT_RUN_TAG
    } else {
        raw_target
    };
    if target == SYNTHETIC_ROOT {
        return None;
    }
    let action = match (request.action, request.data) {
        (NativeAction::Focus, _) => AccessibilityAction::Focus,
        (NativeAction::Click, _) => AccessibilityAction::Press,
        (NativeAction::SetValue, Some(ActionData::Value(value))) => {
            AccessibilityAction::SetValue(value.into())
        }
        (NativeAction::ReplaceSelectedText, Some(ActionData::Value(value))) => {
            AccessibilityAction::ReplaceSelectedText(value.into())
        }
        (NativeAction::Increment, _) => AccessibilityAction::Increment,
        (NativeAction::Decrement, _) => AccessibilityAction::Decrement,
        (NativeAction::Expand, _) => AccessibilityAction::Expand,
        (NativeAction::Collapse, _) => AccessibilityAction::Collapse,
        (NativeAction::ScrollIntoView, _) => AccessibilityAction::ScrollIntoView,
        (NativeAction::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
            AccessibilityAction::SetSelection {
                anchor: selection.anchor.character_index,
                head: selection.focus.character_index,
            }
        }
        _ => return None,
    };
    Some(AccessibilityActionRequest {
        target: AccessibilityId::from_raw(target),
        action,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_core::{Point, Rect as UiRect};
    use ui_runtime::AccessibilityState;

    fn semantic(role: AccessibilityRole) -> AccessibilityNode {
        AccessibilityNode {
            id: AccessibilityId::from_raw(12),
            role,
            label: Some("Name".to_owned()),
            description: None,
            value: Some("hé".to_owned()),
            state: AccessibilityState::default(),
            bounds: Some(UiRect::from_min_max(
                Point::new(2.0, 3.0),
                Point::new(12.0, 13.0),
            )),
            actions: vec![AccessibilityActionKind::Press],
            children: Vec::new(),
            text_selection: None,
            multiline: None,
        }
    }

    #[test]
    fn accesskit_roles_bounds_and_text_selection_use_semantic_ids_and_hidpi_points() {
        let mut text = semantic(AccessibilityRole::TextInput);
        text.text_selection = Some(ui_runtime::AccessibilityTextSelection { anchor: 1, head: 2 });
        let output = native_nodes(&text, 2.0);
        assert_eq!(output.len(), 2);
        assert_eq!(output[1].0.0, 12);
        assert_eq!(output[0].1.value(), Some("hé"));
        assert_eq!(output[0].1.character_lengths(), &[1, 2]);
        let bounds = native_bounds(text.bounds, 2.0).unwrap();
        assert_eq!(bounds, accesskit::Rect::new(4.0, 6.0, 24.0, 26.0));
        assert_eq!(
            native_role(AccessibilityRole::TextInput, Some(true)),
            NativeRole::MultilineTextInput
        );
    }

    #[test]
    fn platform_actions_return_runtime_ids_and_grapheme_positions() {
        let request = ActionRequest {
            action: NativeAction::SetTextSelection,
            target_tree: TreeId::ROOT,
            target_node: native_id(AccessibilityId::from_raw(12)),
            data: Some(ActionData::SetTextSelection(NativeTextSelection {
                anchor: NativeTextPosition {
                    node: text_run_id(AccessibilityId::from_raw(12)),
                    character_index: 1,
                },
                focus: NativeTextPosition {
                    node: text_run_id(AccessibilityId::from_raw(12)),
                    character_index: 3,
                },
            })),
        };
        let translated = translate_action_request(request).unwrap();
        assert_eq!(translated.target.get(), 12);
        assert_eq!(
            translated.action,
            AccessibilityAction::SetSelection { anchor: 1, head: 3 }
        );
    }
}
