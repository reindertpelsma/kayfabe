//! ★★★★ **`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x00801813`)** — the guest handing
//! us a page-directory root, and the control this port refused for every boot before
//! §16.30.
//!
//! # The measurement this module exists for
//!
//! `[measured 2026-08-09, boots `s26_0484a3b_cup2` and `s27_c73d3ab_uvm`]` — both carry
//! `nvkvm: unserviced fn 76 cmd 0x00801813`, and `cuInit`'s own `dmesg` window (cleared
//! immediately before `cup2`, so the capture is `cuInit`'s alone) differs from a
//! **successful** `nvidia-smi`'s by exactly four lines, three of which are the
//! logged-and-proceeded pairs §14.41 already retired. The fourth:
//!
//! ```text
//! nvAssertFailedNoLog: Assertion failed: NULL != pGpuState->pRootInternal @ gpu_vaspace.c:3332
//! ```
//!
//! That line is inside `gvaspaceExternalRootDirRevoke_IMPL`
//! (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:3277`), which has exactly three
//! call sites. §16.29.4 eliminated two; **this module's author re-read all three rather
//! than inherit the elimination**, and both eliminations survive — one of them for a
//! *stronger* reason than was recorded:
//!
//! | site | what it is | verdict, re-derived |
//! |---|---|---|
//! | `gpu_vaspace.c:1251` (`_gvaspaceGpuStateDestruct`) | teardown cleanup | ⊘ **impossible**, confirmed: the call sits inside `if (NULL != pGpuState->pRootInternal)` (`:1246-1253`), which is verbatim the condition the assert tests |
//! | `dma.c:629` (`deviceCtrlCmdDmaUnsetPageDirectory_IMPL`) | the `0x801814` handler | ⊘ **did not run**, and ★ *not* merely because the census lacks `0x801814`: that handler initialises `status = NV_OK` (`dma.c:582`) and RPCs at `dma.c:606-615` **before** it revokes at `:629`, so had it run its own RPC would have been on the wire ahead of the assert |
//! | `dma.c:539` | the **rollback arm** of `0x801813` | ★ the only survivor |
//!
//! # ★★★★ Why `0x00801814` is ABSENT, and why that is CORROBORATION rather than a hole
//!
//! §16.29.5 left this open — *"either the census has a blind spot for it, or the branch
//! differs in the shipped build"* — and offered no third option. There is one, and it is
//! RM's own macro:
//!
//! ```c
//! #define NV_RM_RPC_CONTROL(pGpu, hClient, hObject, cmd, pParams, paramSize, status)  \
//!     do {                                                                            \
//!         OBJRPC *pRpc = GPU_GET_RPC(pGpu);                                           \
//!         NV_ASSERT(pRpc != NULL);                                                    \
//!         if ((status == NV_OK) && (pRpc != NULL))          /* ← the guard */         \
//!         …
//! ```
//! (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:223-242`.)
//!
//! The rollback block at `dma.c:531-551` runs **precisely because `status != NV_OK`**, and
//! `status` is not reassigned before the `UNSET` macro is reached — `gvaspaceExternalRootDirRevoke`'s
//! return value is *discarded* at `:539`. ⇒ **the `UNSET` RPC is structurally unsendable
//! from the rollback arm.** Its absence is not a blind spot and not a different build; it
//! is what RM guarantees.
//!
//! ★★ And it **discriminates**: `dma.c:629`'s RPC is issued with `status` freshly `NV_OK`,
//! so that path *would* have shown `0x801814`. The absence therefore argues **for**
//! `dma.c:539` and **against** `dma.c:629`. ⊘ The census is not saturated, so this is a
//! measurement rather than a cap artefact: `s27` printed **38 distinct** unserviced rows
//! against a sample cap of 64 ([`crate::unserviced::UNSERVICED_SAMPLE_MAX`]), all 45
//! served-control rows, and of those exactly one carries a non-zero result (`0x2080012b`,
//! `x2 REFUSED`). `0x801814` is on none of the three lists.
//!
//! # ⊘⊘ WHAT THIS MODULE DOES NOT CLAIM — `hVASpace` is READ, never assumed
//!
//! `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS` documents `hVASpace` as *"handle for the
//! allocated VA space … **If it's 0, it assumes to use the implicit allocated VA space
//! associated with the client/device pair**"* (`ogkm-580: ctrl0080dma.h:812-815`).
//!
//! ⊘ **That it IS 0 in this boot is an inference from header semantics and has never been
//! measured**, and the counter-hypothesis is live: `0x801813` is UVM's transport for
//! *user* VA spaces, which are not the Device's implicit one. Nothing here writes
//! "the Device's default VA space" into a name, a refusal or a log string. The handle is
//! **reported as it arrived** ([`SetPageDirRecord::h_vaspace`], beside
//! [`SetPageDirLog::valid`]) and the boot is left to say which it is.
//!
//! ★★★ [`SetPageDirLog::valid`] is load-bearing exactly here and is not a convenience:
//! `h_vaspace == 0` is a **real handle value meaning the implicit VAS**, so without a
//! separate "a record is latched" bit, a reported `0` cannot be told from "no `SET` ever
//! arrived". ⊘ That is the `fb_resident_valid` argument, and it is sharper here because
//! the ambiguous value is the *interesting* one.
//!
//! # ⚠ Serving is NECESSARY and is not claimed to be SUFFICIENT
//!
//! Answering `NV_OK` only gets RM **past** `dma.c:508-520`. `gvaspaceExternalRootDirCommit`
//! then runs locally and can still fail on any of eight of its own checks
//! (`ogkm-580: gpu_vaspace.c:3057, 3067, 3085, 3088, 3093, 3094, 3097, 3109`) — notably
//! `:3109`, `NV_ASSERT_OR_RETURN((pGVAS->flags & VASPACE_FLAGS_SHARED_MANAGEMENT) ||
//! vaspaceIsExternallyOwned(pVAS), NV_ERR_NOT_SUPPORTED)`. A failure there takes the same
//! `SLI_LOOP_BREAK` into the same rollback and fires **the same assert at `:3332`**.
//!
//! ⇒ ★★★★ **The falsifier §16.29.6 wrote is too strong and this module refuses to inherit
//! it.** *"If the assert survives, §16.29.4 is refuted"* conflates *"the RPC was the
//! blocker"* with *"the RPC was the only blocker"*. The discriminator is that every one of
//! those eight is an `NV_ASSERT*` and therefore **logs its own file:line**. See §16.30 for
//! the three-way reading; in short, an assert at `:3332` accompanied by a **new** assert
//! from `gpu_vaspace.c:3057-3109` **confirms** the chain and moves the wall inside
//! `commit`, whereas `:3332` **alone and unaccompanied** is what refutes §16.29.4.
//!
//! ★ Note also that the assert at `:3332` is only reachable on a VAS that is **not**
//! externally owned — `gvaspaceExternalRootDirRevoke` returns early for one at
//! `gpu_vaspace.c:3320-3328`. So the boot that fired it already told us something about
//! the VA space, and `:3109` is where that fact gets tested a second time.
//!
//! # ⊘ This link RECORDS what it answers
//!
//! `execution_plane_increments.md` §16.29.5b: *"answering `NV_OK` while recording nothing
//! would be a refusal wearing an acceptance's clothes — the guest would proceed believing
//! its root is installed GSP-side."* Every accepted `SET` is latched into
//! [`SetPageDirLog`] and crosses the shim into the boot census, so the acceptance is
//! auditable from the QEMU log alone.
//!
//! ⊘⊘ **What it does NOT do**, deliberately and for [`crate::gvaspub`]'s reason exactly:
//! it does not create a `Vas`, does not populate `Channel::vas_pdb`, and does not relax
//! any downstream refusal. A served-but-inert data path is this project's forbidden shape;
//! granting a channel a VAS it cannot execute against converts a *loud* refusal into a
//! *silent* timeout. This link's entire output is a record and a status.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kayfabe_abi::generated::ctrl::{
    NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, Nv0080CtrlDmaSetPageDirectoryParams,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_abi::view::PdbAperture;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;

/// `NV_ERR_INVALID_ARGUMENT` (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h`).
///
/// ★ One of the four statuses `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` documents itself as
/// returning (`ogkm-580: ctrl0080dma.h:821-826`: `NV_OK`, `NV_ERR_INVALID_ARGUMENT`,
/// `NV_ERR_INVALID_LIMIT`, `NV_ERR_GENERIC`). ⊘ Deliberately **not**
/// `NV_ERR_NOT_SUPPORTED`: this control *is* supported here, and a malformed request is
/// the guest's error rather than a gap in this port. A refusal must be by a name that is
/// true.
const NV_ERR_INVALID_ARGUMENT: u32 = 0x0000_001F;

/// One `SET_PAGE_DIRECTORY` exactly as it arrived — the RPC header's two handles, and all
/// seven params fields.
///
/// ★ **Whole, not a summary.** The rung this record exists for turns on `h_vaspace`, but
/// `num_entries` decides `gpu_vaspace.c:3093-3097` and `flags` decides `:3085` and
/// `:3109`'s sibling, so a record that kept only the address would be unable to explain
/// the *next* failure. `ch_id`/`sub_device_id`/`pasid` are carried for the same reason
/// [`crate::gvaspub::GvasPublication`] keeps every level: a record that drops a field is
/// the second-best source about its own subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPageDirRecord {
    /// `hClient` from the RPC control header — the namespace the `SET` was issued in.
    ///
    /// ★ Read off the header and never off a params field. That is the exact defect
    /// `gsp_core_bridge.md` §3.2 records against the C's `GPU_PROMOTE_CTX` handler.
    pub client: u32,
    /// `hObject` from the RPC control header.
    ///
    /// ⚠ For this control that is **`hDevice`**, not the VA space: RM issues it as
    /// `NV_RM_RPC_CONTROL(pGpu, hClient, hDevice, …)` (`ogkm-580: dma.c:508-518`). The VA
    /// space is named by [`Self::h_vaspace`] instead — which is the opposite convention
    /// from `0x90f10106`, where the header's `hObject` **is** the VA space
    /// (`gpu_vaspace.c:5174-5177`). ⊘ Two page-directory publications, two different
    /// places to look for the owner; conflating them is how a root gets attributed to the
    /// wrong address space.
    pub object: u32,
    /// `physAddress` — where the new page directory lives, in the aperture named by
    /// [`Self::aperture`].
    ///
    /// ⊘ Guest-physical, per `no_real_phys_only_gpga_or_gpa`; it is stored, never
    /// dereferenced here.
    pub phys_address: u64,
    /// `numEntries` — the directory's size in entries.
    pub num_entries: u32,
    /// `flags`, raw — aperture, `ALL_CHANNELS`, `EXTEND_VASPACE`, `IGNORE_CHANNEL_BUSY`.
    pub flags: u32,
    /// [`Self::flags`]'s aperture field, decoded.
    pub aperture: PdbAperture,
    /// ★★★ `hVASpace` — **reported, not interpreted.** `0` names the client/device pair's
    /// implicit VA space rather than meaning "absent" (`ogkm-580: ctrl0080dma.h:812-815`),
    /// so a consumer must never route it into an "unknown handle" arm. Whether this boot
    /// sends `0` or a real handle is a question for the boot, not for this comment.
    pub h_vaspace: u32,
    /// `chId` — the channel to update. Ignored by RM when `ALL_CHANNELS` is set.
    pub ch_id: u32,
    /// `subDeviceId` — non-zero forces unicast in RM (`ogkm-580: dma.c:494-501`).
    pub sub_device_id: u32,
    /// `pasid` — ignored by RM unless the VA space has ATS enabled
    /// (`ogkm-580: gpu_vaspace.c:3104-3108`).
    pub pasid: u32,
}

/// The shared record, cloneable so the plane and the chain link hold the same one — the
/// shape [`crate::bar2::BarPdeLog`] and [`crate::unserviced::UnservicedLog`] already have,
/// and for their reason: the policy chain sits behind the plane's lock and a reporter must
/// read this without taking it.
#[derive(Debug, Clone, Default)]
pub struct SetPageDirLog {
    latest: Arc<Mutex<Option<SetPageDirRecord>>>,
    total: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
}

impl SetPageDirLog {
    /// A fresh log with nothing recorded.
    #[must_use]
    pub fn new() -> SetPageDirLog {
        SetPageDirLog::default()
    }

    /// The most recent accepted `SET`, if any.
    ///
    /// ★ **Most recent wins, and repeats are counted rather than folded.** RM re-publishes
    /// a root on every re-bind, so "which root is current" and "how many times was one
    /// installed" are different questions; [`Self::total`] answers the second.
    #[must_use]
    pub fn latest(&self) -> Option<SetPageDirRecord> {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many `SET_PAGE_DIRECTORY` commands were accepted, including re-publications.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// How many were refused — a payload that does not hold the struct, a guest-declared
    /// `paramsSize` that is not `sizeof`, or FINN serialization this port does not decode.
    ///
    /// ⊘ A counter and not an absent row, for [`crate::bar2::BarPdeLog::refusals`]'
    /// reason: *"the guest published something we could not read"* and *"the guest
    /// published nothing"* are different diagnoses and only one of them is our defect.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// Whether a record is latched at all.
    ///
    /// ★★★ **The precondition, and it is load-bearing rather than decorative.**
    /// `h_vaspace == 0` is a real, meaningful handle value, so a reported `0` with no
    /// `valid` beside it cannot be distinguished from "nothing ever arrived". Every other
    /// field has the same hazard at `0`; this bit is what makes the whole record readable.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.latest().is_some()
    }

    /// Record one accepted `SET`.
    pub fn publish(&self, rec: SetPageDirRecord) {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(rec);
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one refusal.
    pub fn refuse(&self) {
        self.refused.fetch_add(1, Ordering::Relaxed);
    }

    /// ★★★ Power-on: forget the recorded root.
    ///
    /// **Not optional**, for [`crate::bar2::BarPdeLog::device_reset`]'s reason exactly: a
    /// root that survived a device life would be the *previous* guest's page directory,
    /// and it is `#130`'s property quantified over all device state.
    pub fn device_reset(&self) {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.total.store(0, Ordering::Relaxed);
        self.refused.store(0, Ordering::Relaxed);
    }
}

/// Why a `SET_PAGE_DIRECTORY` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetPageDirRefusal {
    /// The params are FINN-serialized; this port decodes only the flat struct.
    ///
    /// ⊘ Refused rather than decoded flat, for [`crate::inittables::InitTablePolicy`]'s
    /// reason: an unchecked flat answer to a serialized request is the kind of wrong that
    /// never logs.
    Serialized,
    /// The guest's declared `paramsSize` is not `sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS)`,
    /// or the payload cannot hold it.
    ///
    /// ★ Checked **exactly**, not as a lower bound: RM's caller passes the `sizeof`
    /// verbatim (`ogkm-580: dma.c:508-518`), so a different declared size is a guest that
    /// means a different struct.
    SizeMismatch {
        /// What the guest declared.
        declared: u32,
        /// How many bytes of payload actually followed the header.
        available: usize,
    },
}

impl SetPageDirRefusal {
    /// One sentence, for the shell to print verbatim.
    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Self::Serialized => {
                "a FINN-serialized SET_PAGE_DIRECTORY; this port decodes only the flat \
                 struct and answering a serialized request with a flat reply would be a \
                 wrong answer that never logs"
            }
            Self::SizeMismatch { .. } => {
                "a SET_PAGE_DIRECTORY whose declared paramsSize is not sizeof the struct \
                 RM's own caller passes; a different size is a guest that means a \
                 different struct"
            }
        }
    }
}

/// The chain link that answers `0x00801813` and latches the root it carries.
#[derive(Debug, Clone)]
pub struct SetPageDirPolicy {
    driver: DriverAbiTable,
    log: SetPageDirLog,
}

impl SetPageDirPolicy {
    /// Bind the policy to a driver version and a shared log.
    #[must_use]
    pub fn new(driver: DriverAbiTable, log: SetPageDirLog) -> SetPageDirPolicy {
        SetPageDirPolicy { driver, log }
    }

    /// The driver version this link answers as.
    #[must_use]
    pub fn driver(&self) -> DriverAbiTable {
        self.driver
    }
}

impl CommandPolicy for SetPageDirPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        // A payload too short to hold the control header is not a control this link can
        // classify; decline rather than invent a refusal for a message that may not be one.
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY {
            return None;
        }
        // ⊘ From here on every arm ANSWERS. Declining after recognising the id would put
        // `0x801813` back on the unserviced ledger while this link claimed to serve it —
        // the shape where a census says "refused by name" and no name exists.
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            self.log.refuse();
            return Some(Reply {
                rpc_result: NV_ERR_INVALID_ARGUMENT,
                body: Vec::new(),
            });
        }
        let want = Nv0080CtrlDmaSetPageDirectoryParams::SIZE;
        let available = cmd.payload.len().saturating_sub(req.params_at);
        if req.params_size as usize != want || available < want {
            self.log.refuse();
            return Some(Reply {
                rpc_result: NV_ERR_INVALID_ARGUMENT,
                body: Vec::new(),
            });
        }
        let Ok(p) = Nv0080CtrlDmaSetPageDirectoryParams::decode(&cmd.payload[req.params_at..])
        else {
            // Unreachable given the length check above, but written as a refusal rather
            // than an `unwrap`: the two checks are independent statements about the same
            // bytes and only one of them is the guest's.
            self.log.refuse();
            return Some(Reply {
                rpc_result: NV_ERR_INVALID_ARGUMENT,
                body: Vec::new(),
            });
        };
        self.log.publish(SetPageDirRecord {
            client: req.client,
            object: req.object,
            phys_address: p.phys_address,
            num_entries: p.num_entries,
            flags: p.flags,
            aperture: PdbAperture::from_flags(p.flags),
            h_vaspace: p.h_va_space,
            ch_id: p.ch_id,
            sub_device_id: p.sub_device_id,
            pasid: p.pasid,
        });
        // ★ An **empty** body with `NV_OK`. Every field of this struct is `[IN]` — the
        // header documents no output (`ogkm-580: ctrl0080dma.h:785-826`) and RM's caller
        // reads nothing back from it — so there is nothing to reflect, and reflecting the
        // request would be the echo this port stopped doing (task #127).
        Some(Reply {
            rpc_result: NV_OK,
            body: Vec::new(),
        })
    }
}

kayfabe_util::assert_send_sync!(SetPageDirRecord, SetPageDirLog, SetPageDirRefusal);
