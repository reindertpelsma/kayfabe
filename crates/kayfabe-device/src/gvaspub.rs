//! ★★★ **The VA-space page-directory publication, LATCHED** — `0x90f10106` /
//! `0x20800a9f`, the guest telling us where its page directories live, recorded with the
//! `hObject` that names *which* VA space they belong to.
//!
//! # Why this module exists, and it is a measurement rather than a design preference
//!
//! `[measured 2026-08-08]` — a census of all 88 `KAYFABE-RPC` entries in
//! `traces/real_ga106/rpc_transcript_real_ga106.txt`, our own committed transcript of a
//! **real** 580.159.04 driver on a **real** GA106
//! (`docs/design/execution_plane_increments.md` §14.9):
//!
//! | control | occurrences in the whole boot |
//! |---|---|
//! | `0x00801813` `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` | **0** |
//! | `0x00801814` (the paired unset) | **0** |
//! | [`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`] | **4** (entries 57, 61, 66, 70) |
//! | [`NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`] | **1** |
//!
//! `SET_PAGE_DIRECTORY` is the **only** control the port turns into a
//! `kayfabe_arch::ids::Pdb` — `kayfabe_rmrpc::translate_control` is the one site that does
//! it, and this crate deliberately cannot name the event it emits (`gsp_core_bridge.md`
//! §1.2: one crate owns that seam and it is not this one) — and the boot never sends it. ⇒ these two ids are the *only* boot-path source of a
//! page-directory root, and until this module existed the port decoded them, answered
//! them, and **dropped the decoded value on the floor** —
//! `kayfabe_abi::gvaspacepdes`'s own header said so: *"Nothing is recorded from them
//! yet."*
//!
//! # ⊘ This is an OBSERVER. It records and it changes nothing the guest can see.
//!
//! [`GvasPubRecorder::respond`] returns [`None`] on **every** command without exception,
//! so the chain proceeds to [`crate::inittables::InitTablePolicy`] exactly as it did
//! before this link was seated. That is not a convention, it is the whole safety argument:
//! `tests/gvas_publication.rs` drives the served chain with and without this link and
//! asserts the reply bytes are identical, and `tests/gvaspace_pdes.rs` /
//! `tests/cap1b_differential.rs` pin the answer independently.
//!
//! ★ **And it is seated AHEAD of the link that answers, which is why it can see anything
//! at all.** `crate::served_chain` is a `find_map`: `InitTablePolicy` *terminates* the
//! chain for these two ids (it decodes, re-encodes and returns `Some`), so a recorder
//! seated with the other recorders at the tail would never be reached. Re-routing *who
//! answers* was the alternative and it is forbidden — `gvaspace_pdes` and
//! `cap1b_differential` pin `InitTablePolicy` as the answerer.
//!
//! # ★★★ `hObject` is the point, and it is the field a port loses first
//!
//! The client arm is issued with `rmCtrlParams.hObject = hVASpace`
//! (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:5174-5177`), where `hVASpace`
//! is the `VaSpaceApi` resource's own handle (`:4070`). So the RPC header — **not** the
//! params — is the only statement of *which* VA space these levels root.
//! `kayfabe_rmrpc::translate_control` drops it (`crates/kayfabe-rmrpc/src/lib.rs:1256`
//! reads `req.client` and `req.cmd` and never `req.object`), so a publication recorded
//! without it would be a page-directory root attributable to nothing — four
//! indistinguishable roots for four different address spaces.
//!
//! ⊘ Nothing here reaches for `translate_control`. [`kayfabe_abi::view::RpcControlReq`]
//! already carries `object`, and the recorder reads it off the same header
//! `InitTablePolicy` classifies with — the shortest path that keeps the field, which is
//! what §14.8 asked for.
//!
//! # ★★ `levels[0]` is the ROOT, and the aperture beside it is a real fork
//!
//! `_gvaspacePopulatePDEentries` starts at `pGpuState->pFmt->pRoot` and descends via
//! `mmuFmtGetNextLevel` at the **bottom** of its loop, filling `levels[i]` top-down
//! (`ogkm-580: gpu_vaspace.c:3974-4031`); the receiver consumes it **bottom-up**
//! (`for (i = numLevelsToCopy - 1; i >= 0; i--)`, `:4492`), which is the corroborating
//! half — root last consumed, so root is index 0. The receiver then forks on the
//! aperture: `GMMU_APERTURE_VIDEO → ADDR_FBMEM`, `GMMU_APERTURE_SYS_{COH,NONCOH} →
//! ADDR_SYSMEM`, asserting on anything else (`:4503-4511`).
//!
//! ⊘ That fork is why this log keeps the **whole** [`PdeLevel`] and not a bare address.
//! `translate_control`'s own rustdoc predicted the debt: the page-directory event it emits
//! has nowhere to put an aperture, so *"a vidmem-rooted and a sysmem-rooted page directory
//! become the same event"* — safe only while `Pdb` is a bare key, and *"the day a walker follows a
//! PDB it must know whether the address is a framebuffer offset or a guest-physical
//! address"* is the day this record is consumed. It carries the aperture from the first
//! byte so that day is not a migration.
//!
//! # ⊘⊘⊘ What this module deliberately does NOT do
//!
//! It does **not** populate `Channel::vas_pdb`, does not create a `Vas`, and does not
//! relax the `NoVas` refusal at `crates/kayfabe-fwd/src/lib.rs:1650`. `[src]`
//! `docs/design/execution_plane_increments.md` §14.8 measured why: granting the CeUtils
//! channel a VAS *without* the executor being reachable makes `plan_doorbell` pass, makes
//! the `#14` ring-gate vacuous over the shim's empty working set, and makes
//! `commit_doorbell` ring a host channel with no CE behind it — the guest still times out
//! at `mem_mgr.c:463`, but the doorbell now reports **Served** instead of Refused. A
//! served-but-inert path is this project's forbidden shape. This link's entire output is a
//! number in a report.

use std::sync::{Arc, Mutex};

use kayfabe_abi::gvaspacepdes::{
    self, COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
    NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER, ServerReservedPdes,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// How many distinct publications the log remembers.
///
/// ★ Eight, against a real boot's **five** (four client-arm + one global-arm, the §14.9
/// census). Sized with headroom rather than to the measurement, because a port that grew a
/// second device or a second VAS per device would otherwise silently start clipping the
/// one thing this log exists to carry — and [`GvasPubSnapshot::distinct`] keeps counting
/// past the cap, so a full sample is never mistaken for a complete list.
pub const GVAS_PUBLICATION_SAMPLE_MAX: usize = 8;

/// One publication, as it arrived: the two header handles, and the decoded body.
///
/// ★ Keyed on **everything** for de-duplication (see [`GvasPubLog::note`]). Two arms of
/// the same VA space differ in `cmd`; two VA spaces differ in `object`; a re-publication
/// after teardown differs in nothing at all and is a `count` of two, which is a real event
/// and not a duplicate row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GvasPublication {
    /// Which of the two ids carried it —
    /// [`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`] (a VA space under a client's
    /// device) or
    /// [`NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`] (the
    /// GPU group's global VA space, which RM builds for itself).
    ///
    /// ⊘ Kept rather than folded away. The two arms are chosen on whether there is a
    /// resserv call context (`ogkm-580: gpu_vaspace.c:4039-4137`), i.e. on *who owns the
    /// VA space*, and a root whose owner is unknown is a root that cannot be attributed to
    /// a channel later.
    pub cmd: u32,
    /// `hClient` from the RPC control header — the namespace the publication was issued
    /// in.
    pub client: u32,
    /// ★★★ `hObject` from the RPC control header — **the VA space itself**
    /// (`rmCtrlParams.hObject = hVASpace`, `ogkm-580: gpu_vaspace.c:5174-5177`; the handle
    /// is the `VaSpaceApi` resource's own, `:4070`). Without it a root belongs to no
    /// address space and cannot be joined to a channel.
    pub object: u32,
    /// The decoded 184-byte body, whole — including every [`PdeLevel`], not only
    /// `levels[0]`.
    ///
    /// ⊘ Whole, on purpose. `num_levels` is `4` on GA106 with shifts `47, 38, 29, 21`, and
    /// a walker that is handed only the root still has to know the format of the levels
    /// beneath it; discarding them here would make this record the second-best source
    /// about its own subject.
    pub pdes: ServerReservedPdes,
    /// How many times this exact row arrived.
    pub count: u64,
}

/// Everything the log can say at teardown, in one read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GvasPubSnapshot {
    /// Every publication that decoded, including repeats and rows past the cap.
    pub total: u64,
    /// Distinct rows seen — the truth even past [`GVAS_PUBLICATION_SAMPLE_MAX`].
    pub distinct: u64,
    /// ★★ Publications that arrived and **did not decode**: a body the payload was too
    /// short to hold, or 184 bytes that contradict `ctrl90f1.h`'s own rules
    /// (`kayfabe_abi::gvaspacepdes::ServerReservedPdesError`).
    ///
    /// ⊘ A separate number and not an absent row, for [`crate::bar2::BarPdeLog::refusals`]'s
    /// reason exactly: *"the guest published something we could not read"* and *"the guest
    /// published nothing"* are different diagnoses, and only one of them is our defect. An
    /// absence cannot distinguish them.
    pub undecodable: u64,
    /// The rows, in first-seen order, capped at [`GVAS_PUBLICATION_SAMPLE_MAX`].
    pub sample: Vec<GvasPublication>,
}

#[derive(Debug, Default)]
struct GvasPubInner {
    total: u64,
    distinct: u64,
    undecodable: u64,
    sample: Vec<GvasPublication>,
}

/// The shared record. Cloneable so the plane and the chain link hold the same one — the
/// shape [`crate::bar2::BarPdeLog`] and [`crate::census::ControlCensusLog`] already have,
/// and for their reason: the policy chain is behind the plane's lock and a reporter must
/// be able to read this without taking it.
#[derive(Debug, Clone, Default)]
pub struct GvasPubLog {
    inner: Arc<Mutex<GvasPubInner>>,
}

impl GvasPubLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> GvasPubLog {
        GvasPubLog::default()
    }

    /// Record one decoded publication.
    pub fn note(&self, row: GvasPublication) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.total += 1;
        if let Some(seen) = s.sample.iter_mut().find(|r| {
            r.cmd == row.cmd
                && r.client == row.client
                && r.object == row.object
                && r.pdes == row.pdes
        }) {
            seen.count += 1;
            return;
        }
        s.distinct += 1;
        if s.sample.len() < GVAS_PUBLICATION_SAMPLE_MAX {
            s.sample.push(GvasPublication { count: 1, ..row });
        }
    }

    /// Record one publication that arrived and could not be decoded.
    pub fn note_undecodable(&self) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        s.undecodable += 1;
    }

    /// Everything recorded so far.
    #[must_use]
    pub fn snapshot(&self) -> GvasPubSnapshot {
        let s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        GvasPubSnapshot {
            total: s.total,
            distinct: s.distinct,
            undecodable: s.undecodable,
            sample: s.sample.clone(),
        }
    }

    /// ★★★ Power-on: forget every publication.
    ///
    /// **Not optional**, and for [`crate::bar2::BarPdeLog::device_reset`]'s reason rather
    /// than for tidiness: a page-directory root that survived a device life is the
    /// *previous* guest's, and the moment anything consumes this record — which is the
    /// entire point of writing it — a stale row would attribute one guest's address space
    /// to the next one's channel. It is also `#130`'s property quantified over all of the
    /// device's state, and this is device state.
    pub fn device_reset(&self) {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *s = GvasPubInner::default();
    }
}

/// Is this control id one of the two publication arms?
///
/// ★ A free function so the pair has exactly one site. Two ids carry one struct
/// (`kayfabe_abi::gvaspacepdes`), and a port that learned about one of them and not the
/// other would be blind to *every device default VA space* or to *the GPU group's global
/// one*, with nothing failing.
#[must_use]
pub fn is_pde_publication(cmd: u32) -> bool {
    cmd == NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES
        || cmd == NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER
}

/// The observing link: decodes the publication, records it with its handles, and
/// **declines**.
///
/// Seated FIRST in [`crate::served_chain`] — ahead of the link that answers these ids — for
/// the reason this module's header gives.
#[derive(Debug, Clone)]
pub struct GvasPubRecorder {
    driver: DriverAbiTable,
    log: GvasPubLog,
}

impl GvasPubRecorder {
    /// Bind the recorder to a driver version and a shared log.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: GvasPubLog) -> GvasPubRecorder {
        GvasPubRecorder { driver, log }
    }

    /// The driver version this link decodes as.
    #[must_use]
    pub fn driver(&self) -> DriverAbiTable {
        self.driver
    }
}

impl CommandPolicy for GvasPubRecorder {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function == RpcFunction::RmControl
            && let Ok(req) = self.driver.decode_rpc_control(&cmd.payload)
            && is_pde_publication(req.cmd)
        {
            // ⊘ The recorder applies NO size policy of its own. `InitTablePolicy` refuses
            // a `paramsSize` that disagrees with the struct; this link records what the
            // GUEST sent regardless, because a publication that the answering link then
            // refused is exactly the event whose absence from the report would be misread
            // as "the guest never published". Same rule the arming census states: record
            // the malformed request, or be blind to the one whose refusal needs
            // explaining.
            match cmd
                .payload
                .get(req.params_at..req.params_at + COPY_SERVER_RESERVED_PDES_PARAMS_SIZE)
                .map(gvaspacepdes::decode_server_reserved_pdes)
            {
                Some(Ok(pdes)) => self.log.note(GvasPublication {
                    cmd: req.cmd,
                    client: req.client,
                    object: req.object,
                    pdes,
                    count: 1,
                }),
                // Either the payload was too short to hold the body, or the body
                // contradicted its own header's rules.
                Some(Err(_)) | None => self.log.note_undecodable(),
            }
        }
        // ⊘⊘⊘ ALWAYS `None`, on EVERY path including the ones above. This link answers
        // nothing; observing is not altering. `tests/gvas_publication.rs` asserts the
        // served chain's reply bytes are identical with and without it.
        None
    }
}

kayfabe_util::assert_send_sync!(
    GvasPublication,
    GvasPubSnapshot,
    GvasPubLog,
    GvasPubRecorder
);
