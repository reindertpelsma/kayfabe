//! ★ v3-display — the physical-RM side of the display plane (`docs/design/V3_DISPLAY.md` §4).
//!
//! The guest's CPU-RM (KernelDisplay) and, above it, NVKMS drive a display engine that exists only
//! in kayfabe. This link answers the physical-RM display controls the guest routes to "GSP" (us),
//! from the chip's display row ([`kf_chip::display`]) and the virtual monitor ([`kf_disp::edid`]).
//! It forwards nothing to the host: the host's display engine is never touched.
//!
//! Seated ahead of the object seat and the ledger ([`crate::served_chain`]) only when the device
//! was realized with the display plane on (`display=on`); otherwise `GET_IP_VERSION` stays refused
//! and the guest's display engine is amputated (`sweep.rs`, the displayless posture of
//! `V3_HEADLESS_GRAPHICS.md` §3).
//!
//! ## M0: the controls KernelDisplay needs to come up [E]
//!
//! | control | where the guest asks | what we answer |
//! |---|---|---|
//! | `INTERNAL_DISPLAY_GET_IP_VERSION` `0x20800a4b` | `kdispStatePreInitLocked` (`kern_disp.c:324-350`) | the row's `DISPvXXYY` |
//! | `INTERNAL_DISPLAY_GET_STATIC_INFO` `0x20800a01` | `kdispStateInitLocked` (`:442-535`) — failure is fatal | our heads, windows and channel count |
//! | `INTERNAL_INIT_BRIGHTC_STATE_LOAD` `0x20800ac6` | `kdispInitBrightcStateLoad` (`:358-400`) — a non-OK status is fatal | `NV_OK`, the guest's own `status` field left as it sent it (no backlight) |
//! | `INTERNAL_SET_STATIC_EDID_DATA` `0x20800adf` | `kdispSetupAcpiEdid` (`:402-440`) — non-OK is fatal | `NV_OK`; the ACPI EDIDs of a laptop panel have no meaning here |
//! | `INTERNAL_DISPLAY_WRITE_INST_MEM` `0x20800a49` | `instmemStateInitLocked` | `NV_OK`; the instance memory's place is recorded (ctxdma lookup, M2) |

use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NVOS54`'s status word inside the control envelope (as `inittables`, `zbc`).
const CONTROL_STATUS_OFF: usize = 12;

/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO`.
pub const GET_STATIC_INFO: u32 = 0x2080_0a01;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM`.
pub const WRITE_INST_MEM: u32 = 0x2080_0a49;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`.
pub const GET_IP_VERSION: u32 = 0x2080_0a4b;
/// `NV2080_CTRL_CMD_INTERNAL_INIT_BRIGHTC_STATE_LOAD`.
pub const INIT_BRIGHTC_STATE_LOAD: u32 = 0x2080_0ac6;
/// `NV2080_CTRL_CMD_INTERNAL_SET_STATIC_EDID_DATA`.
pub const SET_STATIC_EDID_DATA: u32 = 0x2080_0adf;

/// `sizeof(NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS)` — `feHwSysCap windowPresentMask
/// bFbRemapperEnabled(+pad) numHeads i2cPort internalDispActiveMask embeddedDisplayPortMask
/// bExternalMuxSupported bInternalMuxSupported(+pad) numDispChannels` (`ctrl2080internal.h:71-82`).
pub const STATIC_INFO_SIZE: usize = 36;
/// `sizeof(NV2080_CTRL_INTERNAL_DISPLAY_WRITE_INST_MEM_PARAMS)` — `instMemPhysAddr instMemSize
/// instMemAddrSpace instMemCpuCacheAttr` (`ctrl2080internal.h:891-896`).
pub const WRITE_INST_MEM_SIZE: usize = 24;
/// `NV402C_CTRL_NUM_I2C_PORTS` — "no external daughterboard" (`kern_disp.c:504-511`).
pub const NO_I2C_PORT: u32 = 16;
/// Display channel numbers run core `0`, windows `1..=32`, window-immediates `33..=64`, cursors
/// `73..=80` (`published/disp/v03_00/dev_disp.h`: `NV_PDISP_CHN_NUM_*`); `clientChannelTable` is
/// indexed by it (`kern_disp.c:476-486`), so the count is one past the last cursor.
pub const NUM_DISP_CHANNELS: u32 = 81;

/// ★ Is `class` one of the display objects the guest's kernel allocates and RPCs to us — on any
/// family (the ids are unique across families; the capability table refuses a family's classes
/// to another family's guest before this is asked).
#[must_use]
pub fn is_display_class(class: u32) -> bool {
    class == kf_chip::display::ALL[0].classes.common
        || kf_chip::display::ALL.iter().any(|r| {
            let c = &r.classes;
            [c.display, c.core, c.window, c.window_imm, c.cursor, c.disp_sw].contains(&class)
        })
}

/// Where the guest put display instance memory (`WRITE_INST_MEM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstMem {
    /// Physical address (FB offset or guest physical address, per `addr_space`).
    pub phys: u64,
    /// Size in bytes.
    pub size: u64,
    /// `ADDR_FBMEM` (2) or `ADDR_SYSMEM` (1).
    pub addr_space: u32,
}

/// ★ The link.
pub struct DisplayPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    row: &'static kf_chip::display::DisplayRow,
    /// The guest's instance memory, once stated.
    pub inst_mem: Option<InstMem>,
    /// Every display control this link answered, in order (bounded: the first 256).
    pub seen: Vec<u32>,
}

impl DisplayPolicy {
    /// The link for a chip's display row.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable, row: &'static kf_chip::display::DisplayRow) -> DisplayPolicy {
        DisplayPolicy { driver, row, inst_mem: None, seen: Vec::new() }
    }

    /// ★ `GET_STATIC_INFO`'s reply for our virtual display.
    #[must_use]
    pub fn static_info(&self) -> [u8; STATIC_INFO_SIZE] {
        let mut p = [0u8; STATIC_INFO_SIZE];
        let heads = self.row.heads.min(8);
        let fe_hw_sys_cap = (1u32 << heads) - 1; // HEAD_EXISTS(i) = bit i (`dev_disp.h` v03_00)
        let windows = self.row.windows.min(32);
        let window_mask = if windows == 32 { u32::MAX } else { (1u32 << windows) - 1 };
        put(&mut p, 0, fe_hw_sys_cap);
        put(&mut p, 4, window_mask);
        // bFbRemapperEnabled @8 = 0
        put(&mut p, 12, heads);
        put(&mut p, 16, NO_I2C_PORT);
        // internalDispActiveMask @20 = 0, embeddedDisplayPortMask @24 = 0, muxes @28/@29 = 0
        put(&mut p, 32, NUM_DISP_CHANNELS);
        p
    }

    /// ★ The answer to one display control's params: `Ok(reply params)` or `Err(NV status)`;
    /// `None` when the control is not this link's.
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Option<Result<Vec<u8>, u32>> {
        let r = match cmd {
            GET_IP_VERSION => {
                if params.len() != 4 {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                Ok(self.row.ip_version.to_le_bytes().to_vec())
            }
            GET_STATIC_INFO => {
                if params.len() != STATIC_INFO_SIZE {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                Ok(self.static_info().to_vec())
            }
            // [IN] from the guest (a laptop's ACPI backlight / panel EDIDs); nothing to do
            INIT_BRIGHTC_STATE_LOAD | SET_STATIC_EDID_DATA => Ok(params.to_vec()),
            WRITE_INST_MEM => {
                if params.len() != WRITE_INST_MEM_SIZE {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                let q = |o: usize| u64::from_le_bytes(params[o..o + 8].try_into().unwrap_or([0; 8]));
                let d = |o: usize| u32::from_le_bytes(params[o..o + 4].try_into().unwrap_or([0; 4]));
                self.inst_mem = Some(InstMem { phys: q(0), size: q(8), addr_space: d(16) });
                Ok(params.to_vec())
            }
            _ => return None,
        };
        if self.seen.len() < 256 {
            self.seen.push(cmd);
        }
        Some(r)
    }
}

fn put(p: &mut [u8], off: usize, v: u32) {
    p[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

impl CommandPolicy for DisplayPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let refuse = |status: u32| Some(Reply { rpc_result: status, body: Vec::new() });
        let params = cmd.payload.get(req.params_at..req.params_at + req.params_size as usize);
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            // ⊘ No display control of M0 is FINN-serialized by the guest; one that is would be a
            // layout this link has not measured — refused by name rather than decoded blind.
            return match req.cmd {
                GET_IP_VERSION | GET_STATIC_INFO | INIT_BRIGHTC_STATE_LOAD | SET_STATIC_EDID_DATA | WRITE_INST_MEM => {
                    refuse(NV_ERR_NOT_SUPPORTED)
                }
                _ => None,
            };
        }
        let Some(params) = params else {
            return match req.cmd {
                GET_IP_VERSION | GET_STATIC_INFO | INIT_BRIGHTC_STATE_LOAD | SET_STATIC_EDID_DATA | WRITE_INST_MEM => {
                    refuse(NV_ERR_INVALID_ARGUMENT)
                }
                _ => None,
            };
        };
        match self.answer(req.cmd, params)? {
            Ok(p) => {
                let mut body = cmd.payload.clone();
                body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].copy_from_slice(&NV_OK.to_le_bytes());
                body[req.params_at..req.params_at + p.len()].copy_from_slice(&p);
                Some(Reply { rpc_result: NV_OK, body })
            }
            Err(st) => refuse(st),
        }
    }
}

kf_util::assert_send_sync!(DisplayPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> DisplayPolicy {
        let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench");
        DisplayPolicy::new(abi, &kf_chip::display::AMPERE)
    }

    /// ★ GA10x: the IP version a real GA106 reports, and a static info with four heads, eight
    /// windows and a channel table large enough for every channel number the guest computes.
    #[test]
    fn ga10x_reports_the_real_ip_version_and_a_four_head_static_info() {
        let mut p = policy();
        assert_eq!(p.answer(GET_IP_VERSION, &[0; 4]), Some(Ok(vec![0x00, 0x00, 0x01, 0x04])));
        let si = p.answer(GET_STATIC_INFO, &[0; STATIC_INFO_SIZE]).expect("ours").expect("ok");
        let w = |o: usize| u32::from_le_bytes(si[o..o + 4].try_into().unwrap());
        assert_eq!(w(0), 0xf, "HEAD_EXISTS 0..3");
        assert_eq!(w(4), 0xff, "windows 0..7");
        assert_eq!(w(12), 4);
        assert_eq!(w(16), NO_I2C_PORT);
        assert_eq!(w(32), NUM_DISP_CHANNELS);
        assert!(NUM_DISP_CHANNELS > 73 + 7, "the last cursor channel number fits");
    }

    /// Instance memory is recorded; the fatal-if-refused [IN] controls answer OK; malformed sizes
    /// are refused; foreign controls are not ours.
    #[test]
    fn init_controls_answer_and_inst_mem_is_recorded() {
        let mut p = policy();
        let mut im = [0u8; WRITE_INST_MEM_SIZE];
        im[0..8].copy_from_slice(&0x1234_5000u64.to_le_bytes());
        im[8..16].copy_from_slice(&0x1_0000u64.to_le_bytes());
        im[16..20].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(p.answer(WRITE_INST_MEM, &im), Some(Ok(_))));
        assert_eq!(p.inst_mem, Some(InstMem { phys: 0x1234_5000, size: 0x1_0000, addr_space: 2 }));
        assert!(matches!(p.answer(INIT_BRIGHTC_STATE_LOAD, &[0u8; 4104]), Some(Ok(_))));
        assert!(matches!(p.answer(SET_STATIC_EDID_DATA, &[0u8; 8388]), Some(Ok(_))));
        assert_eq!(p.answer(GET_STATIC_INFO, &[0; 8]), Some(Err(NV_ERR_INVALID_ARGUMENT)));
        assert_eq!(p.answer(0x2080_0101, &[0; 4]), None);
    }
}
