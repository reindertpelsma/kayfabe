//! `kayfabe_device::gvaspub` — the VA-space page-directory publication, latched.
//!
//! ## Why this file exists
//!
//! `[measured 2026-08-08]` over `traces/real_ga106/rpc_transcript_real_ga106.txt` (a real
//! 580.159.04 driver on a real GA106; the census in
//! `docs/design/execution_plane_increments.md` §14.9): `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`
//! — the **only** control this port turns into a page-directory base — occurs **zero** times
//! in the whole boot, while `0x90f10106` occurs four times and `0x20800a9f` once. Those two
//! ids were decoded, answered `NV_OK`, and their decoded value discarded.
//!
//! ## The three properties, and the middle one is the safety argument
//!
//! 1. **Fidelity** — a publication is recorded with the `hClient`/`hObject` it arrived on
//!    and every `PdeLevel` it carried, `levels[0]` first. Two publications differing only
//!    in `hObject` are **two rows**: the handle is the VA space's identity
//!    (`ogkm-580: gpu_vaspace.c:5174-5177`), and folding them would make four roots for four
//!    address spaces indistinguishable.
//! 2. **Neutrality** — ⊘ the recorder answers **nothing**, on every path, and the chain's
//!    reply to a publication is byte-for-byte the reply `InitTablePolicy` produces on its
//!    own. This is the property that makes an observer seated *ahead of the answering link*
//!    legitimate; `crates/kayfabe-abi/tests/gvaspace_pdes.rs` and
//!    `crates/kayfabe-crec/tests/cap1b_differential.rs` pin the answer from the other side.
//! 3. **Truthfulness under stress** — an undecodable publication is a positive number and
//!    not an absent row, the distinct count keeps counting past the sample cap, and a device
//!    reset forgets everything.
//!
//! ## ★ The fixture is a real driver's own publication, not one this file composed
//!
//! `crates/kayfabe-abi/tests/fixtures/ga106_ctl_20800a9f.bin` is the C artifact's captured
//! body for `0x20800a9f` — and because every field of the struct is `[in]`, that captured
//! "reply" **is the request a stock 580.159.04 driver sent on real silicon**. ⚠ It is 176 of
//! 184 bytes (the recorder's `dlen`); the eight this file zero-fills are the tail of
//! `levels[5]`, which a publication with `numLevelsToCopy = 4` leaves zero anyway. That
//! caveat is `gvaspace_pdes.rs`'s and is restated here rather than inherited silently.

use kayfabe_abi::gvaspacepdes::{
    COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, GMMU_FMT_MAX_LEVELS,
    NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
    NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    decode_server_reserved_pdes,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::gvaspub::{GVAS_PUBLICATION_SAMPLE_MAX, GvasPubLog, GvasPubRecorder};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;

fn driver() -> kayfabe_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// The real GA106 driver's own publication body, zero-filled to the full struct — see the
/// module docs for what the eight bytes are and why they are not silicon's.
fn oracle_body() -> Vec<u8> {
    let p = format!(
        "{}/../kayfabe-abi/tests/fixtures/ga106_ctl_20800a9f.bin",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut b = std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p} unreadable: {e}"));
    assert!(
        b.len() <= COPY_SERVER_RESERVED_PDES_PARAMS_SIZE,
        "the fixture is longer than the struct it is a capture of; every assertion below \
         would be measuring a different layout"
    );
    b.resize(COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, 0);
    b
}

fn control_command(client: u32, object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The whole production chain, with the publication log handed back beside it.
fn chain_with_log() -> (Box<dyn CommandPolicy>, GvasPubLog) {
    let log = GvasPubLog::new();
    let chain = kayfabe_device::served_policy(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        driver(),
        kayfabe_device::ChainLogs {
            gvas_pub: log.clone(),
            ..Default::default()
        },
        kayfabe_device::census::ControlCensusLog::new(),
        None,
    );
    (chain, log)
}

fn recorder(log: &GvasPubLog) -> GvasPubRecorder {
    GvasPubRecorder::new(driver(), log.clone())
}

// ── Property 1: fidelity ───────────────────────────────────────────────────────────────

#[test]
fn a_real_publication_is_latched_with_its_handles_and_every_level() {
    let (mut chain, log) = chain_with_log();
    let body = oracle_body();
    let cmd = control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
        &body,
    );
    let reply = chain
        .respond(&cmd)
        .expect("InitTablePolicy answers this id");
    assert_eq!(
        reply.rpc_result, 0,
        "precondition: the publication is served"
    );

    let snap = log.snapshot();
    assert_eq!(snap.total, 1);
    assert_eq!(snap.distinct, 1);
    assert_eq!(snap.undecodable, 0);
    let row = snap.sample.first().expect("one row");
    assert_eq!(
        row.cmd,
        NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER
    );
    assert_eq!(row.client, 0xc1e0_0004);
    // ★★★ The whole point of the increment. `translate_control` drops this field; a row
    // without it is a page-directory root attributable to no address space.
    assert_eq!(row.object, 0x0000_5c01);

    // …and the body is the driver's own, decoded — checked against an independent decode of
    // the same bytes rather than against numbers transcribed into this file, so a decoder
    // change cannot be blessed by an assertion that moved with it.
    let want = decode_server_reserved_pdes(&body).expect("the oracle body decodes");
    assert_eq!(row.pdes, want);
    assert_eq!(row.pdes.levels.len(), GMMU_FMT_MAX_LEVELS);
    // `levels[0]` is the ROOT (`ogkm-580: gpu_vaspace.c:3974-4031`), and the root of a real
    // publication is a real address in a real aperture — never the default.
    assert_ne!(
        row.pdes.levels[0].phys_address, 0,
        "the captured GA106 publication roots at a non-zero address; a zero here means the \
         level array is being read at the wrong offset"
    );
    assert_ne!(row.pdes.levels[0].size, 0);
    assert!(row.pdes.num_levels >= 1);
}

#[test]
fn the_client_arm_is_recorded_too_and_the_two_arms_stay_apart() {
    // ⊘ `0x90f10106` is the arm a real boot hits FOUR times (§14.9); `0x20800a9f` once. A
    // port that learned one and not the other would be blind to every device default VA
    // space, or to the GPU group's global one, with nothing failing.
    let (mut chain, log) = chain_with_log();
    let body = oracle_body();
    for cmd in [
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    ] {
        chain.respond(&control_command(0xc1e0_0004, 0x0000_5c01, cmd, &body));
    }
    let snap = log.snapshot();
    assert_eq!(snap.total, 2);
    assert_eq!(
        snap.distinct, 2,
        "the two arms are chosen on WHO OWNS the VA space; folding them would lose that"
    );
    let ids: Vec<u32> = snap.sample.iter().map(|r| r.cmd).collect();
    assert_eq!(
        ids,
        vec![
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
        ]
    );
}

#[test]
fn two_va_spaces_publishing_the_same_levels_are_two_rows_not_one() {
    // ★★★ The bite this whole increment turns on. `[measured 2026-08-08]` the real boot's
    // four `0x90f10106` all carry `head = 00*8` (`hSubDevice = 0`, `subDeviceId = 0`) and
    // identical `psize`, so the PARAMS cannot tell them apart — only the header's `hObject`
    // can. Dropping it collapses four address spaces into one row.
    let (mut chain, log) = chain_with_log();
    let body = oracle_body();
    for object in [0x0000_5c01u32, 0x0000_5c02] {
        chain.respond(&control_command(
            0xc1e0_0004,
            object,
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            &body,
        ));
    }
    let snap = log.snapshot();
    assert_eq!(snap.total, 2);
    assert_eq!(
        snap.distinct, 2,
        "two VA spaces published the same levels; the ONLY thing that distinguishes them is \
         hObject, and losing it is the failure this assertion exists for"
    );
    let objects: Vec<u32> = snap.sample.iter().map(|r| r.object).collect();
    assert_eq!(objects, vec![0x0000_5c01, 0x0000_5c02]);
    assert!(snap.sample.iter().all(|r| r.count == 1));
}

#[test]
fn a_re_publication_of_the_identical_row_is_a_count_and_not_a_second_row() {
    let (mut chain, log) = chain_with_log();
    let body = oracle_body();
    for _ in 0..3 {
        chain.respond(&control_command(
            0xc1e0_0004,
            0x0000_5c01,
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            &body,
        ));
    }
    let snap = log.snapshot();
    assert_eq!(snap.total, 3);
    assert_eq!(snap.distinct, 1);
    assert_eq!(snap.sample.first().expect("one row").count, 3);
}

// ── Property 2: neutrality — the safety argument ───────────────────────────────────────

#[test]
fn the_recorder_answers_nothing_at_all() {
    // ⊘ Quantified over the mean set, not over the happy path: a publication it CAN decode,
    // one it cannot, a control it does not care about, a payload too short to hold a control
    // header, and a non-control function. `respond` must be `None` on every one of them,
    // because the link is seated AHEAD of the link that answers and any `Some` would
    // short-circuit `find_map` and replace a real reply.
    let log = GvasPubLog::new();
    let mut rec = recorder(&log);
    let body = oracle_body();
    let mut short = body.clone();
    short.truncate(64);

    let mut commands = vec![
        control_command(
            0xc1e0_0004,
            0x0000_5c01,
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            &body,
        ),
        control_command(
            0xc1e0_0004,
            0x0000_5c01,
            NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
            &short,
        ),
        control_command(0xc1e0_0004, 0x0000_5c01, 0x2080_beef, &[]),
    ];
    // A payload far too short to hold the 40-byte control header.
    commands.push(RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 1,
        payload: vec![0u8; 8],
        elements: 1,
        delivered: Vec::new(),
    });
    // And a function that is not a control at all.
    commands.push(RpcCommand {
        function: RpcFunction::RmAlloc,
        code: 0x67,
        sequence: 2,
        payload: vec![0u8; 64],
        elements: 1,
        delivered: Vec::new(),
    });

    for c in &commands {
        assert!(
            rec.respond(c).is_none(),
            "the recorder answered a command; it is seated ahead of the answering link and \
             any reply of its own REPLACES the real one (fn {:?}, payload {} bytes)",
            c.function,
            c.payload.len()
        );
    }
}

#[test]
fn the_chains_reply_to_a_publication_is_the_answering_links_own_reply_byte_for_byte() {
    // ★★★ The property that makes seating an observer ahead of `InitTablePolicy` legal.
    // The control is `InitTablePolicy` **alone**, driven with the same command — so this
    // compares the shipped chain against the link that is supposed to be answering, rather
    // than against itself.
    let chip = kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table");
    let body = oracle_body();
    for id in [
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    ] {
        let cmd = control_command(0xc1e0_0004, 0x0000_5c01, id, &body);
        let (mut chain, _log) = chain_with_log();
        let through_chain = chain.respond(&cmd).expect("the chain answers");
        let mut alone = kayfabe_device::inittables::InitTablePolicy::new(chip, driver());
        let direct = alone.respond(&cmd).expect("InitTablePolicy answers");
        assert_eq!(
            through_chain.rpc_result, direct.rpc_result,
            "0x{id:08x}: the observer changed the RESULT"
        );
        assert_eq!(
            through_chain.body, direct.body,
            "0x{id:08x}: the observer changed the reply BODY"
        );
        // Non-vacuity: this is a real, non-empty, served reply — not two matching refusals.
        assert_eq!(direct.rpc_result, 0);
        assert!(!direct.body.is_empty());
    }
}

// ── Property 3: truthfulness under stress ──────────────────────────────────────────────

#[test]
fn an_undecodable_publication_is_a_positive_number_not_an_absence() {
    // ⊘ "The guest published something we could not read" and "the guest published nothing"
    // are different diagnoses and only one of them is our defect. An absence cannot tell
    // them apart, which is why this counter exists at all.
    let log = GvasPubLog::new();
    let mut rec = recorder(&log);
    // Right length, contradicts its own ABI: `numLevelsToCopy` past `GMMU_FMT_MAX_LEVELS`.
    let mut bad = oracle_body();
    bad[0x20..0x24].copy_from_slice(&99u32.to_le_bytes());
    rec.respond(&control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        &bad,
    ));
    // Too short to hold the body at all.
    let mut short = oracle_body();
    short.truncate(100);
    rec.respond(&control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        &short,
    ));

    let snap = log.snapshot();
    assert_eq!(snap.undecodable, 2);
    assert_eq!(
        snap.total, 0,
        "neither decoded, so neither may be counted as one"
    );
    assert!(snap.sample.is_empty());
}

#[test]
fn the_distinct_count_keeps_counting_past_the_sample_cap() {
    let log = GvasPubLog::new();
    let mut rec = recorder(&log);
    let body = oracle_body();
    let n = GVAS_PUBLICATION_SAMPLE_MAX as u32 + 5;
    for i in 0..n {
        rec.respond(&control_command(
            0xc1e0_0004,
            0x0000_5c00 + i,
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            &body,
        ));
    }
    let snap = log.snapshot();
    assert_eq!(snap.total, u64::from(n));
    assert_eq!(
        snap.distinct,
        u64::from(n),
        "a full sample must never be mistaken for a complete list"
    );
    assert_eq!(snap.sample.len(), GVAS_PUBLICATION_SAMPLE_MAX);
    // First-seen order, so the sample is a prefix and not an arbitrary subset.
    assert_eq!(snap.sample[0].object, 0x0000_5c00);
}

#[test]
fn a_device_reset_forgets_every_publication() {
    // ★★★ Not tidiness: a root that survived a device life is the PREVIOUS guest's, and the
    // whole purpose of recording one is that something will eventually follow it.
    let log = GvasPubLog::new();
    let mut rec = recorder(&log);
    let body = oracle_body();
    rec.respond(&control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        &body,
    ));
    let mut short = body.clone();
    short.truncate(100);
    rec.respond(&control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        &short,
    ));
    assert_ne!(
        log.snapshot(),
        Default::default(),
        "precondition: something was latched"
    );

    log.device_reset();
    assert_eq!(
        log.snapshot(),
        Default::default(),
        "every field, including `undecodable` — a partial reset is the cross-life leak with \
         extra steps"
    );
}

#[test]
fn the_plane_carries_the_publications_into_its_residue_and_a_reset_clears_them() {
    // The seam the boot report reads. Driving the plane's own policy (not a bare recorder)
    // is what proves the shipped composition root wired the log at all.
    let plane = kayfabe_device::plane::RegPlane::new(
        kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table"),
        kayfabe_device::abi::gsp_abi_for(BENCH_DRIVER).expect("the bench driver has a table"),
        Box::new(kayfabe_device::SteppingClock::new(1)) as Box<dyn kayfabe_device::NanoClock>,
    )
    .expect("the GA106 plane realizes");
    assert_eq!(plane.gvas_publications(), Default::default());
    assert_eq!(plane.residue().gvas_pub, plane.gvas_publications());

    let log = plane.gvas_pub_log();
    let mut rec = recorder(&log);
    rec.respond(&control_command(
        0xc1e0_0004,
        0x0000_5c01,
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        &oracle_body(),
    ));
    assert_eq!(plane.gvas_publications().total, 1);
    assert_eq!(
        plane.residue().gvas_pub.sample.first().map(|r| r.object),
        Some(0x0000_5c01),
        "the residue is what a reload is compared against; a publication outside it is \
         state a `#130` recovery check cannot see"
    );

    plane.device_reset();
    assert_eq!(plane.gvas_publications(), Default::default());
    assert_eq!(plane.residue().gvas_pub, Default::default());
}
