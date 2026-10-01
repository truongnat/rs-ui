/// Logical-line splice emitted by text edits and consumed by the text layout cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyLineRange {
    pub start: usize,
    pub removed: usize,
    pub inserted: usize,
}
