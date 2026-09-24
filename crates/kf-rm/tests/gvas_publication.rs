//! The VA-space page-directory publication (`0x90f10106` / `0x20800a9f`) through the served
//! chain — the one chain-level property of the old `kayfabe-device/tests/gvas_publication.rs`
//! that survives v3.
//!
//! ⊘ The old file tested `gvaspub::GvasPubRecorder`, an observer seated at the FRONT of the
//! chain that latched each publication into a log. v3 cuts that recorder with the other P4
//! page-directory links (`V3_P2_PORT_MAP.md` §1: "record the guest's statement as an attribute
//! of the VA-space object" — the memory plane's job), so its fidelity/sample-cap/reset tests
//! have nothing to test. What remains is the **neutrality** property: the served chain's reply
//! to a publication is byte-for-byte the reply `InitTablePolicy` produces on its own — i.e.
//! nothing seated ahead of the answering link (the two observers, the sticky guard, the
//! census) changes a byte of it.
//!
//! ★ The fixture is a real driver's own publication: `kf-abi/tests/fixtures/
//! ga106_ctl_20800a9f.bin` is the C artifact's captured body for `0x20800a9f` — every field is
//! `[in]`, so the captured "reply" is the request a stock 580.159.04 driver sent on real
//! silicon. ⚠ 176 of 184 bytes; the eight zero-filled are the tail of `levels[5]`, which a
//! publication with `numLevelsToCopy = 4` leaves zero anyway.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::gvaspacepdes::{
    COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
    NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

fn oracle_body() -> Vec<u8> {
    let p = format!("{}/../kf-abi/tests/fixtures/ga106_ctl_20800a9f.bin", env!("CARGO_MANIFEST_DIR"));
    let mut b = std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p} unreadable: {e}"));
    assert!(
        b.len() <= COPY_SERVER_RESERVED_PDES_PARAMS_SIZE,
        "the fixture is longer than the struct it is a capture of"
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

#[test]
fn the_chains_reply_to_a_publication_is_the_answering_links_own_reply_byte_for_byte() {
    // ★★★ The control is `InitTablePolicy` **alone**, driven with the same command — so this
    // compares the shipped chain against the link that is supposed to be answering, rather
    // than against itself.
    let body = oracle_body();
    for id in [
        NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    ] {
        let cmd = control_command(0xc1e0_0004, 0x0000_5c01, id, &body);
        let mut chain = kf_rm::served_policy(
            ga106::board(),
            ga106::host(),
            driver(),
            kf_rm::ChainLogs::default(),
            kf_rm::census::ControlCensusLog::new(),
            kf_rm::ObjectLinks::default(),
        );
        let through_chain = chain.respond(&cmd).expect("the chain answers");
        let mut alone = kf_rm::inittables::InitTablePolicy::new(ga106::board(), ga106::host(), driver());
        let direct = alone.respond(&cmd).expect("InitTablePolicy answers");
        assert_eq!(through_chain.rpc_result, direct.rpc_result, "0x{id:08x}: the chain changed the RESULT");
        assert_eq!(through_chain.body, direct.body, "0x{id:08x}: the chain changed the reply BODY");
        // Non-vacuity: this is a real, non-empty, served reply — not two matching refusals.
        assert_eq!(direct.rpc_result, 0);
        assert!(!direct.body.is_empty());
    }
}

/// ⚠ **The P4 cut, pinned so it stays a VISIBLE gap.** The old chain answered
/// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`) with `SetPageDirPolicy`; v3 defers
/// that link to the memory plane (P4). Until it returns, the control falls through every link
/// to the unserviced ledger and is refused by name — and RM **rolls back** on that refusal
/// (`ogkm-580: dma.c:531-551`), which is the wall the old link was built to clear. This test
/// fails the day the gap closes, so the P4 link lands with a test change, not silently.
#[test]
fn set_page_directory_reaches_the_ledger_until_the_memory_plane_answers_it() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs { unserviced: log.clone(), ..Default::default() },
        kf_rm::census::ControlCensusLog::new(),
        kf_rm::ObjectLinks::default(),
    );
    let body = vec![0u8; kf_abi::generated::ctrl::Nv0080CtrlDmaSetPageDirectoryParams::SIZE];
    let cmd = control_command(
        0xc1d0_000a,
        0x5c00_0002,
        kf_abi::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        &body,
    );
    let reply = chain.respond(&cmd);
    assert!(reply.is_none_or(|r| r.rpc_result != 0), "0x00801813 must not be answered NV_OK by a P3 chain");
    assert_eq!(log.total(), 1, "the ledger names it");
}

/// ★ P4: with the memory plane seated (`ObjectLinks::memory`), every page-directory statement
/// reaches it, its reply is HELD for the reconcile, and the publications' replies are still
/// `InitTablePolicy`'s own, byte for byte. `SET_PAGE_DIRECTORY` is answered `NV_OK` with its
/// params echoed instead of reaching the ledger.
#[test]
fn with_the_memory_plane_seated_every_statement_is_carried_and_held() {
    use kf_rm::barpde::MemStatement;
    let got: std::sync::Arc<std::sync::Mutex<Vec<MemStatement>>> = std::sync::Arc::default();
    let g = got.clone();
    let g2 = got.clone();
    let log = kf_rm::unserviced::UnservicedLog::new();
    let mut chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs { unserviced: log.clone(), ..Default::default() },
        kf_rm::census::ControlCensusLog::new(),
        kf_rm::ObjectLinks {
            objects: None,
            memory: Some(kf_rm::MemoryLink {
                sink: std::sync::Arc::new(move |s| g.lock().unwrap().push(s)),
                guest_os: kf_abi::GuestOs::Linux,
            }),
            channels: None,
        },
    );
    // The publication: carried, held, and answered exactly as before.
    let pub_cmd = control_command(0xc1e0_0004, 0x0000_5c01, NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES, &oracle_body());
    let r = chain.respond(&pub_cmd).expect("answered");
    let mut alone = kf_rm::inittables::InitTablePolicy::new(ga106::board(), ga106::host(), driver());
    let direct = alone.respond(&pub_cmd).expect("InitTablePolicy answers");
    assert_eq!((r.rpc_result, &r.body), (direct.rpc_result, &direct.body), "the reply is InitTablePolicy's");
    assert!(chain.holds_for_refresh(&pub_cmd), "held for the reconcile");
    // SET_PAGE_DIRECTORY: a vidmem root for VA space 0x5c00_0007.
    let mut p = vec![0u8; kf_abi::generated::ctrl::Nv0080CtrlDmaSetPageDirectoryParams::SIZE];
    p[0..8].copy_from_slice(&0x0123_4000u64.to_le_bytes());
    p[16..20].copy_from_slice(&0x5c00_0007u32.to_le_bytes());
    let set = control_command(0xc1d0_000a, 0x5c00_0002, kf_abi::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, &p);
    let r = chain.respond(&set).expect("answered");
    assert_eq!(r.rpc_result, 0);
    assert!(chain.holds_for_refresh(&set));
    assert_eq!(log.total(), 0, "nothing reached the ledger");
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 2);
    match got[1] {
        MemStatement::PageDir(st) => {
            assert_eq!((st.client.0, st.vaspace.0, st.pdb.0), (0xc1d0_000a, 0x5c00_0007, 0x0123_4000));
        }
        MemStatement::BarPde(_) => panic!("wrong statement"),
    }
    drop(got);
    // fn 70: carried as a BAR2 root entry and held.
    let mut b = vec![0u8; kf_rm::barpde::UPDATE_BAR_PDE_BODY_SIZE];
    b[0..4].copy_from_slice(&kf_rm::barpde::BAR_TYPE_2.to_le_bytes());
    b[8..16].copy_from_slice(&0x2_efbc_302u64.to_le_bytes());
    let fn70 = RpcCommand { function: RpcFunction::UpdateBarPde, code: 0x46, sequence: 26, payload: b, elements: 1, delivered: Vec::new() };
    assert_eq!(chain.respond(&fn70).expect("answered").rpc_result, 0);
    assert!(chain.holds_for_refresh(&fn70));
    assert!(matches!(g2.lock().unwrap()[2], MemStatement::BarPde(p) if p.entry == 0x2_efbc_302));
    // An ordinary control holds nothing.
    let other = control_command(0xc1d0_000a, 0x5c00_0002, 0x2080_0110, &[0u8; 8]);
    let _ = chain.respond(&other);
    assert!(!chain.holds_for_refresh(&other));
}
