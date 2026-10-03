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

/// ★ 2026-10-04 (branch `v3-windows`, runbook C3; `docs/design/THE_WINDOWS_AXIS.md` §7): fn 72's
/// `bGspNocatEnabled`, read at the PROVISIONAL driver's layout and logged — never decided on.
///
/// The only assignment in ogkm is `if (RMCFG_FEATURE_PLATFORM_WINDOWS) rpcInfo->bGspNocatEnabled =
/// NV_TRUE;` (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10650`), so `1` here says the guest RM
/// was built for Windows. ⚠ Fn 72 precedes fn 1, where a defaulted device may re-select its tables
/// (`kf_rm::ReselectAtFn1`), so the line names the layout it read with and its size against the
/// guest's declared length; a mismatch is printed, not resolved.
pub fn nocat_line(driver: &kf_abi::versions::DriverAbiTable, payload: &[u8]) -> String {
    let version = driver.driver_version();
    match kf_abi::matrix::Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, version) {
        Err(e) => {
            format!("bGspNocatEnabled not read: no GspSystemInfo layout at driver {version}: {e}")
        }
        Ok(layout) => match layout.maybe("bGspNocatEnabled").and_then(|f| f.range()) {
            None => {
                format!("bGspNocatEnabled not read: absent from driver {version}'s GspSystemInfo")
            }
            Some(r) => {
                let value = payload
                    .get(r.clone())
                    .map(|b| b.iter().rev().fold(0u64, |a, &x| (a << 8) | u64::from(x)));
                let size_note = if payload.len() == layout.size() {
                    String::new()
                } else {
                    format!(
                        " ⚠ the guest's body is {} bytes, the layout {}",
                        payload.len(),
                        layout.size()
                    )
                };
                match value {
                    Some(v) => format!(
                        "bGspNocatEnabled={v} at +{} (driver {version}'s layout{size_note}; 1 = a \
                         Windows-built RM, THE_WINDOWS_AXIS §7)",
                        r.start
                    ),
                    None => format!(
                        "bGspNocatEnabled not read: +{} is past the guest's {}-byte body",
                        r.start,
                        payload.len()
                    ),
                }
            }
        },
    }
}

/// Log [`nocat_line`] for one fn 72.
pub fn log_nocat(driver: &kf_abi::versions::DriverAbiTable, payload: &[u8], sequence: u32) {
    eprintln!(
        "kf-gsp: fn 72 GSP_SET_SYSTEM_INFO seq={sequence} {} bytes: {}",
        payload.len(),
        nocat_line(driver, payload)
    );
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

    /// ★ 2026-10-04 (v3-windows, runbook C3): the line reads `bGspNocatEnabled` at the driver's own
    /// layout, and says when the guest's body is not that layout's size.
    #[test]
    fn the_nocat_line_reads_the_flag_at_the_drivers_layout() {
        let v = kf_abi::DriverVersion::parse("580.159.04").unwrap();
        let driver = kf_abi::versions::table_for(v).unwrap();
        let layout =
            kf_abi::matrix::Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, v).unwrap();
        let at = layout.need("bGspNocatEnabled").unwrap().off();
        let mut body = vec![0u8; layout.size()];
        assert!(nocat_line(driver, &body).starts_with("bGspNocatEnabled=0 at +"));
        body[at] = 1;
        let line = nocat_line(driver, &body);
        assert!(
            line.starts_with(&format!("bGspNocatEnabled=1 at +{at} ")),
            "{line}"
        );
        assert!(!line.contains('⚠'), "{line}");
        body.push(0);
        assert!(nocat_line(driver, &body).contains('⚠'));
        assert!(nocat_line(driver, &body[..at]).contains("past the guest's"));
    }

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
