// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **Fn 72's body, kept for a later reader** — the boot display's console size
//! (`docs/design/V3_DISPLAY.md` §4.11.4 row 2).
//!
//! `GSP_SET_SYSTEM_INFO` (fn 72) is asynchronous: the guest queues it before it boots the GSP and
//! never waits for a reply (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4141`,
//! `src/nvidia/src/kernel/vgpu/rpc.c:10495-10530`), so the state machine answers nothing and returns
//! before any [`crate::CommandPolicy`] sees it ([`crate::GspFsm`]'s `NoReply` arm). One field of it
//! still decides a later answer: `consoleMemSize`, the firmware console CPU-RM preserves, which must
//! become region 0 of the table fn 65 serves (`kf_chip::bar0::fb_layout_with_console`).
//!
//! ⊘ **It is NOT decoded here.** Fn 72 arrives before fn 1 (`kernel_gsp.c:4141` vs `:4225`), and
//! fn 1 is where a device whose guest version was DEFAULTED re-selects its tables
//! (`kf_rm::ReselectAtFn1`); that re-selection does not compare `GspSystemInfo`
//! (`kf_abi::versions::pre_fn1_surface_differs`), and `consoleMemSize` sits at offset 48, 56 or 64
//! depending on the version. Decoding at fn 72 could read the wrong field. So the BYTES are kept, in
//! a cell the chain shares across every rebuild (like the census and the memory inbox), and fn 65's
//! encoder decodes them with the table that serves fn 65.
//!
//! The copy is bounded by the largest `GspSystemInfo` in the driver matrix
//! ([`system_info_max`]); the declared length is kept beside it, so a decoder can tell a guest
//! struct of another size from the one its version declares. A later fn 72 — the next driver life —
//! replaces the cell.
//!
//! Concurrency: one writer (the state machine, on whatever thread services the command queue) and
//! one reader (fn 65's policy, on the same thread in kf3); the lock is held for one bounded copy and
//! nothing else, never across a call out.

use std::sync::{Arc, Mutex};

/// The largest `sizeof(GspSystemInfo)` over every version in the driver matrix — the bound on the copy.
/// ⊘ Derived from the matrix, never typed: a sweep that grows the struct grows the bound.
#[must_use]
pub fn system_info_max() -> usize {
    kf_abi::generated::matrix::GSPSYSTEMINFO
        .runs
        .iter()
        .filter_map(|r| r.value)
        .map(kf_abi::matrix::Layout::size)
        .max()
        .unwrap_or(0)
}

/// One stashed fn 72.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashedSystemInfo {
    /// The body's length as the guest's envelope declared it (`sizeof(GspSystemInfo)` at the
    /// guest's version).
    pub declared_len: usize,
    /// The body, at most [`system_info_max`] bytes of it.
    pub bytes: Vec<u8>,
    /// The command's `rpc.sequence`, for the log.
    pub sequence: u32,
}

/// ★ The shared cell. `Clone` hands out the same cell, and two cells are equal when they are the
/// same cell (the state machine that holds one is compared by value).
#[derive(Debug, Clone, Default)]
pub struct SystemInfoCell(Arc<Mutex<Option<StashedSystemInfo>>>);

impl PartialEq for SystemInfoCell {
    fn eq(&self, other: &SystemInfoCell) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SystemInfoCell {}

impl SystemInfoCell {
    /// An empty cell (no fn 72 seen).
    #[must_use]
    pub fn new() -> SystemInfoCell {
        SystemInfoCell::default()
    }

    /// Keep `payload` (fn 72's declared body), replacing whatever an earlier fn 72 left.
    pub fn store(&self, payload: &[u8], sequence: u32) {
        let keep = payload.len().min(system_info_max());
        let stashed = StashedSystemInfo {
            declared_len: payload.len(),
            bytes: payload[..keep].to_vec(),
            sequence,
        };
        match self.0.lock() {
            Ok(mut g) => *g = Some(stashed),
            Err(p) => *p.into_inner() = Some(stashed),
        }
    }

    /// The last fn 72 stored, if any.
    #[must_use]
    pub fn latest(&self) -> Option<StashedSystemInfo> {
        match self.0.lock() {
            Ok(g) => (*g).clone(),
            Err(p) => (*p.into_inner()).clone(),
        }
    }
}

kf_util::assert_send_sync!(SystemInfoCell);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bound_is_the_largest_struct_in_the_matrix() {
        let max = system_info_max();
        assert!(
            max >= 900,
            "GspSystemInfo is ~936 bytes at 580.x, got {max}"
        );
        for r in kf_abi::generated::matrix::GSPSYSTEMINFO.runs {
            if let Some(l) = r.value {
                assert!(l.size() <= max);
            }
        }
    }

    #[test]
    fn a_later_fn72_replaces_the_cell_and_clones_share_it() {
        let a = SystemInfoCell::new();
        let b = a.clone();
        assert_eq!(b.latest(), None);
        a.store(&[1, 2, 3], 7);
        a.store(&[4, 5], 9);
        assert_eq!(
            b.latest(),
            Some(StashedSystemInfo {
                declared_len: 2,
                bytes: vec![4, 5],
                sequence: 9
            })
        );
    }

    #[test]
    fn an_oversized_body_is_cut_to_the_bound_and_its_length_kept() {
        let c = SystemInfoCell::new();
        let big = vec![0xA5u8; system_info_max() + 100];
        c.store(&big, 1);
        let s = c.latest().unwrap();
        assert_eq!(s.declared_len, big.len());
        assert_eq!(s.bytes.len(), system_info_max());
    }
}
