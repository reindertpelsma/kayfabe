//! ★ Windows Code43 batch, 2026-10-07 — **subdevice controls answered the way NVIDIA's own
//! virtual-GPU guest RM answers them** (the `_VF` HAL), per VM and never forwarded.
//!
//! ## Why these, and why this answer
//!
//! `[measured]` run24 (`traces/windows_code43_walls_20261007/`): Windows StartDevice gives up at
//! the first refusal it does not tolerate, and at `a6f84d0d` that refusal is
//! `NV2080_CTRL_CMD_GET_RC_RECOVERY` (`0x2080220e`). In the Code43-free VFIO boots
//! (vfio-8/9/10) the same RPC is index 2515, followed by `SET_RC_RECOVERY` (2516) and, a little
//! later, `PERF_GET_POWERSTATE` (2571).
//!
//! All three are `ROUTE_TO_PHYSICAL | PHYSICAL_IMPLEMENTED_ON_VGPU_GUEST` in the guest's own
//! export table (flags `0x40154` and `0x40048`, `ogkm-580.65.06:
//! src/nvidia/generated/g_subdevice_nvoc.c:6896-6905, 7164-7195`). That second flag is NVIDIA's
//! statement that a *virtual* GPU's guest answers the control itself, and OGKM carries the guest
//! bodies:
//!
//! | control | vGPU-guest body | what this link answers |
//! |---|---|---|
//! | `GET_RC_RECOVERY` `0x2080220e` | `subdeviceCtrlCmdGetRcRecovery_VF`: `rcEnable = DISABLED` (`kernel_rc_ctrl.c:321-329`) | the VM's own setting, which is always `DISABLED` |
//! | `SET_RC_RECOVERY` `0x2080220d` | `subdeviceCtrlCmdSetRcRecovery_56cd7a`: `return NV_OK` for any value (`g_subdevice_nvoc.h:7788-7790`) | `DISABLED` → `NV_OK`; `ENABLED` → `NV_ERR_NOT_SUPPORTED`; anything else → `NV_ERR_INVALID_ARGUMENT` |
//! | `PERF_GET_POWERSTATE` `0x2080205a` | `subdeviceCtrlCmdPerfGetPowerstate_VF`: `powerState = AC` (`kern_perf_ctrl.c:293-308`) | `NV2080_CTRL_PERF_POWER_SOURCE_AC` |
//!
//! Param layouts: `NV2080_CTRL_CMD_RC_RECOVERY_PARAMS { NvU32 rcEnable }` with
//! `DISABLED = 0`, `ENABLED = 1` (`ctrl2080rc.h:251-269`); `NV2080_CTRL_PERF_GET_POWERSTATE_PARAMS
//! { { NvU32 powerState } }` with `AC = 0` (`ctrl2080perf.h:143-148, 759-765`).
//!
//! ## ⊘ What the answers do NOT claim
//!
//! - **RC recovery.** `DISABLED` is the VM's *own* setting: kayfabe performs no robust-channel
//!   recovery on the VM's behalf, and the host's global RC policy is a privileged control
//!   (`RMCTRL_FLAGS_PRIVILEGED`, `0x4` in `0x40154`) that an unprivileged host client can neither
//!   read nor set. Nothing reaches the host. ⊘ The VF HAL accepts `SET ENABLED` without effect;
//!   this link is stricter and refuses it, because accepting it would make the next `GET` a lie
//!   or the `SET` a silent no-op. Implementing RC recovery for the VM's own channels is what
//!   would let it accept `ENABLED`.
//! - **Measured difference from the VFIO boots, recorded so nobody reads it as a match.** The
//!   physical GSP of a passthrough RTX 4070 answered `rcEnable = ENABLED` (1) at index 2515, and
//!   Windows then sent `SET_RC_RECOVERY(ENABLED)` (params at body offset 40, the 575+ control
//!   header; vfio-8/9/10 identical). A vGPU guest answers `DISABLED`. Whether Windows tolerates
//!   `DISABLED` (and then sends `SET(DISABLED)`) is what run25 measures.
//! - **Power.** `AC` is the source NVIDIA's guest RM reports for every virtual GPU ("we do not
//!   initialize perf engine for guest"); this VM has no battery. Nothing about P-states, clocks or
//!   power policy is answered here.
//!
//! ## Hostile guest
//!
//! Every input is the guest's: a serialized (FINN) envelope, a `paramsSize` other than the
//! struct's, or a declared window past the payload is refused, never decoded. The link holds
//! two counters and one enum; nothing a guest sends can grow it.

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NVOS54`'s status word inside the control envelope (as `inittables`, `zbc`).
const CONTROL_STATUS_OFF: usize = 12;

/// `NV2080_CTRL_CMD_SET_RC_RECOVERY` (`ctrl2080rc.h:255`).
pub const SET_RC_RECOVERY: u32 = 0x2080_220d;
/// `NV2080_CTRL_CMD_GET_RC_RECOVERY` (`ctrl2080rc.h:261`).
pub const GET_RC_RECOVERY: u32 = 0x2080_220e;
/// `NV2080_CTRL_CMD_PERF_GET_POWERSTATE` (`ctrl2080perf.h:759`).
pub const PERF_GET_POWERSTATE: u32 = 0x2080_205a;
/// `NV2080_CTRL_CMD_RC_RECOVERY_DISABLED` (`ctrl2080rc.h:268`).
pub const RC_RECOVERY_DISABLED: u32 = 0;
/// `NV2080_CTRL_CMD_RC_RECOVERY_ENABLED` (`ctrl2080rc.h:269`).
pub const RC_RECOVERY_ENABLED: u32 = 1;
/// `NV2080_CTRL_PERF_POWER_SOURCE_AC` (`ctrl2080perf.h:143`).
pub const POWER_SOURCE_AC: u32 = 0;
/// `sizeof(NV2080_CTRL_CMD_RC_RECOVERY_PARAMS)` and `sizeof(NV2080_CTRL_PERF_GET_POWERSTATE_PARAMS)`.
pub const PARAMS_SIZE: usize = 4;

/// The VM's RC-recovery setting. Only one value exists today; the type is the place where an
/// implemented `Enabled` would go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcRecovery {
    /// No robust-channel recovery is performed on the VM's behalf.
    Disabled,
}

impl RcRecovery {
    fn wire(self) -> u32 {
        match self {
            RcRecovery::Disabled => RC_RECOVERY_DISABLED,
        }
    }
}

/// ★ The link. Seated with the answering links; claims only the three controls above.
#[derive(Debug)]
pub struct VfGuestPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    rc: RcRecovery,
    /// `SET_RC_RECOVERY(DISABLED)` accepted.
    pub rc_sets: u64,
    /// `SET_RC_RECOVERY` refused (ENABLED, an unknown value, or a malformed envelope).
    pub rc_refused: u64,
}

impl VfGuestPolicy {
    /// A fresh per-VM link: RC recovery `DISABLED`.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> VfGuestPolicy {
        VfGuestPolicy {
            driver,
            rc: RcRecovery::Disabled,
            rc_sets: 0,
            rc_refused: 0,
        }
    }

    /// The VM's current RC-recovery setting.
    #[must_use]
    pub fn rc_recovery(&self) -> RcRecovery {
        self.rc
    }

    /// Whether `cmd` is one of this link's controls.
    #[must_use]
    pub fn claims(cmd: u32) -> bool {
        matches!(cmd, SET_RC_RECOVERY | GET_RC_RECOVERY | PERF_GET_POWERSTATE)
    }

    /// ★ The answer to one control's params (a pure function of the VM's state, testable without
    /// an envelope): `Ok(reply params)` or `Err(NV status)`.
    ///
    /// # Errors
    /// `NV_ERR_INVALID_ARGUMENT` for a params size other than four bytes or an `rcEnable` that is
    /// neither value; `NV_ERR_NOT_SUPPORTED` for `SET_RC_RECOVERY(ENABLED)` and any other control.
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Result<Vec<u8>, u32> {
        if !Self::claims(cmd) {
            return Err(NV_ERR_NOT_SUPPORTED);
        }
        let Ok(word) = <[u8; PARAMS_SIZE]>::try_from(params) else {
            if cmd == SET_RC_RECOVERY {
                self.rc_refused += 1;
            }
            return Err(NV_ERR_INVALID_ARGUMENT);
        };
        match cmd {
            // `[OUT]` only: the guest's input word is not read (vfio-10 sends stack garbage).
            GET_RC_RECOVERY => Ok(self.rc.wire().to_le_bytes().to_vec()),
            SET_RC_RECOVERY => match u32::from_le_bytes(word) {
                RC_RECOVERY_DISABLED => {
                    self.rc = RcRecovery::Disabled;
                    self.rc_sets += 1;
                    Ok(word.to_vec())
                }
                RC_RECOVERY_ENABLED => {
                    self.rc_refused += 1;
                    Err(NV_ERR_NOT_SUPPORTED)
                }
                _ => {
                    self.rc_refused += 1;
                    Err(NV_ERR_INVALID_ARGUMENT)
                }
            },
            PERF_GET_POWERSTATE => Ok(POWER_SOURCE_AC.to_le_bytes().to_vec()),
            _ => Err(NV_ERR_NOT_SUPPORTED),
        }
    }
}

impl CommandPolicy for VfGuestPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if !Self::claims(req.cmd) {
            return None;
        }
        let refuse = |status: u32| {
            Some(Reply {
                rpc_result: status,
                body: Vec::new(),
            })
        };
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            if req.cmd == SET_RC_RECOVERY {
                self.rc_refused += 1;
            }
            return refuse(NV_ERR_NOT_SUPPORTED);
        }
        let Some(params) = req
            .params_at
            .checked_add(req.params_size as usize)
            .and_then(|end| cmd.payload.get(req.params_at..end))
        else {
            if req.cmd == SET_RC_RECOVERY {
                self.rc_refused += 1;
            }
            return refuse(NV_ERR_INVALID_ARGUMENT);
        };
        match self.answer(req.cmd, params) {
            Ok(p) => {
                let mut body = cmd.payload.clone();
                body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
                    .copy_from_slice(&NV_OK.to_le_bytes());
                body[req.params_at..req.params_at + p.len()].copy_from_slice(&p);
                Some(Reply {
                    rpc_result: NV_OK,
                    body,
                })
            }
            Err(st) => refuse(st),
        }
    }
}

kf_util::assert_send_sync!(VfGuestPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    /// A `GSP_RM_CONTROL` payload at the bench ABI: header, then `params`, with `params_size`
    /// declared independently so a lie can be built.
    fn control(cmd: u32, params: &[u8], declared: u32, flags: u32) -> RpcCommand {
        let a = abi();
        let w = a.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xff03_0000u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4].copy_from_slice(&declared.to_le_bytes());
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

    fn params_of(p: &mut VfGuestPolicy, cmd: &RpcCommand) -> Result<u32, u32> {
        let r = p.respond(cmd).expect("claimed");
        if r.rpc_result != NV_OK {
            return Err(r.rpc_result);
        }
        let at = abi().rm_control_wire().params_off;
        assert_eq!(&r.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4], &[0; 4]);
        Ok(u32::from_le_bytes(r.body[at..at + 4].try_into().unwrap()))
    }

    /// ★ The VFIO sequence, answered as the VF HAL does: GET reports DISABLED whatever the guest's
    /// uninitialised input word was (vfio-10 sent 0x4da066c8), SET(DISABLED) is accepted, a second
    /// GET still reads DISABLED, and the power source is AC.
    #[test]
    fn get_reports_disabled_set_disabled_is_accepted_and_power_is_ac() {
        let mut p = VfGuestPolicy::new(abi());
        let garbage = 0x4da0_66c8u32.to_le_bytes();
        assert_eq!(
            params_of(&mut p, &control(GET_RC_RECOVERY, &garbage, 4, 0)),
            Ok(RC_RECOVERY_DISABLED)
        );
        assert_eq!(
            params_of(&mut p, &control(SET_RC_RECOVERY, &[0; 4], 4, 0)),
            Ok(RC_RECOVERY_DISABLED)
        );
        assert_eq!(
            params_of(&mut p, &control(GET_RC_RECOVERY, &[0xff; 4], 4, 0)),
            Ok(RC_RECOVERY_DISABLED)
        );
        assert_eq!(p.rc_sets, 1);
        assert_eq!(p.rc_recovery(), RcRecovery::Disabled);
        assert_eq!(
            params_of(&mut p, &control(PERF_GET_POWERSTATE, &[0xff; 4], 4, 0)),
            Ok(POWER_SOURCE_AC)
        );
    }

    /// ⊘ ENABLED is refused NOT_SUPPORTED and changes nothing; any other value is
    /// INVALID_ARGUMENT; the next GET still reports DISABLED.
    #[test]
    fn set_enabled_and_unknown_values_are_refused_and_change_nothing() {
        let mut p = VfGuestPolicy::new(abi());
        assert_eq!(
            params_of(
                &mut p,
                &control(SET_RC_RECOVERY, &RC_RECOVERY_ENABLED.to_le_bytes(), 4, 0)
            ),
            Err(NV_ERR_NOT_SUPPORTED)
        );
        for v in [2u32, 0x8000_0000, u32::MAX] {
            assert_eq!(
                params_of(&mut p, &control(SET_RC_RECOVERY, &v.to_le_bytes(), 4, 0)),
                Err(NV_ERR_INVALID_ARGUMENT)
            );
        }
        assert_eq!(p.rc_refused, 4);
        assert_eq!(p.rc_sets, 0);
        assert_eq!(
            params_of(&mut p, &control(GET_RC_RECOVERY, &[0; 4], 4, 0)),
            Ok(RC_RECOVERY_DISABLED)
        );
    }

    /// ⊘ Hostile envelopes: a wrong `paramsSize` (short, long, zero, huge), a declared window past
    /// the payload, and a FINN-serialized envelope are all refused; none panics.
    #[test]
    fn hostile_envelopes_are_refused_without_decoding() {
        let mut p = VfGuestPolicy::new(abi());
        for cmd in [GET_RC_RECOVERY, SET_RC_RECOVERY, PERF_GET_POWERSTATE] {
            for (params, declared) in [
                (&[0u8; 0][..], 0u32),
                (&[0u8; 3][..], 3),
                (&[0u8; 8][..], 8),
                (&[0u8; 4][..], 5),
                (&[0u8; 4][..], u32::MAX),
            ] {
                assert_eq!(
                    params_of(&mut p, &control(cmd, params, declared, 0)),
                    Err(NV_ERR_INVALID_ARGUMENT),
                    "{cmd:#x} declared {declared} with {} bytes",
                    params.len()
                );
            }
            // RMAPI_RPC_FLAGS_SERIALIZED (the bit `kf_abi::rpc_params_are_serialized` reads).
            let serialized = (0..32)
                .map(|b| 1u32 << b)
                .find(|f| kf_abi::rpc_params_are_serialized(*f))
                .expect("a serialized flag bit");
            assert_eq!(
                params_of(&mut p, &control(cmd, &[0; 4], 4, serialized)),
                Err(NV_ERR_NOT_SUPPORTED)
            );
        }
        // A truncated header is not a control this link can read: not claimed.
        let mut short = control(GET_RC_RECOVERY, &[0; 4], 4, 0);
        short.payload.truncate(10);
        assert!(p.respond(&short).is_none());
        assert_eq!(p.rc_recovery(), RcRecovery::Disabled);
    }

    /// The link claims exactly its three controls and nothing of another function.
    #[test]
    fn claims_only_its_three_controls() {
        let mut p = VfGuestPolicy::new(abi());
        for cmd in [0x2080_220c, 0x2080_2209, 0x2080_205b, 0x2080_0301] {
            assert!(p.respond(&control(cmd, &[0; 4], 4, 0)).is_none());
        }
        let mut alloc = control(GET_RC_RECOVERY, &[0; 4], 4, 0);
        alloc.function = RpcFunction::RmAlloc;
        assert!(p.respond(&alloc).is_none());
    }
}
