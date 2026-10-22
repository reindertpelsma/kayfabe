//! Immutable same-die PRECOMP snapshot, obtained at realize, never on a vCPU.
use crate::capsderive::Die;
use crate::class::{ClassTable, Row};
use crate::regs::Regs;

/// Bounded host facts, tied to the selected die, class and public layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPrecomp {
    die: Die,
    class: u32,
    windows: u32,
    layout: Vec<(String, Row)>,
    words: Vec<(u32, u32)>,
}

/// Form the plan from public host headers and read it once, without guest inputs.
///
/// # Errors
/// Missing public vocabulary, invalid dimensions or failed/incomplete host read.
pub fn query(
    _table: &ClassTable,
    _regs: &Regs,
    _row: &kf_chip::display::DisplayRow,
    _die: Die,
    _read: impl FnMut(&[u32]) -> Result<Vec<u32>, String>,
) -> Result<HostPrecomp, String> {
    Err("PRECOMP snapshot is not implemented".into())
}

impl HostPrecomp {
    pub(crate) fn words_for(
        &self,
        _table: &ClassTable,
        _row: &kf_chip::display::DisplayRow,
        _die: Die,
    ) -> Option<&[(u32, u32)]> {
        None
    }
}
