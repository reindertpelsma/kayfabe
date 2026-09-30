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
#[path = "support/rpcwire.rs"]
mod rpcwire;

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
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn chain(display: bool, log: &kf_rm::unserviced::UnservicedLog) -> Box<dyn CommandPolicy> {
    chain_with_objects(display, log, None)
}

fn chain_with_objects(
    display: bool,
    log: &kf_rm::unserviced::UnservicedLog,
    objects: Option<Box<dyn kf_rm::rmrpc::RmObjects>>,
) -> Box<dyn CommandPolicy> {
    let links = kf_rm::ObjectLinks {
        objects: objects.map(|o| {
            kf_rm::rmrpc::ObjectPolicy::over(
                &driver(),
                kf_abi::GuestOs::Linux,
                o,
                Default::default(),
            )
        }),
        memory: None,
        channels: None,
        display: display.then(|| (&kf_chip::display::AMPERE).into()),
    };
    kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs {
            unserviced: log.clone(),
            ..Default::default()
        },
        kf_rm::census::ControlCensusLog::new(),
        links,
    )
}

const CLIENT: u32 = 0xc1d0_0001;
const DISP: u32 = 0xcafe_0073;
const CORE: u32 = 0xcafe_0d00;

fn rpc(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn alloc(client: u32, parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<u8> = [client, parent, handle, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend_from_slice(params);
    rpc(RpcFunction::RmAlloc, 0x67, payload)
}

fn core_alloc(client: u32, handle: u32) -> RpcCommand {
    let l = kf_disp::layout::for_version("580.159.04").unwrap();
    alloc(
        client,
        DISP,
        handle,
        0xc67d,
        &vec![0; l.size("NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS").unwrap()],
    )
}

fn free(client: u32, object: u32) -> RpcCommand {
    rpc(
        RpcFunction::Free,
        0x0a,
        [client, DISP, object, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
    )
}

fn accept(c: &mut dyn CommandPolicy, cmd: &RpcCommand) {
    assert_eq!(
        c.respond(cmd).expect("object seat answered").rpc_result,
        NV_OK
    );
}

fn root_and_display(c: &mut dyn CommandPolicy) {
    accept(
        c,
        &rpc(
            RpcFunction::RmAlloc,
            0x67,
            rpcwire::client_root_alloc_body(0x41, CLIENT, 1234),
        ),
    );
    accept(c, &alloc(CLIENT, CLIENT, DISP, 0xc670, &[]));
}

fn core_state(c: &mut dyn CommandPolicy) -> u64 {
    let l = kf_disp::layout::for_version("580.159.04").unwrap();
    let s = "NVC370_CTRL_CMD_GET_CHANNEL_INFO_PARAMS";
    let mut p = kf_disp::layout::Params::new(l, s, &vec![0; l.size(s).unwrap()]).unwrap();
    p.set("channelClass", 0xc67d);
    let r = c
        .respond(&control(
            l.k32("NVC370_CTRL_CMD_GET_CHANNEL_INFO").unwrap(),
            0,
            &p.buf,
        ))
        .unwrap();
    assert_eq!(r.rpc_result, NV_OK);
    kf_disp::layout::Params::new(l, s, &r.body[PARAMS_AT..])
        .unwrap()
        .get("channelState")
        .unwrap()
}

fn graph_chain() -> Box<dyn CommandPolicy> {
    chain_with_objects(
        true,
        &Default::default(),
        Some(Box::new(kf_rm::rmrpc::GraphObjects::new(
            kf_chip::Family::Ampere,
        ))),
    )
}

/// A well-shaped but rejected alloc must neither create a phantom channel nor displace
/// the live channel. The final free tests identity, not merely IDLE versus IDLE.
#[test]
fn rejected_allocations_do_not_publish_or_replace_display_channels() {
    let mut c = graph_chain();
    assert_ne!(
        c.respond(&core_alloc(CLIENT, CORE)).unwrap().rpc_result,
        NV_OK,
        "undeclared client"
    );
    assert_eq!(core_state(&mut *c), 0x80, "DEALLOC, not a phantom channel");
    root_and_display(&mut *c);
    accept(&mut *c, &core_alloc(CLIENT, CORE));
    assert_eq!(core_state(&mut *c), 1, "IDLE");
    let other = CORE + 1;
    accept(&mut *c, &alloc(CLIENT, DISP, other, 0xc372, &[]));
    assert_ne!(
        c.respond(&core_alloc(CLIENT, other)).unwrap().rpc_result,
        NV_OK,
        "conflicting class at a live handle"
    );
    assert_ne!(
        c.respond(&core_alloc(CLIENT + 1, CORE + 2))
            .unwrap()
            .rpc_result,
        NV_OK,
        "another undeclared client"
    );
    let mut malformed = core_alloc(CLIENT, CORE + 3);
    malformed.payload[24..28].copy_from_slice(&SERIALIZED.to_le_bytes());
    assert_ne!(c.respond(&malformed).unwrap().rpc_result, NV_OK);
    assert_eq!(core_state(&mut *c), 1);
    accept(&mut *c, &free(CLIENT, CORE));
    assert_eq!(
        core_state(&mut *c),
        0x80,
        "the original channel still owned the registry entry"
    );
}

#[test]
fn no_object_seat_means_no_display_allocation_even_with_display_on() {
    let mut c = chain(true, &Default::default());
    assert!(
        c.respond(&core_alloc(CLIENT, CORE))
            .is_none_or(|r| r.rpc_result != NV_OK)
    );
    assert_eq!(core_state(&mut *c), 0x80);
}

/// RM alloc has no large-RPC path: a short allocation is refused, not held. A later
/// whole request must still be accepted, and its parent's accepted free releases it.
#[test]
fn short_display_allocation_is_refused_without_publishing_or_wedging() {
    let mut c = graph_chain();
    root_and_display(&mut *c);
    let whole = core_alloc(CLIENT, CORE);
    let mut head = whole.clone();
    head.payload.truncate(36);
    assert_ne!(
        c.respond(&head).unwrap().rpc_result,
        NV_OK,
        "short, not a fragment"
    );
    assert_eq!(core_state(&mut *c), 0x80);
    accept(&mut *c, &whole);
    assert_eq!(core_state(&mut *c), 1);
    accept(&mut *c, &free(CLIENT, DISP));
    assert_eq!(
        core_state(&mut *c),
        0x80,
        "accepted parent free releases the channel"
    );
}

#[test]
fn rejected_free_does_not_release_display_and_accepted_client_free_allows_recycle() {
    use kf_rm::{
        rmgraph::RmEvent,
        rmrpc::{GraphObjects, ObjectsRefusal, PageDirStatement, RmObjects},
    };
    struct RejectCoreFree(GraphObjects);
    impl RmObjects for RejectCoreFree {
        fn apply(&mut self, ev: RmEvent, params: &[u8]) -> Result<(), ObjectsRefusal> {
            if matches!(ev, RmEvent::Free { handle, .. } if handle.0 == CORE) {
                return Err(ObjectsRefusal::Host {
                    what: "injected free refusal",
                });
            }
            self.0.apply(ev, params)
        }
        fn page_dir(&mut self, st: PageDirStatement) -> Result<(), ObjectsRefusal> {
            self.0.page_dir(st)
        }
    }
    let mut c = chain_with_objects(
        true,
        &Default::default(),
        Some(Box::new(RejectCoreFree(GraphObjects::new(
            kf_chip::Family::Ampere,
        )))),
    );
    for _ in 0..2 {
        root_and_display(&mut *c);
        accept(&mut *c, &core_alloc(CLIENT, CORE));
        assert_ne!(c.respond(&free(CLIENT, CORE)).unwrap().rpc_result, NV_OK);
        assert_eq!(
            core_state(&mut *c),
            1,
            "refused free left the channel intact"
        );
        accept(&mut *c, &free(CLIENT, CLIENT));
        assert_eq!(core_state(&mut *c), 0x80);
    }
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
    let mut v: Vec<(u32, Vec<u8>)> = m
        .claimed()
        .into_iter()
        .map(|c| (c, vec![0u8; size_of(c)]))
        .collect();
    // the derived sizes for the struct-shaped ones
    for (c, p) in &mut v {
        let s = match *c {
            kf_rm::display::GET_STATIC_INFO => {
                Some("NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS")
            }
            kf_rm::display::WRITE_INST_MEM => {
                Some("NV2080_CTRL_INTERNAL_DISPLAY_WRITE_INST_MEM_PARAMS")
            }
            x if x == kf_disp::model::CHANNEL_PUSHBUFFER => {
                Some("NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS")
            }
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS") => {
                Some("NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS")
            }
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2") => {
                Some("NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS")
            }
            x if Some(x) == l.k32("NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED") => {
                Some("NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS")
            }
            x if Some(x) == l.k32("NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE") => {
                Some("NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE_PARAMS")
            }
            x if Some(x) == l.k32("NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_UNIX_CONSOLE") => {
                Some("NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_UNIX_CONSOLE_PARAMS")
            }
            x if Some(x)
                == l.k32("NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES") =>
            {
                Some("NV2080_CTRL_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES_PARAMS")
            }
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
    assert_eq!(
        set.len(),
        33 + 6,
        "the NVKMS bring-up set (with the console pair and the display-SW object's query) and the six internal controls"
    );
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut c = chain(false, &log);
    for (cmd, p) in &set {
        let r = c.respond(&control(*cmd, 0, p));
        assert!(
            r.as_ref().is_none_or(|r| r.rpc_result != NV_OK),
            "{cmd:#010x} answered OK with the display off: {r:?}"
        );
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
        let r = c
            .respond(&control(*cmd, 0, p))
            .unwrap_or_else(|| panic!("{cmd:#010x} not answered"));
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
    let r = c
        .respond(&control(kf_rm::display::GET_IP_VERSION, 0, &[0; 4]))
        .expect("answered");
    assert_eq!(
        (r.rpc_result, &r.body[PARAMS_AT..]),
        (NV_OK, &[0x00, 0x00, 0x01, 0x04][..]),
        "GA10x DISPv0401"
    );
    for (cmd, _) in &set {
        let r = c
            .respond(&control(*cmd, SERIALIZED, &[0; 16]))
            .expect("refused by name");
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
        for (i, v) in [0xc1d0_0001u32, 0xcafe_0001, 0xcafe_00c0, 0xc0b5, 0, 8]
            .iter()
            .enumerate()
        {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0; 8]);
        RpcCommand {
            function: RpcFunction::RmAlloc,
            code: 0x67,
            sequence: 8,
            payload: b,
            elements: 1,
            delivered: Vec::new(),
        }
    };
    let free = RpcCommand {
        function: RpcFunction::Free,
        code: 0x0a,
        sequence: 9,
        payload: [0xc1d0_0001u32, 0xcafe_0001, 0xcafe_00c0, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
        elements: 1,
        delivered: Vec::new(),
    };
    let cmds = [
        control(0x2080_0101, 0, &[0; 8]),
        control(0x0073_ffff, 0, &[0; 8]),
        control(0x2080_0a70, 0, &[]),
        alloc,
        free,
    ];
    let (log_off, log_on) = (
        kf_rm::unserviced::UnservicedLog::new(),
        kf_rm::unserviced::UnservicedLog::new(),
    );
    let (mut off, mut on) = (chain(false, &log_off), chain(true, &log_on));
    for cmd in &cmds {
        assert_eq!(on.respond(cmd), off.respond(cmd), "{:?}", cmd.function);
        assert_eq!(on.holds_for_refresh(cmd), off.holds_for_refresh(cmd));
    }
    assert_eq!(log_on.total(), log_off.total());
    assert_eq!(log_on.sample(), log_off.sample());
}
