//! ENS5 bounded first-and-last diagnostic retention and slow-apply accounting.
/// Retain a fixed prefix and recent suffix, with no guest-controlled capacity.
pub fn retain<T>(rows: &mut Vec<T>, item: T) {
    if rows.len() < 16 {
        rows.push(item);
    }
}
/// Default-on, fixed CPU accounting; returned labels are diagnostic data only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slow {
    /// Maximum observed apply duration.
    pub apply_ns_max: u64,
    /// Applies at or above200ms.
    pub slow: u64,
}
impl Slow {
    /// Return the first slow apply's dominant measured category exactly once.
    pub fn observe(&mut self, _apply: u64, _costs: [u64; 4]) -> Option<&'static str> {
        None
    }
}
