//! ★ Windows Code43 batch, 2026-10-07 (`traces/windows_code43_walls_20261007/README.md`, run25):
//! the RPCs Windows StartDevice sends right after the run24 abort point, through the WHOLE served
//! chain (census → sticky guard → links → object seat → ledger), in VFIO order:
//!
//! 1. `GET_RC_RECOVERY` / `SET_RC_RECOVERY` — answered by `kf_rm::vfguest`, never by the ledger;
//! 2. `NV01_EVENT_KERNEL_CALLBACK` (0x78) under the subdevice and under `NV04_DISPLAY_COMMON` —
//!    accepted by the object seat as edges;
//! 3. `NV0073_CTRL_CMD_EVENT_SET_NOTIFICATION` on the display-common object — answered from its
//!    own bound events, refused for an event that was never bound there;
//! 4. `PERF_GET_POWERSTATE` — AC.

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;
const NV_OK: u32 = 0;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
const NV_ERR_INVALID_STATE: u32 = 0x40;
const CLIENT: u32 = 0xc1d0_0002;
const DEVICE: u32 = 0xff01_0000;
const SUBDEVICE: u32 = 0xff03_0000;
const DISP_COMMON: u32 = 0xff0a_0000;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

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

fn control(object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&CLIENT.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    rpc(RpcFunction::RmControl, 0x4c, payload)
}

fn alloc(parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<u8> = [CLIENT, parent, handle, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend_from_slice(params);
    rpc(RpcFunction::RmAlloc, 0x67, payload)
}

/// `NV0005_ALLOC_PARAMETERS { hParentClient, hSrcResource, hClass, notifyIndex, data }`, with a
/// non-zero `data` the port must never touch.
fn event_params(src: u32, notify: u32) -> Vec<u8> {
    let mut p: Vec<u8> = [0u32, src, 0x78, notify]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    p.extend_from_slice(&0xffff_8000_dead_beefu64.to_le_bytes());
    p
}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn chain(log: &kf_rm::unserviced::UnservicedLog) -> Box<dyn CommandPolicy> {
    let links = kf_rm::ObjectLinks {
        objects: Some(kf_rm::rmrpc::ObjectPolicy::over(
            &driver(),
            kf_abi::GuestOs::Linux,
            Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
            Default::default(),
        )),
        memory: None,
        channels: None,
        display: Some((&kf_chip::display::AMPERE).into()),
        console: None,
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

fn status(c: &mut dyn CommandPolicy, cmd: &RpcCommand) -> u32 {
    c.respond(cmd).expect("answered").rpc_result
}

fn params(c: &mut dyn CommandPolicy, cmd: &RpcCommand) -> Result<Vec<u8>, u32> {
    let r = c.respond(cmd).expect("answered");
    if r.rpc_result != NV_OK {
        return Err(r.rpc_result);
    }
    Ok(r.body[PARAMS_AT..].to_vec())
}

fn objects(c: &mut dyn CommandPolicy) {
    let root = rpc(
        RpcFunction::RmAlloc,
        0x67,
        rpcwire::client_root_alloc_body(0x41, CLIENT, 1234),
    );
    assert_eq!(status(c, &root), NV_OK, "client");
    let dev = rpcwire::device_params(0, 0, 0);
    assert_eq!(
        status(c, &alloc(CLIENT, DEVICE, 0x80, &dev)),
        NV_OK,
        "device"
    );
    assert_eq!(
        status(c, &alloc(DEVICE, SUBDEVICE, 0x2080, &[0; 4])),
        NV_OK,
        "subdevice"
    );
    assert_eq!(
        status(c, &alloc(DEVICE, DISP_COMMON, 0x73, &[])),
        NV_OK,
        "display common"
    );
}

#[test]
fn the_vfio_sequence_after_the_run24_abort_point_is_served() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut c = chain(&log);
    objects(&mut *c);

    // 1. RC recovery (stub, owner ruling §S): GET reports ENABLED whatever the guest's input
    //    word, SET(ENABLED) is recorded (the VFIO sequence), a third value is refused 0x1f.
    assert_eq!(
        params(
            &mut *c,
            &control(SUBDEVICE, 0x2080_220e, &words(&[0x4da0_66c8]))
        ),
        Ok(words(&[1]))
    );
    assert_eq!(
        params(&mut *c, &control(SUBDEVICE, 0x2080_220d, &words(&[1]))),
        Ok(words(&[1]))
    );
    assert_eq!(
        params(&mut *c, &control(SUBDEVICE, 0x2080_220d, &words(&[2]))),
        Err(NV_ERR_INVALID_ARGUMENT)
    );

    // 2. Event objects under the subdevice and the display-common object.
    assert_eq!(
        status(
            &mut *c,
            &alloc(SUBDEVICE, 0xff06_0040, 0x78, &event_params(SUBDEVICE, 0x2c))
        ),
        NV_OK,
        "0x78 under the subdevice"
    );
    assert_eq!(
        status(
            &mut *c,
            &alloc(
                DISP_COMMON,
                0xff06_0070,
                0x78,
                &event_params(DISP_COMMON, 0)
            )
        ),
        NV_OK,
        "0x78 under NV04_DISPLAY_COMMON"
    );

    // 2b. The 23 subdevice notifier armings, in VFIO order (vfio-10 RPCs 2518-2566, all
    //     REPEAT): accepted where owner ruling §S classifies the index (including every index the
    //     real GSP never posted in vfio-8/9/10), refused where the real GSP does post it.
    let windows_order = [
        44u32, 43, 113, 120, 4, 33, 139, 157, 197, 122, 158, 2, 26, 12, 23, 24, 1, 7, 45, 34, 118,
        178, 182,
    ];
    // Refused: the four indices the real GSP posts in vfio-8/9/10 (owner decision pending).
    let refused = [33u32, 139, 45, 34];
    for ev in windows_order {
        let got = params(
            &mut *c,
            &control(SUBDEVICE, 0x2080_0301, &words(&[ev, 2, 0, 0, 0])),
        );
        if refused.contains(&ev) {
            assert_eq!(got, Err(0x56), "notifier {ev} must stay refused");
        } else {
            assert_eq!(
                got,
                Ok(words(&[ev, 2, 0, 0, 0])),
                "notifier {ev} is accepted under the ruling"
            );
        }
    }

    // 3. The display-common notifier: the bound event is enabled; an event bound to the
    //    subdevice is not bound here and is refused as RM refuses it.
    let set = 0x0073_0301;
    assert_eq!(
        params(
            &mut *c,
            &control(DISP_COMMON, set, &words(&[0, 0xff06_0070, 1, 2]))
        ),
        Ok(words(&[0, 0xff06_0070, 1, 2]))
    );
    assert_eq!(
        params(
            &mut *c,
            &control(DISP_COMMON, set, &words(&[0, 0xff06_0040, 2, 2]))
        ),
        Err(NV_ERR_INVALID_STATE)
    );

    // 4. Power source.
    assert_eq!(
        params(&mut *c, &control(SUBDEVICE, 0x2080_205a, &words(&[7]))),
        Ok(words(&[0]))
    );

    // None of it reached the ledger.
    let ledger: Vec<Option<u32>> = log.sample().iter().map(|u| u.cmd).collect();
    for cmd in [0x2080_220e, 0x2080_220d, 0x0073_0301, 0x2080_205a] {
        assert!(!ledger.contains(&Some(cmd)), "{cmd:#x} reached the ledger");
    }

    // The event's FREE unbinds it: the same enable is then refused.
    let free = rpc(
        RpcFunction::Free,
        0x0a,
        words(&[CLIENT, DISP_COMMON, 0xff06_0070, 0]),
    );
    assert_eq!(status(&mut *c, &free), NV_OK);
    assert_eq!(
        params(
            &mut *c,
            &control(DISP_COMMON, set, &words(&[0, 0xff06_0070, 2, 1]))
        ),
        Err(NV_ERR_INVALID_STATE)
    );
}
