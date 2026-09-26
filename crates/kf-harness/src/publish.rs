//! ★★★ **Publish the guest's tables into a host VA space the way production does** — one walk
//! (the GPU's DIFF against the slot's committed placements), [`kf_mem::apply::apply_entry`], and
//! the per-run verdict back to the walker (commit on ack, `kf_cuda::diffmodel`).
//!
//! [`Recorded`] wraps a target and keeps the rows WE placed, so a gate can read the guest's GP
//! entries and segments "through our mappings" exactly as a Translated channel does in kf-qemu
//! (`PlacedRows`) — the CPU record a reader needs, which the walk's diff cannot provide.

use kf_cuda::walk::{WalkEntry, WalkKernel};
use kf_mem::apply::{ApplyCfg, Applied, DiffRun, apply_entry};
use kf_mem::ledger::{Desired, MapTarget, Mapped};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// A target that records the rows it placed, `va → (len, offset, ram)`.
pub struct Recorded<T: MapTarget> {
    /// The wrapped target.
    pub inner: T,
    rows: RefCell<BTreeMap<u64, (u64, u64, bool)>>,
}

impl<T: MapTarget> Recorded<T> {
    /// Wrap `inner`.
    pub fn new(inner: T) -> Self {
        Recorded { inner, rows: RefCell::new(BTreeMap::new()) }
    }

    /// Where `[va, va+len)` lives in OUR placements: `(ram, offset)` when one row covers it.
    #[must_use]
    pub fn resolve(&self, va: u64, len: u64) -> Option<(bool, u64)> {
        let r = self.rows.borrow();
        let (&start, &(rlen, off, ram)) = r.range(..=va).next_back()?;
        let end = va.checked_add(len)?;
        (end <= start.checked_add(rlen)?).then(|| (ram, off + (va - start)))
    }

    /// Rows placed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.borrow().len()
    }

    /// Nothing placed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.borrow().is_empty()
    }
}

impl<T: MapTarget> MapTarget for Recorded<T> {
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let m = self.inner.map(d, defer)?;
        if m == Mapped::Placed {
            self.rows.borrow_mut().insert(d.va, (d.len, d.off, d.ram));
        }
        Ok(m)
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        self.rows.borrow_mut().remove(&va);
        self.inner.unmap(va, defer)
    }
    fn invalidate(&self) -> Result<(), String> {
        self.inner.invalidate()
    }
    fn va_extent(&self) -> Option<u64> {
        self.inner.va_extent()
    }
    fn reserved(&self) -> Vec<(u64, u64)> {
        self.inner.reserved()
    }
}

/// What one [`publish`] did.
#[derive(Debug, Clone, Default)]
pub struct Published {
    /// The diff's runs (0 ⇒ the tables were unchanged since the last commit).
    pub runs: usize,
    /// The apply.
    pub applied: Applied,
    /// The walk's GPU time.
    pub gpu_us: u64,
}

/// ★ One publication of `root` (slot `slot`) into `target`: walk → diff → apply → verdict.
///
/// # Errors
/// The walk or its report, refused by name. A run the host refused is NOT an error here — it is
/// in `applied.refused`, acknowledged FAILED, and re-emitted by the next publish.
pub fn publish(
    walk: &mut WalkKernel,
    slot: u32,
    root: u64,
    target: &dyn MapTarget,
    store_bytes: u64,
    ram_offset: &dyn Fn(u64, u64) -> Option<u64>,
) -> Result<Published, String> {
    walk.submit(&[WalkEntry { pdb: root, slot }]).map_err(|e| e.to_string())?;
    let c = walk.wait(10_000).map_err(|e| e.to_string())?;
    let r = &c.report;
    r.validate().map_err(|e| format!("report: {e}"))?;
    r.require_diff().map_err(|e| format!("report: {e}"))?;
    if r.truncated() {
        return Err(format!("report TRUNCATED (flags={:#x} refuse_mask={:#x})", r.header.flags, r.header.refuse_mask));
    }
    let e = r.pdbs.first().ok_or("report: no entry")?;
    let first = e.first_run as usize;
    let runs: Vec<DiffRun> = r.runs[first..first + e.run_count as usize]
        .iter()
        .map(|m| DiffRun {
            unmap: m.op == kf_cuda::abi::KFWR_OP_UNMAP,
            va: m.va,
            len: m.len,
            at: m.gpga,
            ap: m.aperture(),
            held: m.flags & kf_cuda::abi::KFWR_RF_HELD != 0,
            kind: ((m.flags >> 16) & 0xff) as u8,
        })
        .collect();
    let applied = apply_entry(target, &runs, &ApplyCfg { store_bytes, grain: 0x1000, ram_offset, usermode: None });
    let mut codes = vec![kf_cuda::abi::KFWR_ACK_FAILED; r.runs.len()];
    codes[first..first + applied.codes.len()].copy_from_slice(&applied.codes);
    walk.ack(r.header.generation, codes).map_err(|e| e.to_string())?;
    Ok(Published { runs: runs.len(), applied, gpu_us: c.gpu_us })
}
