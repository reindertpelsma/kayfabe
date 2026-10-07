//! ★ DIAGNOSTIC, default off (`KF3_DIAG_ZERO_OK=1`), NEVER a product answer: Windows Code43,
//! hypothesis 5 (`traces/windows_code43_walls_20261007/README.md`, "Hypothesis 5", 2026-10-07).
//!
//! The hypothesis: StartDevice's stage after the paging channel (thermal and power initialisation;
//! vfio-10 sends THERMAL legacy queries `0x2080852e`/`0x20808530`/`0x2080852a` right after the
//! abort point) consumes the result of an earlier power, thermal, perf or clock query that kayfabe
//! leaves unserviced, and fails before it sends an RPC.
//!
//! The test: with the flag on, the controls in [`DIAG_ZERO_OK`] — each one the guest's own client
//! sent, kayfabe left unserviced in run36 (`68c5879f`), and the real GSP answered with status 0 in
//! vfio-10 — are answered `NV_OK` with their params ZEROED. Zero is not a claim about the GPU; it
//! is the least specific "OK" the test can give. If the abort moves past VFIO 2861, the class is
//! implicated and the owner decides how these queries are answered for real (§S: power, thermal and
//! P-state are host-owned stubs — "refused or reported absent, never filled with invented values").
//! If it does not move, zero answers to this class do not change the outcome.
//!
//! ⊘ What it never does: forward anything to the host, touch GPU state, or answer a control outside
//! the list. A serialized (FINN) envelope or a params window past the payload is left unserviced,
//! exactly as with the flag off. The link is placed just before the unserviced ledger, so a control
//! another link answers is never shadowed. The list is captured from one boot of one die: that is
//! acceptable for a default-off diagnostic and is the reason it can never become a product answer.

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
/// Answers logged per run (the rest are counted only).
const LOG_CAP: u64 = 64;

/// The diagnostic set: run36 (`68c5879f`) unserviced ∩ vfio-10 status 0, restricted to the power,
/// thermal, perf and clock area. Names from OGKM 580.65.06 where it has them; the interface byte
/// (`ctrl2080base.h`: `0x20` PERF, `0x26` PMGR, `0x27` POWER, `0x28` LPWR, `0x05` THERMAL, `0x10`
/// CLK; `| 0x80` = legacy non-privileged) otherwise.
pub const DIAG_ZERO_OK: &[(u32, &str)] = &[
    (0x2080_8524, "THERMAL legacy 0x24 (4 B in vfio-10)"),
    (0x2080_a70a, "POWER legacy 0x0a (26 B)"),
    (0x2080_a630, "PMGR legacy 0x30 (1160 B)"),
    (0x2080_a801, "LPWR legacy 0x01 (1028 B)"),
    (0x2080_a060, "PERF legacy 0x60 (28 B)"),
    (0x2080_9004, "CLK legacy 0x04 (1544 B)"),
    (0x2081_0108, "NV2081 binary API 0x08 (992 B)"),
    (0x2081_010d, "NV2081 binary API 0x0d (0 B)"),
    (0x2080_205b, "NV2080_CTRL_CMD_PERF_SET_POWERSTATE"),
    (0x2080_2068, "NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE"),
    (0x2080_2801, "NV2080_CTRL_CMD_LPWR_DIFR_CTRL"),
    (0x2080_2806, "LPWR 0x06 (4 B)"),
    (
        0x0080_0106,
        "NV0080_CTRL_CMD_BIF_GET_PCIE_POWER_CONTROL_MASK",
    ),
];

/// ★ BISECT (2026-10-07, run38; `traces/windows_code43_walls_20261007/README.md`, "Bisect").
/// The subset of [`DIAG_ZERO_OK`] this build answers. Run38: the three controls Windows sends
/// right before it creates the paging channel (run36 RPCs 520-522, vfio-10 2834-2836).
/// `0x2080a801` is also sent early (run36 RPC 152), and that occurrence is answered too.
pub const BISECT_STEP: &[u32] = &[0x2080_a801, 0x2081_010d, 0x2080_a630];

/// Whether `KF3_DIAG_ZERO_OK=1` is set (read once).
#[must_use]
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_DIAG_ZERO_OK").is_some_and(|v| v == "1"))
}

/// The diagnostic link.
#[derive(Debug, Clone)]
pub struct DiagZeroOk {
    driver: kf_abi::versions::DriverAbiTable,
    answered: u64,
}

impl DiagZeroOk {
    /// A link answering [`DIAG_ZERO_OK`].
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> DiagZeroOk {
        DiagZeroOk {
            driver,
            answered: 0,
        }
    }

    /// The listed name of `cmd`, if it is in the set AND in this bisect step's half.
    #[must_use]
    pub fn listed(cmd: u32) -> Option<&'static str> {
        if !BISECT_STEP.contains(&cmd) {
            return None;
        }
        DIAG_ZERO_OK
            .iter()
            .find(|(c, _)| *c == cmd)
            .map(|(_, n)| *n)
    }
}

impl CommandPolicy for DiagZeroOk {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let name = Self::listed(req.cmd)?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            eprintln!(
                "kf-rm: DIAG-ZERO-OK {:#010x} ({name}) left unserviced: serialized params",
                req.cmd
            );
            return None;
        }
        let end = req.params_at.checked_add(req.params_size as usize)?;
        if end > cmd.payload.len() {
            return None;
        }
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        if let Some(w) = body.get_mut(st..st + 4) {
            w.copy_from_slice(&NV_OK.to_le_bytes());
        }
        body[req.params_at..end].fill(0);
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: DIAG-ZERO-OK {:#010x} ({name}) client={:#x} object={:#x}: NV_OK, {} params byte(s) zeroed (KF3_DIAG_ZERO_OK, diagnostic only) #{}",
                req.cmd, req.client, req.object, req.params_size, self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(DiagZeroOk);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_unique_and_holds_no_internal_or_gr_control() {
        let mut ids: Vec<u32> = DIAG_ZERO_OK.iter().map(|(c, _)| *c).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DIAG_ZERO_OK.len());
        for c in ids {
            // NV2080 INTERNAL (0x0a/0x0b) and GR (0x12) are outside the experiment.
            let iface = (c >> 8) & 0xff;
            assert!(
                !matches!(c >> 16, 0x2080) || !matches!(iface, 0x0a | 0x0b | 0x12),
                "{c:#x}"
            );
        }
    }

    #[test]
    fn only_listed_controls_of_this_bisect_step_are_named() {
        for c in BISECT_STEP {
            assert!(DIAG_ZERO_OK.iter().any(|(d, _)| d == c), "{c:#x}");
            assert!(DiagZeroOk::listed(*c).is_some());
        }
        for (c, _) in DIAG_ZERO_OK {
            assert_eq!(DiagZeroOk::listed(*c).is_some(), BISECT_STEP.contains(c));
        }
        assert!(DiagZeroOk::listed(0x2080_852e).is_none());
        assert!(DiagZeroOk::listed(0x2080_1220).is_none());
    }

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    /// A `GSP_RM_CONTROL` payload at the bench ABI (the `vfguest` test's shape).
    fn control(cmd: u32, params: &[u8], flags: u32) -> RpcCommand {
        let w = abi().rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xff03_0000u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[w.status_off..w.status_off + 4].copy_from_slice(&0x56u32.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
        payload[w.rpc_flags_off..w.rpc_flags_off + 4].copy_from_slice(&flags.to_le_bytes());
        payload[w.params_off..].copy_from_slice(params);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0x4c,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    #[test]
    fn a_listed_control_gets_ok_and_zeroed_params_and_nothing_else_is_answered() {
        let mut p = DiagZeroOk::new(abi());
        let w = abi().rm_control_wire();
        let r = p
            .respond(&control(BISECT_STEP[0], &[0xab; 4], 0))
            .expect("listed");
        assert_eq!(r.rpc_result, NV_OK);
        assert_eq!(&r.body[w.status_off..w.status_off + 4], &[0; 4]);
        assert_eq!(&r.body[w.params_off..], &[0; 4]);
        assert!(p.respond(&control(0x2080_852e, &[0xab; 4], 0)).is_none());
        let mut free = control(BISECT_STEP[0], &[0; 4], 0);
        free.function = RpcFunction::Free;
        assert!(p.respond(&free).is_none());
    }
}
