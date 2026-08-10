//! `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER` (`0x20800a9b`) at the **reply-plane
//! boundary** — the control `cuInit` died on, and the only one this port answers with the
//! identity on the guest's own bytes.
//!
//! ## ⊘ What this file is for, beyond "the control is served"
//!
//! Three things no `kayfabe-abi` unit test can say, and one no differential can:
//!
//! 1. That `InitTablePolicy` reads the params from the **right offset** and splices them
//!    back at the right one, inside the right envelope, with the right inner status.
//! 2. ★★★ That **not one byte moved**. The params are pure `[IN]`
//!    (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1792-1823`), so
//!    a real GSP writes nothing back and the identity is the byte-accurate reply. This is
//!    exactly checkable and needs no capture to compare against — which is why
//!    `kayfabe-crec/tests/cap1b_differential.rs` names this file rather than admitting a gap
//!    it could not close. (`cap1b` is an `nvidia-smi` capture; this control has exactly one
//!    issuer and it is `nvidia-uvm`.)
//! 3. That the **hostile** size CPU-RM itself refuses is refused here too, with a status,
//!    rather than being silently capped into an `NV_OK` about a buffer we only half read.
//! 4. ⊘ That the observer seat still SEES the command now that an answering link terminates
//!    the chain for it. That is the §14.8 trap in its saturated-instrument form: a recorder
//!    seated below the answerer would report zero registrations, and *"none arrived"* and
//!    *"I cannot see them"* are the same reading from there.
//!
//! ⚠ Written **with** the `cap1b_differential.rs` row rather than after it.

use kayfabe_abi::faultbuffer::{
    FAULT_BUFFER_MAX_PAGES, FAULT_BUFFER_PAGE_SIZE,
    NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
    NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
    NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER,
    REGISTER_ACCESS_CNTR_BUFFER_PARAMS_SIZE, REGISTER_CLIENT_SHADOW_FAULT_BUFFER_PARAMS_SIZE,
    REGISTER_FAULT_BUFFER_PARAMS_SIZE, SHADOW_FAULT_BUFFER_MAX_PAGES,
    SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::faultbuffer::{FaultBufferLog, FaultBufferNote};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;
/// `NV_ERR_NOT_SUPPORTED`.
const NV_ERR_NOT_SUPPORTED: u32 = 0x0000_0056;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// The params `kgmmuFaultBufferReplayableAllocate_IMPL` builds: handles, size, page list.
fn params(size: u32, pages: &[u64]) -> Vec<u8> {
    let mut b = vec![0u8; REGISTER_FAULT_BUFFER_PARAMS_SIZE];
    b[0..4].copy_from_slice(&0xc1d0_0013u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x5c00_0031u32.to_le_bytes());
    b[8..12].copy_from_slice(&size.to_le_bytes());
    for (i, p) in pages.iter().enumerate() {
        let o = 16 + i * 8;
        b[o..o + 8].copy_from_slice(&p.to_le_bytes());
    }
    b
}

fn control(cmd: u32, p: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT];
    payload[0..4].copy_from_slice(&0xc1d0_0013u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0002u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(p.len() as u32).to_le_bytes());
    payload.extend_from_slice(p);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 9,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn status_of(body: &[u8]) -> u32 {
    u32::from_le_bytes([
        body[CONTROL_STATUS_OFF],
        body[CONTROL_STATUS_OFF + 1],
        body[CONTROL_STATUS_OFF + 2],
        body[CONTROL_STATUS_OFF + 3],
    ])
}

/// The stock registration: `0x31000` bytes, which is `0x20800a59`'s own
/// `replayableFaultBufferSize` for this chip, i.e. 49 pages.
fn stock_pages() -> Vec<u64> {
    (0..49u64).map(|i| 0x1_4000_0000 + i * 0x1000).collect()
}

// =====================================================================================
// 1. ★★★ The reply is the IDENTITY on the guest's own params
// =====================================================================================

/// Not one byte of the 2064-byte `[IN]` window moves, and the envelope says `NV_OK`.
#[test]
fn the_reply_is_the_guests_own_params_byte_for_byte() {
    let mut p = policy();
    let sent = params(0x0003_1000, &stock_pages());
    let cmd = control(NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER, &sent);
    let reply = p.respond(&cmd).expect("the control is served");

    assert_eq!(reply.rpc_result, 0, "the envelope is NV_OK");
    assert_eq!(
        status_of(&reply.body),
        0,
        "the inner control status is NV_OK"
    );
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + REGISTER_FAULT_BUFFER_PARAMS_SIZE],
        &sent[..],
        "★ pure [IN]: a real GSP writes nothing back, so any difference here is invention"
    );
    // ⊘ And the whole payload, not only the params window: the header's own echoed fields
    // are what CPU-RM matches the reply against.
    assert_eq!(
        &reply.body[..PARAMS_AT],
        &cmd.payload[..PARAMS_AT],
        "the control header is echoed except for the two fields a GSP owns"
    );
}

/// ★★ The size a stock guest sends is the one `0x20800a59` already told it, and this port
/// serves both — so the two controls cannot disagree about how many pages exist.
///
/// ⊘ This is a claim about **two controls agreeing**, which is why it cannot live in either
/// one's unit tests: it is checked by driving both through one policy.
#[test]
fn the_size_the_guest_registers_is_the_size_this_port_advertised() {
    let mut p = policy();
    let advertised = p
        .respond(&control(
            WantedTable::GmmuStaticInfo.cmd_id(),
            &vec![0u8; WantedTable::GmmuStaticInfo.params_size()],
        ))
        .expect("0x20800a59 is served");
    let replayable = u32::from_le_bytes([
        advertised.body[PARAMS_AT],
        advertised.body[PARAMS_AT + 1],
        advertised.body[PARAMS_AT + 2],
        advertised.body[PARAMS_AT + 3],
    ]);
    assert_eq!(replayable, 0x0003_1000, "the advertised replayable size");

    let pages = u64::from(replayable).div_ceil(FAULT_BUFFER_PAGE_SIZE) as usize;
    assert_eq!(pages, 49);
    assert!(
        pages <= FAULT_BUFFER_MAX_PAGES,
        "★ the size this port advertises must fit the PTE array the guest will send back, \
         or we would have built the wall ourselves"
    );
    // …and a registration of exactly that many pages is served.
    let sent = params(replayable, &stock_pages());
    let r = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &sent,
        ))
        .expect("served");
    assert_eq!(status_of(&r.body), 0);
}

// =====================================================================================
// 2. The refusals, each with a status rather than a silence
// =====================================================================================

/// ★★ A `faultBufferSize` past CPU-RM's own bound is refused, not capped into an `NV_OK`.
///
/// `kgmmuFaultBufferReplayableAllocate_IMPL` refuses `NV_ERR_BUFFER_TOO_SMALL` above
/// `NV2080_CTRL_INTERNAL_GMMU_FAULT_BUFFER_MAX_PAGES` **before** it sends the control
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1242-1248`), so no stock guest can
/// reach this — only a hostile one, which is the guest this port is for.
#[test]
fn a_size_past_the_vendors_bound_is_refused() {
    let mut p = policy();
    let max = FAULT_BUFFER_MAX_PAGES as u32 * FAULT_BUFFER_PAGE_SIZE as u32;

    let ok = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &params(max, &[0x1000]),
        ))
        .expect("served");
    assert_eq!(
        ok.rpc_result, 0,
        "256 pages is exactly the bound, and legal"
    );

    let over = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &params(max + 1, &[0x1000]),
        ))
        .expect("answered");
    assert_eq!(
        over.rpc_result, NV_ERR_NOT_SUPPORTED,
        "★ a capped read would have let this port answer 'registered' about a buffer it \
         recorded only the first 256 pages of"
    );
    assert!(over.body.is_empty(), "a refusal carries no body");
}

/// A params window shorter than the struct is refused — the guest's `paramsSize` is its own
/// assertion and is checked against the layout, not trusted.
#[test]
fn a_short_params_window_is_refused() {
    let mut p = policy();
    let r = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &vec![0u8; REGISTER_FAULT_BUFFER_PARAMS_SIZE - 8],
        ))
        .expect("answered");
    assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED);
}

// =====================================================================================
// 2b. ★★★ `0x20800a9d` — the client shadow buffer, the rung serving the first EXPOSED
// =====================================================================================

/// The params `kgmmuClientShadowFaultBufferRegister` builds (`ogkm-580: kern_gmmu.c:1782-1813`).
fn shadow_params(size: u32, meta: u32, ty: u32, pages: &[u64]) -> Vec<u8> {
    let mut b = vec![0u8; REGISTER_CLIENT_SHADOW_FAULT_BUFFER_PARAMS_SIZE];
    b[0..8].copy_from_slice(&0x3_0000_0000u64.to_le_bytes());
    b[8..12].copy_from_slice(&size.to_le_bytes());
    b[12..16].copy_from_slice(&meta.to_le_bytes());
    for (i, p) in pages.iter().enumerate() {
        let o = 16 + i * 8;
        b[o..o + 8].copy_from_slice(&p.to_le_bytes());
    }
    let ty_off = 16 + SHADOW_FAULT_BUFFER_MAX_PAGES * 8;
    b[ty_off..ty_off + 4].copy_from_slice(&ty.to_le_bytes());
    b
}

/// The stock non-replayable shadow registration: `0x120c20` bytes = 289 pages.
fn stock_shadow_pages() -> Vec<u64> {
    (0..289u64).map(|i| 0x3_4000_0000 + i * 0x1000).collect()
}

/// Not one byte of the 24 032-byte `[IN]` window moves, and the envelope says `NV_OK`.
#[test]
fn the_shadow_reply_is_the_guests_own_params_byte_for_byte() {
    let mut p = policy();
    let sent = shadow_params(
        0x0012_0c20,
        0,
        SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
        &stock_shadow_pages(),
    );
    let cmd = control(
        NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
        &sent,
    );
    let reply = p.respond(&cmd).expect("the control is served");
    assert_eq!(reply.rpc_result, 0);
    assert_eq!(status_of(&reply.body), 0);
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + REGISTER_CLIENT_SHADOW_FAULT_BUFFER_PARAMS_SIZE],
        &sent[..],
        "★ pure [IN] over 24 032 bytes: any difference here is invention"
    );
}

/// ★★ The shadow size the guest registers is the `nonReplayableFaultBufferSize` this port
/// advertises through `0x20800a59` — again a claim about **two controls agreeing**.
#[test]
fn the_shadow_size_the_guest_registers_is_the_size_this_port_advertised() {
    let mut p = policy();
    let advertised = p
        .respond(&control(
            WantedTable::GmmuStaticInfo.cmd_id(),
            &vec![0u8; WantedTable::GmmuStaticInfo.params_size()],
        ))
        .expect("0x20800a59 is served");
    // `nonReplayableFaultBufferSize` is the third `NvU32` of the static-info body.
    let non_replayable = u32::from_le_bytes([
        advertised.body[PARAMS_AT + 8],
        advertised.body[PARAMS_AT + 9],
        advertised.body[PARAMS_AT + 10],
        advertised.body[PARAMS_AT + 11],
    ]);
    assert_eq!(
        non_replayable, 0x0012_0c20,
        "the advertised non-replayable size"
    );
    let pages = u64::from(non_replayable).div_ceil(FAULT_BUFFER_PAGE_SIZE) as usize;
    assert_eq!(pages, 289);
    assert!(
        pages <= SHADOW_FAULT_BUFFER_MAX_PAGES,
        "★ the size this port advertises must fit the 3000-entry PTE array the guest sends \
         back, or we would have built the wall ourselves"
    );
    let r = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
            &shadow_params(
                non_replayable,
                0,
                SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
                &stock_shadow_pages(),
            ),
        ))
        .expect("served");
    assert_eq!(status_of(&r.body), 0);
}

/// ★★ A shadow geometry past CPU-RM's own 3000-page bound is refused — and the METADATA term
/// is what decides the boundary case, which is the half a `size`-only check would miss.
#[test]
fn a_shadow_geometry_past_the_vendors_bound_is_refused() {
    let mut p = policy();
    let page = FAULT_BUFFER_PAGE_SIZE as u32;
    let at_bound = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
            &shadow_params(
                3000 * page,
                0,
                SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
                &[0x1000],
            ),
        ))
        .expect("answered");
    assert_eq!(at_bound.rpc_result, 0, "3000 pages is exactly the bound");

    let over = p
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
            &shadow_params(
                3000 * page,
                1,
                SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
                &[0x1000],
            ),
        ))
        .expect("answered");
    assert_eq!(
        over.rpc_result, NV_ERR_NOT_SUPPORTED,
        "★ one byte of metadata is one more page and it crosses the bound. This assertion is \
         the only thing standing between a `size`-only check and shipping — the metadata term \
         is zero in every configuration this port targets."
    );
}

/// ⊘ The shadow buffer's own UNREGISTER is `0x20800a9e`, **not** `0x20800a9c`, and neither is
/// served. Pinned because confusing the two is the obvious mistake: they are adjacent ids for
/// adjacent buffers, and `0x20800a9c` unregisters the *replayable hardware* buffer.
#[test]
fn the_shadow_unregister_is_a9e_and_neither_unregister_is_served() {
    assert!(
        WantedTable::from_cmd(0x2080_0a9e).is_none(),
        "shadow unregister"
    );
    assert!(
        WantedTable::from_cmd(0x2080_0a9c).is_none(),
        "hw unregister"
    );
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a9d),
        Some(WantedTable::RegisterClientShadowFaultBuffer),
        "and the two REGISTERs are distinct ids for distinct buffers"
    );
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a9b),
        Some(WantedTable::RegisterFaultBuffer)
    );
}

// =====================================================================================
// 2c. ★★★ `0x20800a1d` — the access-counter buffer, reachable only once 0xB83110 is served
// =====================================================================================

fn accesscntr_params(index: u32, size: u32, pages: &[u64]) -> Vec<u8> {
    let mut b = vec![0u8; REGISTER_ACCESS_CNTR_BUFFER_PARAMS_SIZE];
    b[0..4].copy_from_slice(&index.to_le_bytes());
    b[4..8].copy_from_slice(&size.to_le_bytes());
    for (i, p) in pages.iter().enumerate() {
        let o = 8 + i * 8;
        b[o..o + 8].copy_from_slice(&p.to_le_bytes());
    }
    b
}

/// Identity echo over the 520-byte `[IN]` window.
#[test]
fn the_access_cntr_reply_is_the_guests_own_params_byte_for_byte() {
    let mut p = policy();
    let sent = accesscntr_params(0, 256 * 32, &[0x5_0000_0000, 0x5_0000_1000]);
    let cmd = control(
        NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER,
        &sent,
    );
    let reply = p.respond(&cmd).expect("the control is served");
    assert_eq!(reply.rpc_result, 0);
    assert_eq!(status_of(&reply.body), 0);
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + REGISTER_ACCESS_CNTR_BUFFER_PARAMS_SIZE],
        &sent[..],
        "★ pure [IN]: any difference here is invention"
    );
}

/// ★★★ ZERO is refused — and it is the SAME zero, one layer down.
///
/// A zero-size access-counter buffer is exactly what this port produced before it served
/// BAR0 `0xB83110`, and the physical receiver refuses it by name
/// (`numBufferPages == 0`, `ogkm-580: access_cntr_buffer_ctrl.c:231-253`). ⊘ This port must
/// not answer `NV_OK` about a buffer that cannot exist just because the guest asked politely.
#[test]
fn a_zero_or_oversize_access_cntr_registration_is_refused() {
    let mut p = policy();
    let answer = |p: &mut InitTablePolicy, size: u32| {
        p.respond(&control(
            NV2080_CTRL_CMD_INTERNAL_UVM_REGISTER_ACCESS_CNTR_BUFFER,
            &accesscntr_params(0, size, &[0x1000]),
        ))
        .expect("answered")
        .rpc_result
    };
    assert_eq!(
        answer(&mut p, 0),
        NV_ERR_NOT_SUPPORTED,
        "zero pages is illegal"
    );
    assert_eq!(
        answer(&mut p, 256 * 32),
        0,
        "two pages, the advertised size"
    );
    assert_eq!(
        answer(&mut p, 64 * 4096),
        0,
        "64 pages is exactly the bound"
    );
    assert_eq!(
        answer(&mut p, 64 * 4096 + 1),
        NV_ERR_NOT_SUPPORTED,
        "65 pages is one too many"
    );
}

/// ⊘ Its UNREGISTER partner `0x20800a1e` is not served either — the third pair, same ruling.
#[test]
fn the_access_cntr_unregister_is_a1e_and_is_not_served() {
    assert!(WantedTable::from_cmd(0x2080_0a1e).is_none());
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a1d),
        Some(WantedTable::RegisterAccessCntrBuffer)
    );
}

/// ⊘ The UNREGISTER neighbour is one command away and is **not** served — deliberately, and
/// it is asserted so that serving it later is a decision rather than a drift.
///
/// Modelling the receiver's stateful double-register refusal (`ogkm-580: kern_gmmu.c:3117`)
/// without its partner would build a latch that can only close: a guest that unregistered
/// and re-registered would meet a wall of our own making. CPU-RM logs and ignores the
/// unregister's failure (`ogkm-580: kern_gmmu.c:1325-1333`), so refusing it costs nothing.
#[test]
fn the_unregister_neighbour_is_not_served() {
    let mut p = policy();
    assert!(
        p.respond(&control(0x2080_0a9c, &[0u8; 0])).is_none(),
        "0x20800a9c is left to the port's ordinary named refusal"
    );
    assert!(WantedTable::from_cmd(0x2080_0a9c).is_none());
}

// =====================================================================================
// 3. ★★★ The observer seat still sees it — the saturated-instrument trap
// =====================================================================================

/// The recorder is seated in the full chain **ahead** of the link that now answers, so the
/// registration is still written down.
///
/// ⊘ This is the test the seat move exists for. With the recorder at its old tail seat,
/// `InitTablePolicy` would terminate the chain first and this count would be **zero** — and
/// zero is indistinguishable from "the guest never registered a buffer", which is exactly
/// how an instrument goes blind without a red test.
#[test]
fn the_full_chain_answers_and_still_records() {
    let log = FaultBufferLog::new();
    let mut chain = kayfabe_device::served_policy(
        chip(),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
        kayfabe_device::ChainLogs {
            fault_buffer: log.clone(),
            ..Default::default()
        },
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks::default(),
        // ⊘ No model name: this file's subject is not the static-info body, and declaring
        // one would make it depend on a value no assertion here reads.
        kayfabe_device::staticinfo::GpuNames::default(),
    );
    let sent = params(0x0003_1000, &stock_pages());
    let reply = chain
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &sent,
        ))
        .expect("the chain serves it");

    assert_eq!(reply.rpc_result, 0, "the chain answers NV_OK");
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + REGISTER_FAULT_BUFFER_PARAMS_SIZE],
        &sent[..],
        "the guard and the census changed no params byte either"
    );
    assert_eq!(
        log.total(),
        1,
        "★ the observer seat saw the command it records"
    );
    match log.sample().first() {
        Some(FaultBufferNote::Registered(r)) => {
            assert_eq!(r.h_client, 0xc1d0_0013);
            assert_eq!(r.h_object, 0x5c00_0031);
            assert_eq!(r.size, 0x0003_1000);
            assert_eq!(r.pages.len(), 49, "the page list the guest actually filled");
            assert_eq!(r.pages[0], 0x1_4000_0000);
        }
        other => panic!("expected one decoded registration, got {other:?}"),
    }
}

/// ★★ The shadow registration is recorded through the same seat, into its **own** counter and
/// its **own** note variant.
///
/// ⊘ The separation is the assertion. One combined count could not say which of the two
/// promises a boot took on — `0x20800a9b` says a register we serve stays empty; `0x20800a9d`
/// says we will write packets into the guest's sysmem — and the report prints a different
/// sentence for each.
#[test]
fn the_two_registrations_are_counted_and_recorded_apart() {
    let log = FaultBufferLog::new();
    let mut chain = kayfabe_device::served_policy(
        chip(),
        *table_for(BENCH_DRIVER).expect("bench ABI"),
        kayfabe_device::ChainLogs {
            fault_buffer: log.clone(),
            ..Default::default()
        },
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks::default(),
        // ⊘ No model name: this file's subject is not the static-info body, and declaring
        // one would make it depend on a value no assertion here reads.
        kayfabe_device::staticinfo::GpuNames::default(),
    );
    chain
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER,
            &params(0x0003_1000, &stock_pages()),
        ))
        .expect("served");
    let sent = shadow_params(
        0x0012_0c20,
        0,
        SHADOW_FAULT_BUFFER_NON_REPLAYABLE,
        &stock_shadow_pages(),
    );
    let reply = chain
        .respond(&control(
            NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_CLIENT_SHADOW_FAULT_BUFFER,
            &sent,
        ))
        .expect("served");
    assert_eq!(reply.rpc_result, 0);
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + REGISTER_CLIENT_SHADOW_FAULT_BUFFER_PARAMS_SIZE],
        &sent[..],
        "the guard and the census changed no shadow params byte either"
    );

    assert_eq!(log.total(), 1, "★ one HARDWARE registration, not two");
    assert_eq!(log.shadow_total(), 1, "★ one SHADOW registration, not two");
    let s = log.sample();
    assert_eq!(s.len(), 2, "both are remembered, in arrival order");
    assert!(matches!(s[0], FaultBufferNote::Registered(_)));
    match &s[1] {
        FaultBufferNote::ShadowRegistered(r) => {
            assert_eq!(r.size, 0x0012_0c20);
            assert_eq!(r.metadata_size, 0);
            assert_eq!(r.buffer_type, SHADOW_FAULT_BUFFER_NON_REPLAYABLE);
            assert!(r.type_is_known());
            assert_eq!(r.pages.len(), 289);
            assert_eq!(r.queue_gpa, 0x3_0000_0000);
        }
        other => panic!("expected a decoded SHADOW registration, got {other:?}"),
    }
}
