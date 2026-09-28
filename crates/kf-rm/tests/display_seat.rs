//! ★ v3-display step (1) (`docs/design/V3_DISPLAY.md`, the stop note's next step (1)): the display
//! seat through the WHOLE served chain (census → sticky guard → links → ledger).
//!
//! What this file pins:
//! - **default off** (`ObjectLinks::display = None`, the device default): every control the display
//!   model claims reaches the unserviced ledger and none is answered `NV_OK` — the displayless
//!   posture is untouched by step (1);
//! - **on** (`Some(row)`): the M0 controls and the NVKMS bring-up controls are answered (none reaches
//!   the ledger), a FINN-serialized claimed control is refused by name before the ledger;
//! - **on, everything else**: a control the display link does not claim, an alloc of a non-display
//!   class and a free get the SAME reply, byte for byte, with the display seat on as with it off,
//!   and reach the ledger the same number of times — the link declines all of it.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `RMAPI_RPC_FLAGS_SERIALIZED`.
const SERIALIZED: u32 = 1 << 1;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

fn control(cmd: u32, flags: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0001u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0xcafe_0073u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[20..24].copy_from_slice(&flags.to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand { function: RpcFunction::RmControl, code: 0x4c, sequence: 7, payload, elements: 1, delivered: Vec::new() }
}

fn chain(display: bool, log: &kf_rm::unserviced::UnservicedLog) -> Box<dyn CommandPolicy> {
    let links = kf_rm::ObjectLinks {
        objects: None,
        memory: None,
        channels: None,
        display: display.then_some(&kf_chip::display::AMPERE),
    };
    kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs { unserviced: log.clone(), ..Default::default() },
        kf_rm::census::ControlCensusLog::new(),
        links,
    )
}

/// Every control the display model claims, with a request of the derived size (4 bytes for the
/// IP version, the guest's own size for the two [IN] blobs).
fn claimed() -> Vec<(u32, Vec<u8>)> {
    let m = kf_rm::display::model_for(&driver(), &kf_chip::display::AMPERE).expect("derived");
    let l = m.layouts();
    let size_of = |cmd: u32| -> usize {
        match cmd {
            kf_rm::display::GET_IP_VERSION => 4,
            kf_rm::display::INIT_BRIGHTC_STATE_LOAD => 4104,
            kf_rm::display::SET_STATIC_EDID_DATA => 8388,
            _ => 64,
        }
    };
    let mut v: Vec<(u32, Vec<u8>)> = m.claimed().into_iter().map(|c| (c, vec![0u8; size_of(c)])).collect();
    // the derived sizes for the struct-shaped ones
    for (c, p) in &mut v {
        let s = match *c {
            kf_rm::display::GET_STATIC_INFO => Some("NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS"),
            kf_rm::display::WRITE_INST_MEM => Some("NV2080_CTRL_INTERNAL_DISPLAY_WRITE_INST_MEM_PARAMS"),
            x if x == kf_disp::model::CHANNEL_PUSHBUFFER => Some("NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS"),
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS") => Some("NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS"),
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2") => Some("NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS"),
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED") => Some("NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS"),
            _ => None,
        };
        if let Some(s) = s {
            *p = vec![0u8; l.size(s).expect(s)];
        }
    }
    v
}

/// ★ Default off: the display model's whole claim set reaches the ledger, and nothing answers any
/// of it `NV_OK` — `GET_IP_VERSION` refused is the displayless posture.
#[test]
fn default_off_every_claimed_control_still_reaches_the_ledger() {
    let set = claimed();
    assert_eq!(set.len(), 30 + 6, "the NVKMS bring-up set and the six internal controls");
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut c = chain(false, &log);
    for (cmd, p) in &set {
        let r = c.respond(&control(*cmd, 0, p));
        assert!(r.as_ref().is_none_or(|r| r.rpc_result != NV_OK), "{cmd:#010x} answered OK with the display off: {r:?}");
    }
    assert_eq!(log.total(), set.len() as u64, "each reached the ledger");
}

/// ★ On: the claimed set is answered by the display link (the ledger sees none of it), and a
/// FINN-serialized claimed control is refused by name, before the ledger.
#[test]
fn on_the_claimed_set_is_answered_and_the_finn_form_is_refused() {
    let set = claimed();
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut c = chain(true, &log);
    let mut ok = Vec::new();
    for (cmd, p) in &set {
        let r = c.respond(&control(*cmd, 0, p)).unwrap_or_else(|| panic!("{cmd:#010x} not answered"));
        if r.rpc_result == NV_OK {
            ok.push(*cmd);
        }
    }
    assert_eq!(log.total(), 0, "nothing claimed reached the ledger");
    // the ones sent at their derived size answer OK (the rest were sent at a wrong size, and are
    // refused `INVALID_ARGUMENT` by the link — still never the ledger's)
    for cmd in [
        kf_rm::display::GET_IP_VERSION,
        kf_rm::display::GET_STATIC_INFO,
        kf_rm::display::WRITE_INST_MEM,
        kf_rm::display::INIT_BRIGHTC_STATE_LOAD,
        kf_rm::display::SET_STATIC_EDID_DATA,
        0x0073_0101, // SYSTEM_GET_CAPS_V2: m0a's first refusal
        0x0073_0102, // SYSTEM_GET_NUM_HEADS
        0x0073_0107, // SYSTEM_GET_SUPPORTED
        0x0073_0151, // SYSTEM_MAP_SHARED_DATA
    ] {
        assert!(ok.contains(&cmd), "{cmd:#010x} answered OK");
    }
    let r = c.respond(&control(kf_rm::display::GET_IP_VERSION, 0, &[0; 4])).expect("answered");
    assert_eq!((r.rpc_result, &r.body[PARAMS_AT..]), (NV_OK, &[0x00, 0x00, 0x01, 0x04][..]), "GA10x DISPv0401");
    for (cmd, _) in &set {
        let r = c.respond(&control(*cmd, SERIALIZED, &[0; 16])).expect("refused by name");
        assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED, "{cmd:#010x}");
    }
    assert_eq!(log.total(), 0);
}

/// ★ On, everything else: the display link declines it all, so the chain's reply is the same with
/// the seat on as off — a control it does not claim, an alloc of a non-display class, a free.
#[test]
fn on_everything_else_is_answered_exactly_as_off() {
    let alloc = {
        let mut b = vec![0u8; 32];
        for (i, v) in [0xc1d0_0001u32, 0xcafe_0001, 0xcafe_00c0, 0xc0b5, 0, 8].iter().enumerate() {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0; 8]);
        RpcCommand { function: RpcFunction::RmAlloc, code: 0x67, sequence: 8, payload: b, elements: 1, delivered: Vec::new() }
    };
    let free = RpcCommand {
        function: RpcFunction::Free,
        code: 0x0a,
        sequence: 9,
        payload: [0xc1d0_0001u32, 0xcafe_0001, 0xcafe_00c0, 0].iter().flat_map(|v| v.to_le_bytes()).collect(),
        elements: 1,
        delivered: Vec::new(),
    };
    let cmds = [control(0x2080_0101, 0, &[0; 8]), control(0x0073_ffff, 0, &[0; 8]), control(0x2080_0a70, 0, &[]), alloc, free];
    let (log_off, log_on) = (kf_rm::unserviced::UnservicedLog::new(), kf_rm::unserviced::UnservicedLog::new());
    let (mut off, mut on) = (chain(false, &log_off), chain(true, &log_on));
    for cmd in &cmds {
        assert_eq!(on.respond(cmd), off.respond(cmd), "{:?}", cmd.function);
        assert_eq!(on.holds_for_refresh(cmd), off.holds_for_refresh(cmd));
    }
    assert_eq!(log_on.total(), log_off.total());
    assert_eq!(log_on.sample(), log_off.sample());
}
