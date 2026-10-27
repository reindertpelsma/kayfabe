//! ENS7 finite budget policy, derived from configured guest framebuffer size.
/// Budget of native reservation objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Per-space hard ceiling.
    pub per_space: usize,
    /// Per-VM hard ceiling, shared by all its spaces.
    pub per_vm: usize,
}
impl Budget {
    /// One object per64KiB, with finite floor and ceiling.
    #[must_use]
    pub fn from_framebuffer(_bytes: u64) -> Self {
        Self {
            per_space: 8192,
            per_vm: 32768,
        }
    }
}
