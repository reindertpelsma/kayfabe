//! ★ v3-gfx — **the per-VM ZBC table.** `GF100_ZBC_CLEAR` controls, answered here and NEVER
//! forwarded (`V3_HEADLESS_GRAPHICS.md` §2.2).
//!
//! ## Why authored, not forwarded
//!
//! The hardware ZBC table is **GPU-global**: a forwarded `SET_ZBC_COLOR_CLEAR` would put a guest
//! entry in the table every host process — and every other VM — shares, with finite slots. That is
//! a cross-tenant resource, so it is `author_host_flags_never_forward_them`'s posture: the guest
//! gets a table of its own, consistent with itself, and the host's is never touched.
//!
//! ## Why a table nobody's hardware reads is still correct
//!
//! v3 places every host-twin mapping as an uncompressed kind (`kf-host` `kind_override: 0`), so no
//! surface the guest renders to carries comptags, and the ROP never resolves a ZBC index for it —
//! a clear is written as pixels. ⊘ The day real compression lands (comptag backing on the store),
//! this decision re-opens: the guest's indices would then have to be the hardware's.
//!
//! ## ⊘ CORRECTION 2026-10-15 (F-1): the defaults are PER DIE
//!
//! The paragraph below describes ONE GA102 capture, which this module used to serve to every die. A sweep of 13 dies on idle rentals
//! (`traces/constants_measure_20261014b/tier2/`, `KF_PROBE_ZBC=1`) showed it right for Ampere, Ada and Blackwell and WRONG for Turing
//! (colour formats 1/2/4 and a non-zero colorDS). The defaults now come from the published table per die (`zbc.*` rows, branch `*`: host
//! hardware state), and a die without a measured row starts with an EMPTY table, counted and logged by name (`defaults`).
//!
//! ## What a fresh table holds (the GA102 capture - historical, see the correction above)
//!
//! The defaults a real GSP's physical RM installs, `[measured vgfx 2026-09-26, RTX 3080 Ti
//! GA102, 580.159.04]` by `GET_ZBC_CLEAR_TABLE_ENTRY` on an idle host: color 1 = 0, 2 = all-ones,
//! 3 = FP32 1.0 (`format` reported 0); depth 1 = 0.0, 2 = 1.0 (`format` 1 = FP32); stencil 1 = 0,
//! 2 = 1, 3 = 0xff (`format` 1 = U8). ⊘ `colorDS` reads back as zero on the host even for an entry
//! set with a non-zero one, so it is reported as zero here too. The index RANGES are the host's
//! own (`GET_ZBC_CLEAR_TABLE_SIZE`, unprivileged, asked once at realize —
//! [`crate::HostFacts::zbc_table_sizes`]), so they follow the die.

use kf_abi::zbc::{self as z, TableType, ZbcValue};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
const NV_ERR_INSUFFICIENT_RESOURCES: u32 = 0x1a;
/// `NVOS54`'s status word inside the control envelope (as `inittables`).
const CONTROL_STATUS_OFF: usize = 12;

/// One table: `(start, end)` inclusive index range and the occupied slots.
#[derive(Debug, Clone)]
struct Table {
    start: u32,
    end: u32,
    slots: std::collections::BTreeMap<u32, ZbcValue>,
}

impl Table {
    fn new(start: u32, end: u32, defaults: &[ZbcValue]) -> Table {
        let mut slots = std::collections::BTreeMap::new();
        for (i, v) in defaults.iter().enumerate() {
            let idx = start + i as u32;
            if idx <= end {
                slots.insert(idx, *v);
            }
        }
        Table { start, end, slots }
    }

    /// Add `v` (dedup by value) → `Ok(index)`, or `Err` when the range is full.
    fn add(&mut self, v: ZbcValue) -> Result<u32, ()> {
        if let Some((&i, _)) = self.slots.iter().find(|(_, s)| **s == v) {
            return Ok(i);
        }
        let free = (self.start..=self.end)
            .find(|i| !self.slots.contains_key(i))
            .ok_or(())?;
        self.slots.insert(free, v);
        Ok(free)
    }
}

/// ★ F-1 (matrix `V3_CONSTANT_MATRIX_20261014.md` 5.3.5): the defaults the host's physical RM installs, measured PER DIE and published in
/// `kf_rm::consttab` (`zbc.color` / `zbc.depth` / `zbc.stencil`, 11 words per entry: `colorFB[4] colorDS[4] depth stencil format`).
/// The old code served ONE capture (a GA102's) to every die; Turing differs (formats 1/2/4, colorDS non-zero). A die with no measured row
/// gets an EMPTY default table (`valid = 0` everywhere - nothing is claimed; RM adds entries on demand), counted and logged by name.
fn defaults(t: TableType, die: kf_chip::DieId) -> Vec<ZbcValue> {
    let key = match t {
        TableType::Color => "zbc.color",
        TableType::Depth => "zbc.depth",
        TableType::Stencil => "zbc.stencil",
    };
    match crate::consttab::lookup(key, die, 0) {
        Ok(row) => row
            .body()
            .as_chunks::<44>()
            .0
            .iter()
            .map(|c| {
                let w = |i: usize| {
                    u32::from_le_bytes([c[4 * i], c[4 * i + 1], c[4 * i + 2], c[4 * i + 3]])
                };
                ZbcValue {
                    color_fb: [w(0), w(1), w(2), w(3)],
                    color_ds: [w(4), w(5), w(6), w(7)],
                    depth: w(8),
                    stencil: w(9),
                    format: w(10),
                }
            })
            .collect(),
        Err(e) => {
            ZBC_DEFAULTS_REFUSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            eprintln!(
                "W349REFUSE zbc defaults ({key}) die={die}: {e}; the ZBC table starts EMPTY (valid=0, nothing claimed)"
            );
            Vec::new()
        }
    }
}

/// How many (table, die) default sets were refused for want of a T3 row (counted clean fallback).
pub static ZBC_DEFAULTS_REFUSED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// ★ The link. Seated with the answering links; claims only `GSP_RM_CONTROL` ids `0x9096xxxx`.
pub struct ZbcPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    /// `None` = the host die reported no ZBC table ⇒ every ZBC control is refused, as there.
    tables: Option<[Table; 3]>,
    /// The guest kernel's last `SET_ZBC_REFERENCED` (diagnostic only).
    pub referenced: bool,
}

/// `NV0080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED` — `{subdevInstance, bZbcSurfacesExist}`.
pub const SET_ZBC_REFERENCED_0080: u32 = 0x0080_200a;
/// `NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED` — `{bZbcSurfacesExist}`.
pub const SET_ZBC_REFERENCED_2080: u32 = 0x2080_0a69;

impl ZbcPolicy {
    /// A fresh per-VM table over the host's index ranges (`[(start, end); 3]` in
    /// [`TableType::ALL`] order), or a refusing link when the host has none.
    #[must_use]
    pub fn new(
        driver: kf_abi::versions::DriverAbiTable,
        sizes: Option<[(u32, u32); 3]>,
        die: kf_chip::DieId,
    ) -> ZbcPolicy {
        let tables = sizes.map(|s| {
            TableType::ALL.map(|t| {
                let (start, end) = s[t.wire() as usize - 1];
                Table::new(start, end, &defaults(t, die))
            })
        });
        ZbcPolicy {
            driver,
            tables,
            referenced: false,
        }
    }

    /// ★ The answer to one ZBC control's params (a pure function of the table, testable without
    /// an envelope): `Ok(reply params)` or `Err(NV status)`.
    ///
    /// # Errors
    /// `NV_ERR_NOT_SUPPORTED` (no table on this die, or a privileged/unmodelled control),
    /// `NV_ERR_INVALID_ARGUMENT` (size, table type, index outside the range), or
    /// `NV_ERR_INSUFFICIENT_RESOURCES` (the range is full).
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Result<Vec<u8>, u32> {
        let tables = self.tables.as_mut().ok_or(NV_ERR_NOT_SUPPORTED)?;
        let need = match cmd {
            z::SET_ZBC_COLOR_CLEAR => z::SET_COLOR_PARAMS_SIZE,
            z::SET_ZBC_DEPTH_CLEAR => z::SET_DEPTH_PARAMS_SIZE,
            z::SET_ZBC_STENCIL_CLEAR => z::SET_STENCIL_PARAMS_SIZE,
            z::GET_ZBC_CLEAR_TABLE_SIZE => z::GET_SIZE_PARAMS_SIZE,
            z::GET_ZBC_CLEAR_TABLE_ENTRY => z::GET_ENTRY_PARAMS_SIZE,
            // GET_ZBC_CLEAR_TABLE (legacy) and the privileged SET_ZBC_CLEAR_TABLE: not modelled.
            _ => return Err(NV_ERR_NOT_SUPPORTED),
        };
        if params.len() != need {
            return Err(NV_ERR_INVALID_ARGUMENT);
        }
        let w = |i: usize| z::word(params, i).unwrap_or(0);
        let mut out = params.to_vec();
        match cmd {
            z::SET_ZBC_COLOR_CLEAR => {
                let v = ZbcValue {
                    color_fb: [w(0), w(1), w(2), w(3)],
                    format: 0,
                    ..ZbcValue::default()
                };
                tables[0]
                    .add(v)
                    .map_err(|()| NV_ERR_INSUFFICIENT_RESOURCES)?;
            }
            z::SET_ZBC_DEPTH_CLEAR => {
                tables[1]
                    .add(ZbcValue {
                        depth: w(0),
                        format: w(1),
                        ..ZbcValue::default()
                    })
                    .map_err(|()| NV_ERR_INSUFFICIENT_RESOURCES)?;
            }
            z::SET_ZBC_STENCIL_CLEAR => {
                tables[2]
                    .add(ZbcValue {
                        stencil: w(0),
                        format: w(1),
                        ..ZbcValue::default()
                    })
                    .map_err(|()| NV_ERR_INSUFFICIENT_RESOURCES)?;
            }
            z::GET_ZBC_CLEAR_TABLE_SIZE => {
                let t = TableType::from_wire(w(2)).ok_or(NV_ERR_INVALID_ARGUMENT)?;
                let tab = &tables[t.wire() as usize - 1];
                z::put(&mut out, 0, tab.start);
                z::put(&mut out, 1, tab.end);
            }
            z::GET_ZBC_CLEAR_TABLE_ENTRY => {
                // value{0..10} format(10) index(11) bIndexValid(12) tableType(13)
                let t = TableType::from_wire(w(13)).ok_or(NV_ERR_INVALID_ARGUMENT)?;
                let tab = &tables[t.wire() as usize - 1];
                let idx = w(11);
                if idx < tab.start || idx > tab.end {
                    return Err(NV_ERR_INVALID_ARGUMENT);
                }
                let v = tab.slots.get(&idx).copied();
                z::put_value(&mut out, &v.unwrap_or_default());
                z::put(&mut out, 10, v.map_or(0, |v| v.format));
                z::put(&mut out, 12, u32::from(v.is_some()));
            }
            _ => return Err(NV_ERR_NOT_SUPPORTED),
        }
        Ok(out)
    }
}

impl CommandPolicy for ZbcPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        // ★ v3-gfx: the guest kernel's "are any ZBC-kind surfaces allocated" statement
        // (`NV0080/NV2080_CTRL_CMD_INTERNAL_MEMSYS_SET_ZBC_REFERENCED`, `mem_mgr_gm107.c:274-292`,
        // `[IN]` only) — its only physical consequence is that RM MAY flush the ZBC table when
        // nothing references it. Our table is per-VM and hardware never reads it (module docs),
        // so the statement is recorded and answered; nothing is flushed or forwarded.
        if req.cmd == SET_ZBC_REFERENCED_0080 || req.cmd == SET_ZBC_REFERENCED_2080 {
            let at = if req.cmd == SET_ZBC_REFERENCED_0080 {
                4
            } else {
                0
            };
            self.referenced = cmd.payload.get(req.params_at + at).is_some_and(|b| *b != 0);
            let mut body = cmd.payload.clone();
            body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].copy_from_slice(&NV_OK.to_le_bytes());
            return Some(Reply {
                rpc_result_private: None,
                rpc_result: NV_OK,
                body,
            });
        }
        if req.cmd >> 16 != 0x9096 {
            return None;
        }
        let refuse = |status: u32| {
            Some(Reply {
                rpc_result_private: None,
                rpc_result: status,
                body: Vec::new(),
            })
        };
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return refuse(NV_ERR_NOT_SUPPORTED);
        }
        let Some(params) = cmd
            .payload
            .get(req.params_at..req.params_at + req.params_size as usize)
        else {
            return refuse(NV_ERR_INVALID_ARGUMENT);
        };
        match self.answer(req.cmd, params) {
            Ok(p) => {
                let mut body = cmd.payload.clone();
                body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
                    .copy_from_slice(&NV_OK.to_le_bytes());
                body[req.params_at..req.params_at + p.len()].copy_from_slice(&p);
                Some(Reply {
                    rpc_result_private: None,
                    rpc_result: NV_OK,
                    body,
                })
            }
            Err(st) => refuse(st),
        }
    }
}

kf_util::assert_send_sync!(ZbcPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ZbcPolicy {
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        // `[measured vgfx 2026-09-26]` the GA102 host's ranges: color 1..30, depth/stencil 1..15.
        ZbcPolicy::new(
            abi,
            Some([(1, 30), (1, 15), (1, 15)]),
            kf_chip::DieId::new(kf_chip::arch::GA100, 2),
        )
    }
    fn entry(p: &mut ZbcPolicy, t: TableType, idx: u32) -> (bool, [u32; 14]) {
        let mut q = vec![0u8; z::GET_ENTRY_PARAMS_SIZE];
        z::put(&mut q, 11, idx);
        z::put(&mut q, 13, t.wire());
        let r = p.answer(z::GET_ZBC_CLEAR_TABLE_ENTRY, &q).expect("entry");
        let w: [u32; 14] = std::array::from_fn(|i| z::word(&r, i).unwrap());
        (w[12] != 0, w)
    }

    /// ★ A fresh table reads exactly as a real GSP's does (the measured defaults), the ranges are
    /// the host's, a SET lands in the first free slot and a repeat SET is deduplicated.
    #[test]
    fn a_fresh_table_is_the_gsps_defaults_and_sets_land_in_the_first_free_slot() {
        let mut p = policy();
        let mut q = vec![0u8; z::GET_SIZE_PARAMS_SIZE];
        z::put(&mut q, 2, 1);
        let r = p.answer(z::GET_ZBC_CLEAR_TABLE_SIZE, &q).expect("size");
        assert_eq!(
            (z::word(&r, 0), z::word(&r, 1), z::word(&r, 2)),
            (Some(1), Some(30), Some(1))
        );
        assert_eq!(entry(&mut p, TableType::Color, 2).1[0..4], [0xffff_ffff; 4]);
        assert_eq!(entry(&mut p, TableType::Color, 3).1[0..4], [0x3f80_0000; 4]);
        assert_eq!(entry(&mut p, TableType::Depth, 2).1[8], 0x3f80_0000);
        assert_eq!(entry(&mut p, TableType::Stencil, 3).1[9], 0xff);
        assert!(
            !entry(&mut p, TableType::Color, 4).0,
            "slot 4 is empty on a fresh table"
        );
        // The host UMD's own SET (bytes from the measured trace): colorFB 0xff000000 x4, format 0x28.
        let mut s = vec![0u8; z::SET_COLOR_PARAMS_SIZE];
        for i in 0..4 {
            z::put(&mut s, i, 0xff00_0000);
        }
        z::put(&mut s, 7, 0x3f80_0000);
        z::put(&mut s, 8, 0x28);
        p.answer(z::SET_ZBC_COLOR_CLEAR, &s).expect("set");
        p.answer(z::SET_ZBC_COLOR_CLEAR, &s).expect("set again");
        let (valid, w) = entry(&mut p, TableType::Color, 4);
        assert!(valid);
        assert_eq!(w[0..4], [0xff00_0000; 4]);
        assert_eq!(w[4..8], [0; 4], "colorDS reads back zero, as on the host");
        assert!(
            !entry(&mut p, TableType::Color, 5).0,
            "the repeat SET was deduplicated"
        );
    }

    /// ⊘ The table is finite and never spills: a full range answers INSUFFICIENT_RESOURCES; an
    /// index outside the range, a bad table type or a wrong size is INVALID_ARGUMENT; no host
    /// ranges ⇒ every ZBC control NOT_SUPPORTED; the privileged SET_ZBC_CLEAR_TABLE is refused.
    #[test]
    fn the_table_is_finite_and_malformed_requests_are_refused() {
        let mut p = policy();
        let mut s = vec![0u8; z::SET_DEPTH_PARAMS_SIZE];
        z::put(&mut s, 1, 1);
        for d in 0..13u32 {
            z::put(&mut s, 0, 0x1000 + d);
            p.answer(z::SET_ZBC_DEPTH_CLEAR, &s).expect("fits");
        }
        z::put(&mut s, 0, 0x9999);
        assert_eq!(
            p.answer(z::SET_ZBC_DEPTH_CLEAR, &s),
            Err(NV_ERR_INSUFFICIENT_RESOURCES)
        );
        let mut q = vec![0u8; z::GET_ENTRY_PARAMS_SIZE];
        z::put(&mut q, 11, 16);
        z::put(&mut q, 13, 2);
        assert_eq!(
            p.answer(z::GET_ZBC_CLEAR_TABLE_ENTRY, &q),
            Err(NV_ERR_INVALID_ARGUMENT)
        );
        z::put(&mut q, 11, 1);
        z::put(&mut q, 13, 4);
        assert_eq!(
            p.answer(z::GET_ZBC_CLEAR_TABLE_ENTRY, &q),
            Err(NV_ERR_INVALID_ARGUMENT)
        );
        assert_eq!(
            p.answer(z::GET_ZBC_CLEAR_TABLE_SIZE, &[0u8; 8]),
            Err(NV_ERR_INVALID_ARGUMENT)
        );
        assert_eq!(
            p.answer(z::SET_ZBC_CLEAR_TABLE, &[0u8; 60]),
            Err(NV_ERR_NOT_SUPPORTED)
        );
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        assert_eq!(
            ZbcPolicy::new(abi, None, kf_chip::DieId::new(kf_chip::arch::GA100, 2))
                .answer(z::GET_ZBC_CLEAR_TABLE_SIZE, &[0u8; 12]),
            Err(NV_ERR_NOT_SUPPORTED)
        );
    }
}
