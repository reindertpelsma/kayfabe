//! ⚠⚠ **AWAITING OWNER CONFIRMATION — default off (`KF3_SW_RUNLIST_HOST_OWNED=1`).**
//!
//! 2026-10-07 (Windows Code43, task B): the software-runlist submit treated as **host-owned
//! scheduling**, option (b) of `traces/windows_code43_walls_20261007/README.md` ("Stop: an owner
//! decision is needed for the software runlist"). The owner has NOT decided between (a), (b) and
//! (c); the controller recommended (b) for one experiment, behind this flag.
//!
//! What the control is (read, 2026-10-07): retail Windows 580.88 sends control
//! [`kf_abi::sw_runlist::EvidenceCell::observed_control`] (`0x20801111`, 40 bytes, no OGKM name or
//! layout) to put a guest-built runlist on the scheduler; its export row flags it
//! `ROUTE_TO_PHYSICAL` (kernel-privileged by default). vfio-10 answers it `NV_OK` with the request
//! echoed. The second graphics TSG of a Windows kernel client is scheduled ONLY by it.
//!
//! What (b) does: kayfabe owns scheduling. Every Translated kernel channel is already scheduled on
//! the HOST at birth (`kf_chan::host`'s ring birth, an unprivileged `GPFIFO_SCHEDULE` of
//! kayfabe's own group); with the flag on, kf-qemu also opens its guest-side gate at birth
//! (`kf_qemu::chan`, `sw_runlist_host_owned`), so a kernel channel runs whether or not the guest
//! ever schedules it. This link answers the submit `NV_OK` with the request echoed, as vfio-10
//! does, and IGNORES its contents: no record of the guest's runlist buffer is read, no field of
//! the 40 bytes is decoded, nothing reaches the host. Windows' ordering, timeslices and removals
//! therefore have no effect (the host scheduler decides), which is the semantic the owner must
//! accept or reject.
//!
//! ⊘ Never: a privileged host verb, a new emulated channel, a host action taken from guest bytes,
//! an answer before the guest declared the cell's Windows identity, an answer to a serialized
//! envelope or to any other params size.

use kf_abi::sw_runlist;
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
/// Answers logged per run (the rest are counted only).
const LOG_CAP: u64 = 16;

/// The flag. Read by kf-rm (this link) and by kf-qemu (the birth-time gate) — one name.
pub const FLAG: &str = "KF3_SW_RUNLIST_HOST_OWNED";

/// Whether the flag is on (`=1`).
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).as_deref() == Ok("1")
}

/// The link.
#[derive(Debug, Clone)]
pub struct SwRunlistHostOwnedPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    cell: Option<&'static sw_runlist::EvidenceCell>,
    identity: bool,
    answered: u64,
}

impl SwRunlistHostOwnedPolicy {
    /// A link for the guest driver `driver`. Inert until a matching identity is declared.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> SwRunlistHostOwnedPolicy {
        SwRunlistHostOwnedPolicy {
            driver,
            cell: sw_runlist::cell(driver.driver_version()),
            identity: false,
            answered: 0,
        }
    }

    /// Answers given so far.
    #[must_use]
    pub fn answered(&self) -> u64 {
        self.answered
    }
}

impl CommandPolicy for SwRunlistHostOwnedPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let cell = self.cell?;
        if cmd.function == RpcFunction::SetGuestSystemInfo {
            // Every new declaration replaces the old one; a changed or malformed one revokes.
            self.identity = cell.matches_identity(&cmd.payload)
                && kf_abi::guestsysinfo::decode_declared_vgx(&cmd.payload).ok()
                    == self.driver.vgx_version();
            return None;
        }
        if cmd.function != RpcFunction::RmControl || !self.identity {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != cell.observed_control
            || req.rmapi_rpc_flags != 0
            || req.params_size as usize != cell.observed_control_size
        {
            return None;
        }
        let end = req.params_at.checked_add(cell.observed_control_size)?;
        if end > cmd.payload.len() {
            return None;
        }
        // The request echoed (vfio-10's answer); only the status word is ours.
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            kf_util::klog!(
                "kf-rm: SW-RUNLIST HOST-OWNED {:#010x} client={:#x} object={:#x}: NV_OK, request echoed, \
                 contents ignored (kernel channels are scheduled at birth) — AWAITING OWNER CONFIRMATION #{}",
                req.cmd,
                req.client,
                req.object,
                self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(SwRunlistHostOwnedPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .expect("580.65.06 has a wire table")
    }

    fn control(
        abi: &kf_abi::versions::DriverAbiTable,
        cmd: u32,
        params: &[u8],
        flags: u32,
    ) -> RpcCommand {
        let w = abi.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xff00_8250u32.to_le_bytes());
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

    /// run41/42's raw words, as bytes.
    fn params() -> Vec<u8> {
        [0xff00_8250u32, 0, 0xff00_0100, 1, 0x6000, 1, 1, 0, 0, 0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }

    #[test]
    fn without_a_declared_identity_nothing_is_answered() {
        let w = windows_abi();
        let mut p = SwRunlistHostOwnedPolicy::new(w);
        assert!(p.cell.is_some(), "580.65.06 has the Windows 580.88 cell");
        assert!(p.respond(&control(&w, 0x2080_1111, &params(), 0)).is_none());
        assert_eq!(p.answered(), 0);
    }

    #[test]
    fn with_the_identity_the_submit_is_echoed_with_nv_ok_and_nothing_else_is_answered() {
        let w = windows_abi();
        let mut p = SwRunlistHostOwnedPolicy::new(w);
        p.identity = true;
        let req = control(&w, 0x2080_1111, &params(), 0);
        let r = p.respond(&req).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        let st = w.rm_control_wire().status_off;
        assert_eq!(&r.body[st..st + 4], &[0; 4]);
        let pa = w.rm_control_wire().params_off;
        assert_eq!(
            &r.body[pa..],
            &params()[..],
            "the request is echoed, not rewritten"
        );
        // Another size, a serialized envelope, the neighbour control, another function: no.
        assert!(p.respond(&control(&w, 0x2080_1111, &[0; 36], 0)).is_none());
        assert!(p.respond(&control(&w, 0x2080_1111, &params(), 1)).is_none());
        assert!(p.respond(&control(&w, 0x2080_1110, &params(), 0)).is_none());
        let mut free = control(&w, 0x2080_1111, &params(), 0);
        free.function = RpcFunction::Free;
        assert!(p.respond(&free).is_none());
        assert_eq!(p.answered(), 1);
    }

    #[test]
    fn a_driver_without_a_cell_answers_nothing() {
        let bench = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        let mut p = SwRunlistHostOwnedPolicy::new(bench);
        assert!(p.cell.is_none());
        p.identity = true;
        assert!(
            p.respond(&control(&bench, 0x2080_1111, &params(), 0))
                .is_none()
        );
    }
}
