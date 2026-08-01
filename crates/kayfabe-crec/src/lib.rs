//! # kayfabe-crec — the C↔Rust trace differential
//!
//! `docs/design/c_rust_trace_differential.md`: the C artifact is **not** history. It is a
//! second implementation of the same contract and the only one a real NVIDIA driver has
//! ever accepted end to end, so it is a *standing oracle* — and the recorded trace is the
//! durable form of it, because the bench that produced it is perishable.
//!
//! This crate is the consumer of that artifact. Four pieces:
//!
//! | module | owns |
//! |---|---|
//! | [`format`] | the C recorder's binary format, decoded — cross-validated against `rec_dump.py` |
//! | [`ga10x`] | the **GA10x** register model: the first real `GspModel`, and the reason `kayfabe-gsp` needs none |
//! | [`oracle`] | the guest RAM a recorded capture can answer, and precisely the part it cannot |
//! | [`replay`] | the transaction-segmented replay against `GspFsm`, and both sides' decoded projection |
//! | [`ledger`] | the reader of §6.3's MUST-DIFFER table: expected-differ vs **finding** |
//!
//! ## ★★★ Read these four limits before believing any result
//!
//! They were measured *before* the capture (`c_rust_trace_differential.md` §5a), and this
//! crate's own run reproduces every one of them:
//!
//! 1. **The completion plane has no C oracle at all.** The C never observes a host
//!    completion source; it *forges* completions. 17 isolate call sites, zero poll/event
//!    verbs. A green diff says nothing about it.
//! 2. **The diff will never be green end to end, and a green diff would itself be the
//!    bug.** The C has no refusal vocabulary — it echoes `NV_OK` for essentially
//!    everything — so every MUST-DIFFER row is a position where the C emits a positive
//!    event and we emit a refusal.
//! 3. **Only a hermetic capture can be closed over.** With `m2fwd=on` the host GPU DMAs
//!    into guest RAM behind the recorder. `cap1` is the one that is hermetic.
//! 4. **`cap1` raises exactly one interrupt in 359 062 records** — and this harness
//!    measures *which* one. It is the `INTR_LEAF_TRIGGER` self-test the driver's own
//!    `_osVerifyInterrupts` writes (`C: nvkvm_gpu_emul.c:4326-4345`), **not** a GSP
//!    SWGEN0: `nvkvm_gsp_raise_swgen0` is reachable only from
//!    `nvkvm_gsp_deliver_events`, which returns immediately when no os-event is
//!    registered, and no CUDA process runs in `cap1`. So the C posts 202 status elements
//!    and announces **none** of them; the guest picks up `GSP_INIT_DONE` and every RPC
//!    reply by polling. A differential that expected a busy interrupt plane here would be
//!    surprised, and one that expected *any* GSP interrupt would be wrong.
//!
//! ## ★★ And the fifth, which this crate discovered rather than inherited
//!
//! **The C's guest-RAM read set is a strict subset of ours, so a hermetic capture cannot
//! by itself close a replay of a correct GSP.** Two of the ledger's own rows say why:
//! GSP-D8 (the C computes `sharedMemPhysAddr + offset` and never reads the region's page
//! table) and GSP-D2 (the C has no flow control and never reads the peer's status-queue
//! read pointer). Both are addresses our implementation *must* read and the capture
//! cannot answer. [`oracle::ReconKind`] is how that is made visible instead of invisible:
//! every such read is filled under a **named assumption** and counted, and the count is
//! part of the result.
//!
//! ## Where the trace lives
//!
//! `traces/cap1_coldboot_hermetic.rec`, committed uncompressed so this crate needs no
//! decoder, no external binary and no third-party dependency. [`cap1_path`] resolves it,
//! and honours `KAYFABE_C_TRACE_CAP1` for a caller that has it elsewhere.

pub mod format;
pub mod ga10x;
pub mod ledger;
pub mod oracle;
pub mod replay;

pub use format::{CHeader, CKind, CRecord, CTrace, CrecError};
pub use ga10x::{Ga10xArch, Ga10xGspModel};
pub use ledger::{Census, Classified, Verdict, census, classify};
pub use oracle::{Answer, OracleRam, ReconKind, Reconstruction, Unobserved};
pub use replay::{Fill, Note, Projected, Replay, ReplayResult, Txn, Unprojected};

use std::path::PathBuf;

/// The RPC function ids the bench's driver uses, all explicit in its X-macro table
/// (`ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82,
/// 83, 86, 113, 254, 256`; every id is identical at `ogkm-580`).
///
/// ★★ Duplicated from the conformance suite's `gspworld::FUNCTIONS` **with a mechanism,
/// not a comment**: `tests/tests/c_trace_differential.rs` asserts `bench_abi()` equals
/// `P580.abi()` field for field. The duplication exists because this crate must be usable
/// without the test crate while the test crate depends on it.
///
/// ★ That mechanism earned its keep on the first run: three ids here were wrong
/// (`continuation_record`, `gsp_rm_alloc`, `post_event`) and the assertion caught all
/// three before a single divergence had been interpreted. `cap1` independently confirms
/// `gsp_rm_alloc = 103` — 45 of its 178 recorded commands carry that function id.
pub const FUNCTIONS: kayfabe_gsp::FunctionCodes = kayfabe_gsp::FunctionCodes {
    set_guest_system_info: 1,
    free: 10,
    dup_object: 21,
    unloading_guest_driver: 47,
    get_gsp_static_info: 65,
    continuation_record: 71,
    gsp_set_system_info: 72,
    set_registry: 73,
    gsp_rm_control: 76,
    gsp_rm_alloc: 103,
    gsp_init_done: 0x1001,
    post_event: 0x1003,
    // `E(RC_TRIGGERED, 0x1004)` — `ogkm-610: rpc_global_enums.h:257`,
    // `ogkm-580: :256`. The simulated-fault carrier (task #111); it appears in no
    // recorded capture, because the C artifact never emitted one.
    rc_triggered: 0x1004,
};

/// Axis A for the driver the capture was recorded against — **580.159.04**, keyed on the
/// full `major.minor.patch` through `kayfabe_abi::versions`, never on a literal.
///
/// This is a *value*, and [`replay::Replay::new`] takes it as an argument: a second driver
/// version is a second value here and never a second code path in `kayfabe-gsp`.
///
/// # Panics
///
/// If `kayfabe_abi::versions::BENCH_DRIVER` has no wire table, or if the table describes
/// an element layout `kayfabe-gsp` rejects. Both are build-time facts about the version
/// tables, not run-time conditions.
#[must_use]
pub fn bench_abi() -> kayfabe_gsp::GspAbi {
    let table = kayfabe_abi::versions::table_for(kayfabe_abi::versions::BENCH_DRIVER)
        .expect("the bench driver has a wire table");
    let wire = table.gsp_element_wire();
    let transport = match wire.transport() {
        None => kayfabe_gsp::TransportHdr::None,
        Some(t) => kayfabe_gsp::TransportHdr::Mctp {
            header_off: t.header_off,
            header_word: t.header_word,
            nvdm_off: t.nvdm_off,
            nvdm_word: t.nvdm_word,
        },
    };
    let init = table.gsp_init_args_wire();
    kayfabe_gsp::GspAbi {
        msgq: kayfabe_gsp::MsgqAbi {
            // `MSGQ_VERSION = 0`, `MSGQ_MSG_SIZE_MIN = 16`, `MSGQ_FLAGS_SWAP_RX = 1`
            // (`ogkm-580: src/common/shared/msgq/inc/msgq/msgq_priv.h:37-38` + `msgq.h:30-39`);
            // `RM_PAGE_SIZE = 4096` (`ogkm-580: rm_page_size.h:38`) — the DRIVER's page
            // size, not the host's.
            version: 0,
            msg_size_min: 16,
            swap_rx_flag: 1,
            region_page_size: 4096,
        },
        element: kayfabe_gsp::ElementLayout::new(
            wire.hdr_size(),
            wire.checksum_off(),
            wire.seqnum_off(),
            wire.elem_count_off(),
            transport,
        )
        .expect("the version table describes a real element"),
        rpc: kayfabe_gsp::RpcAbi {
            header_version: 0x0300_0000,
            codes: FUNCTIONS,
        },
        element_size_max: table.gsp_element_size_max(),
        init_args: kayfabe_gsp::InitArgsLayout {
            shared_mem_pa_off: 0,
            pte_count_off: 8,
            cmd_queue_off_off: 16,
            stat_queue_off_off: 24,
            min_size: init.min_size(),
            element_hdr_size_off: init.element_hdr_size_off(),
        },
        driver: *table,
    }
}

/// The committed hermetic cold-boot capture.
///
/// `KAYFABE_C_TRACE_CAP1` overrides, for a caller that keeps the artifact outside the
/// repository. Nothing else is searched: a differential that silently ran against a
/// *different* file than it reported would be worse than one that did not run.
#[must_use]
pub fn cap1_path() -> PathBuf {
    if let Ok(p) = std::env::var("KAYFABE_C_TRACE_CAP1") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../traces/cap1_coldboot_hermetic.rec")
}

/// Load and decode the capture at [`cap1_path`].
///
/// # Errors
///
/// The `io::Error` if the file is missing, or the [`CrecError`] if it is not a well-formed
/// capture. Both are returned rather than panicked, so a caller can say which happened.
pub fn load_cap1() -> Result<Result<CTrace, CrecError>, std::io::Error> {
    let blob = std::fs::read(cap1_path())?;
    Ok(CTrace::parse(&blob))
}
