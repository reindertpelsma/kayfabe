//! `kayfabe_device::setpagedir` — `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`),
//! served and recorded.
//!
//! ## Why this file exists
//!
//! `[measured 2026-08-09, boots `s26_0484a3b_cup2` and `s27_c73d3ab_uvm`]` both carry
//! `nvkvm: unserviced fn 76 cmd 0x00801813`, and `cuInit`'s own `dmesg` window differs from
//! a **successful** `nvidia-smi`'s by exactly one line that is neither shared with a
//! successful device open nor already known harmless:
//! `NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332`, the rollback arm of this very
//! control (`ogkm-580: dma.c:531-551`). See `crates/kayfabe-device/src/setpagedir.rs` for
//! the full chain and for why `0x801814`'s absence **corroborates** it.
//!
//! ## The four properties
//!
//! 1. **Service** — the control is answered `NV_OK` and no longer reaches the unserviced
//!    ledger.
//! 2. **Fidelity** — every one of the seven params fields and both header handles are
//!    recorded exactly as they arrived. ★ Chiefly `hVASpace`, which this port **reports and
//!    does not interpret**.
//! 3. ★★★ **`hVASpace == 0` is a VALUE, not an absence** — it must latch, must set `valid`,
//!    and must never take a refusing arm.
//! 4. **Targeting** — seating this link changes the reply to nothing else. ⊘ This is the
//!    property that lets the next boot's census be diffed line-by-line against `s27`'s.

use kayfabe_abi::generated::ctrl::{
    NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, Nv0080CtrlDmaSetPageDirectoryParams,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_abi::view::PdbAperture;
use kayfabe_device::setpagedir::{SetPageDirLog, SetPageDirPolicy};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;

/// `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`
/// (`ogkm-580: src/nvidia/inc/kernel/rmapi/rmapi.h:161-163`).
const RMAPI_RPC_FLAGS_SERIALIZED: u32 = 1 << 1;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// `NV_ERR_INVALID_ARGUMENT`.
const NV_ERR_INVALID_ARGUMENT: u32 = 0x0000_001F;

fn driver() -> kayfabe_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS`, encoded at the generator-pinned offsets
/// (`ogkm-580: ctrl0080dma.h:802-810`).
#[allow(clippy::too_many_arguments)]
fn params(
    phys: u64,
    num_entries: u32,
    flags: u32,
    h_vaspace: u32,
    ch_id: u32,
    sub_device_id: u32,
    pasid: u32,
) -> Vec<u8> {
    let mut b = vec![0u8; Nv0080CtrlDmaSetPageDirectoryParams::SIZE];
    b[0..8].copy_from_slice(&phys.to_le_bytes());
    b[8..12].copy_from_slice(&num_entries.to_le_bytes());
    b[12..16].copy_from_slice(&flags.to_le_bytes());
    b[16..20].copy_from_slice(&h_vaspace.to_le_bytes());
    b[20..24].copy_from_slice(&ch_id.to_le_bytes());
    b[24..28].copy_from_slice(&sub_device_id.to_le_bytes());
    b[28..32].copy_from_slice(&pasid.to_le_bytes());
    b
}

fn control_command(client: u32, object: u32, cmd: u32, declared: u32, body: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + body.len()];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&declared.to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(body);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// A well-formed `SET`, with the params the caller wants.
fn set_command(client: u32, object: u32, body: &[u8]) -> RpcCommand {
    control_command(
        client,
        object,
        NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        body.len() as u32,
        body,
    )
}

/// The whole production chain, with the install log handed back beside it.
fn chain_with_log() -> (Box<dyn CommandPolicy>, SetPageDirLog) {
    let log = SetPageDirLog::new();
    let chain = kayfabe_device::served_policy(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        driver(),
        kayfabe_device::ChainLogs {
            set_page_dir: log.clone(),
            ..Default::default()
        },
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks::default(),
        kayfabe_device::staticinfo::GpuNames::default(),
    );
    (chain, log)
}

fn policy(log: &SetPageDirLog) -> SetPageDirPolicy {
    SetPageDirPolicy::new(driver(), log.clone())
}

// ── Property 1: service ────────────────────────────────────────────────────────────────

#[test]
fn the_whole_served_chain_answers_set_page_directory_with_nv_ok() {
    let (mut chain, log) = chain_with_log();
    let body = params(0x1_0000_2000, 512, 0, 0xcaf0_0005, 0, 0, 0);
    let reply = chain
        .respond(&set_command(0xc1d0_000a, 0x5c00_0002, &body))
        .expect(
            "0x00801813 must be ANSWERED by the production chain; it reached the unserviced \
             ledger in every boot before this link existed",
        );
    assert_eq!(
        reply.rpc_result, NV_OK,
        "RM reads this control's status and ROLLS BACK on failure (ogkm-580: dma.c:531-551); \
         any non-zero here reproduces the wall verbatim"
    );
    // ★★★★ THIS ASSERTION USED TO DEMAND `reply.body.is_empty()`, on the grounds that
    // "every field of NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS is [IN] (ogkm-580:
    // ctrl0080dma.h:785-826), so there is nothing to reflect". ⊘ The premise is true, the
    // citation is real, and the conclusion is the bug — so this test was pinning the defect
    // in place with a correct quotation.
    //
    // The copy-back is the TRANSPORT's, and it never reads the SDK header:
    // `portMemCopy(pParamStructPtr, paramsSize, rpc_params->params, paramsSize)` runs on
    // the NV_OK path for every control (`ogkm-580: rpc.c:11085-11090`). And an empty body
    // is not an absence — `RpcCommand::reply` zero-fills to the request's own length
    // (`kayfabe-gsp/src/rpc.rs:472-475`). ⇒ the empty body CLEARED the caller's
    // `numEntries`, and `dma.c:523` then handed the zeroed struct to
    // `gvaspaceExternalRootDirCommit`, where `numEntries == 0` makes `vaLimitNew`
    // 0xFFFF_FFFF_FFFF_FFFF and `gpu_vaspace.c:3094` fires. That is `s28_933a709_spd`.
    //
    // ⇒ the observable property is not "nothing is reflected" but **"the caller's struct
    // survives the round trip"**, which for an all-`[IN]` control means byte-identical.
    let sent = set_command(0xc1d0_000a, 0x5c00_0002, &body);
    assert_eq!(
        reply.body.len(),
        sent.payload.len(),
        "the reply must be a FULL-LENGTH payload: RpcCommand::reply zero-fills anything \
         shorter (kayfabe-gsp/src/rpc.rs:472-475), and the guest copies paramsSize bytes \
         of it back over the caller's struct (ogkm-580: rpc.c:11085-11090)"
    );
    // ⊘ The assertion is on the PARAMS REGION, not on the whole payload. `StickyAnswerGuard`
    // wraps this chain (`lib.rs:1083`) and unconditionally rewrites the reply's
    // `rmctrlFlags`/`rmctrlAccessRight` header words to `0`, so a whole-payload equality
    // would pass only for a fixture whose flags happen to be zero and would break the day
    // the fixture got realistic — a test green for a reason unrelated to its name.
    assert_eq!(
        &reply.body[PARAMS_AT..PARAMS_AT + Nv0080CtrlDmaSetPageDirectoryParams::SIZE],
        &sent.payload[PARAMS_AT..PARAMS_AT + Nv0080CtrlDmaSetPageDirectoryParams::SIZE],
        "every params field is [IN], so the caller's numEntries/physAddress/hVASpace must \
         come back EXACTLY as sent. Zeros here are gpu_vaspace.c:3094 in the guest: \
         numEntries=0 makes vaLimitNew 0xFFFF_FFFF_FFFF_FFFF."
    );
    assert_eq!(
        log.total(),
        1,
        "the acceptance must be RECORDED, not merely returned"
    );
    assert_eq!(log.refused(), 0);
}

// ── Property 2: fidelity ───────────────────────────────────────────────────────────────

#[test]
fn every_params_field_and_both_header_handles_are_recorded_as_they_arrived() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    // ⊘ Deliberately seven DISTINCT values: a decode that transposed two fields would
    // still pass against a body of zeros or repeats.
    let body = params(
        0x0000_00ab_cdef_1000,
        0x0000_0200,
        0x0000_0005,
        0xcaf0_0005,
        7,
        1,
        0x1234,
    );
    assert!(
        p.respond(&set_command(0xc1d0_000a, 0x5c00_0002, &body))
            .is_some()
    );

    let rec = log.latest().expect("an accepted SET latches a record");
    assert_eq!(rec.client, 0xc1d0_000a, "hClient comes from the RPC HEADER");
    assert_eq!(
        rec.object, 0x5c00_0002,
        "hObject comes from the header too — and for THIS control it is hDevice, not the \
         VA space (ogkm-580: dma.c:508-518)"
    );
    assert_eq!(rec.phys_address, 0x0000_00ab_cdef_1000);
    assert_eq!(rec.num_entries, 0x0000_0200);
    assert_eq!(rec.flags, 0x0000_0005);
    assert_eq!(
        rec.aperture,
        PdbAperture::SysmemCoherent,
        "the aperture is bits 1:0 of flags; 0x5 is _SYSMEM_COH with ALL_CHANNELS set"
    );
    assert_eq!(
        rec.h_vaspace, 0xcaf0_0005,
        "hVASpace is a PARAMS field, not a header one"
    );
    assert_eq!(rec.ch_id, 7);
    assert_eq!(rec.sub_device_id, 1);
    assert_eq!(rec.pasid, 0x1234);
}

#[test]
fn a_reinstall_replaces_the_record_and_counts_both() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    let first = params(0x1000, 512, 0, 0, 0, 0, 0);
    let second = params(0x9000, 1024, 0, 0, 0, 0, 0);
    assert!(p.respond(&set_command(1, 2, &first)).is_some());
    assert!(p.respond(&set_command(1, 2, &second)).is_some());
    assert_eq!(
        log.total(),
        2,
        "a re-installation is a real event; RM re-publishes a root on every re-bind and a \
         latch alone cannot say whether that happened once or twice"
    );
    assert_eq!(
        log.latest().expect("latched").phys_address,
        0x9000,
        "most recent wins"
    );
}

// ── Property 3: ★★★ hVASpace == 0 is a VALUE ───────────────────────────────────────────

#[test]
fn h_vaspace_zero_is_latched_as_a_value_and_never_takes_a_refusing_arm() {
    // ★★★ The trap this test exists for. `hVASpace = 0` NAMES the client/device pair's
    // implicit VA space (`ogkm-580: ctrl0080dma.h:812-815`); it does not mean "absent".
    // A port that routed a zero handle into an unknown/refuse arm would refuse exactly the
    // case the header documents as the common one.
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    let body = params(0x4000, 512, 0, 0, 0, 0, 0);
    let reply = p
        .respond(&set_command(0xc1d0_000a, 0x5c00_0002, &body))
        .expect("a zero hVASpace must be ANSWERED");
    assert_eq!(reply.rpc_result, NV_OK, "and answered NV_OK, not refused");
    assert_eq!(log.refused(), 0, "a zero handle is not a malformed request");

    let rec = log.latest().expect("and it must LATCH");
    assert_eq!(rec.h_vaspace, 0);
    assert!(
        log.valid(),
        "⊘⊘ the whole point: `valid` is what separates \"installed into VA space 0\" from \
         \"no SET ever arrived\". Without it the report cannot tell a measurement from an \
         absence — the same shape as decoding the C oracle's dlen=0 rows to zeros."
    );
}

#[test]
fn an_empty_log_is_not_valid_and_reads_zero_on_every_field() {
    let log = SetPageDirLog::new();
    assert!(!log.valid(), "nothing arrived, so nothing is latched");
    assert_eq!(log.latest(), None);
    assert_eq!((log.total(), log.refused()), (0, 0));
}

// ── Refusals, by a name that is true ───────────────────────────────────────────────────

#[test]
fn a_declared_size_that_is_not_sizeof_is_refused_and_latches_nothing() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    // ★ Checked EXACTLY, not as a lower bound: RM's caller passes the `sizeof` verbatim
    // (`ogkm-580: dma.c:508-518`), so a different declared size is a guest that means a
    // different struct.
    let body = params(0x4000, 512, 0, 0, 0, 0, 0);
    let cmd = control_command(
        1,
        2,
        NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        (Nv0080CtrlDmaSetPageDirectoryParams::SIZE - 4) as u32,
        &body,
    );
    let reply = p
        .respond(&cmd)
        .expect("a recognised id is ANSWERED even when refused");
    assert_eq!(reply.rpc_result, NV_ERR_INVALID_ARGUMENT);
    assert_eq!(log.refused(), 1);
    assert_eq!(log.total(), 0);
    assert!(!log.valid(), "a refusal must not latch a record");
}

#[test]
fn a_truncated_payload_is_refused_rather_than_read_past() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    let short = vec![0u8; Nv0080CtrlDmaSetPageDirectoryParams::SIZE - 8];
    let cmd = control_command(
        1,
        2,
        NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        Nv0080CtrlDmaSetPageDirectoryParams::SIZE as u32,
        &short,
    );
    let reply = p.respond(&cmd).expect("answered");
    assert_eq!(reply.rpc_result, NV_ERR_INVALID_ARGUMENT);
    assert_eq!(log.refused(), 1);
    assert!(!log.valid());
}

#[test]
fn finn_serialized_params_are_refused_rather_than_decoded_flat() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    let body = params(0x4000, 512, 0, 0, 0, 0, 0);
    let mut cmd = set_command(1, 2, &body);
    cmd.payload[20..24].copy_from_slice(&RMAPI_RPC_FLAGS_SERIALIZED.to_le_bytes());
    let reply = p.respond(&cmd).expect("answered");
    assert_eq!(
        reply.rpc_result, NV_ERR_INVALID_ARGUMENT,
        "a serialized payload is not the flat struct this port decodes; answering it flat \
         is the kind of wrong that never logs"
    );
    assert_eq!(log.refused(), 1);
    assert!(!log.valid());
}

// ── Property 4: ★★★ targeting ──────────────────────────────────────────────────────────

#[test]
fn the_link_declines_every_function_and_control_that_is_not_0x801813() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);

    // The paired UNSET, one id away and meaning the opposite. ⊘ It must NOT be swept up:
    // this port has no verb for a revocation, and answering one NV_OK would tell the guest
    // a root was removed while ours still stands.
    let unset = control_command(1, 2, 0x0080_1814, 8, &[0u8; 8]);
    assert!(
        p.respond(&unset).is_none(),
        "0x801814 is the REVOCATION and is not modelled; it must fall through to the ledger"
    );

    // A neighbouring control this link must not touch.
    let other = control_command(1, 2, 0x2080_0a9f, 184, &[0u8; 184]);
    assert!(p.respond(&other).is_none());

    // A different function entirely.
    let mut not_a_control = set_command(1, 2, &params(1, 1, 0, 0, 0, 0, 0));
    not_a_control.function = RpcFunction::UpdateBarPde;
    assert!(p.respond(&not_a_control).is_none());

    assert_eq!(
        (log.total(), log.refused()),
        (0, 0),
        "and none of them touched the log"
    );
}

#[test]
fn seating_the_link_changes_no_other_reply_byte() {
    // ★★★ The invariance control, and it is what makes the next boot's census diffable
    // against `s27`'s line by line. `s24`/`s25` proved observational changes this way and
    // `s26` proved a targeted one; a link that answered anything extra would move census
    // rows that have nothing to do with this rung.
    let (mut with, _log) = chain_with_log();
    let mut without = kayfabe_device::served_policy(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        driver(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks::default(),
        kayfabe_device::staticinfo::GpuNames::default(),
    );

    // A spread over ids the chain really answers, ids it really refuses, and the
    // neighbouring page-directory controls.
    for (cmd, size) in [
        (0x2080_0a9f_u32, 184_usize),
        (0x90f1_0106, 184),
        (0x0080_1814, 8),
        (0x2080_0a36, 88),
        (0x2080_182a, 8),
        (0x2080_1303, 16),
    ] {
        let c = control_command(0xc1d0_000a, 0x5c00_0002, cmd, size as u32, &vec![0u8; size]);
        let a = with.respond(&c);
        let b = without.respond(&c);
        assert_eq!(
            a.as_ref().map(|r| (r.rpc_result, r.body.clone())),
            b.as_ref().map(|r| (r.rpc_result, r.body.clone())),
            "seating SetPageDirPolicy changed the reply to {cmd:#010x}; this rung is only \
             readable if everything that should not move is byte-identical"
        );
    }
}

// ── Device lifetime ────────────────────────────────────────────────────────────────────

#[test]
fn a_device_reset_forgets_the_installed_root() {
    let log = SetPageDirLog::new();
    let mut p = policy(&log);
    assert!(
        p.respond(&set_command(1, 2, &params(0x7000, 512, 0, 0, 0, 0, 0)))
            .is_some()
    );
    assert!(log.valid());

    log.device_reset();
    assert!(
        !log.valid(),
        "★★★ a root that survived a device life is the PREVIOUS guest's page directory, \
         and the whole point of recording it is that something will eventually follow it"
    );
    assert_eq!((log.total(), log.refused()), (0, 0));
}
