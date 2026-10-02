use std::collections::HashSet;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    #[default]
    Single,
    Multiple,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionModel<K: Eq + Hash> {
    mode: SelectionMode,
    selected: HashSet<K>,
    anchor: Option<K>,
}

impl<K: Eq + Hash + Clone> SelectionModel<K> {
    pub fn new(mode: SelectionMode) -> Self {
        Self {
            mode,
            selected: HashSet::new(),
            anchor: None,
        }
    }

    pub fn single() -> Self {
        Self::new(SelectionMode::Single)
    }

    pub fn multiple() -> Self {
        Self::new(SelectionMode::Multiple)
    }

    pub fn select(&mut self, key: K) {
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key);
    }

    pub fn toggle(&mut self, key: K) {
        if self.mode == SelectionMode::Single {
            if self.selected.remove(&key) {
                self.anchor = None;
            } else {
                self.select(key);
            }
            return;
        }
        if !self.selected.remove(&key) {
            self.selected.insert(key.clone());
        }
        self.anchor = Some(key);
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn contains(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    pub fn selected(&self) -> &HashSet<K> {
        &self.selected
    }

    pub fn select_range(&mut self, ordered_keys: &[K], target: K) {
        if self.mode == SelectionMode::Single {
            self.select(target);
            return;
        }
        let Some(target_index) = ordered_keys.iter().position(|key| key == &target) else {
            self.select(target);
            return;
        };
        let anchor = self.anchor.clone().unwrap_or_else(|| target.clone());
        let Some(anchor_index) = ordered_keys.iter().position(|key| key == &anchor) else {
            self.select(target);
            return;
        };
        self.selected.clear();
        for key in &ordered_keys[anchor_index.min(target_index)..=anchor_index.max(target_index)] {
            self.selected.insert(key.clone());
        }
        self.anchor = Some(anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_selection_replaces_and_toggle_keeps_one() {
        let mut selection = SelectionModel::single();
        selection.select("row-a");
        selection.toggle("row-b");
        assert!(!selection.contains(&"row-a"));
        assert!(selection.contains(&"row-b"));
        selection.toggle("row-b");
        assert!(!selection.contains(&"row-b"));
    }

    #[test]
    fn multiple_selection_toggles_and_selects_range_by_key() {
        let mut selection = SelectionModel::multiple();
        selection.select("key-b");
        selection.select_range(&["key-a", "key-b", "key-c", "key-d"], "key-d");
        assert_eq!(selection.selected().len(), 3);
        assert!(selection.contains(&"key-c"));
        selection.toggle("key-c");
        assert!(!selection.contains(&"key-c"));
    }

    #[test]
    fn clear_removes_selection_and_anchor() {
        let mut selection = SelectionModel::multiple();
        selection.select("key-a");
        selection.clear();
        assert!(selection.selected().is_empty());
        assert_eq!(selection.anchor(), None);
    }
}
