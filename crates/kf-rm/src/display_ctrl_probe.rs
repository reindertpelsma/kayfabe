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

/// ⚠⚠ **PROBE — default off (`KF3_DISPLAY_PRIVATE_PROBE=1`), never a shipped behaviour.**
///
/// 2026-10-08 (H-commit, `traces/display_reply_diff_20261008/`): inside the modeset commit
/// (`[measured, VFIO DVI reference boots 1-3, RTX 4070 / 580.88, 2026-10-08]`, boot3 12.4946-12.4965 s)
/// Windows sends `GET_HDCP_STATE`, then two NV04_DISPLAY_COMMON controls that ogkm-595.84 does not
/// name, `0x00730122` (8 bytes) and `0x00730128` (20 bytes). The real GSP answers both `NV_OK`;
/// kf3 refuses both `NOT_SUPPORTED` (runs 88-96). This probe answers them with the real GPU's
/// reply, for the one request shape it was measured on — never a guessed layout:
/// - `0x00730122`: request `{0, displayId}`, reply byte-identical to the request (boots 1-3);
///   answered for subdevice 0 and a single-bit display id, echoed.
/// - `0x00730128`: request all-zero, reply words `{0, 1, 0xff, 0, 0}` (boots 1-3, all three calls
///   of each boot); answered for the all-zero request only.
///
/// ⊘ Never: another size, another request shape, a serialized envelope, or anything sent anywhere.
pub const PRIVATE_FLAG: &str = "KF3_DISPLAY_PRIVATE_PROBE";

/// `(command, params size)` — sizes as the 580.88 guest sends them.
pub const PRIVATE: [(u32, usize); 2] = [(0x0073_0122, 8), (0x0073_0128, 20)];

/// The reply words of `0x00730128` the real GPU gave the all-zero request.
const PRIVATE_0128_REPLY: [u32; 5] = [0, 1, 0xff, 0, 0];

/// Whether [`PRIVATE_FLAG`] is on (`=1`).
#[must_use]
pub fn private_enabled() -> bool {
    std::env::var(PRIVATE_FLAG).as_deref() == Ok("1")
}

/// The reply params for one private control, or `None` (not this probe's: refused as before).
#[must_use]
pub fn private_answer(cmd: u32, params: &[u8]) -> Option<Vec<u8>> {
    let &(_, size) = PRIVATE.iter().find(|(c, _)| *c == cmd)?;
    if params.len() != size {
        return None;
    }
    let w = |i: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            params.get(4 * i..4 * i + 4)?.try_into().ok()?,
        ))
    };
    match cmd {
        0x0073_0122 => (w(0)? == 0 && w(1)?.count_ones() == 1).then(|| params.to_vec()),
        0x0073_0128 => params.iter().all(|b| *b == 0).then(|| {
            PRIVATE_0128_REPLY
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect()
        }),
        _ => None,
    }
}

/// The private-control probe link.
#[derive(Debug, Clone)]
pub struct DisplayPrivateProbe {
    driver: kf_abi::versions::DriverAbiTable,
    answered: u64,
}

impl DisplayPrivateProbe {
    /// A link for the guest driver `driver`.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> DisplayPrivateProbe {
        DisplayPrivateProbe {
            driver,
            answered: 0,
        }
    }
}

impl CommandPolicy for DisplayPrivateProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return None;
        }
        let size = req.params_size as usize;
        let params = cmd
            .payload
            .get(req.params_at..req.params_at.checked_add(size)?)?;
        let out = private_answer(req.cmd, params)?;
        let mut body = cmd.payload.clone();
        body.get_mut(req.params_at..req.params_at + size)?
            .copy_from_slice(&out);
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: PROBE {PRIVATE_FLAG} {:#010x} client={:#x} object={:#x}: NV_OK with the real GPU's measured reply — a probe, not a shipped behaviour #{}",
                req.cmd, req.client, req.object, self.answered
            );
        }
        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(DisplayPrivateProbe);

/// ⚠⚠ **PROBE — default off (`KF3_DISPLAY_HDMI_PROBE=1`), never a shipped behaviour.**
///
/// 2026-10-09 (H-hdmi, `traces/display_reply_diff_20261008/` §0 rows P9-P10, P24-P26, P32-P33, P41):
/// `[measured, VFIO DVI reference boot3, RTX 4070, 2026-10-08]` the real GPU presents the monitor's
/// connector `0x100` as an HDMI-A connector driven in DVI mode (`GET_CONNECTOR_DATA` present, index 3,
/// `HDMI_A`, location 3; `DFP_GET_INFO` flags `0x00105300`), and the guest then sends the HDMI path
/// (`SET_HDMI_SINK_CAPS`, `GET_HDMI_GPU_CAPS`, `GET_HDMI_SCDC_DATA`, 28 `DFP_SET_ELD_AUDIO_CAPS`,
/// `SET_HDMI_ENABLE` inside the commit, and SPD-infoframe writes) that kf3's DVI-D connector never
/// sees. This probe answers those controls with the real GPU's reply, for the measured request shape
/// only: the two identity controls for display `0x100` only; the HDMI controls for one display bit
/// of kf3's four (`0x100..=0x800`); `GET_HDMI_SCDC_DATA` with the real GPU's own status `0x14`.
pub const HDMI_FLAG: &str = "KF3_DISPLAY_HDMI_PROBE";

/// ⚠⚠ **PROBE — default off (`KF3_DISPLAY_XBAR_PROBE=1`), never a shipped behaviour.**
///
/// 2026-10-09 (H-xbar, §0 rows P6, P22, P26, P30): `[measured, VFIO DVI reference boot3, RTX 4070,
/// 2026-10-08]` the real GPU answers `SYSTEM_GET_CAPS_V2` with `capsTbl {0x81, 0x2f}` (CROSS_BAR and
/// GLITCHLESS_MODESET among them) and the guest then routes each display through `DFP_ASSIGN_SOR`
/// (8 calls, one inside the commit). This probe answers the caps with the measured bytes, and
/// `DFP_ASSIGN_SOR` with kf3's own fixed topology — display `0x100 << i` on SOR `i`, type SINGLE —
/// which is also what the real GPU answered for `0x100` (SOR 0, SINGLE).
pub const XBAR_FLAG: &str = "KF3_DISPLAY_XBAR_PROBE";

/// Whether [`HDMI_FLAG`] is on (`=1`).
#[must_use]
pub fn hdmi_enabled() -> bool {
    std::env::var(HDMI_FLAG).as_deref() == Ok("1")
}

/// Whether [`XBAR_FLAG`] is on (`=1`).
#[must_use]
pub fn xbar_enabled() -> bool {
    std::env::var(XBAR_FLAG).as_deref() == Ok("1")
}

/// `NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA` (72 bytes).
const GET_CONNECTOR_DATA: u32 = 0x0073_0250;
/// `NV0073_CTRL_CMD_DFP_GET_INFO` (16 bytes).
const DFP_GET_INFO: u32 = 0x0073_1140;
/// `NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_ENABLE` (12 bytes).
const SET_HDMI_ENABLE: u32 = 0x0073_0273;
/// `NV0073_CTRL_CMD_SPECIFIC_SET_HDMI_SINK_CAPS` (12 bytes).
const SET_HDMI_SINK_CAPS: u32 = 0x0073_0293;
/// `NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_GPU_CAPS` (8 bytes).
const GET_HDMI_GPU_CAPS: u32 = 0x0073_02a2;
/// `NV0073_CTRL_CMD_SPECIFIC_GET_HDMI_SCDC_DATA` (12 bytes).
const GET_HDMI_SCDC_DATA: u32 = 0x0073_02a6;
/// `NV0073_CTRL_CMD_DFP_SET_ELD_AUDIO_CAPS` (120 bytes).
const DFP_SET_ELD_AUDIO_CAPS: u32 = 0x0073_1144;
/// `NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2` (2 bytes).
const SYSTEM_GET_CAPS_V2: u32 = 0x0073_0101;
/// `NV0073_CTRL_CMD_DFP_ASSIGN_SOR` (80 bytes).
const DFP_ASSIGN_SOR: u32 = 0x0073_1152;

/// The display whose identity the real GPU's reply describes (and kf3's first connector).
const MEASURED_DISPLAY: u32 = 0x100;
/// `GET_CONNECTOR_DATA(0x100)` reply words 2..=7: flags, (pad), count, index, type, location.
const CONNECTOR_DATA_WORDS: [(usize, u32); 5] = [(2, 1), (4, 1), (5, 3), (6, 0x61), (7, 3)];
/// `DFP_GET_INFO(0x100)` reply `flags`.
const DFP_INFO_FLAGS: u32 = 0x0010_5300;
/// `GET_HDMI_GPU_CAPS` reply word 1 (`caps`).
const HDMI_GPU_CAPS: u32 = 6;
/// The real GPU's status for `GET_HDMI_SCDC_DATA` on the DVI-mode monitor.
const SCDC_STATUS: u32 = 0x14;
/// `SYSTEM_GET_CAPS_V2.capsTbl`.
const CAPS_V2: [u8; 2] = [0x81, 0x2f];
/// `NV0073_CTRL_DFP_SOR_TYPE_SINGLE`.
const SOR_TYPE_SINGLE: u32 = 1;
/// kf3's SORs (one per head, `kf_disp::model`): display `0x100 << i` is on SOR `i`.
const SORS: u32 = 4;

/// The `(status, reply params)` for one control under the H-hdmi / H-xbar probes, or `None` (not
/// this probe's: answered as without it).
#[must_use]
pub fn topology_answer(hdmi: bool, xbar: bool, cmd: u32, params: &[u8]) -> Option<(u32, Vec<u8>)> {
    let w = |i: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            params.get(4 * i..4 * i + 4)?.try_into().ok()?,
        ))
    };
    let set = |out: &mut Vec<u8>, i: usize, v: u32| {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes())
    };
    // one display of kf3's four, subdevice 0
    let one_display = || -> Option<u32> {
        let id = w(1)?;
        (w(0)? == 0 && id.count_ones() == 1 && (0x100..0x100 << SORS).contains(&id)).then_some(id)
    };
    let size = |n: usize| (params.len() == n).then_some(());
    match cmd {
        GET_CONNECTOR_DATA if hdmi => {
            size(72)?;
            // the measured request (RTX 4070, 2026-10-08): subdevice 0, display 0x100, everything else zero
            let ok = w(0)? == 0 && w(1)? == MEASURED_DISPLAY && params[8..].iter().all(|b| *b == 0);
            ok.then(|| {
                let mut out = params.to_vec();
                for (i, v) in CONNECTOR_DATA_WORDS {
                    set(&mut out, i, v);
                }
                (NV_OK, out)
            })
        }
        DFP_GET_INFO if hdmi => {
            size(16)?;
            let ok = w(0)? == 0 && w(1)? == MEASURED_DISPLAY && w(2)? == 0 && w(3)? == 0;
            ok.then(|| {
                let mut out = params.to_vec();
                set(&mut out, 2, DFP_INFO_FLAGS);
                (NV_OK, out)
            })
        }
        SET_HDMI_ENABLE | SET_HDMI_SINK_CAPS if hdmi => {
            size(12)?;
            one_display()?;
            Some((NV_OK, params.to_vec()))
        }
        GET_HDMI_GPU_CAPS if hdmi => {
            size(8)?;
            params.iter().all(|b| *b == 0).then(|| {
                let mut out = params.to_vec();
                set(&mut out, 1, HDMI_GPU_CAPS);
                (NV_OK, out)
            })
        }
        GET_HDMI_SCDC_DATA if hdmi => {
            size(12)?;
            one_display()?;
            Some((SCDC_STATUS, params.to_vec()))
        }
        DFP_SET_ELD_AUDIO_CAPS if hdmi => {
            size(120)?;
            one_display()?;
            Some((NV_OK, params.to_vec()))
        }
        SYSTEM_GET_CAPS_V2 if xbar => {
            size(2)?;
            params
                .iter()
                .all(|b| *b == 0)
                .then(|| (NV_OK, CAPS_V2.to_vec()))
        }
        DFP_ASSIGN_SOR if xbar => {
            size(80)?;
            let id = one_display()?;
            let sor = id.trailing_zeros() - 8;
            // sorExcludeMask (byte 8) must leave the display's own SOR; no slave, no 2Head1Or
            if params[8] & (1 << sor) != 0 || w(3)? != 0 || params[20] != 0 {
                return None;
            }
            let mut out = params.to_vec();
            out[24..80].fill(0);
            let s = sor as usize;
            set(&mut out, 6 + s, id); // sorAssignList[sor]
            set(&mut out, 10 + 2 * s, id); // sorAssignListWithTag[sor].displayMask
            set(&mut out, 11 + 2 * s, SOR_TYPE_SINGLE); // .sorType
            Some((NV_OK, out))
        }
        _ => None,
    }
}

/// The H-hdmi / H-xbar probe link.
#[derive(Debug, Clone)]
pub struct DisplayTopologyProbe {
    driver: kf_abi::versions::DriverAbiTable,
    hdmi: bool,
    xbar: bool,
    answered: u64,
}

impl DisplayTopologyProbe {
    /// A link for the guest driver `driver`, with the halves that are on.
    #[must_use]
    pub fn new(
        driver: kf_abi::versions::DriverAbiTable,
        hdmi: bool,
        xbar: bool,
    ) -> DisplayTopologyProbe {
        DisplayTopologyProbe {
            driver,
            hdmi,
            xbar,
            answered: 0,
        }
    }
}

impl CommandPolicy for DisplayTopologyProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return None;
        }
        let size = req.params_size as usize;
        let params = cmd
            .payload
            .get(req.params_at..req.params_at.checked_add(size)?)?;
        let (status, out) = topology_answer(self.hdmi, self.xbar, req.cmd, params)?;
        let mut body = cmd.payload.clone();
        body.get_mut(req.params_at..req.params_at + size)?
            .copy_from_slice(&out);
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&status.to_le_bytes());
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: PROBE {} {:#010x} client={:#x} object={:#x}: status {status:#x} with the real GPU's measured reply shape — a probe, not a shipped behaviour #{}",
                if self.hdmi && self.xbar {
                    "KF3_DISPLAY_HDMI_PROBE+KF3_DISPLAY_XBAR_PROBE"
                } else if self.hdmi {
                    HDMI_FLAG
                } else {
                    XBAR_FLAG
                },
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

kf_util::assert_send_sync!(DisplayTopologyProbe);

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

    /// ⚠ `KF3_DISPLAY_PRIVATE_PROBE`: the two unnamed commit-time controls get the real GPU's reply
    /// for exactly the measured request shape (VFIO DVI reference, RTX 4070, 2026-10-08); every other shape, size or id is not answered.
    #[test]
    fn the_private_probe_answers_only_the_measured_shapes() {
        let mut p = DisplayPrivateProbe::new(abi());
        let w = abi().rm_control_wire();
        let words = |v: &[u32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
        // 0x00730122 {0, 0x100}: echoed
        let r = p
            .respond(&control(0x0073_0122, &words(&[0, 0x100])))
            .expect("answered");
        assert_eq!(&r.body[w.status_off..w.status_off + 4], &[0; 4]);
        assert_eq!(&r.body[w.params_off..], &words(&[0, 0x100])[..]);
        // 0x00730128 all-zero: the measured reply (RTX 4070, 2026-10-08)
        let r = p
            .respond(&control(0x0073_0128, &[0u8; 20]))
            .expect("answered");
        assert_eq!(&r.body[w.params_off..], &words(&[0, 1, 0xff, 0, 0])[..]);
        // hostile / unmeasured shapes are not answered
        for (c, params) in [
            (0x0073_0122, words(&[1, 0x100])),
            (0x0073_0122, words(&[0, 0x300])),
            (0x0073_0122, words(&[0, 0])),
            (0x0073_0122, words(&[0, 0x100, 0])),
            (0x0073_0128, words(&[0, 0, 0, 0, 1])),
            (0x0073_0128, vec![0u8; 16]),
            (0x0073_0129, vec![0u8; 20]),
        ] {
            assert!(
                p.respond(&control(c, &params)).is_none(),
                "{c:#x} {params:?}"
            );
        }
        assert_eq!(private_answer(0x0073_0122, &[0u8; 3]), None, "short");
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn padded(s: &str, n: usize) -> Vec<u8> {
        let mut v = hex(s);
        v.resize(n, 0);
        v
    }

    /// ⚠ `KF3_DISPLAY_HDMI_PROBE` / `KF3_DISPLAY_XBAR_PROBE`: for the measured requests the reply is
    /// byte-for-byte the real GPU's (VFIO DVI reference boot3, RTX 4070, 2026-10-08, display
    /// `0x100`); off, nothing is answered; hostile or unmeasured shapes are not answered.
    #[test]
    fn the_topology_probes_answer_the_measured_requests_byte_for_byte() {
        let cases: [(u32, Vec<u8>, u32, Vec<u8>); 8] = [
            (
                GET_CONNECTOR_DATA,
                padded("0000000000010000", 72),
                NV_OK,
                padded(
                    "00000000000100000100000000000000010000000300000061000000030000",
                    72,
                ),
            ),
            (
                DFP_GET_INFO,
                hex("00000000000100000000000000000000"),
                NV_OK,
                hex("00000000000100000053100000000000"),
            ),
            (
                SET_HDMI_ENABLE,
                hex("000000000001000000000000"),
                NV_OK,
                hex("000000000001000000000000"),
            ),
            (
                SET_HDMI_SINK_CAPS,
                hex("000000000001000000000000"),
                NV_OK,
                hex("000000000001000000000000"),
            ),
            (
                GET_HDMI_GPU_CAPS,
                hex("0000000000000000"),
                NV_OK,
                hex("0000000006000000"),
            ),
            (
                GET_HDMI_SCDC_DATA,
                hex("000000000001000035000000"),
                0x14,
                hex("000000000001000035000000"),
            ),
            (SYSTEM_GET_CAPS_V2, hex("0000"), NV_OK, hex("812f")),
            (
                DFP_ASSIGN_SOR,
                padded("0000000000010000", 80),
                NV_OK,
                padded(
                    "000000000001000000000000000000000000000000000000000100000000000000000000000000000001000001",
                    80,
                ),
            ),
        ];
        for (cmd, req, st, rep) in &cases {
            assert_eq!(
                topology_answer(true, true, *cmd, req),
                Some((*st, rep.clone())),
                "{cmd:#x}"
            );
            assert_eq!(
                topology_answer(false, false, *cmd, req),
                None,
                "{cmd:#x} off"
            );
        }
        // each half alone answers only its own controls
        assert!(topology_answer(true, false, SYSTEM_GET_CAPS_V2, &[0, 0]).is_none());
        assert!(topology_answer(false, true, DFP_GET_INFO, &cases[1].1).is_none());
        // ELD audio caps: echoed for one of kf3's displays
        let mut eld = vec![0u8; 120];
        eld[4..8].copy_from_slice(&0x100u32.to_le_bytes());
        assert_eq!(
            topology_answer(true, false, DFP_SET_ELD_AUDIO_CAPS, &eld),
            Some((NV_OK, eld.clone()))
        );
        // ASSIGN_SOR for display 0x400: SOR 2
        let mut a = vec![0u8; 80];
        a[4..8].copy_from_slice(&0x400u32.to_le_bytes());
        let (_, r) = topology_answer(false, true, DFP_ASSIGN_SOR, &a).unwrap();
        let w = |i: usize| u32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap());
        assert_eq!((w(8), w(14), w(15)), (0x400, 0x400, SOR_TYPE_SINGLE));
        // hostile / unmeasured shapes
        let word = |i: usize, v: u32, n: usize| {
            let mut p = vec![0u8; n];
            p[4..8].copy_from_slice(&0x100u32.to_le_bytes());
            p[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
            p
        };
        for (cmd, p) in [
            (GET_CONNECTOR_DATA, word(1, 0x200, 72)), // not the measured display
            (GET_CONNECTOR_DATA, word(0, 1, 72)),     // subdevice 1
            (GET_CONNECTOR_DATA, word(4, 1, 72)),     // a non-zero input field
            (GET_CONNECTOR_DATA, vec![0u8; 68]),      // short
            (DFP_GET_INFO, word(1, 0x100 | 0x200, 16)),
            (SET_HDMI_ENABLE, word(1, 0x1000, 12)), // not one of kf3's four displays
            (SET_HDMI_ENABLE, word(1, 0x300, 12)),  // two displays
            (SET_HDMI_ENABLE, word(1, 0x100, 16)),  // wrong size
            (GET_HDMI_GPU_CAPS, word(1, 1, 8)),
            (GET_HDMI_SCDC_DATA, word(1, 0, 12)),
            (DFP_SET_ELD_AUDIO_CAPS, word(1, 0x80, 120)),
            (SYSTEM_GET_CAPS_V2, vec![1, 0]),
            (SYSTEM_GET_CAPS_V2, vec![0; 4]),
            (DFP_ASSIGN_SOR, {
                let mut p = word(1, 0x100, 80);
                p[8] = 1; // excludes SOR 0, the display's own
                p
            }),
            (DFP_ASSIGN_SOR, word(3, 0x200, 80)), // a slave display
            (DFP_ASSIGN_SOR, word(1, 0x10000, 80)),
            (0x0073_1153, word(1, 0x100, 80)),
        ] {
            assert!(
                topology_answer(true, true, cmd, &p).is_none(),
                "{cmd:#x} {p:?}"
            );
        }
        // through the link: status and params land in the reply body
        let mut link = DisplayTopologyProbe::new(abi(), true, true);
        let wire = abi().rm_control_wire();
        let r = link
            .respond(&control(GET_HDMI_SCDC_DATA, &cases[5].1))
            .expect("answered");
        assert_eq!(
            &r.body[wire.status_off..wire.status_off + 4],
            &0x14u32.to_le_bytes()
        );
        assert_eq!(&r.body[wire.params_off..], &cases[5].3[..]);
        assert!(link.respond(&control(0x0073_011d, &[0u8; 272])).is_none());
    }
}
