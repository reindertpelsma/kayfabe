//! ★ Windows Code43 batch, 2026-10-07 — **subdevice controls answered per VM and never
//! forwarded**: RC-recovery policy as a STUB under owner ruling §S (`docs/OWNER_RULINGS.md`,
//! 2026-10-07), and the power source the way NVIDIA's own virtual-GPU guest RM answers it.
//!
//! ⊘ *Superseded 2026-10-07 (owner ruling §S, after DIAGNOSTIC run26):* the table row and the
//! "RC recovery" bullet below describe the earlier DISABLED-only answer. RC recovery is now a
//! stub: GET reports the VM's recorded setting, which starts ENABLED (what the passthrough GSP
//! answered in vfio-8/9/10); SET records ENABLED or DISABLED; any other value is 0x1f. No
//! robust-channel recovery is performed for the VM by this link and nothing reaches the host.
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

/// ★ 2026-10-07 (audit `docs/audits/2026-10-07-code43-gr-derive-compliance.md` S4/S5): the three
/// controls' ids, params sizes and values at the GUEST's driver version, read from the driver
/// matrix (`kf_abi::generated::matrix`, measured per ogkm tag by `tools/drivermatrix`) instead of
/// typed by hand. The status word's offset is the version's `rm_control_wire().status_off`.
/// `[matrix]` `PERF_GET_POWERSTATE` and its params are absent at 535.309.01 and 545.23.08; there
/// the link does not claim it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfGuestIds {
    /// `NV2080_CTRL_CMD_SET_RC_RECOVERY`.
    pub set_rc_recovery: u32,
    /// `NV2080_CTRL_CMD_GET_RC_RECOVERY`.
    pub get_rc_recovery: u32,
    /// `sizeof(NV2080_CTRL_CMD_RC_RECOVERY_PARAMS)`.
    pub rc_params_size: usize,
    /// `NV2080_CTRL_CMD_RC_RECOVERY_DISABLED`.
    pub rc_disabled: u32,
    /// `NV2080_CTRL_CMD_RC_RECOVERY_ENABLED`.
    pub rc_enabled: u32,
    /// `NV2080_CTRL_CMD_PERF_GET_POWERSTATE` with `sizeof(NV2080_CTRL_PERF_GET_POWERSTATE_PARAMS)`
    /// and `NV2080_CTRL_PERF_POWER_SOURCE_AC`, where the version has all three.
    pub powerstate: Option<(u32, usize, u32)>,
}

impl VfGuestIds {
    /// The ids at `version`; `None` where the RC-recovery pair is not measured there.
    #[must_use]
    pub fn at(version: kf_abi::DriverVersion) -> Option<VfGuestIds> {
        use kf_abi::generated::matrix as m;
        use kf_abi::matrix::Resolved;
        let id = |r: &'static kf_abi::matrix::ValueRuns| r.at_u32(version).ok().flatten();
        let powerstate = (|| {
            let size = Resolved::of(&m::NV2080_CTRL_PERF_GET_POWERSTATE_PARAMS, version)
                .ok()?
                .size();
            Some((
                id(&m::CTRL_CMDS_NV2080_CTRL_CMD_PERF_GET_POWERSTATE)?,
                size,
                id(&m::CTRL_VALUES_NV2080_CTRL_PERF_POWER_SOURCE_AC)?,
            ))
        })();
        Some(VfGuestIds {
            set_rc_recovery: id(&m::CTRL_CMDS_NV2080_CTRL_CMD_SET_RC_RECOVERY)?,
            get_rc_recovery: id(&m::CTRL_CMDS_NV2080_CTRL_CMD_GET_RC_RECOVERY)?,
            rc_params_size: Resolved::of(&m::NV2080_CTRL_CMD_RC_RECOVERY_PARAMS, version)
                .ok()?
                .size(),
            rc_disabled: id(&m::CTRL_CMDS_NV2080_CTRL_CMD_RC_RECOVERY_DISABLED)?,
            rc_enabled: id(&m::CTRL_CMDS_NV2080_CTRL_CMD_RC_RECOVERY_ENABLED)?,
            powerstate,
        })
    }
}

/// The VM's RC-recovery setting. Only one value exists today; the type is the place where an
/// implemented `Enabled` would go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcRecovery {
    /// No robust-channel recovery is performed on the VM's behalf.
    Disabled,
    /// ⚠ STUB under owner ruling §S (2026-10-07): the guest's recorded policy is ENABLED. kayfabe
    /// performs no per-VM robust-channel recovery on its account and touches no host policy; the
    /// setting is privileged host management with no compute or display effect on the guest.
    Enabled,
}

impl RcRecovery {
    fn wire(self, ids: &VfGuestIds) -> u32 {
        match self {
            RcRecovery::Disabled => ids.rc_disabled,
            RcRecovery::Enabled => ids.rc_enabled,
        }
    }
}

/// ★ The link. Seated with the answering links; claims only the three controls above.
#[derive(Debug)]
pub struct VfGuestPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    /// The controls at the guest's version (`None`: unmeasured there, nothing is claimed).
    ids: Option<VfGuestIds>,
    rc: RcRecovery,
    /// `SET_RC_RECOVERY` accepted.
    pub rc_sets: u64,
    /// `SET_RC_RECOVERY` refused (ENABLED, an unknown value, or a malformed envelope).
    pub rc_refused: u64,
}

impl VfGuestPolicy {
    /// A fresh per-VM link: RC recovery recorded `ENABLED` (stub, owner ruling §S).
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> VfGuestPolicy {
        VfGuestPolicy {
            driver,
            ids: VfGuestIds::at(driver.driver_version()),
            rc: RcRecovery::Enabled,
            rc_sets: 0,
            rc_refused: 0,
        }
    }

    /// The VM's current RC-recovery setting.
    #[must_use]
    pub fn rc_recovery(&self) -> RcRecovery {
        self.rc
    }

    /// Whether `cmd` is one of this link's controls at the guest's version.
    #[must_use]
    pub fn claims(&self, cmd: u32) -> bool {
        self.ids.is_some_and(|i| {
            cmd == i.set_rc_recovery
                || cmd == i.get_rc_recovery
                || i.powerstate.is_some_and(|(c, _, _)| c == cmd)
        })
    }

    /// ★ The answer to one control's params (a pure function of the VM's state, testable without
    /// an envelope): `Ok(reply params)` or `Err(NV status)`.
    ///
    /// # Errors
    /// `NV_ERR_INVALID_ARGUMENT` for a params size other than four bytes or an `rcEnable` that is
    /// neither value; `NV_ERR_NOT_SUPPORTED` for any other control.
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Result<Vec<u8>, u32> {
        let Some(ids) = self.ids.filter(|_| self.claims(cmd)) else {
            return Err(NV_ERR_NOT_SUPPORTED);
        };
        // ⚠ STUB (owner ruling §S, power is host-owned — the AC reading is ASSUMED from that
        // ruling, owner to confirm; `docs/OWNER_RULINGS.md` §S): the vGPU-guest body's constant
        // (`kern_perf_ctrl.c:293-308`), never a host reading.
        if let Some((c, size, ac)) = ids.powerstate
            && cmd == c
        {
            return if params.len() == size {
                let mut out = vec![0u8; size];
                out[..4].copy_from_slice(&ac.to_le_bytes());
                Ok(out)
            } else {
                Err(NV_ERR_INVALID_ARGUMENT)
            };
        }
        let word = match <[u8; 4]>::try_from(params) {
            Ok(w) if params.len() == ids.rc_params_size => w,
            _ => {
                if cmd == ids.set_rc_recovery {
                    self.rc_refused += 1;
                }
                return Err(NV_ERR_INVALID_ARGUMENT);
            }
        };
        if cmd == ids.get_rc_recovery {
            // `[OUT]` only: the guest's input word is not read (vfio-10 sends stack garbage).
            return Ok(self.rc.wire(&ids).to_le_bytes().to_vec());
        }
        let v = u32::from_le_bytes(word);
        if v == ids.rc_disabled {
            self.rc = RcRecovery::Disabled;
        } else if v == ids.rc_enabled {
            // ⚠ STUB (owner ruling §S): recorded, no host action, no recovery promised.
            self.rc = RcRecovery::Enabled;
        } else {
            self.rc_refused += 1;
            return Err(NV_ERR_INVALID_ARGUMENT);
        }
        self.rc_sets += 1;
        Ok(word.to_vec())
    }
}

impl CommandPolicy for VfGuestPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if !self.claims(req.cmd) {
            return None;
        }
        let set_rc = self.ids.map(|i| i.set_rc_recovery);
        let refuse = |status: u32| {
            Some(Reply {
                rpc_result: status,
                body: Vec::new(),
            })
        };
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            if Some(req.cmd) == set_rc {
                self.rc_refused += 1;
            }
            return refuse(NV_ERR_NOT_SUPPORTED);
        }
        let Some(params) = req
            .params_at
            .checked_add(req.params_size as usize)
            .and_then(|end| cmd.payload.get(req.params_at..end))
        else {
            if Some(req.cmd) == set_rc {
                self.rc_refused += 1;
            }
            return refuse(NV_ERR_INVALID_ARGUMENT);
        };
        match self.answer(req.cmd, params) {
            Ok(p) => {
                let mut body = cmd.payload.clone();
                let st = self.driver.rm_control_wire().status_off;
                if let Some(w) = body.get_mut(st..st + 4) {
                    w.copy_from_slice(&NV_OK.to_le_bytes());
                }
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

    fn ids() -> VfGuestIds {
        VfGuestIds::at(kf_abi::versions::BENCH_DRIVER).expect("measured at the bench")
    }

    /// ★ The generated ids are the SDK's (`ctrl2080rc.h:251-269`, `ctrl2080perf.h:143-148,
    /// 759-765` at ogkm-580.159.04) at both audited guest contracts, and the power-state control
    /// is absent where the matrix says so.
    #[test]
    fn the_ids_are_the_generated_sdk_values() {
        let v580_65_06 = kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        };
        for v in [v580_65_06, kf_abi::versions::BENCH_DRIVER] {
            assert_eq!(
                VfGuestIds::at(v),
                Some(VfGuestIds {
                    set_rc_recovery: 0x2080_220d,
                    get_rc_recovery: 0x2080_220e,
                    rc_params_size: 4,
                    rc_disabled: 0,
                    rc_enabled: 1,
                    powerstate: Some((0x2080_205a, 4, 0)),
                }),
                "{v}"
            );
        }
        let v545 = kf_abi::DriverVersion {
            major: 545,
            minor: 23,
            patch: 8,
        };
        assert_eq!(VfGuestIds::at(v545).map(|i| i.powerstate), Some(None));
        assert_eq!(abi().rm_control_wire().status_off, 12);
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
        assert_eq!(
            &r.body[abi().rm_control_wire().status_off..abi().rm_control_wire().status_off + 4],
            &[0; 4]
        );
        Ok(u32::from_le_bytes(r.body[at..at + 4].try_into().unwrap()))
    }

    /// ★ The VFIO sequence under the stub (owner ruling §S): GET reports ENABLED whatever the
    /// guest's uninitialised input word was (vfio-10 sent 0x4da066c8), SET(ENABLED) is recorded,
    /// SET(DISABLED) is recorded and read back, and the power source is AC.
    #[test]
    fn rc_recovery_stub_records_both_values_and_power_is_ac() {
        let mut p = VfGuestPolicy::new(abi());
        let garbage = 0x4da0_66c8u32.to_le_bytes();
        assert_eq!(
            params_of(&mut p, &control(ids().get_rc_recovery, &garbage, 4, 0)),
            Ok(ids().rc_enabled)
        );
        assert_eq!(
            params_of(
                &mut p,
                &control(ids().set_rc_recovery, &1u32.to_le_bytes(), 4, 0)
            ),
            Ok(ids().rc_enabled)
        );
        assert_eq!(
            params_of(
                &mut p,
                &control(ids().set_rc_recovery, &0u32.to_le_bytes(), 4, 0)
            ),
            Ok(ids().rc_disabled)
        );
        assert_eq!(
            params_of(&mut p, &control(ids().get_rc_recovery, &[0xff; 4], 4, 0)),
            Ok(ids().rc_disabled)
        );
        assert_eq!(p.rc_recovery(), RcRecovery::Disabled);
        assert_eq!(p.rc_sets, 2);
        assert_eq!(
            params_of(
                &mut p,
                &control(ids().powerstate.unwrap().0, &[0xff; 4], 4, 0)
            ),
            Ok(ids().powerstate.unwrap().2)
        );
    }

    /// ⊘ Values other than ENABLED/DISABLED are INVALID_ARGUMENT and change nothing.
    #[test]
    fn unknown_values_are_refused_and_change_nothing() {
        let mut p = VfGuestPolicy::new(abi());
        for v in [2u32, 0x8000_0000, u32::MAX] {
            assert_eq!(
                params_of(
                    &mut p,
                    &control(ids().set_rc_recovery, &v.to_le_bytes(), 4, 0)
                ),
                Err(NV_ERR_INVALID_ARGUMENT)
            );
        }
        assert_eq!((p.rc_refused, p.rc_sets), (3, 0));
        assert_eq!(
            params_of(&mut p, &control(ids().get_rc_recovery, &[0; 4], 4, 0)),
            Ok(ids().rc_enabled)
        );
    }

    /// ⊘ Hostile envelopes: a wrong `paramsSize` (short, long, zero, huge), a declared window past
    /// the payload, and a FINN-serialized envelope are all refused; none panics.
    #[test]
    fn hostile_envelopes_are_refused_without_decoding() {
        let mut p = VfGuestPolicy::new(abi());
        for cmd in [
            ids().get_rc_recovery,
            ids().set_rc_recovery,
            ids().powerstate.unwrap().0,
        ] {
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
        let mut short = control(ids().get_rc_recovery, &[0; 4], 4, 0);
        short.payload.truncate(10);
        assert!(p.respond(&short).is_none());
        assert_eq!(p.rc_recovery(), RcRecovery::Enabled);
    }

    /// The link claims exactly its three controls and nothing of another function.
    #[test]
    fn claims_only_its_three_controls() {
        let mut p = VfGuestPolicy::new(abi());
        for cmd in [0x2080_220c, 0x2080_2209, 0x2080_205b, 0x2080_0301] {
            assert!(p.respond(&control(cmd, &[0; 4], 4, 0)).is_none());
        }
        let mut alloc = control(ids().get_rc_recovery, &[0; 4], 4, 0);
        alloc.function = RpcFunction::RmAlloc;
        assert!(p.respond(&alloc).is_none());
    }
}
