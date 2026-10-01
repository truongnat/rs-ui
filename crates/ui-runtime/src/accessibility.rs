//! Retained accessibility semantics, independent of the paint/display-list path.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use ui_core::{Rect, Transform};

use crate::{
    DirtyFlags, EditCommand, NodeId, RuntimeError, Selection, TextEditor, TextPosition, UiTree,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccessibilityId(u64);

impl AccessibilityId {
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Construct an ID received from a platform adapter.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilityRole {
    Window,
    Group,
    Button,
    Text,
    TextInput,
    Checkbox,
    Radio,
    Switch,
    Tab,
    TabList,
    Menu,
    MenuItem,
    Dialog,
    Tooltip,
    Tree,
    TreeItem,
    Table,
    Row,
    Cell,
    ColumnHeader,
    RowHeader,
    ScrollArea,
    Separator,
    ProgressBar,
    Slider,
    Link,
    Image,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessibilityState {
    pub disabled: Option<bool>,
    pub focused: Option<bool>,
    pub selected: Option<bool>,
    pub checked: Option<bool>,
    pub expanded: Option<bool>,
    pub pressed: Option<bool>,
    pub read_only: Option<bool>,
    pub required: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilityActionKind {
    Focus,
    Press,
    SetValue,
    ReplaceSelectedText,
    Increment,
    Decrement,
    Expand,
    Collapse,
    ScrollIntoView,
    SetSelection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityAction {
    Focus,
    Press,
    SetValue(String),
    ReplaceSelectedText(String),
    Increment,
    Decrement,
    Expand,
    Collapse,
    ScrollIntoView,
    SetSelection { anchor: usize, head: usize },
}

impl AccessibilityAction {
    pub const fn kind(&self) -> AccessibilityActionKind {
        match self {
            Self::Focus => AccessibilityActionKind::Focus,
            Self::Press => AccessibilityActionKind::Press,
            Self::SetValue(_) => AccessibilityActionKind::SetValue,
            Self::ReplaceSelectedText(_) => AccessibilityActionKind::ReplaceSelectedText,
            Self::Increment => AccessibilityActionKind::Increment,
            Self::Decrement => AccessibilityActionKind::Decrement,
            Self::Expand => AccessibilityActionKind::Expand,
            Self::Collapse => AccessibilityActionKind::Collapse,
            Self::ScrollIntoView => AccessibilityActionKind::ScrollIntoView,
            Self::SetSelection { .. } => AccessibilityActionKind::SetSelection,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilitySemantics {
    pub role: AccessibilityRole,
    pub label: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub state: AccessibilityState,
    pub actions: Vec<AccessibilityActionKind>,
    pub labelled_by: Vec<NodeId>,
    pub hidden: bool,
    pub decorative: bool,
    pub multiline: Option<bool>,
}

impl AccessibilitySemantics {
    pub fn new(role: AccessibilityRole) -> Self {
        Self {
            role,
            label: None,
            description: None,
            value: None,
            state: AccessibilityState::default(),
            actions: default_actions(role),
            labelled_by: Vec::new(),
            hidden: false,
            decorative: false,
            multiline: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityTextSelection {
    /// Logical grapheme positions in the current text model.
    pub anchor: usize,
    pub head: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: AccessibilityId,
    pub role: AccessibilityRole,
    pub label: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub state: AccessibilityState,
    /// Clipped world-space logical points, before any HiDPI conversion.
    pub bounds: Option<Rect>,
    pub actions: Vec<AccessibilityActionKind>,
    pub children: Vec<AccessibilityId>,
    pub text_selection: Option<AccessibilityTextSelection>,
    pub multiline: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticUpdate {
    pub root: Option<AccessibilityId>,
    pub roots: Vec<AccessibilityId>,
    pub focused: Option<AccessibilityId>,
    pub added: Vec<AccessibilityNode>,
    pub changed: Vec<AccessibilityNode>,
    pub removed: Vec<AccessibilityId>,
    /// `Some(None)` means focus was cleared; `None` means unchanged.
    pub focus_changed: Option<Option<AccessibilityId>>,
}

impl SemanticUpdate {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.changed.is_empty()
            && self.removed.is_empty()
            && self.focus_changed.is_none()
    }

    pub fn change_count(&self) -> usize {
        self.added.len() + self.changed.len() + self.removed.len()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemanticTreeStats {
    pub visited_runtime_nodes: u64,
    pub rebuilt_semantic_nodes: u64,
    pub reused_subtrees: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityActionRequest {
    pub target: AccessibilityId,
    pub action: AccessibilityAction,
}

pub trait AccessibilityBackend {
    fn update(&mut self, update: &SemanticUpdate);
    fn set_focus(&mut self, focus: Option<AccessibilityId>);
    fn poll_actions(&mut self) -> Vec<AccessibilityActionRequest>;
}

#[derive(Default)]
pub struct HeadlessAccessibilityBackend {
    nodes: BTreeMap<AccessibilityId, AccessibilityNode>,
    focused: Option<AccessibilityId>,
    actions: VecDeque<AccessibilityActionRequest>,
    update_count: u64,
}

impl HeadlessAccessibilityBackend {
    pub fn nodes(&self) -> &BTreeMap<AccessibilityId, AccessibilityNode> {
        &self.nodes
    }

    pub const fn focused(&self) -> Option<AccessibilityId> {
        self.focused
    }

    pub const fn update_count(&self) -> u64 {
        self.update_count
    }

    pub fn push_action(&mut self, request: AccessibilityActionRequest) {
        self.actions.push_back(request);
    }
}

impl AccessibilityBackend for HeadlessAccessibilityBackend {
    fn update(&mut self, update: &SemanticUpdate) {
        for node in update.added.iter().chain(&update.changed) {
            self.nodes.insert(node.id, node.clone());
        }
        for id in &update.removed {
            self.nodes.remove(id);
        }
        if let Some(focus) = update.focus_changed {
            self.focused = focus;
        }
        self.update_count += 1;
    }

    fn set_focus(&mut self, focus: Option<AccessibilityId>) {
        self.focused = focus;
    }

    fn poll_actions(&mut self) -> Vec<AccessibilityActionRequest> {
        self.actions.drain(..).collect()
    }
}

#[derive(Clone, Debug)]
struct CachedSubtree {
    exposed_roots: Vec<AccessibilityId>,
    all_ids: Vec<AccessibilityId>,
    world: Transform,
    clip: Option<Rect>,
}

#[derive(Default)]
pub struct SemanticTree {
    nodes: BTreeMap<AccessibilityId, AccessibilityNode>,
    ids_by_node: HashMap<NodeId, AccessibilityId>,
    node_by_id: HashMap<AccessibilityId, NodeId>,
    subtrees: HashMap<NodeId, CachedSubtree>,
    next_id: u64,
    focused: Option<AccessibilityId>,
    stats: SemanticTreeStats,
}

impl SemanticTree {
    pub fn nodes(&self) -> &BTreeMap<AccessibilityId, AccessibilityNode> {
        &self.nodes
    }

    pub fn focused(&self) -> Option<AccessibilityId> {
        self.focused
    }

    pub fn accessibility_id(&self, node: NodeId) -> Option<AccessibilityId> {
        self.ids_by_node.get(&node).copied()
    }

    pub fn runtime_node(&self, id: AccessibilityId) -> Option<NodeId> {
        self.node_by_id.get(&id).copied()
    }

    pub const fn stats(&self) -> SemanticTreeStats {
        self.stats
    }

    pub fn update(
        &mut self,
        tree: &mut UiTree,
        root: NodeId,
    ) -> Result<SemanticUpdate, RuntimeError> {
        if !tree.nodes.contains_key(&root) {
            return Err(RuntimeError::UnknownNode(root));
        }
        self.stats = SemanticTreeStats::default();
        let before = std::mem::take(&mut self.nodes);
        let before_focus = self.focused;
        let mut visited = Vec::new();
        let root_node = &tree.nodes[&root];
        let root_bounds = root_node
            .hit_test
            .transform
            .transform_rect(root_node.hit_test.bounds);
        let root_clip =
            (root_bounds.width() > 0.0 && root_bounds.height() > 0.0).then_some(root_bounds);
        let (roots, active) = build_subtree(
            tree,
            self,
            root,
            Transform::IDENTITY,
            root_clip,
            &before,
            &mut visited,
        );
        let visited_count = visited.len() as u64;
        for id in visited {
            if let Some(node) = tree.nodes.get_mut(&id) {
                node.dirty.remove(DirtyFlags::ACCESSIBILITY);
            }
        }
        self.subtrees
            .retain(|node, _| tree.nodes.contains_key(node));
        self.ids_by_node
            .retain(|node, _| tree.nodes.contains_key(node));
        self.node_by_id
            .retain(|_, node| tree.nodes.contains_key(node));
        let active_nodes: HashSet<_> = active.into_iter().collect();
        let mut after = before.clone();
        after.retain(|id, _| active_nodes.contains(id));
        for id in self.nodes.keys().copied().collect::<Vec<_>>() {
            if active_nodes.contains(&id) {
                after.insert(id, self.nodes[&id].clone());
            }
        }
        self.nodes = after;
        self.focused = tree
            .focus_manager()
            .focused()
            .and_then(|node| self.ids_by_node.get(&node).copied())
            .filter(|id| self.nodes.contains_key(id));
        let mut update = SemanticUpdate::default();
        for (id, node) in &self.nodes {
            match before.get(id) {
                None => update.added.push(node.clone()),
                Some(previous) if previous != node => update.changed.push(node.clone()),
                Some(_) => {}
            }
        }
        update.removed = before
            .keys()
            .filter(|id| !self.nodes.contains_key(id))
            .copied()
            .collect();
        if self.focused != before_focus {
            update.focus_changed = Some(self.focused);
        }
        update.root = roots.first().copied();
        update.roots = roots;
        update.focused = self.focused;
        update.added.sort_by_key(|node| node.id);
        update.changed.sort_by_key(|node| node.id);
        update.removed.sort_unstable();
        self.stats.visited_runtime_nodes = visited_count;
        self.stats.rebuilt_semantic_nodes = update.added.len() as u64 + update.changed.len() as u64;
        Ok(update)
    }

    pub fn update_backend(
        &mut self,
        tree: &mut UiTree,
        root: NodeId,
        backend: &mut impl AccessibilityBackend,
    ) -> Result<SemanticUpdate, RuntimeError> {
        let update = self.update(tree, root)?;
        if !update.is_empty() {
            backend.update(&update);
        }
        if let Some(focus) = update.focus_changed {
            backend.set_focus(focus);
        }
        Ok(update)
    }

    pub fn route_action(
        &self,
        tree: &mut UiTree,
        request: AccessibilityActionRequest,
    ) -> Result<bool, RuntimeError> {
        let Some(node_id) = self.node_by_id.get(&request.target).copied() else {
            return Ok(false);
        };
        let Some(semantic) = self.nodes.get(&request.target) else {
            return Ok(false);
        };
        if !semantic.actions.contains(&request.action.kind()) {
            return Ok(false);
        }
        match request.action {
            AccessibilityAction::Focus => tree.request_focus(node_id),
            AccessibilityAction::Press => {
                tree.accessibility_press(node_id)?;
                Ok(true)
            }
            AccessibilityAction::SetValue(value) => {
                let Some(editor) = tree.state.get_mut::<TextEditor>(node_id) else {
                    return Ok(false);
                };
                if semantic.state.read_only == Some(true) {
                    return Ok(false);
                }
                editor.execute(EditCommand::SelectAll, &mut EmptyClipboard);
                editor.execute(EditCommand::InsertText(value), &mut EmptyClipboard);
                tree.invalidate_accessibility_node(node_id);
                Ok(true)
            }
            AccessibilityAction::ReplaceSelectedText(value) => {
                let Some(editor) = tree.state.get_mut::<TextEditor>(node_id) else {
                    return Ok(false);
                };
                if semantic.state.read_only == Some(true) {
                    return Ok(false);
                }
                editor.execute(EditCommand::InsertText(value), &mut EmptyClipboard);
                tree.invalidate_accessibility_node(node_id);
                Ok(true)
            }
            AccessibilityAction::SetSelection { anchor, head } => {
                let Some(editor) = tree.state.get_mut::<TextEditor>(node_id) else {
                    return Ok(false);
                };
                editor.set_selection(Selection {
                    anchor: TextPosition::new(anchor),
                    head: TextPosition::new(head),
                });
                tree.invalidate_accessibility_node(node_id);
                Ok(true)
            }
            action => {
                tree.accessibility_action_event(node_id, action)?;
                Ok(true)
            }
        }
    }
}

#[derive(Default)]
struct BuildResult {
    exposed_roots: Vec<AccessibilityId>,
    all_ids: Vec<AccessibilityId>,
}

fn build_subtree(
    tree: &UiTree,
    semantics: &mut SemanticTree,
    node_id: NodeId,
    parent_world: Transform,
    parent_clip: Option<Rect>,
    before: &BTreeMap<AccessibilityId, AccessibilityNode>,
    visited: &mut Vec<NodeId>,
) -> (Vec<AccessibilityId>, Vec<AccessibilityId>) {
    let Some(node) = tree.nodes.get(&node_id) else {
        return (Vec::new(), Vec::new());
    };
    let world = node.hit_test.transform.then(parent_world);
    let clip = node
        .hit_test
        .clip
        .map(|local| world.transform_rect(local))
        .and_then(|local| intersect_optional(parent_clip, Some(local)))
        .or(parent_clip);
    let clean = !node.dirty.contains(DirtyFlags::ACCESSIBILITY);
    if clean
        && let Some(cached) = semantics.subtrees.get(&node_id)
        && cached.world == world
        && cached.clip == clip
    {
        semantics.stats.reused_subtrees += 1;
        return (cached.exposed_roots.clone(), cached.all_ids.clone());
    }
    visited.push(node_id);
    if !node.hit_test.visible || node.accessibility.as_ref().is_some_and(|spec| spec.hidden) {
        semantics.subtrees.insert(
            node_id,
            CachedSubtree {
                exposed_roots: Vec::new(),
                all_ids: Vec::new(),
                world,
                clip,
            },
        );
        return (Vec::new(), Vec::new());
    }
    let child_world = if let Some(offset) = tree.scroll_transform(node_id) {
        Transform::translation(-offset.x, -offset.y).then(world)
    } else {
        world
    };
    let child_clip = if tree.scroll_transform(node_id).is_some() {
        let viewport = world.transform_rect(node.hit_test.bounds);
        intersect_optional(clip, Some(viewport))
    } else {
        clip
    };
    let mut child_roots = Vec::new();
    let mut child_all = Vec::new();
    for child in &node.children {
        let (roots, all) = build_subtree(
            tree,
            semantics,
            *child,
            child_world,
            child_clip,
            before,
            visited,
        );
        child_roots.extend(roots);
        child_all.extend(all);
    }
    let mut result = BuildResult {
        exposed_roots: child_roots,
        all_ids: child_all,
    };
    if let Some(spec) = node.accessibility.as_ref().filter(|spec| !spec.decorative) {
        let id = *semantics.ids_by_node.entry(node_id).or_insert_with(|| {
            semantics.next_id += 1;
            let id = AccessibilityId(semantics.next_id);
            semantics.node_by_id.insert(id, node_id);
            id
        });
        let bounds = world.transform_rect(node.hit_test.bounds);
        let bounds = clip.map_or(Some(bounds), |clip| bounds.intersect(clip));
        let text_editor = (spec.role == AccessibilityRole::TextInput)
            .then(|| tree.state.get::<TextEditor>(node_id))
            .flatten();
        let text_value = text_editor.map(|editor| editor.buffer.text());
        let selection = text_editor.map(|editor| AccessibilityTextSelection {
            anchor: editor.selection.anchor.grapheme_index(),
            head: editor.selection.head.grapheme_index(),
        });
        let label = spec
            .label
            .clone()
            .or_else(|| node.accessibility_text.clone())
            .or_else(|| resolve_relationship_label(tree, &spec.labelled_by));
        let mut state = spec.state;
        if node.focus_policy.focusable {
            state.disabled = Some(node.focus_policy.disabled);
        }
        if tree.focus_manager().focused() == Some(node_id) {
            state.focused = Some(true);
        } else if state.focused.is_some() {
            state.focused = Some(false);
        }
        state = role_filtered_state(spec.role, state);
        let mut actions = spec.actions.clone();
        if state.read_only == Some(true) {
            actions.retain(|action| *action != AccessibilityActionKind::SetValue);
        }
        if node.focus_policy.focusable && !actions.contains(&AccessibilityActionKind::Focus) {
            actions.push(AccessibilityActionKind::Focus);
        }
        let semantic_node = AccessibilityNode {
            id,
            role: spec.role,
            label,
            description: spec.description.clone(),
            value: text_value.or_else(|| spec.value.clone()),
            state,
            bounds,
            actions,
            children: result.exposed_roots.clone(),
            text_selection: selection,
            multiline: (spec.role == AccessibilityRole::TextInput)
                .then_some(spec.multiline)
                .flatten(),
        };
        if before.get(&id) != Some(&semantic_node) {
            semantics.stats.rebuilt_semantic_nodes += 1;
        }
        semantics.nodes.insert(id, semantic_node);
        result.exposed_roots = vec![id];
        result.all_ids.push(id);
    }
    result.all_ids.sort_unstable();
    semantics.subtrees.insert(
        node_id,
        CachedSubtree {
            exposed_roots: result.exposed_roots.clone(),
            all_ids: result.all_ids.clone(),
            world,
            clip,
        },
    );
    (result.exposed_roots, result.all_ids)
}

impl UiTree {
    pub fn set_accessibility_semantics(
        &mut self,
        node: NodeId,
        semantics: AccessibilitySemantics,
    ) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if target.accessibility.as_ref() != Some(&semantics) {
            target.accessibility = Some(semantics);
            self.invalidate_accessibility_chain(node);
        }
        Ok(())
    }

    pub fn clear_accessibility_semantics(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if target.accessibility.take().is_some() {
            self.invalidate_accessibility_chain(node);
        }
        Ok(())
    }

    /// Set plain semantic text content used as a naming fallback or relationship target.
    pub fn set_accessibility_text_content(
        &mut self,
        node: NodeId,
        text: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        let text = text.into();
        if target.accessibility_text.as_deref() != Some(&text) {
            target.accessibility_text = Some(text);
            self.invalidate_all_accessibility();
        }
        Ok(())
    }

    pub fn clear_accessibility_text_content(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        let target = self
            .nodes
            .get_mut(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        if target.accessibility_text.take().is_some() {
            self.invalidate_all_accessibility();
        }
        Ok(())
    }

    /// Mark semantics dirty after an application mutates editor state stored in `StateStore`.
    pub fn invalidate_accessibility(&mut self, node: NodeId) -> Result<(), RuntimeError> {
        self.nodes
            .get(&node)
            .ok_or(RuntimeError::UnknownNode(node))?;
        self.invalidate_accessibility_chain(node);
        Ok(())
    }

    pub(crate) fn invalidate_accessibility_node(&mut self, node: NodeId) {
        self.invalidate_accessibility_chain(node);
    }

    pub(crate) fn invalidate_accessibility_chain(&mut self, node: NodeId) {
        let mut current = Some(node);
        while let Some(id) = current {
            let Some(target) = self.nodes.get_mut(&id) else {
                break;
            };
            target.dirty.insert(DirtyFlags::ACCESSIBILITY);
            current = target.parent;
        }
    }

    pub(crate) fn invalidate_accessibility_subtree(&mut self, node: NodeId) {
        let children = if let Some(target) = self.nodes.get_mut(&node) {
            target.dirty.insert(DirtyFlags::ACCESSIBILITY);
            target.children.clone()
        } else {
            return;
        };
        for child in children {
            self.invalidate_accessibility_subtree(child);
        }
    }

    fn invalidate_all_accessibility(&mut self) {
        for node in self.nodes.values_mut() {
            node.dirty.insert(DirtyFlags::ACCESSIBILITY);
        }
    }
}

fn resolve_relationship_label(tree: &UiTree, labelled_by: &[NodeId]) -> Option<String> {
    labelled_by.iter().find_map(|id| {
        tree.nodes
            .get(id)
            .and_then(|node| node.accessibility_text.as_ref())
            .or_else(|| {
                tree.nodes
                    .get(id)
                    .and_then(|node| node.accessibility.as_ref()?.label.as_ref())
            })
            .cloned()
    })
}

fn intersect_optional(first: Option<Rect>, second: Option<Rect>) -> Option<Rect> {
    match (first, second) {
        (Some(a), Some(b)) => a.intersect(b),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

fn role_filtered_state(
    role: AccessibilityRole,
    mut state: AccessibilityState,
) -> AccessibilityState {
    if !matches!(
        role,
        AccessibilityRole::Button | AccessibilityRole::MenuItem | AccessibilityRole::Tab
    ) {
        state.pressed = None;
    }
    if !matches!(
        role,
        AccessibilityRole::Checkbox | AccessibilityRole::Radio | AccessibilityRole::Switch
    ) {
        state.checked = None;
    }
    if !matches!(
        role,
        AccessibilityRole::Tab
            | AccessibilityRole::TreeItem
            | AccessibilityRole::Row
            | AccessibilityRole::Cell
    ) {
        state.selected = None;
    }
    if !matches!(
        role,
        AccessibilityRole::TreeItem | AccessibilityRole::MenuItem | AccessibilityRole::Dialog
    ) {
        state.expanded = None;
    }
    if role != AccessibilityRole::TextInput {
        state.read_only = None;
        state.required = None;
    }
    state
}

fn default_actions(role: AccessibilityRole) -> Vec<AccessibilityActionKind> {
    match role {
        AccessibilityRole::Button | AccessibilityRole::Link | AccessibilityRole::MenuItem => {
            vec![AccessibilityActionKind::Press]
        }
        AccessibilityRole::TextInput => vec![
            AccessibilityActionKind::Focus,
            AccessibilityActionKind::SetValue,
            AccessibilityActionKind::ReplaceSelectedText,
            AccessibilityActionKind::SetSelection,
        ],
        AccessibilityRole::Slider | AccessibilityRole::ProgressBar => vec![
            AccessibilityActionKind::Increment,
            AccessibilityActionKind::Decrement,
        ],
        AccessibilityRole::TreeItem => vec![
            AccessibilityActionKind::Focus,
            AccessibilityActionKind::Expand,
            AccessibilityActionKind::Collapse,
        ],
        AccessibilityRole::ScrollArea => vec![AccessibilityActionKind::ScrollIntoView],
        _ => Vec::new(),
    }
}

#[derive(Default)]
struct EmptyClipboard;
impl crate::Clipboard for EmptyClipboard {
    fn get_text(&mut self) -> Option<String> {
        None
    }
    fn set_text(&mut self, _text: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Constraints, Dimension, FocusPolicy, HitTestState, LayoutStyle, PaintState, Size};
    use ui_core::Point;

    fn tree() -> (UiTree, NodeId) {
        let mut tree = UiTree::new();
        let root = tree
            .create_node(None, LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.layout(root, Constraints::loose(Size::new(400.0, 300.0)))
            .unwrap();
        (tree, root)
    }

    #[test]
    fn identities_are_stable_and_do_not_depend_on_traversal_index() {
        let (mut tree, root) = tree();
        let first = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let second = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_semantics(
            first,
            AccessibilitySemantics::new(AccessibilityRole::Button),
        )
        .unwrap();
        tree.set_accessibility_semantics(
            second,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let first_id = semantics.accessibility_id(first).unwrap();
        let second_id = semantics.accessibility_id(second).unwrap();
        tree.reparent(second, Some(first)).unwrap();
        semantics.update(&mut tree, root).unwrap();
        assert_eq!(semantics.accessibility_id(first), Some(first_id));
        assert_eq!(semantics.accessibility_id(second), Some(second_id));
        assert_ne!(first_id, second_id);
    }

    #[test]
    fn decorative_wrappers_are_flattened_and_hidden_subtrees_are_omitted() {
        let (mut tree, root) = tree();
        let wrapper = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(Some(wrapper), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let hidden = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let hidden_child = tree
            .create_node(Some(hidden), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let mut decorative = AccessibilitySemantics::new(AccessibilityRole::Group);
        decorative.decorative = true;
        tree.set_accessibility_semantics(wrapper, decorative)
            .unwrap();
        tree.set_accessibility_semantics(
            child,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut hidden_semantics = AccessibilitySemantics::new(AccessibilityRole::Group);
        hidden_semantics.hidden = true;
        tree.set_accessibility_semantics(hidden, hidden_semantics)
            .unwrap();
        tree.set_accessibility_semantics(
            hidden_child,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        assert_eq!(semantics.accessibility_id(wrapper), None);
        assert_eq!(semantics.accessibility_id(hidden_child), None);
        assert!(
            semantics
                .nodes()
                .contains_key(&semantics.accessibility_id(child).unwrap())
        );
    }

    #[test]
    fn labels_follow_explicit_text_relationship_priority_and_ignore_debug_identity() {
        let (mut tree, root) = tree();
        let label = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let input = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_text_content(label, "Email address")
            .unwrap();
        let mut spec = AccessibilitySemantics::new(AccessibilityRole::TextInput);
        spec.labelled_by.push(label);
        tree.set_accessibility_semantics(input, spec).unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let id = semantics.accessibility_id(input).unwrap();
        assert_eq!(
            semantics.nodes()[&id].label.as_deref(),
            Some("Email address")
        );
        tree.set_accessibility_semantics(
            input,
            AccessibilitySemantics {
                label: Some("Account email".to_owned()),
                ..AccessibilitySemantics::new(AccessibilityRole::TextInput)
            },
        )
        .unwrap();
        semantics.update(&mut tree, root).unwrap();
        assert_eq!(
            semantics.nodes()[&id].label.as_deref(),
            Some("Account email")
        );
    }

    #[test]
    fn focus_action_uses_focus_manager_and_press_uses_click_dispatch() {
        use crate::{EventKind, EventType, ListenerPhase};
        let (mut tree, root) = tree();
        let button = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_semantics(
            button,
            AccessibilitySemantics::new(AccessibilityRole::Button),
        )
        .unwrap();
        tree.set_focus_policy(
            button,
            FocusPolicy {
                focusable: true,
                ..FocusPolicy::default()
            },
        )
        .unwrap();
        let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = clicks.clone();
        tree.add_event_listener(
            button,
            EventType::Click,
            ListenerPhase::Bubble,
            move |event| {
                if matches!(event.kind(), EventKind::Click(_)) {
                    observed.set(observed.get() + 1);
                }
            },
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let id = semantics.accessibility_id(button).unwrap();
        assert!(
            semantics
                .route_action(
                    &mut tree,
                    AccessibilityActionRequest {
                        target: id,
                        action: AccessibilityAction::Focus
                    }
                )
                .unwrap()
        );
        assert_eq!(tree.focus_manager().focused(), Some(button));
        assert!(
            semantics
                .route_action(
                    &mut tree,
                    AccessibilityActionRequest {
                        target: id,
                        action: AccessibilityAction::Press
                    }
                )
                .unwrap()
        );
        assert_eq!(clicks.get(), 1);
    }

    #[test]
    fn text_input_actions_use_editor_grapheme_api_and_semantics_diff_only_that_node() {
        let (mut tree, root) = tree();
        let editor_node = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.state_mut()
            .insert(editor_node, TextEditor::new("héllo"));
        let mut spec = AccessibilitySemantics::new(AccessibilityRole::TextInput);
        spec.state.read_only = Some(false);
        spec.multiline = Some(true);
        tree.set_accessibility_semantics(editor_node, spec).unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let id = semantics.accessibility_id(editor_node).unwrap();
        let changed = semantics
            .route_action(
                &mut tree,
                AccessibilityActionRequest {
                    target: id,
                    action: AccessibilityAction::SetSelection { anchor: 1, head: 3 },
                },
            )
            .unwrap();
        assert!(changed);
        assert!(
            semantics
                .route_action(
                    &mut tree,
                    AccessibilityActionRequest {
                        target: id,
                        action: AccessibilityAction::ReplaceSelectedText("X".to_owned()),
                    }
                )
                .unwrap()
        );
        assert_eq!(
            tree.state()
                .get::<TextEditor>(editor_node)
                .unwrap()
                .buffer
                .text(),
            "hXlo"
        );
        let update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(update.changed.len(), 1);
        assert_eq!(
            update.changed[0].text_selection.unwrap(),
            AccessibilityTextSelection { anchor: 2, head: 2 }
        );
    }

    #[test]
    fn nested_transform_scroll_and_clip_define_semantic_bounds_in_logical_points() {
        let (mut tree, root) = tree();
        let scroll = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(Some(scroll), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_hit_test_state(
            scroll,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(100.0, 100.0)),
                transform: Transform::translation(20.0, 30.0),
                ..HitTestState::default()
            },
        )
        .unwrap();
        tree.set_hit_test_state(
            child,
            HitTestState {
                bounds: Rect::from_min_size(Point::ZERO, Size::new(80.0, 80.0)),
                transform: Transform::translation(50.0, 60.0),
                ..HitTestState::default()
            },
        )
        .unwrap();
        tree.attach_scroll_view(scroll, Size::new(100.0, 100.0), Size::new(100.0, 300.0))
            .unwrap();
        tree.scroll_by(scroll, Point::new(0.0, 40.0)).unwrap();
        tree.set_accessibility_semantics(
            child,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let bounds = semantics.nodes()[&semantics.accessibility_id(child).unwrap()]
            .bounds
            .unwrap();
        assert_eq!(bounds.min, Point::new(70.0, 50.0));
        assert_eq!(bounds.max, Point::new(120.0, 130.0));
    }

    #[test]
    fn unchanged_update_is_empty_and_node_removal_is_incremental() {
        let (mut tree, root) = tree();
        let child = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_semantics(
            child,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        assert_eq!(semantics.update(&mut tree, root).unwrap().added.len(), 1);
        assert!(semantics.update(&mut tree, root).unwrap().is_empty());
        let child_id = semantics.accessibility_id(child).unwrap();
        tree.remove_subtree(child).unwrap();
        let update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(update.removed, vec![child_id]);
    }

    #[test]
    fn focus_scope_and_disabled_state_are_respected() {
        let (mut tree, root) = tree();
        let dialog = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let inside = tree
            .create_node(Some(dialog), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let outside = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        for node in [inside, outside] {
            tree.set_accessibility_semantics(
                node,
                AccessibilitySemantics::new(AccessibilityRole::Button),
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
        }
        tree.set_focus_policy(
            outside,
            FocusPolicy {
                focusable: true,
                disabled: true,
                ..FocusPolicy::default()
            },
        )
        .unwrap();
        tree.push_focus_scope(dialog, true, false, None).unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let outside_id = semantics.accessibility_id(outside).unwrap();
        assert_eq!(semantics.nodes()[&outside_id].state.disabled, Some(true));
        assert!(
            !semantics
                .route_action(
                    &mut tree,
                    AccessibilityActionRequest {
                        target: outside_id,
                        action: AccessibilityAction::Focus
                    }
                )
                .unwrap()
        );
        assert_eq!(tree.focus_manager().focused(), Some(inside));
    }

    #[test]
    fn layout_and_scroll_dirty_only_visible_semantic_path() {
        let (mut tree, root) = tree();
        let parent = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let child = tree
            .create_node(
                Some(parent),
                LayoutStyle {
                    width: Dimension::Points(20.0),
                    ..LayoutStyle::default()
                },
                PaintState::default(),
            )
            .unwrap();
        tree.set_accessibility_semantics(
            parent,
            AccessibilitySemantics::new(AccessibilityRole::Group),
        )
        .unwrap();
        tree.set_accessibility_semantics(
            child,
            AccessibilitySemantics::new(AccessibilityRole::Text),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        semantics.update(&mut tree, root).unwrap();
        assert!(semantics.stats().reused_subtrees > 0);
        tree.set_accessibility_text_content(child, "Updated")
            .unwrap();
        let update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(update.changed.len(), 1);
    }

    #[test]
    fn role_specific_selected_and_expanded_state_updates_only_the_target_node() {
        let (mut tree, root) = tree();
        let item = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        let mut spec = AccessibilitySemantics::new(AccessibilityRole::TreeItem);
        spec.state.selected = Some(false);
        spec.state.expanded = Some(false);
        spec.state.checked = Some(true); // Not meaningful for a tree item.
        tree.set_accessibility_semantics(item, spec.clone())
            .unwrap();
        let mut semantics = SemanticTree::default();
        semantics.update(&mut tree, root).unwrap();
        let id = semantics.accessibility_id(item).unwrap();
        assert_eq!(semantics.nodes()[&id].state.selected, Some(false));
        assert_eq!(semantics.nodes()[&id].state.expanded, Some(false));
        assert_eq!(semantics.nodes()[&id].state.checked, None);
        spec.state.selected = Some(true);
        spec.state.expanded = Some(true);
        tree.set_accessibility_semantics(item, spec).unwrap();
        let update = semantics.update(&mut tree, root).unwrap();
        assert_eq!(update.changed.len(), 1);
        assert_eq!(update.changed[0].id, id);
    }

    #[test]
    fn hover_does_not_dirty_accessibility_and_headless_backend_drops_unchanged_frames() {
        use crate::InteractionState;
        let (mut tree, root) = tree();
        let button = tree
            .create_node(Some(root), LayoutStyle::default(), PaintState::default())
            .unwrap();
        tree.set_accessibility_semantics(
            button,
            AccessibilitySemantics::new(AccessibilityRole::Button),
        )
        .unwrap();
        let mut semantics = SemanticTree::default();
        let mut backend = HeadlessAccessibilityBackend::default();
        semantics
            .update_backend(&mut tree, root, &mut backend)
            .unwrap();
        assert_eq!(backend.nodes().len(), 1);
        assert_eq!(backend.update_count(), 1);
        tree.set_interaction(
            button,
            InteractionState {
                hovered: true,
                ..InteractionState::default()
            },
        )
        .unwrap();
        let update = semantics
            .update_backend(&mut tree, root, &mut backend)
            .unwrap();
        assert!(update.is_empty());
        assert_eq!(backend.update_count(), 1);
        let id = semantics.accessibility_id(button).unwrap();
        backend.push_action(AccessibilityActionRequest {
            target: id,
            action: AccessibilityAction::Press,
        });
        assert_eq!(backend.poll_actions().len(), 1);
        assert!(backend.poll_actions().is_empty());
    }
}
