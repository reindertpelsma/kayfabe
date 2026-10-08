//! ⚠⚠ **PROBE — default off (`KF3_DISPLAY_CTRL_PROBE=1`), never a shipped behaviour.**
//!
//! 2026-10-08 (Windows 580.88, H-modeset): `[measured, run53 at 80169b57 and run55 at a88764b3,
//! 2026-10-08]` nvlddmkm never commits a mode on kf3 (the core channel gets setup and LUT methods,
//! no `UPDATE`; no `NVC372 IS_MODE_POSSIBLE` is ever sent), and six `NV04_DISPLAY_COMMON` controls
//! that vfio-10's real GSP answers status 0 are refused by kayfabe. This probe answers those six
//! `NV_OK` with the request ECHOED (no field written, nothing sent to the host), to learn cheaply
//! whether any of them gates the modeset. For four, the echo IS what vfio-10's GSP replied
//! (`[measured, vfio-10 reference]` reply bytes equal to the request): `QUERY_DISPLAY_IDS_WITH_MUX`
//! (`0x73013d`, 8 bytes: no mux), `CHECK_SIDEBAND_I2C_SUPPORT` (`0x73014b`, 8 bytes: none),
//! `VRR_DISPLAY_INFO` (`0x73012c`, 12 bytes) and the private `0x7302a3` (12 bytes). For two it is
//! not: `GET_HOTPLUG_CONFIG` (`0x730109`, 24 bytes; the real GPU sets a display mask) and
//! `DP_GET_CAPS` (`0x731369`, 60 bytes; the real GPU reports its DP capabilities) answer all-zero
//! here ("no hotplug configured", "no DP capability"), which is a truthful description of kf3's one
//! DVI/TMDS connector only in the sense that it has no DP. The real answers (derived from
//! `kf_disp::model`'s topology, layouts from ogkm) are the design if the probe moves the wall.
//!
//! ⊘ Never: answer another control or another params size, a serialized envelope, or touch anything.

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const LOG_CAP: u64 = 32;

/// The flag.
pub const FLAG: &str = "KF3_DISPLAY_CTRL_PROBE";

/// `(command, params size)` — sizes as vfio-10 carries them at 580.88.
pub const ECHOED: [(u32, usize); 6] = [
    (0x0073_013d, 8),
    (0x0073_014b, 8),
    (0x0073_012c, 12),
    (0x0073_02a3, 12),
    (0x0073_0109, 24),
    (0x0073_1369, 60),
];

/// Whether the flag is on (`=1`).
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).as_deref() == Ok("1")
}

/// The probe link.
#[derive(Debug, Clone)]
pub struct DisplayCtrlProbe {
    driver: kf_abi::versions::DriverAbiTable,
    answered: u64,
}

impl DisplayCtrlProbe {
    /// A link for the guest driver `driver`.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> DisplayCtrlProbe {
        DisplayCtrlProbe {
            driver,
            answered: 0,
        }
    }
}

impl CommandPolicy for DisplayCtrlProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let &(_, size) = ECHOED.iter().find(|(c, _)| *c == req.cmd)?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != size
        {
            return None;
        }
        cmd.payload
            .get(req.params_at..req.params_at.checked_add(size)?)?;
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: PROBE {FLAG} {:#010x} client={:#x} object={:#x}: NV_OK, request echoed — a probe, not a shipped behaviour #{}",
                req.cmd, req.client, req.object, self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(DisplayCtrlProbe);

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .expect("580.65.06 has a wire table")
    }

    fn control(cmd: u32, params: &[u8]) -> RpcCommand {
        let a = abi();
        let w = a.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[w.status_off..w.status_off + 4].copy_from_slice(&0x56u32.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
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
    fn only_the_six_at_their_sizes_are_echoed() {
        let mut p = DisplayCtrlProbe::new(abi());
        for (c, n) in ECHOED {
            let params: Vec<u8> = (0..n).map(|i| i as u8).collect();
            let r = p.respond(&control(c, &params)).expect("answered");
            let w = abi().rm_control_wire();
            assert_eq!(&r.body[w.status_off..w.status_off + 4], &[0; 4]);
            assert_eq!(&r.body[w.params_off..], &params[..]);
            assert!(p.respond(&control(c, &vec![0u8; n + 4])).is_none());
        }
        assert!(p.respond(&control(0x0073_011d, &[0u8; 272])).is_none());
    }
}
