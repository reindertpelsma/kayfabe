//! ⚠⚠ **PROBE — default off (`KF3_PREEMPT_BIND_PROBE=1`), never a shipped behaviour.**
//!
//! 2026-10-08 (Windows 580.88, H-preempt-bind): `[measured, run53 at 80169b57, 2026-10-08]` every
//! failed D3D device creation ends with two `NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND`
//! (`0x20801211`, 112-byte params at 580.x) RmControl RPCs, both refused `0x56`, and then the Free
//! of everything; vfio-10's real GSP answers both with status 0. The question this probe answers
//! cheaply: **does the refusal cause the failure?** With the flag on, the control is answered
//! `NV_OK`, the request echoed, and NOTHING is sent to the host: no preemption buffer is bound, no
//! preemption mode is set, no GPU state changes. It is an answer, not a forged completion (no GPU
//! work stands behind a bind), but it is also not true: the guest is told its GFXP/CILP buffers are
//! bound when they are not. That is why it is a probe. The real design (a host-authored bind on the
//! VM's host twin from unprivileged RM verbs, or an owner ruling that preemption is host-owned) is
//! recorded in `traces/windows_code43_walls_20261007/README.md` (run54) and awaits the owner.
//!
//! ⊘ Never: forward the guest's params, act on a guest VA, answer any other control or any other
//! params size (the generated layout at the guest's driver version), or answer a serialized (FINN)
//! envelope. Every answer is logged with the request's decoded fields (bounded: [`LOG_CAP`]).

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
/// Answers logged per run (the rest are counted only).
const LOG_CAP: u64 = 32;

/// The flag.
pub const FLAG: &str = "KF3_PREEMPT_BIND_PROBE";

/// Whether the flag is on (`=1`).
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).as_deref() == Ok("1")
}

/// The command id and params layout at one driver version.
#[derive(Debug, Clone, Copy)]
struct Shape {
    cmd: u32,
    size: usize,
    flags_off: usize,
    h_client_off: usize,
    h_channel_off: usize,
    gfxp_off: usize,
    cilp_off: usize,
}

impl Shape {
    fn at(version: kf_abi::DriverVersion) -> Option<Shape> {
        use kf_abi::generated::matrix as m;
        let cmd = m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_PREEMPTION_BIND
            .at_u32(version)
            .ok()??;
        let l =
            kf_abi::matrix::Resolved::of(&m::NV2080_CTRL_GR_CTXSW_PREEMPTION_BIND_PARAMS, version)
                .ok()?;
        Some(Shape {
            cmd,
            size: l.size(),
            flags_off: l.need("flags").ok()?.off(),
            h_client_off: l.need("hClient").ok()?.off(),
            h_channel_off: l.need("hChannel").ok()?.off(),
            gfxp_off: l.need("gfxpPreemptMode").ok()?.off(),
            cilp_off: l.need("cilpPreemptMode").ok()?.off(),
        })
    }
}

/// The probe link.
#[derive(Debug, Clone)]
pub struct PreemptBindProbe {
    driver: kf_abi::versions::DriverAbiTable,
    shape: Option<Shape>,
    answered: u64,
}

impl PreemptBindProbe {
    /// A link for the guest driver `driver` (inert if the control is not measured there).
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> PreemptBindProbe {
        PreemptBindProbe {
            driver,
            shape: Shape::at(driver.driver_version()),
            answered: 0,
        }
    }

    /// Answers given so far.
    #[must_use]
    pub fn answered(&self) -> u64 {
        self.answered
    }
}

fn word(b: &[u8], off: usize) -> u32 {
    b.get(off..off + 4)
        .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl CommandPolicy for PreemptBindProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let shape = self.shape?;
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != shape.cmd
            || kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != shape.size
        {
            return None;
        }
        let end = req.params_at.checked_add(shape.size)?;
        let p = cmd.payload.get(req.params_at..end)?;
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: PROBE {FLAG} {:#010x} client={:#x} object={:#x} flags={:#x} hClient={:#x} hChannel={:#x} gfxp={} cilp={}: NV_OK, request echoed, NOTHING bound on the host — a probe, not a shipped behaviour #{}",
                req.cmd,
                req.client,
                req.object,
                word(p, shape.flags_off),
                word(p, shape.h_client_off),
                word(p, shape.h_channel_off),
                word(p, shape.gfxp_off),
                word(p, shape.cilp_off),
                self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(PreemptBindProbe);

/// ⚠⚠ **PROBE — default off (`KF3_ZCULL_BIND_PROBE=1`).** 2026-10-08 (Windows run56): with the
/// preemption binds answered, each failed D3D create sends `GR_CTXSW_ZCULL_BIND` from the KMD's kernel
/// client naming the UMD client's channel, which the channel plane refuses `0x1b` (another client's
/// channel). This probe answers ONLY that cross-client case `NV_OK` with the request echoed and
/// nothing bound on the host; a same-client bind still reaches the channel plane as before.
pub const ZCULL_FLAG: &str = "KF3_ZCULL_BIND_PROBE";

/// Whether the ZCULL probe flag is on (`=1`).
#[must_use]
pub fn zcull_enabled() -> bool {
    std::env::var(ZCULL_FLAG).as_deref() == Ok("1")
}

/// The ZCULL cross-client probe link.
#[derive(Debug, Clone)]
pub struct ZcullBindProbe {
    driver: kf_abi::versions::DriverAbiTable,
    shape: Option<(u32, usize)>,
    answered: u64,
}

impl ZcullBindProbe {
    /// A link for the guest driver `driver` (inert if the control is not measured there).
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> ZcullBindProbe {
        use kf_abi::generated::matrix as m;
        let v = driver.driver_version();
        let shape = (|| {
            let cmd = m::CTRL_CMDS_NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND
                .at_u32(v)
                .ok()??;
            let l =
                kf_abi::matrix::Resolved::of(&m::NV2080_CTRL_GR_CTXSW_ZCULL_BIND_PARAMS, v).ok()?;
            Some((cmd, l.size()))
        })();
        ZcullBindProbe {
            driver,
            shape,
            answered: 0,
        }
    }
}

impl CommandPolicy for ZcullBindProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let (id, size) = self.shape?;
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != id
            || kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != size
        {
            return None;
        }
        let p = cmd
            .payload
            .get(req.params_at..req.params_at.checked_add(size)?)?;
        let named = word(p, 0);
        if named == req.client {
            return None;
        }
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: PROBE {ZCULL_FLAG} {:#010x} from client={:#x} names hClient={:#x} hChannel={:#x} va={:#x} mode={}: NV_OK, request echoed, NOTHING bound on the host — a probe #{}",
                req.cmd,
                req.client,
                named,
                word(p, 4),
                u64::from(word(p, 8)) | (u64::from(word(p, 12)) << 32),
                word(p, 16),
                self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(ZcullBindProbe);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zcull_probe_answers_only_the_cross_client_bind() {
        let abi = windows_abi();
        let mut p = ZcullBindProbe::new(abi);
        let mut params = vec![0u8; 24];
        params[0..4].copy_from_slice(&0xc1d0_0040u32.to_le_bytes());
        let r = p
            .respond(&control(&abi, 0x2080_1208, &params, 0))
            .expect("cross-client answered");
        assert_eq!(r.rpc_result, 0);
        params[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        assert!(
            p.respond(&control(&abi, 0x2080_1208, &params, 0)).is_none(),
            "same client goes to the channel plane"
        );
        assert!(
            p.respond(&control(&abi, 0x2080_1208, &[0u8; 20], 0))
                .is_none()
        );
    }

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
    fn the_shape_at_580_is_the_generated_one_vfio10_shows() {
        let s = Shape::at(windows_abi().driver_version()).expect("measured at 580");
        assert_eq!(s.cmd, 0x2080_1211);
        assert_eq!(s.size, 112, "vfio-10: 112-byte params");
    }

    #[test]
    fn only_the_bind_of_the_generated_size_is_answered_and_the_params_are_echoed() {
        let abi = windows_abi();
        let mut p = PreemptBindProbe::new(abi);
        let mut params = vec![0u8; 112];
        params[0] = 2;
        params[16] = 0xaa;
        let r = p
            .respond(&control(&abi, 0x2080_1211, &params, 0))
            .expect("answered");
        assert_eq!(r.rpc_result, 0);
        let w = abi.rm_control_wire();
        assert_eq!(&r.body[w.status_off..w.status_off + 4], &[0, 0, 0, 0]);
        assert_eq!(
            &r.body[w.params_off..],
            &params[..],
            "echoed, never rewritten"
        );
        assert_eq!(p.answered(), 1);
        // Another size, another control, a serialized envelope: declined.
        assert!(
            p.respond(&control(&abi, 0x2080_1211, &[0u8; 104], 0))
                .is_none()
        );
        assert!(p.respond(&control(&abi, 0x2080_1210, &params, 0)).is_none());
        assert!(
            p.respond(&control(&abi, 0x2080_1211, &params, 0xffff_ffff))
                .is_none()
        );
        // A payload that claims the size but is short: declined, no panic.
        let mut short = control(&abi, 0x2080_1211, &params, 0);
        short.payload.truncate(w.params_off + 50);
        assert!(p.respond(&short).is_none());
    }
}
