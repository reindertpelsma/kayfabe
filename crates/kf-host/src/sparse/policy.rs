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
    pub fn from_framebuffer(bytes: u64) -> Self {
        let per_vm = (bytes / super::SPARSE_GRAIN).clamp(8192, 131072) as usize;
        Self {
            per_space: per_vm.min(65536),
            per_vm,
        }
    }
}
