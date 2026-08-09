//! GA10x (GA106) — the [`Arch`] the **shipped port** classifies objects with.
//!
//! ## ★★★ Why this is not `Ad10xArch` with a different name
//!
//! [`crate::Ad10xArch`] and [`crate::Gh100Arch`] are `MockArch` in every respect except
//! their GSP register model, and that composition is right for what they are for: a
//! second and third generation measured against the first, in a workspace where nothing
//! shipped an `Arch` at all. This one is different in the only way that matters — **it is
//! linked into the QEMU archive a real guest driver talks to** — and two of `MockArch`'s
//! answers are wrong there rather than merely provisional:
//!
//! - **`classify`.** `MockArch`'s table is keyed on `mock_classes`, which are *invented*
//!   ids in the `0xF0xx` range. Handed NVIDIA's real `NV01_ROOT` (`0x0`) or
//!   `NV20_SUBDEVICE_0` (`0x2080`) it answers [`ObjectKind::Unknown`], and an
//!   `RmGraph` that cannot tell a client root from a subdevice cannot enforce a single
//!   one of its parenting rules. `kayfabe_mocks::WireClassArch` exists precisely because
//!   a wire-bytes test needed the real ids — but it does not name `NV20_SUBDEVICE_0`.
//!   ⊘ The reason given here used to *begin* *"but it is in `kayfabe-mocks`, whose
//!   manifest says 'Test-only; never a production dependency'"* — a premise this crate
//!   itself falsifies, since it depends on `kayfabe-mocks` on a normal edge and
//!   `kayfabe-qemu-raw` depends on this one. The missing class id is the whole reason,
//!   and it is enough of one (`docs/design/mock_fidelity_audit.md` finding G).
//! - **the data-plane seams.** `MockArch`'s `mmu()`, `userd()` and `pushbuffer()` answer
//!   with made-up geometry and a made-up doorbell encoding. In a *test* that is the point.
//!   In the product it is the measured "mock wall" in its worst form: a plausible answer
//!   on the one axis where a wrong answer is a silent memory-safety fact about the guest.
//!
//! ## ★★★ The data plane, seam by seam — and what still refuses
//!
//! Since `E4` the USERD model ([`Ga10xUserd`]) and the pushbuffer codec
//! ([`Ga10xPushbuffer`]) are **built**; [`UnbuiltUserd`] / [`UnbuiltPushbuffer`] remain as
//! the vocabulary's own spelling of a refusal, as [`UnbuiltGmmu`] did after `#149`, and
//! are no longer what this `Arch` answers with.
//!
//! ★★★ **`E5` closed the seam `E4` refused at, and the shape of the fix is the finding.**
//! A real `LAUNCH_DMA` carries **none** of its operands — they were written by *earlier,
//! separate* method runs — so [`PushbufferAbi::decode_method`], handed one
//! `(header, args)` pair and nothing else, was *structurally incapable* of producing a
//! [`PushMethod::CeLaunchDma`] with real `dst`/`src`/`len`. `MockArch` hid that for the
//! whole life of the seam by inventing a one-method encoding that packs both operands, a
//! length and a work kind into a single method's arguments.
//!
//! The owner's ruling (`execution_plane_increments.md` §8.2.1) was that statelessness was
//! never a requirement — *"if the protocol holds a state to emulate an action, so may
//! you"* — so [`PushbufferAbi::decode_run`] carries the channel's own
//! [`kayfabe_arch::MethodState`] and [`Ga10xPushbuffer::decode_run`] accumulates
//! `SET_OBJECT` → `OFFSET_*` → `LINE_*` → `LAUNCH_DMA` into one fact. ⊘ The refusals grew
//! rather than shrank: a launch whose operands never arrived, a multi-line copy, a
//! constant fill and a zero-transfer launch all still decode to [`PushMethod::Opaque`].
//!
//! ★ **And the seam upstream of all of it is CLOSED (§8.2.3).** A GPFIFO entry names a
//! GPU **virtual** address — `[measured]`, see [`Ga10xPushbuffer::gpfifo_entries`] — and
//! [`kayfabe_arch::PushRange::va`] now says so in the type. `kayfabe_fwd::read_pushbuffer`
//! resolves it through the issuing channel's address table, MISS = FAULT, before any
//! guest byte is fetched.
//!
//! ★★★ **`decode_doorbell` is now built (`E3`), and it is the one seam here that could
//! not have been closed by reading.** It used to answer `None`. It answers a
//! `(runlist, chid)` pair now because the encoding was *measured*, not because the
//! appetite grew: `execution_plane_increments.md` §2.1 argues this is the riskiest
//! increment in the whole execution plane precisely because a wrong doorbell decode
//! **cannot fail loudly** — on the Mode-2 path we are the GSP, so a ring routed to the
//! wrong channel has no second party to notice. The mock cannot catch it
//! (`MockArch::token_for` is the inverse of the mock's own decode) and the C artifact
//! cannot either (`c_rust_trace_differential.md`: the completion plane has **no** C
//! oracle). See [`Ga10xArch::decode_doorbell`] for the two instruments that did, and
//! `docs/design/doorbell_token_encoding.md` for what they do and do not settle.
//!
//! ★★ **The page-table format is now real** ([`Ga10xGmmu`]), and the reason is a boot, not
//! an appetite. `kbusVerifyBar2_GM107` writes sixteen bytes through the **BAR2 CPU
//! mapping** — a GMMU-*translated* aperture — and reads them back through the untranslated
//! BAR0 window; on 2026-08-01 the guest read `0x0` and `RmInitAdapter` failed with
//! `NV_ERR_MEMORY_ERROR`. There is no way past that statement without translating an
//! address, and translating an address without a format is exactly the invention this
//! module refuses to make. So the format is transcribed from the driver's own level table
//! (`ogkm-580: kern_gmmu_fmt_gp10x.c`, `kern_gmmu_fmt_ga10x.c`) with its two GA10x traps
//! encoded rather than commented — see [`Ga10xGmmu`].
//!
//! ⊘ None of this makes the data plane *complete*. Nothing here submits work; what exists
//! is an address decoder, a token decoder, a USERD geometry and a method framer, and every
//! input none of them can express is still a loud refusal.
//!
//! ## ⊘ `gsp()` is `None`, deliberately, and it is not the same GSP question
//!
//! GA10x's GSP **register** model is `kayfabe_device::ga10x::Ga10xGspModel`, where stage
//! Q4 moved it on 2026-07-31 so that the shipped archive and the `cap1` differential read
//! through one encoder. `kayfabe_device::RegPlane` takes it from the `ChipProfile`, never
//! from an `Arch`. Answering `Some` here would require a second copy of that model in a
//! second crate — two descriptions of one chip that can disagree — to satisfy a seam
//! nothing on this path calls (`Arch::gsp` has exactly two consumers, both inside
//! `kayfabe_gsp::boot`, which this value never reaches). `None` is the honest answer:
//! *this `Arch` does not carry a GSP register model.*

use kayfabe_abi::generated::classes as nv;
use kayfabe_abi::submit;
use kayfabe_arch::gsp::GspModel;
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuVa, Pdb, RunlistId, VChid};
use kayfabe_arch::{
    Aperture, Arch, CeWork, DoorbellTarget, GmmuFmt, GmmuVersion, HostClasses, LevelShift,
    MethodState, ObjectKind, PageSize, PdeEdge, PteDecode, PushMethod, PushRange, PushbufferAbi,
    UserdModel,
};

/// The GA10x architecture, as the port ships it: a **real** class table and a **real**
/// page-table format over a data plane that is otherwise still refusing. See the module
/// docs, and [`Ga10xGmmu`] for what changed and why.
#[derive(Debug, Default)]
pub struct Ga10xArch {
    mmu: Ga10xGmmu,
    userd: Ga10xUserd,
    push: Ga10xPushbuffer,
}

impl Ga10xArch {
    /// The architecture.
    #[must_use]
    pub fn new() -> Ga10xArch {
        Ga10xArch::default()
    }
}

impl Arch for Ga10xArch {
    /// ★ Names the refusal in the string, because this value's `name()` is what
    /// `Spine`'s multi-GPU homogeneity guard reports and what a `Debug` of the object
    /// model prints. An operator reading "GA10x (GA106)" would have no way to know the
    /// data plane is not there.
    fn name(&self) -> &'static str {
        "GA10x (GA106, object model + GMMU + doorbell + USERD/pushbuffer + CE launch operands; \
         pushbuffer ranges name a GPU VA nothing resolves)"
    }

    /// NVIDIA's real class ids. The constants are `kayfabe-abi`'s, per decision #2's
    /// quarantine — this crate transcribes none of them.
    ///
    /// ★ `NV01_ROOT` and `NV01_ROOT_CLIENT` are **one resource kind**: RM's own
    /// `is_client_root_class` says so, which is why the newer spelling does not get an
    /// arm of its own.
    ///
    /// ★★ The channel arm is where an `Arch` runs out of information, and the shape is a
    /// protocol fact rather than this port's limitation. There is exactly ONE GPFIFO
    /// channel class per architecture: a CUDA process's GR channel and its CE channel are
    /// both `AMPERE_CHANNEL_GPFIFO_A`, separated by `NV_CHANNEL_ALLOC_PARAMS.engineType`
    /// — a *params* fact `RmEvent::Alloc` has nowhere to carry. So the class maps to
    /// [`EngineKind::GrCompute`] and a CE channel becomes one only when its
    /// `AMPERE_DMA_COPY_B` engine object arrives and the core's refinement pass rewrites
    /// it.
    ///
    /// ⊘ An id not named here is [`ObjectKind::Unknown`] — **not** a guess. The graph
    /// stores an unknown-kind node and enforces no parenting rule it cannot justify,
    /// which is the right failure: a wrong `ObjectKind` would put a node under a rule it
    /// does not belong to and the guest would never see why.
    fn classify(&self, class: ClassId) -> ObjectKind {
        match class.0 {
            nv::NV01_ROOT | nv::NV01_ROOT_CLIENT => ObjectKind::Client,
            nv::NV01_DEVICE_0 => ObjectKind::Device,
            nv::NV20_SUBDEVICE_0 => ObjectKind::Subdevice,
            nv::NV01_EVENT_KERNEL_CALLBACK_EX => ObjectKind::Event,
            nv::FERMI_VASPACE_A => ObjectKind::VaSpace,
            nv::KEPLER_CHANNEL_GROUP_A => ObjectKind::Tsg,
            nv::FERMI_CONTEXT_SHARE_A => ObjectKind::CtxShare,
            nv::AMPERE_CHANNEL_GPFIFO_A => ObjectKind::Channel {
                engine: EngineKind::GrCompute,
            },
            nv::AMPERE_COMPUTE_B => ObjectKind::EngineObject {
                engine: EngineKind::GrCompute,
            },
            // ★★ `AMPERE_B` is the **3D** object on the SAME engine (`ENG_GR(0)`), and it
            // is labelled `GrGraphics` rather than `GrCompute` because that is what it is:
            // RM picks it over the compute class precisely when `kgraphicsIsGFXSupported`
            // (`ogkm-580: kernel_graphics.c:2502-2513`).
            //
            // ⊘ The label is **checked to be inert**, not assumed to be: every consumer
            // that reads an `EngineKind` treats the two GR variants identically —
            // `kayfabe_abi::rc::EngineRoute::for_engine` (`rc.rs:115`) maps both to
            // `NV2080_ENGINE_TYPE_GRAPHICS`, `kayfabe_isolate_host::rm::engine_type_for`
            // (`rm.rs:1654`) to the same `ENGINE_TYPE_GRAPHICS`, and
            // `kayfabe_fwd::completion_arm` (`fwd:4159`) splits only `NvEnc` out. So the
            // refinement this enables rewrites the golden-image channel's label and
            // changes no decision — which is the reason the truthful name is affordable.
            nv::AMPERE_B => ObjectKind::EngineObject {
                engine: EngineKind::GrGraphics,
            },
            nv::AMPERE_DMA_COPY_B => ObjectKind::EngineObject {
                engine: EngineKind::Ce,
            },
            // ★★★ `NV2081_BINAPI` — an **opaque leaf under a Subdevice**, and
            // [`ObjectKind::Other`]'s first constructor in this port. That variant's own doc
            // is *"anything else the graph tracks as a plain node"*, which is exactly what
            // this is: RM registers it `Parents = RS_LIST(classId(Subdevice))`,
            // `RS_FLAGS_ALLOC_NON_PRIVILEGED`, and every control on it is tunnelled whole to
            // GSP without the kernel reading it
            // (`ogkm-580: resource_list.h:439-448`, `rmapi/binary_api.c:61-127`).
            //
            // ⊘ The label is **checked to be inert**, not assumed to be — the hazard that
            // bit §14.21 and §14.24 runs in this direction. `ObjectKind::Other` and
            // `ObjectKind::Unknown` are each matched in **zero** places in the tree: the
            // graph branches only on `Device`, `Client`, `Memory` and `Event`
            // (`rmgraph.rs:1346, 1429, 2372, 2399`), the projection only on `VaSpace`,
            // `Tsg`, `CtxShare`, `Channel` and `EngineObject` (`project.rs:726, 758`), and
            // `origin_of_kind` (`rmgraph.rs:2228`) is a discriminant compare no caller can
            // aim at this variant. So the change from `Unknown` to `Other` moves no
            // decision, and that is what makes the truthful name affordable.
            //
            // ⚠ ⊘ NOT `EngineObject` — that is the one variant that rewrites a **sibling**
            // node's routing (`project.rs:726-739`), and this class has no engine.
            nv::NV2081_BINAPI => ObjectKind::Other,
            _ => ObjectKind::Unknown,
        }
    }

    /// ★★★ **The GA10x `USERD_INDEX` chid recovery, landed.** See
    /// [`decode_userd_index_chid`] — this is a thin forward to it so the free function can
    /// be differentialled directly.
    ///
    /// The refusal this replaced (`VChid(0)` for every input) is gone because it had become
    /// a boot blocker: more than one channel exists on the path to first compute, and
    /// collapsing them all to one vChid turns *every* second channel into a
    /// `ProjectionError::VchidCollision`.
    fn vchid_from_userd_flags(&self, flags: u32) -> Option<VChid> {
        decode_userd_index_chid(flags)
    }

    /// ★★★ **E3 — the GA10x work-submit token, decoded.** Two fields and nothing else:
    /// `VECTOR` = chid in bits **11:0**, `RUNLIST_ID` in bits **22:16**.
    ///
    /// # Where the encoding comes from — and which instrument settled it
    ///
    /// It is RM's, and RM writes it in two lines
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c`,
    /// `kfifoGenerateWorkSubmitTokenHal_GA100`):
    ///
    /// ```text
    /// val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _RUNLIST_ID, runlistId, val);
    /// val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _VECTOR,     chId,      val);
    /// ```
    ///
    /// with the field positions in the tree's own
    /// `src/common/inc/swref/published/ampere/ga100/dev_ctrl.h:26-27`, and GA106 bound to
    /// that `_GA100` implementation by the driver's own dispatch table
    /// (`g_kernel_fifo_nvoc.c:649-652`, `ChipHal: GA100 | GA102 | … | GA106 | …`).
    ///
    /// ⊘ **None of the three sentences above is what makes this decoder trustworthy** —
    /// they are a reading, and a transcribed reading is the exact failure
    /// `isolate_the_drivers_own_checks` names. Two instruments do:
    ///
    /// 1. `tests/tests/doorbell_token.rs::ga106_hardware_tokens_decode_to_rms_own_chids`
    ///    replays tokens a **real GA106 handed real channels**, each paired with a
    ///    `(runlistId, chid)` that came from RM's *channel-ID manager* — a
    ///    `NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS` bitmask diffed across the
    ///    allocation — so the expected value never passes through this function, its
    ///    inverse, or the token.
    /// 2. `tests/tests/worksubmit_token_oracle.rs` compiles
    ///    `kfifoGenerateWorkSubmitTokenHal_GA100` *itself* and differentials this decoder
    ///    against it over the whole field space, which is what pins the two **widths**
    ///    (hardware only ever showed small values).
    ///
    /// # What makes a fabricated token `None`
    ///
    /// RM's encoder starts from `val = 0` and sets exactly those two fields, so bits
    /// **15:12**, **31:23** and everything above 32 are zero in *every* token RM can
    /// produce. A token with any of them set was not generated by the driver, and the
    /// refusal is therefore derived from the encoder rather than invented as a
    /// plausibility rule. ⊘ It is a **necessary**, not sufficient, condition: 0x0000_0004
    /// is well-formed whether or not any channel holds chid 4, and only the core's
    /// exec-plane index can answer that (`FwdFault::UnknownVchid`).
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        decode_work_submit_token(token)
    }

    fn mmu(&self) -> &dyn GmmuFmt {
        &self.mmu
    }

    fn userd(&self) -> &dyn UserdModel {
        &self.userd
    }

    /// `false` for every control: nothing here is ACK-only, so every control travels the
    /// ordinary path and is answered — or refused — by name.
    fn is_case2_control(&self, _cmd: ControlCmd) -> bool {
        false
    }

    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        &self.push
    }

    /// `None`. See the module docs: the GA10x GSP **register** model is
    /// `kayfabe_device::ga10x::Ga10xGspModel` and reaches the FSM through the
    /// `ChipProfile`, not through here.
    fn gsp(&self) -> Option<&dyn GspModel> {
        None
    }

    /// The host-forwarding class profile ([`crate::host_classes::Ga10xHostClasses`]).
    ///
    /// ★ This is the ONE `Arch` method that describes a *different* GPU: every other
    /// answer here is about the emulated board the guest drives, and this one is about
    /// the real board the isolate forwards to. They are the same generation on this
    /// bench and that is a coincidence of hardware, not a property.
    fn host_classes(&self) -> Option<&dyn HostClasses> {
        Some(&crate::host_classes::Ga10xHostClasses)
    }
}

// ── ★★★ The USERD_INDEX chid recovery — `vchid_from_userd_flags`, landed ──────────────

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE`, `10:8` — the channel's index **within** a
/// USERD page (`ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:184`).
const USERD_INDEX_VALUE_SHIFT: u32 = 8;
/// Ditto, width. ★★ This number does **double duty** and that is the whole subtlety of
/// this decode — see [`USERD_CHANNELS_PER_PAGE`].
///
/// ⊘ **MEASURED 2026-08-03 (mutation, not a reading): this width is NOT pinned as a mask.**
/// Widening the mask alone — `& ((1 << (WIDTH + 1)) - 1)`, leaving the multiplier at 8 —
/// leaves the whole oracle sweep green. The reason is principled rather than a gap in the
/// sweep: the extra bit is [`USERD_INDEX_FIXED_SHIFT`], and **every** flags word in which
/// that bit is set is one this function refuses *before* the mask runs (`_PAGE_FIXED` with
/// `_FIXED` is `NV_ERR_INVALID_STATE`; `_FIXED` without `_PAGE_FIXED` names no chid). So
/// there is no input on which a four-bit read differs from a three-bit one, and no test can
/// distinguish them. What the sweep *does* pin is the width's other job — widening it as a
/// constant moves [`USERD_CHANNELS_PER_PAGE`] to 16 and goes red immediately.
const USERD_INDEX_VALUE_WIDTH: u32 = 3;

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_FIXED`, `11:11` — sits **between** the two halves of
/// the chid (`alloc_channel.h:186`), which is why this is not a contiguous field read.
const USERD_INDEX_FIXED_SHIFT: u32 = 11;

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_VALUE`, `20:12` — the USERD **page**
/// (`alloc_channel.h:201`).
const USERD_INDEX_PAGE_VALUE_SHIFT: u32 = 12;
/// Ditto, width.
const USERD_INDEX_PAGE_VALUE_WIDTH: u32 = 9;

/// `NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED`, `21:21` (`alloc_channel.h:203`).
const USERD_INDEX_PAGE_FIXED_SHIFT: u32 = 21;

/// **The multiplier**, and the single most load-bearing number in this file.
///
/// ## ★★★ There are TWO of these, reached by two unrelated routes
///
/// This one is the **writer's**: CPU-RM splits the chid with
/// `numChannelsPerUserd = NVBIT(DRF_SIZE(NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE))`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2793`) — a number that
/// comes from **the width of a flag field**, which is why it is written here as
/// `1 << USERD_INDEX_VALUE_WIDTH` and not as a literal.
///
/// Physical RM does **not** recover the chid with it. The reader
/// (`kchannelAllocHwID_GM107`, `kernel_channel_gm107.c:456-476`) extracts the two
/// subfields and passes them down *separately*; the recombination happens one frame later
/// in `kfifoChidMgrAllocChid_IMPL` (`kernel_fifo.c:782-785`) as
/// `userdPageIdx * pChidMgr->pGlobalChIDHeap->ownerGranularity + internalIdx`, and that
/// `ownerGranularity` was set from `RM_PAGE_SIZE / userdBar1Size` (`kernel_fifo.c:338-341`)
/// with `userdBar1Size` from the halified `kfifoGetUserdSizeAlign` — `1 <<
/// NV_RAMUSERD_BASE_SHIFT` = 512 on the `_GM107` implementation GA106 binds. So the
/// reader's multiplier is **a page size divided by the size of a USERD**.
///
/// 4096 / 512 = 8 = `1 << 3`. The two agree on GA106 **as a fact, not as a definition**.
///
/// ⊘ Nothing above is what makes this trustworthy — it is a reading, and a transcribed
/// reading is the failure `isolate_the_drivers_own_checks` names.
/// `tests/tests/userd_chid_oracle.rs` is the instrument: it compiles RM's writer, RM's
/// reader, RM's recombination **and** RM's granularity computation out of the driver's own
/// files, runs a chid round trip through them, and asserts that the granularity the
/// driver's own eheap ended up holding equals this constant. That assertion is what
/// *demonstrates* the equality above instead of assuming it, and it is what will go red on
/// the release or the chip where the two part company.
///
/// ★ `pub` on purpose, and it is the ONE constant here that is: the differential in
/// `tests/tests/userd_chid_oracle.rs` has to compare *our* multiplier against the
/// granularity NVIDIA's own eheap ended up holding, and a test that re-derived it from
/// `1 << 3` would be comparing the driver against a second transcription instead of
/// against us.
pub const USERD_CHANNELS_PER_PAGE: u32 = 1 << USERD_INDEX_VALUE_WIDTH;

/// Recover a channel's chid from the `NVOS04_FLAGS` word CPU-RM put on the
/// `ALLOC_CHANNEL` RPC — or `None` when the word does not name a channel at all.
///
/// # The two `None` arms are RM's, not ours
///
/// `kchannelAllocHwID_GM107` has three shapes, and only one of them yields a chid
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:456-476`):
///
/// 1. `_PAGE_FIXED = _TRUE` and `_FIXED = _FALSE` → `bForceUserdPage`, the two subfields
///    are extracted, and the recombination names a chid. This is what CPU-RM's GSP-client
///    path always writes (`kernel_channel.c:2795-2802` sets `_FIXED=_FALSE` and
///    `_PAGE_FIXED=_TRUE` in the same breath as the values).
/// 2. `_PAGE_FIXED = _TRUE` **and** `_FIXED = _TRUE` → RM's own
///    `NV_ASSERT_OR_RETURN(…, NV_ERR_INVALID_STATE)` refuses the allocation outright.
/// 3. `_PAGE_FIXED = _FALSE` → no page is forced. `_FIXED = _TRUE` then asks for a
///    specific index *within a page RM will choose*, which is a constraint on the
///    allocation and **not a chid**; `_FIXED = _FALSE` asks for nothing at all. In both
///    cases `kfifoChidMgrAllocChid` picks the chid off the heap, so the flags word does
///    not carry one.
///
/// Arms 2 and 3 are why this returns `Option` and why the trait method was widened. A
/// `VChid` return had to answer *something* for them, and boundary-1 posture is that guest
/// bytes are hostile: a guest can set those bits, and inventing a channel number for a
/// word that names none is exactly the silent mis-route this decode exists to avoid.
///
/// # ⊘ There is deliberately NO upper bound on the chid here
///
/// The real bound is `kfifoChidMgrGetNumChannels` — runtime state that depends on the
/// chip's FIFO configuration and that nothing at this seam has. So the only bound applied
/// is the one the **field widths** impose: `_PAGE_VALUE` is nine bits and the multiplier
/// is eight, so the widest chid this encoding can express is `511 * 8 + 7 = 4095`, and a
/// larger one is not representable rather than rejected. ⊘ A plausible-looking constant
/// (`4096`? `2048`?) would be an invented bound wearing the shape of a sourced one; the
/// caller (`kayfabe_core`'s exec-plane index) is what answers "is there a live channel
/// there", and it answers with a miss.
///
/// ★ Note what this makes of RM's own round trip. The writer divides the chid by 8 and
/// `FLD_SET_DRF_NUM` masks the quotient into nine bits, so **CPU-RM itself cannot express
/// a chid ≥ 4096 in these flags**: it writes `((chid / 8) & 0x1FF) * 8 + (chid % 8)`, and
/// `4096` goes on the wire as `0`. The oracle sweeps past that edge and records the
/// writer's chid and the reader's chid *separately* for exactly this reason — it is a
/// lossiness in the driver, not a disagreement with this decoder.
#[must_use]
pub fn decode_userd_index_chid(flags: u32) -> Option<VChid> {
    let page_fixed = (flags >> USERD_INDEX_PAGE_FIXED_SHIFT) & 1 != 0;
    let index_fixed = (flags >> USERD_INDEX_FIXED_SHIFT) & 1 != 0;
    // Arm 3: nothing forces a USERD page, so the flags name no channel.
    if !page_fixed {
        return None;
    }
    // Arm 2: RM answers NV_ERR_INVALID_STATE. So do we — with a refusal, not a chid.
    if index_fixed {
        return None;
    }
    let value = (flags >> USERD_INDEX_VALUE_SHIFT) & ((1 << USERD_INDEX_VALUE_WIDTH) - 1);
    let page = (flags >> USERD_INDEX_PAGE_VALUE_SHIFT) & ((1 << USERD_INDEX_PAGE_VALUE_WIDTH) - 1);
    // RM's own expression, in RM's own order: page * granularity + index-within-page.
    let chid = page * USERD_CHANNELS_PER_PAGE + value;
    // Lossless by the two masks above: 9 bits times 8 plus 3 bits is at most 4095.
    Some(VChid(chid as u16))
}

/// ★★★ The work-submit-token decode, **shared by three generations** — and the sharing
/// is sourced, not assumed (`#156`).
///
/// [`Ga10xArch::decode_doorbell`] holds the evidence for this encoding and the two
/// instruments that settled it. What this function adds is the *scope* of that evidence:
/// it is not GA10x's encoding, it is the encoding RM uses on **GA10x, AD10x and GH100
/// alike**, and it lives at file scope so `ad10x` and `gh100` can call it instead of
/// delegating to `MockArch`'s invented one.
///
/// ## The citation, two independent halves
///
/// 1. **The generator is one function for all three.** The driver's own dispatch table
///    binds `kfifoGenerateWorkSubmitTokenHal_GA100` to
///    `ChipHal: GA100 | GA102 | GA103 | GA104 | GA106 | GA107 | AD102 | AD103 | AD104 |
///    AD106 | AD107 | GH100` (`ogkm-580:
///    src/nvidia/generated/g_kernel_fifo_nvoc.c:648-652`). Blackwell is where it changes
///    — `GB202`-and-later and `GB100` take different HALs (`:644-647`, `:653-656`).
/// 2. **The field positions are one header for all three.** `NV_CTRL_VF_DOORBELL_VECTOR`
///    `11:0` and `_RUNLIST_ID` `22:16` are defined in exactly two places in the tree,
///    `turing/tu102/dev_ctrl.h:36-37` and `ampere/ga100/dev_ctrl.h:26-27`. Neither
///    `ada/ad102/` nor `hopper/gh100/` carries a `dev_ctrl.h` at all, so neither
///    overrides them.
///
/// ## ⊘ What this does NOT establish, and it is the same limit as everywhere else here
///
/// **Compiling for a generation is not booting on one.** The two instruments that settled
/// this encoding — the replayed GA106 hardware tokens and the differential against RM's
/// compiled `_GA100` encoder — both speak about the *implementation* the driver binds to
/// three generations. That is a strictly stronger position than `MockArch`'s invented
/// encoding, which was answering here for Ada and Hopper before `#156` and which
/// `execution_plane_increments.md` §2.1 records as the one wrong answer that **cannot
/// fail loudly**. It is not a run on an Ada or Hopper board.
pub fn decode_work_submit_token(token: u64) -> Option<DoorbellTarget> {
    let raw = u32::try_from(token).ok()?;
    // The two fields RM defines…
    let vector = raw & 0x0000_0FFF; // NV_CTRL_VF_DOORBELL_VECTOR      11:0
    let runlist = (raw >> 16) & 0x0000_007F; // NV_CTRL_VF_DOORBELL_RUNLIST_ID 22:16
    // …and everything RM's encoder cannot have written.
    if raw & !0x007F_0FFF != 0 {
        return None;
    }
    Some(DoorbellTarget {
        // Both casts are lossless by the masks above (12 and 7 bits into a u16).
        vchid: VChid(vector as u16),
        runlist: RunlistId(runlist as u16),
    })
}

/// ★★★ The work-submit-token **encode** — the inverse of [`decode_work_submit_token`], and
/// what `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`0xc36f0108`) must reply with.
///
/// ## Why an encoder exists at all, when the guest computes its own
///
/// For a **userspace** channel the guest never asks us: `0xc36f0108` carries flags `0x10008`
/// (no `ROUTE_TO_PHYSICAL`), so `kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL` generates the
/// token locally from `pKernelChannel->ChID` (`ogkm-580: kernel_channel.c:3283`). The
/// **internal** variant — the scrubber/CeUtils channel — carries `0x10244`, routes to us
/// (`g_kernel_channel_nvoc.c:551`), and we must answer. This function is that answer.
///
/// ⇒ it must produce what the guest would have produced for itself, or the two paths
/// disagree about the same channel.
///
/// ## ★ The vChid is the GUEST's, and that is forced rather than chosen
///
/// The guest kernel allocates its own `ChID` before anything reaches us and smuggles it
/// through `NV_CHANNEL_ALLOC_PARAMS.flags` `USERD_INDEX` — see
/// [`decode_userd_index_chid`], and `execution_plane_increments.md` §13.3. The host driver
/// picks a different chid for the real channel; a guest vChid is meaningless on the host.
/// So the doorbell **write** is where the two are reconciled, and this reply carries the
/// guest's own number so that `Ga10xArch::decode_doorbell` can recover it.
///
/// ## ⊘ Out of range is REFUSED, and RM's truncation is not a counter-example
///
/// RM's encoder **truncates** — `chid = 4096` comes back as `0x00000000` and `runlist = 128`
/// as `0`. ⊘ That is a **differential against RM's own compiled `_GA100` encoder**
/// (`tests/oracle/worksubmit_token_oracle.c`, `#163`), i.e. NVIDIA's code executing, not a
/// live boot — the epistemic level is *its encoder ran and produced these bytes*, which is
/// exactly what this function has to match and is not a claim about a running GPU.
/// Mimicking the truncation would let a wrong number name a *different real channel*
/// silently.
///
/// ★ Refusing is not "stricter than hardware on a reachable input", because the input cannot
/// reach here out of range: [`decode_userd_index_chid`]'s own note proves it — *"9 bits times
/// 8 plus 3 bits is at most 4095"* — which is exactly the 12-bit field. So this refusal is an
/// internal-consistency check on a path the guest cannot drive, not a fidelity departure
/// (`mock_fidelity_both_directions` is about departures on inputs hardware DOES see).
#[must_use]
pub fn encode_work_submit_token(runlist: RunlistId, vchid: VChid) -> Option<u64> {
    // The two field widths RM defines — `NV_CTRL_VF_DOORBELL_VECTOR` 11:0 and
    // `_RUNLIST_ID` 22:16 (`turing/tu102/dev_ctrl.h:36-37`, `ampere/ga100/dev_ctrl.h:26-27`).
    if vchid.0 > 0x0FFF || runlist.0 > 0x007F {
        return None;
    }
    Some((u64::from(runlist.0) << 16) | u64::from(vchid.0))
}

// ── ★★★ The GA10x GMMU, VER2 — `#149`, the first translated aperture ────────────────

/// Level index of the root page directory (`PD3`, virtual address bits 48:47).
const L_PD3: u8 = 0;
/// `PD2`, bits 46:38.
const L_PD2: u8 = 1;
/// `PD1`, bits 37:29 — **and, on this generation only, a 512 MiB leaf**.
const L_PD1: u8 = 2;
/// `PD0`, bits 28:21 — a **16-byte dual** entry, and also a 2 MiB leaf.
const L_PD0: u8 = 3;
/// The big-page table, bits 20:16.
const L_PT_BIG: u8 = 4;
/// The small-page table, bits 20:12.
const L_PT_SMALL: u8 = 5;

/// `NV_MMU_VER2_PTE_VALID`, bit 0 — and the bit that tells a leaf from a directory at a
/// level that can hold either (`ogkm-580: src/nvidia/src/libraries/mmu/gmmu_fmt.c:71-76`,
/// `gmmuFmtEntryIsPte`: at a level that is both `bPageTable` and a page directory, the
/// PTE's valid field decides).
const VER2_VALID: u64 = 1 << 0;

/// `NV_MMU_VER2_PTE_VOL` / `_PDE_VOL`, bit 3.
///
/// ★★ **This is how SPARSE is spelled on this regime.** `kgmmuFmtFamiliesInit_GM200`
/// builds the sparse templates as *valid clear, volatile set* for a PTE and *aperture
/// INVALID, volatile set* for a PDE (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_gm200.c:46-70`, whose own comment
/// is "GM20X supports sparse directly in HW by setting the volatile bit when the valid bit
/// is clear"). Reading that bit is the whole of what makes
/// [`kayfabe_arch::PteDecode::Sparse`] distinguishable here; without it a declared-empty
/// range and a never-written one are the same bits.
const VER2_VOL: u64 = 1 << 3;

/// `NV_MMU_VER2_PTE_READ_ONLY`, bit 6.
const VER2_READ_ONLY: u64 = 1 << 6;

/// `NV_MMU_VER2_PDE_APERTURE` / `_PTE_APERTURE`, bits 2:1.
const VER2_APERTURE_SHIFT: u32 = 1;
/// Ditto, width.
const VER2_APERTURE_MASK: u64 = 0x3;

/// `NV_MMU_VER2_PDE_ADDRESS_SHIFT` / `_PTE_ADDRESS_SHIFT` — 12
/// (`ogkm-580: src/common/inc/swref/published/pascal/gp100/dev_mmu.h:111, 114, 156`).
const VER2_ADDR_SHIFT: u32 = 12;

/// `NV_MMU_VER2_PDE_ADDRESS_VID` / `_PTE_ADDRESS_VID` = `(35-3):8`, i.e. bits 32:8 — a
/// 25-bit field (`dev_mmu.h:113, 140`).
const VER2_ADDR_VID_BITS: u32 = 25;

/// `NV_MMU_VER2_PDE_ADDRESS_SYS` / `_PTE_ADDRESS_SYS` = `53:8` — a 46-bit field
/// (`dev_mmu.h:119, 139`).
const VER2_ADDR_SYS_BITS: u32 = 46;

/// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SHIFT` — **8, not 12** (`dev_mmu.h:104`). The big
/// half of a dual entry names its sub-table 256 bytes at a time; the small half names it
/// 4 KiB at a time. Using one shift for both puts every big-page table at
/// one-sixteenth of its real address.
const VER2_BIG_ADDR_SHIFT: u32 = 8;

/// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_VID` = `(35-3):(8-4)`, i.e. bits 32:4 — 29 bits
/// (`dev_mmu.h:102`).
const VER2_BIG_ADDR_VID_BITS: u32 = 29;

/// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SYS` = `53:(8-4)`, i.e. bits 53:4 — 50 bits
/// (`dev_mmu.h:103`).
const VER2_BIG_ADDR_SYS_BITS: u32 = 50;

/// Extract a field of `bits` bits starting at bit `lo`, shifted left by `shift`.
const fn field_addr(raw: u64, lo: u32, bits: u32, shift: u32) -> u64 {
    ((raw >> lo) & ((1u64 << bits) - 1)) << shift
}

/// The aperture nibble of a **PDE** — `GMMU_APERTURE_INVALID`, `_VIDEO`, `_SYS_COH`,
/// `_SYS_NONCOH` in that order (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_fmt_gm10x.c:165-182`, which binds
/// the enum to `NV_MMU_PDE_APERTURE_BIG_*` values 0..3).
///
/// ⊘ `None` for `INVALID` — a PDE aperture of zero is not an aperture, it is the encoding
/// for *"this sub-level is absent"*, and the enum's own doc says the value exists "only for
/// GPU PDEs to distinguish invalid sub-levels".
fn pde_aperture(raw: u64) -> Option<Aperture> {
    match (raw >> VER2_APERTURE_SHIFT) & VER2_APERTURE_MASK {
        0 => None,
        1 => Some(Aperture::Vidmem),
        2 => Some(Aperture::SysmemCoherent),
        _ => Some(Aperture::SysmemNonCoherent),
    }
}

/// The aperture nibble of a **PTE** — `_VIDEO`, `_PEER`, `_SYS_COH`, `_SYS_NONCOH`
/// (`kern_gmmu_fmt_gm10x.c:184-201`).
///
/// ⚠ **The two tables are NOT the same table**, and that is the single easiest thing to get
/// wrong on this regime: a PDE's `1` is video memory and a PTE's `0` is. A decoder that
/// shared one function between them would put every leaf one aperture out.
fn pte_aperture(raw: u64) -> Aperture {
    match (raw >> VER2_APERTURE_SHIFT) & VER2_APERTURE_MASK {
        0 => Aperture::Vidmem,
        1 => Aperture::Peer,
        2 => Aperture::SysmemCoherent,
        _ => Aperture::SysmemNonCoherent,
    }
}

/// Where a PTE's (or a single PDE's) target lives, given its aperture.
fn ver2_address(raw: u64, aperture: Aperture) -> u64 {
    let bits = match aperture {
        Aperture::Vidmem | Aperture::Peer => VER2_ADDR_VID_BITS,
        Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => VER2_ADDR_SYS_BITS,
    };
    field_addr(raw, 8, bits, VER2_ADDR_SHIFT)
}

/// Decode an 8-byte word as a leaf mapping of `size`, or `None` if its valid bit is clear.
fn ver2_leaf(raw: u64, size: PageSize) -> Option<PteDecode> {
    if raw & VER2_VALID == 0 {
        return None;
    }
    let aperture = pte_aperture(raw);
    Some(PteDecode::Leaf {
        phys: ver2_address(raw, aperture),
        aperture,
        size,
        read_only: raw & VER2_READ_ONLY != 0,
    })
}

/// A slot with no target: `Sparse` if the guest set the volatile bit, else `Invalid`.
///
/// ★ One function, called from every level, so the two states cannot come apart at one
/// level and not another. `reachability_on_transition.md` §3.6 hole 6 is the cost of
/// conflating them: valid→sparse is an unmap the guest declared and invalid→sparse is
/// nothing at all.
const fn ver2_empty(raw: u64) -> PteDecode {
    if raw & VER2_VOL != 0 {
        PteDecode::Sparse
    } else {
        PteDecode::Invalid
    }
}

/// Decode an 8-byte word as a single (non-dual) page-directory entry pointing at
/// `child_level`.
fn ver2_pde(raw: u64, child_level: u8) -> PteDecode {
    match pde_aperture(raw) {
        None => ver2_empty(raw),
        Some(aperture) => PteDecode::Pde {
            edge: PdeEdge {
                next: ver2_address(raw, aperture),
                aperture,
                child_level,
            },
            also: None,
        },
    }
}

/// ★★★ **The GA10x page-table format — `GMMU_FMT_VERSION_2`, with this generation's own
/// two leaf levels.**
///
/// # The tree, from RM's own diagram (`ogkm-580:
/// `src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_fmt_ga10x.c:29-46`)
///
/// ```text
///  PD3 [48:47] → PD2 [46:38] → PD1 [37:29] / PT_512M [37:29]
///                                   ↓
///                              PD0 [28:21] / PT_HUGE [28:21]   (a 16-byte DUAL entry)
///                                ↙        ↘
///                       PT_SMALL [20:12]   PT_BIG [20:16]
/// ```
///
/// The geometry is `kgmmuFmtInitLevels_GP10X`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:48-105`);
/// the **one** GA10x delta is `kgmmuFmtInitLevels_GA10X`'s single statement,
/// `pLevels[2].bPageTable = NV_TRUE` (`kern_gmmu_fmt_ga10x.c:52`).
///
/// # ★★★ TWO THINGS ON THIS EXACT CHIP THAT A "LEAVES ARE PTEs" DESIGN GETS WRONG
///
/// Both are encoded below rather than commented, because both have already cost this
/// project weeks (`#13`; `resume_from_fault.md` §6 hole 7):
///
/// 1. **`PD0`'s entry is SIXTEEN bytes and names TWO sub-tables.**
///    `NV_MMU_VER2_DUAL_PDE__SIZE` is 16 (`dev_mmu.h:112`) and
///    `kgmmuFmtInitPdeMulti_GP10X` fills a big half and a small half from *different bit
///    ranges with different shifts* (`kern_gmmu_fmt_gp10x.c:107-137`). A decode that
///    returns one of them drops a whole sub-tree with no diagnostic. Both come back, as
///    [`kayfabe_arch::PteDecode::Pde::edge`] (small) and `also` (big).
/// 2. **`PD1` is itself a leaf level on GA10x, and only on GA10x.** Its slot with the
///    valid bit set is a **512 MiB** page, not a pointer. Its known producer is the guest
///    kernel-RM's copy-engine utility identity-mapping the whole framebuffer heap; this
///    format **decodes it faithfully** and the *binding* site declines it
///    ([`kayfabe_mmu::walker::leaf_disposition`]) — the separation `#13`'s round-4 silent
///    drop was built by collapsing.
///
/// ⊘ `PD0` can also be a 2 MiB leaf by the same `bPageTable && numSubLevels` rule
/// (`kern_gmmu_fmt_gp10x.c:87`), and that one is not GA10x-specific — it is decoded here
/// for the same reason, off the same bit.
///
/// # ★★ Why the level numbering has SIX rows and a walk is FIVE deep
///
/// The rows are the *format's own* numbering, ascending from the root, exactly as
/// [`kayfabe_arch::GmmuFmt::level_shift`]'s contract describes: "it may contain more rows
/// than [`kayfabe_arch::GmmuFmt::levels`] counts a single walk to be deep: where a regime
/// offers two alternative leaf tables … those are two rows, reachable only via
/// [`kayfabe_arch::PdeEdge::child_level`]". Rows 4 and 5 are alternatives, not successive
/// levels, so a small-page walk reads five tables and never six. The walker never
/// increments a level itself — every child level is stated by the edge.
#[derive(Debug, Default, Clone, Copy)]
pub struct Ga10xGmmu;

impl Ga10xGmmu {
    /// The format.
    #[must_use]
    pub fn new() -> Ga10xGmmu {
        Ga10xGmmu
    }
}

/// Every leaf size this regime can express, ascending.
///
/// ★ **Exhaustive is the contract** ([`kayfabe_arch::GmmuFmt::page_sizes`]) and it is what
/// makes an un-enumerated size a loud fault instead of a silent drop. 512 MiB is in the
/// list because GA10x really can map one — leaving it out to avoid the `#13` alias would
/// turn a decodable leaf into `UnknownLeafSize` and re-create the drop at the other end.
static GA10X_PAGE_SIZES: &[PageSize] = &[
    PageSize(4 << 10),
    PageSize(64 << 10),
    PageSize(2 << 20),
    PageSize(512 << 20),
];

impl GmmuFmt for Ga10xGmmu {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }

    fn page_sizes(&self) -> &[PageSize] {
        GA10X_PAGE_SIZES
    }

    /// 8 bytes everywhere except `PD0`, whose dual entry is 16 (`dev_mmu.h:97, 112, 157`).
    ///
    /// ⊘ A level this format does not have answers **0**, which
    /// [`kayfabe_mmu::walker::decode_page`] and its point-query sibling both treat as
    /// `BadGeometry` — a refusal, never a guessed width.
    fn entry_size(&self, level: u8) -> u8 {
        match level {
            L_PD3 | L_PD2 | L_PD1 | L_PT_BIG | L_PT_SMALL => 8,
            L_PD0 => 16,
            _ => 0,
        }
    }

    /// Five: `PD3 → PD2 → PD1 → PD0 → PT_SMALL`. See the type's docs for why that is not
    /// the row count.
    fn levels(&self) -> u8 {
        5
    }

    fn level_shift(&self, level: u8) -> Option<LevelShift> {
        // ★ `entries` is a COUNT read off each level's `virtAddrBitHi:Lo` pair, never
        // derived from a page size — `PT_BIG` holds **32** entries in a page that could
        // hold 512 (bits 20:16), and dividing 4 KiB by 8 would over-read it by 3 840
        // bytes. This is `LevelShift`'s own warning, instantiated.
        let (shift, entries) = match level {
            L_PD3 => (47, 4),        // 48:47
            L_PD2 => (38, 512),      // 46:38
            L_PD1 => (29, 512),      // 37:29
            L_PD0 => (21, 256),      // 28:21
            L_PT_BIG => (16, 32),    // 20:16
            L_PT_SMALL => (12, 512), // 20:12
            _ => return None,
        };
        Some(LevelShift { shift, entries })
    }

    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        let lo = raw as u64;
        match level {
            // Pure directories: no valid bit to consult, the aperture IS the validity.
            L_PD3 => ver2_pde(lo, L_PD2),
            L_PD2 => ver2_pde(lo, L_PD1),
            // ★★★ GA10x only: a PD1 slot with the valid bit set is a 512 MiB PAGE.
            L_PD1 => ver2_leaf(lo, PageSize(512 << 20)).unwrap_or_else(|| ver2_pde(lo, L_PD0)),
            L_PD0 => {
                // ★★★ A 2 MiB leaf is spelled in the LOW half's valid bit; a dual PDE is
                // spelled in the two halves' aperture fields. The valid bit is asked
                // first because that is the order `gmmuFmtEntryIsPte` asks it in.
                if let Some(leaf) = ver2_leaf(lo, PageSize(2 << 20)) {
                    return leaf;
                }
                let hi = (raw >> 64) as u64;
                // The SMALL half: aperture at hi bits 2:1 (`_APERTURE_SMALL` 66:65),
                // address at hi bits 32:8 / 53:8 (`_ADDRESS_SMALL_VID/_SYS` 96:72 /
                // 117:72), shift 12 (`_ADDRESS_SHIFT`).
                let small = pde_aperture(hi).map(|aperture| PdeEdge {
                    next: ver2_address(hi, aperture),
                    aperture,
                    child_level: L_PT_SMALL,
                });
                // The BIG half: aperture at lo bits 2:1, address at lo bits 32:4 / 53:4,
                // shift **8**.
                let big = pde_aperture(lo).map(|aperture| PdeEdge {
                    next: field_addr(
                        lo,
                        4,
                        match aperture {
                            Aperture::Vidmem | Aperture::Peer => VER2_BIG_ADDR_VID_BITS,
                            Aperture::SysmemCoherent | Aperture::SysmemNonCoherent => {
                                VER2_BIG_ADDR_SYS_BITS
                            }
                        },
                        VER2_BIG_ADDR_SHIFT,
                    ),
                    aperture,
                    child_level: L_PT_BIG,
                });
                // ★★ SMALL first, and the order is load-bearing for a point query: a
                // translation follows `edge` before `also`, and the small-page table is
                // the one RM populates for ordinary mappings — including every mapping the
                // bus aperture makes. ⊘ Neither is dropped when both are present.
                match (small, big) {
                    (None, None) => ver2_empty(lo),
                    (Some(e), also) => PteDecode::Pde { edge: e, also },
                    (None, Some(e)) => PteDecode::Pde {
                        edge: e,
                        also: None,
                    },
                }
            }
            L_PT_BIG => ver2_leaf(lo, PageSize(64 << 10)).unwrap_or_else(|| ver2_empty(lo)),
            L_PT_SMALL => ver2_leaf(lo, PageSize(4 << 10)).unwrap_or_else(|| ver2_empty(lo)),
            // ⊘ A level this format does not have decodes to nothing, and NOT to a
            // plausible entry. `Invalid` rather than `Sparse`: sparse is a declaration
            // the guest made, and a level that does not exist carries none.
            _ => PteDecode::Invalid,
        }
    }
}

/// A GMMU format that decodes **nothing** — kept as the vocabulary's own *"this generation
/// has no page-table format"*.
///
/// ⚠ **No longer what [`Ga10xArch`] answers with**: `#149` built the real one
/// ([`Ga10xGmmu`]) because the guest's bus aperture is a translated window and a boot
/// stops dead at it. This type stays because the statement it makes is still one an
/// adapter may need to make, and because deleting it would leave "refusing" with no
/// spelling at all.
///
/// ★ Zero levels and no page sizes rather than plausible ones. #13's corollary L3 is
/// that a walk hitting an un-enumerated leaf size must be a loud fault and never a silent
/// drop (*"the GA10x PD1 512M-leaf gap cost weeks"*); an empty enumeration makes **every**
/// size un-enumerated, so the first walk faults instead of the first *unusual* walk.
///
/// ★ Zero levels and no page sizes rather than plausible ones. #13's corollary L3 is
/// that a walk hitting an un-enumerated leaf size must be a loud fault and never a silent
/// drop (*"the GA10x PD1 512M-leaf gap cost weeks"*); an empty enumeration makes **every**
/// size un-enumerated, so the first walk faults instead of the first *unusual* walk.
#[derive(Debug, Default)]
pub struct UnbuiltGmmu;

impl GmmuFmt for UnbuiltGmmu {
    /// ⚠ `Ver2` is the only honest-ish answer available — GA10x really is a VER2-regime
    /// MMU (Pascal…Ada) — and it is a **claim about the regime, not about this codec**.
    /// Nothing downstream may read it as "the format is implemented": [`Self::levels`] is
    /// `0`, which is the field that says whether a walk is possible.
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }

    fn page_sizes(&self) -> &[PageSize] {
        &[]
    }

    fn entry_size(&self, _level: u8) -> u8 {
        0
    }

    fn levels(&self) -> u8 {
        0
    }

    fn level_shift(&self, _level: u8) -> Option<LevelShift> {
        None
    }

    /// Every entry is [`PteDecode::Invalid`] — **not** [`PteDecode::Sparse`]. Sparse is a
    /// declaration the guest made (*"there is deliberately nothing here"*); `Invalid` is
    /// the absence of one, which is what this codec actually knows.
    fn decode_entry(&self, _level: u8, _raw: u128) -> PteDecode {
        PteDecode::Invalid
    }
}

/// ★★ **GA10x's real USERD geometry** (`E4`) — 512 bytes with the two cursors at `0x88`
/// and `0x8C`.
///
/// Every number is `kayfabe_abi::submit`'s, per decision #2's quarantine; this type is
/// the mapping from that ABI into the [`UserdModel`] seam and transcribes nothing.
///
/// ⚠ **The size is a HAL choice, not a chip header's.** `kfifoGetUserdSizeAlign` is
/// halified two ways and GA106 takes the `else` arm — Maxwell's — so the size comes from
/// `published/maxwell/gm107/dev_ram.h` even though the chip is Ampere. Reaching for
/// `published/ampere/ga102/dev_ram.h` instead finds **no `NV_RAMUSERD` at all**, and the
/// `a_table_does_not_decide_behaviour` lesson is exactly this shape: read the ladder that
/// binds the table, not only the table. `tests/oracle/pushbuffer_abi_oracle.c` compiles
/// the driver's own function rather than trusting this paragraph.
#[derive(Debug, Default)]
pub struct Ga10xUserd;

impl UserdModel for Ga10xUserd {
    fn userd_size(&self) -> u64 {
        submit::USERD_SIZE
    }

    fn gp_get_offset(&self) -> u64 {
        submit::USERD_GP_GET
    }

    fn gp_put_offset(&self) -> u64 {
        submit::USERD_GP_PUT
    }
}

/// A USERD model with no geometry — kept as the vocabulary's own *"this generation has no
/// USERD layout"*.
///
/// ⚠ **No longer what [`Ga10xArch`] answers with**: `E4` built the real one
/// ([`Ga10xUserd`]). This type stays for [`UnbuiltGmmu`]'s reason — the statement it makes
/// is one an adapter may still need to make, and deleting it would leave "refusing" with
/// no spelling.
///
/// ★ `userd_size() == 0` is the load-bearing zero: a caller sizing a USERD mapping from
/// it maps nothing rather than mapping a plausible-but-wrong window over guest memory.
#[derive(Debug, Default)]
pub struct UnbuiltUserd;

impl UserdModel for UnbuiltUserd {
    fn userd_size(&self) -> u64 {
        0
    }

    fn gp_get_offset(&self) -> u64 {
        0
    }

    fn gp_put_offset(&self) -> u64 {
        0
    }
}

/// ★★★ **GA10x's real pushbuffer codec** (`E4`) — `AMPERE_CHANNEL_GPFIFO_A`'s GPFIFO
/// entry format and `NVC56F_DMA_*` method framing, mapped into the core's four decoded
/// fact kinds.
///
/// Every bit position is `kayfabe_abi::submit`'s, cited to `ogkm-580`, and none is
/// transcribed here (decision #2's quarantine). What lives here is the *mapping* from a
/// framed method run to [`PushMethod`], and the refusals that mapping makes.
///
/// # ★★★ What this codec DECODES — and it is three facts, not four
///
/// | fact | the exact shape accepted | why nothing looser |
/// |---|---|---|
/// | [`PushMethod::SetObject`] | one **incrementing** run, `SET_OBJECT`, count 1 | the class is `NVCLASS` (`15:0`) and *not* the whole word: bits `20:16` are `ENGINE` |
/// | [`PushMethod::SemRelease`] | one **incrementing** run at `SEM_ADDR_LO`, count 5, with `SEM_EXECUTE.OPERATION == RELEASE` | the five words are one fact; six of the eight operations are **acquires**, and decoding one as a release reports a completion the guest is still waiting for |
/// | [`PushMethod::TlbInvalidate`] | one **incrementing** run at `MEM_OP_A`, count 4, with `MEM_OP_D.OPERATION` an `MMU_TLB_INVALIDATE`, `PDB_ONE` | the class header's own comment is *"MEM_OP_D MUST be preceded by MEM_OPs A-C"*; `PDB_ALL` carries no address at all |
///
/// ⊘ **`LAUNCH_DMA` is still [`PushMethod::Opaque`] at THIS entry point, and always will
/// be.** A real `AMPERE_DMA_COPY_B` launch carries **none** of its operands:
/// `OFFSET_IN_UPPER…OFFSET_OUT_LOWER` are a *separate earlier run*,
/// `LINE_LENGTH_IN`/`LINE_COUNT` another, `SET_SEMAPHORE_A/B/PAYLOAD` a third, and
/// `LAUNCH_DMA` itself is one word of flags (see `kayfabe_isolate_host::rm::ce_pushbuffer`,
/// which is the encoder a real GA106 executed at rung R17). [`PushbufferAbi::decode_method`]
/// is handed one `(header, args)` pair and nothing else, so it *cannot* produce a
/// [`PushMethod::CeLaunchDma`] whose `dst`, `src` and `len` are anything but invented —
/// and a decoder that answered anyway would hand the address plane a destination the
/// guest never wrote, which is a **silent memory-safety fact about the guest**.
///
/// ★★★ **[`Ga10xPushbuffer::decode_run`] is where a launch decodes**, because it is the
/// one that can see the state the engine holds. `E4` recorded this as a seam that had to
/// change shape rather than a value that had to be filled in, and that is what `E5` did.
///
/// `MockArch` hid the whole question: `mock_method::CE_LAUNCH_DMA` packs destination,
/// source, length and work kind into **one** method's arguments, an encoding no NVIDIA
/// chip has. So the seam looked sufficient for as long as the only implementer was the
/// mock (`mock_fidelity_both_directions`).
///
/// # The refusals, and what each one stops
///
/// - **An argument count that does not match the header's exactly** ⇒ `Opaque`. The core
///   parser clamps a truncated run to what is left in the range
///   (`kayfabe_fwd::decode_methods`), so a range that ends mid-run yields a short `args`;
///   decoding it would read zeros as a semaphore address.
/// - **A non-incrementing framing of a modelled address** ⇒ `Opaque`. `NV_PUSH_nU` — the
///   driver's own macro, and `rm::ce_pushbuffer` — writes `SEC_OP_INC_METHOD`; a
///   `NON_INC` run at `SEM_ADDR_LO` writes five words to *one* register and is a
///   different operation.
/// - **A `SEC_OP` the class header does not define** (`RESERVED6`, and `GRP2_USE_TERT`
///   with an unenumerated `TERT_OP`) ⇒ `method_len` is `0` and the decode is `Opaque`.
/// - **A GPFIFO entry with `LENGTH == 0`** ⇒ **no range at all**. Its low byte is
///   `OPCODE`, not `GET_HI`, so reading an address out of it fabricates a pointer.
/// - **A ring whose byte length is not a whole number of 8-byte entries** ⇒ **no ranges
///   at all**. If the ring's framing is wrong we do not know where entries start, and
///   decoding the prefix would be answering a question we cannot answer.
#[derive(Debug, Default)]
pub struct Ga10xPushbuffer;

impl Ga10xPushbuffer {
    /// Read a 64-bit address held as `(low dword in place, high dword)` — the shape both
    /// `SEM_ADDR_LO`/`_HI` and the GPFIFO entry use.
    fn addr40(lo: u32, hi: u32) -> u64 {
        u64::from(lo & 0xFFFF_FFFC) | (u64::from(hi & 0xFF) << 32)
    }

    /// Decode the five-word host-FIFO semaphore run, or `None` if it is not a release.
    fn sem_release(args: &[u32]) -> Option<PushMethod> {
        let [addr_lo, addr_hi, payload_lo, payload_hi, execute] = *args.first_chunk::<5>()?;
        if execute & submit::fifo::SEM_EXECUTE_OPERATION_MASK
            != submit::fifo::SEM_EXECUTE_OPERATION_RELEASE
        {
            // Six of the eight operations are acquires, and REDUCTION is neither.
            return None;
        }
        // ★ `PAYLOAD_SIZE` decides whether `SEM_PAYLOAD_HI` is part of the value. With
        // `_32BIT` the engine writes four bytes, and reading the high word anyway
        // invents the top half of a fence.
        let payload = if execute & submit::fifo::SEM_EXECUTE_PAYLOAD_SIZE_64BIT != 0 {
            u64::from(payload_lo) | (u64::from(payload_hi) << 32)
        } else {
            u64::from(payload_lo)
        };
        Some(PushMethod::SemRelease {
            addr: GpuVa(Self::addr40(addr_lo, addr_hi)),
            payload,
        })
    }

    /// The engine-register slot [`MethodState`] holds a CE method address in, or `None`
    /// if this codec mirrors no register for it.
    ///
    /// ⊘ **The numbering is private to this codec.** A method address *is* a register
    /// index on real hardware; this is that index, compacted into the sixteen slots
    /// `kayfabe_arch::METHOD_SLOTS` allows. Nothing outside this file may assume a slot
    /// means anything.
    fn ce_slot(method: u32) -> Option<usize> {
        match method {
            // `OFFSET_IN_UPPER`, `_IN_LOWER`, `_OUT_UPPER`, `_OUT_LOWER` — consecutive at
            // 0x400..0x40c, which is why ONE incrementing header of count 4 writes the
            // whole source-and-destination pair (`rm::ce_pushbuffer` does exactly that).
            submit::ce::OFFSET_IN_UPPER => Some(0),
            submit::ce::OFFSET_IN_LOWER => Some(1),
            submit::ce::OFFSET_OUT_UPPER => Some(2),
            submit::ce::OFFSET_OUT_LOWER => Some(3),
            submit::ce::LINE_LENGTH_IN => Some(4),
            submit::ce::LINE_COUNT => Some(5),
            // ★ The per-operand phys-mode target — the residency signal for a physical
            // operand (`memmgrTestCeUtils`' vid→sys copy, E10). Latched like any other CE
            // register write; read in `ce_launch` only when the operand is physical.
            submit::ce::SET_SRC_PHYS_MODE => Some(6),
            submit::ce::SET_DST_PHYS_MODE => Some(7),
            // ★ The remap (constant-fill) registers. `SET_REMAP_COMPONENTS` is latched
            // like any other write and read in `remap_fill`; see its docs for why the map
            // is REQUIRED rather than defaulted.
            submit::ce::SET_REMAP_CONST_A => Some(8),
            submit::ce::SET_REMAP_CONST_B => Some(9),
            submit::ce::SET_REMAP_COMPONENTS => Some(10),
            // ★★★ E10e — the ENGINE-class completion semaphore. Three consecutive methods
            // at 0x240/0x244/0x248, which is why RM writes them with one `NV_PUSH_INC_3U`
            // (`ogkm-580: channel_utils.c:671-673, :838-840`). ⚠ `_A` is the address's HIGH
            // half; see `kayfabe_arch::CeCompletion`.
            submit::ce::SET_SEMAPHORE_A => Some(11),
            submit::ce::SET_SEMAPHORE_B => Some(12),
            submit::ce::SET_SEMAPHORE_PAYLOAD => Some(13),
            _ => None,
        }
    }

    /// ★★★ **Decode a launch's engine-class completion semaphore** — `Some(Some(..))` for a
    /// one-word release, `Some(None)` for *"this launch releases nothing"*, and `None` for a
    /// release this codec cannot express, which refuses the whole launch.
    ///
    /// # ⊘ Why an unexpressible release refuses the COPY as well
    ///
    /// A launch and its release are one act: the engine writes the payload **after** the
    /// copy retires, which is the only ordering that makes a completion truthful. Reporting
    /// the copy while dropping the release would produce bytes that move and a semaphore that
    /// never advances — the guest spins forever on a copy we actually performed, and our own
    /// logs say we served it. Refusing the pair is loud in the one place a boot can see it.
    /// (Same discipline as `MULTI_LINE_ENABLE`, one field over.)
    ///
    /// The two refusals, each a distinct thing:
    /// - **`RELEASE_FOUR_WORD_SEMAPHORE`** (`clc7b5.h:100`) — sixteen bytes, of which eight
    ///   are a **hardware timestamp** the engine samples. This port has no such clock, and
    ///   writing the payload while leaving the timestamp words stale is a completion a
    ///   timestamp-reading guest would misdate. `RELEASE_CONDITIONAL_INTR_SEMAPHORE`
    ///   (value 3) is refused with it: the release is conditional on engine state we do not
    ///   model, so *"did it release?"* is not answerable.
    /// - **A one-word release whose `SET_SEMAPHORE_*` registers were never latched.** `0` is
    ///   a legal payload and a legal address, so written-ness is asked for separately — the
    ///   same rule the offsets already follow, and for the sharper reason: an unlatched
    ///   address would send a completion to physical/virtual zero.
    fn ce_completion(
        state: &MethodState,
        subch: usize,
        flags: u32,
    ) -> Option<Option<kayfabe_arch::CeCompletion>> {
        match flags & submit::ce::LAUNCH_SEMAPHORE_TYPE_MASK {
            submit::ce::LAUNCH_SEMAPHORE_TYPE_NONE => Some(None),
            submit::ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD => {
                let hi = state.latched(subch, 11)?;
                let lo = state.latched(subch, 12)?;
                let payload = state.latched(subch, 13)?;
                Some(Some(kayfabe_arch::CeCompletion {
                    addr: GpuVa(
                        (u64::from(hi & submit::ce::SET_SEMAPHORE_A_UPPER_MASK) << 32)
                            | u64::from(lo),
                    ),
                    payload: u64::from(payload),
                }))
            }
            _ => None,
        }
    }

    /// ★★★ **Decode a remap-enabled launch's constant fill: the repeating pattern and the
    /// size of one element** — or `None`, which is a refusal and never a default.
    ///
    /// # What the component map actually decides, and why ignoring it is two bugs
    ///
    /// `[src]` `ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:330-420` — the driver's
    /// own three memset entry points on this class:
    /// - `memset_1` writes an `NvU8` into `CONST_B` with `COMPONENT_SIZE_ONE` /
    ///   `NUM_DST_COMPONENTS_ONE` ⇒ a **1-byte** element;
    /// - `memset_4` writes an `NvU32` with `COMPONENT_SIZE_FOUR` ⇒ a **4-byte** element,
    ///   and divides its byte count by four before pushing `LINE_LENGTH_IN`;
    /// - `memset_8` spreads a 64-bit value across `CONST_A` (X) and `CONST_B` (Y) with
    ///   `NUM_DST_COMPONENTS_TWO` ⇒ an **8-byte** element.
    ///
    /// So the map fixes both the pattern's **period** and the unit `LINE_LENGTH_IN`
    /// counts. ⚠ RM's own scrub path is the *1-byte* map (`ogkm-580:
    /// channel_utils.c:1029-1033`), which is why `memmgrMemSet(…, value, …)` writes
    /// `value & 0xFF` to every byte — the same end-state its `TRANSFER_TYPE_PROCESSOR`
    /// arm produces with `portMemSet(pDst, value, size)` (`ogkm-580: mem_utils.c:1122`).
    /// A decoder that assumed a 4-byte little-endian pattern would agree with hardware
    /// only on byte-uniform values, i.e. exactly where a test cannot see the difference.
    ///
    /// # ⊘ Each `None` is a different thing this codec cannot say
    ///
    /// - **`SET_REMAP_COMPONENTS` was never latched.** The map is not optional and its
    ///   reset value is not something this port has measured; every real writer in reach
    ///   pushes it immediately before the launch. Defaulting it would invent both the
    ///   element size and the pattern.
    /// - **A `CONST_A`/`CONST_B` the map selects was never latched.** Same rule as the
    ///   offsets: `0` is a legal pattern, so *written*-ness is asked for separately.
    /// - **A selector naming a SOURCE component** (`SRC_X..SRC_W`). That is a remapped
    ///   **copy** — a swizzle of source bytes — not a constant fill, and
    ///   [`kayfabe_arch::CeWork::Fill`] has no way to express it.
    /// - **`NO_WRITE`.** The engine skips that component; the destination keeps its old
    ///   bytes. A fill reported over it would claim writes that never happen.
    /// - **An element size that does not divide 4.** [`kayfabe_arch::CeWork::Fill`]
    ///   carries a `u32` whose phase downstream is `pattern[addr % 4]`, so it can express
    ///   periods 1, 2 and 4 exactly and **cannot** express 3, 6, 8, 12 or 16. ⊘ Refused by
    ///   name rather than truncated: `memset_8`'s 8-byte period is a real shape the driver
    ///   emits, and answering it with its low four bytes would write the wrong half of
    ///   every other element. Widening `CeWork::Fill` is the fix; this refusal is the
    ///   placeholder that cannot be mistaken for support.
    ///
    /// Returns `(pattern, element_bytes)` with `element_bytes ∈ {1, 2, 4}` and `pattern`
    /// already normalised so that `pattern.to_le_bytes()[a % 4]` is the byte hardware
    /// writes at destination address `a` — **given** the element alignment `ce_launch`
    /// separately requires.
    fn remap_fill(state: &MethodState, subch: usize) -> Option<(u32, u64)> {
        let map = state.latched(subch, 10)?;
        let comp_bytes = submit::ce::remap_component_bytes(map);
        let num_dst = submit::ce::remap_num_dst_components(map);
        // 1..=4 components of 1..=4 bytes: at most 16, so this cannot overflow a u32 and
        // the `usize` below is exact on every target this builds for.
        let element = comp_bytes * num_dst;
        if !4u32.is_multiple_of(element) {
            return None;
        }
        // Lay one element out byte by byte, in destination order.
        let mut elem = [0u8; 4];
        for c in 0..num_dst {
            let konst = match submit::ce::remap_dst_sel(map, c) {
                submit::ce::REMAP_DST_SEL_CONST_A => state.latched(subch, 8)?,
                submit::ce::REMAP_DST_SEL_CONST_B => state.latched(subch, 9)?,
                // `SRC_*`, `NO_WRITE`, and the two values the header does not define.
                _ => return None,
            };
            let bytes = konst.to_le_bytes();
            for b in 0..comp_bytes {
                elem[(c * comp_bytes + b) as usize] = bytes[b as usize];
            }
        }
        // Repeat the element across four bytes. `element` divides 4, so this is exact.
        let mut pattern = [0u8; 4];
        for (i, p) in pattern.iter_mut().enumerate() {
            *p = elem[i % element as usize];
        }
        Some((u32::from_le_bytes(pattern), u64::from(element)))
    }

    /// A latched `SET_{SRC,DST}_PHYS_MODE` slot → [`kayfabe_arch::PhysTarget`]. An operand
    /// that never wrote the register is `LocalFb`, the engine's own reset value
    /// (`ogkm-580: clc7b5.h:68`) — faithful, not a guess.
    fn phys_target(latched: Option<u32>) -> kayfabe_arch::PhysTarget {
        use kayfabe_arch::PhysTarget;
        match latched.map(|v| v & submit::ce::PHYS_MODE_TARGET_MASK) {
            Some(1) => PhysTarget::CoherentSysmem,
            Some(2) => PhysTarget::NonCoherentSysmem,
            Some(3) => PhysTarget::Peer,
            _ => PhysTarget::LocalFb,
        }
    }

    /// Assemble the latched operands on `subch` into a [`PushMethod::CeLaunchDma`], or
    /// `None` — which the caller turns into [`PushMethod::Opaque`].
    ///
    /// # ⊘ Every `None` here is a REFUSAL, and each one is a different thing we cannot say
    ///
    /// - **Nothing bound, or a class that is not the copy engine.** A subchannel's method
    ///   addresses mean whatever its bound object says they mean; `0x300` on a compute
    ///   object is not `LAUNCH_DMA`. Firing without checking would decode a graphics
    ///   method as a copy.
    /// - **`DATA_TRANSFER_TYPE == NONE`.** The engine moves no bytes; the launch exists to
    ///   release a semaphore. There is no copy to report.
    /// - **`MULTI_LINE_ENABLE == TRUE`.** Then `LINE_LENGTH_IN` is a *per-line* byte count
    ///   and the region is `LINE_COUNT` lines separated by `PITCH_IN`/`PITCH_OUT` — which
    ///   is **not** the contiguous `len` bytes [`PushMethod::CeLaunchDma`] means. Reporting
    ///   `length × count` would name a span the engine never touches when the pitch
    ///   exceeds the line, and would *under*-report the destination's extent when it does
    ///   not. The address plane acts on that number.
    /// - **`REMAP_ENABLE == TRUE` with a component map this codec cannot express.** The
    ///   map is decoded by [`Ga10xPushbuffer::remap_fill`], which lists its own five
    ///   refusals; each is a distinct thing about the fill we cannot say, and none of them
    ///   is *"fills are unsupported"*.
    /// - **`REMAP_ENABLE == TRUE` with a destination not aligned to the element.** The
    ///   engine phases the pattern from the **start of the transfer** — element `i` of the
    ///   fill lands at `dst + i·element` — while [`kayfabe_arch::CeWork::Fill`]'s
    ///   downstream phases it from the **absolute destination address** (`pattern[a % 4]`,
    ///   which is what makes a split fill byte-identical to a whole one). Those two agree
    ///   for every byte **iff** `dst` is a multiple of the element size. ⊘ So the
    ///   alignment is checked here, where the element size is known, rather than assumed
    ///   from the fact that the drivers in reach happen to align their memsets.
    /// - **An operand that was never latched.** The heart of it. `0` is a legal offset and
    ///   a legal length, so [`MethodState`] tracks *written*-ness separately and this asks
    ///   for it. A `CeLaunchDma` assembled from `unwrap_or_default()` is precisely the
    ///   invented destination `E4` refused to produce, one increment later and harder to
    ///   see.
    ///
    /// ⊘ And [`kayfabe_arch::CeWork::Scrub`] is **never** produced: `MEMORY_SCRUB_ENABLE`
    /// is not a field of this class at all — see
    /// [`submit::ce::NO_MEMORY_SCRUB_ON_THIS_CLASS`] for the C bug that is.
    fn ce_launch(state: &MethodState, subch: usize, flags: u32) -> Option<PushMethod> {
        // ★★★ `subchannel_speaks`, not `object(subch)?` — see that predicate's docs for
        // UVM's own encoder, which binds the CE class on subchannel 0 and issues every CE
        // method on subchannel 4. The old form refused all of them, and RM's `channel_utils`
        // path (bind and fire on one subchannel) is why nothing noticed.
        if !state.subchannel_speaks(subch, ClassId(nv::AMPERE_DMA_COPY_B)) {
            return None;
        }
        // ★★★ **`DATA_TRANSFER_TYPE == NONE` is a RELEASE, not a nothing** — and reporting
        // it as `None` cost the boot `uvm_channel_manager_create`.
        //
        // ⊘ This arm used to `return None`, with the sentence *"the engine moves no bytes;
        // the launch exists to release a semaphore. There is no copy to report."* Every
        // clause of that is true and the conclusion does not follow: there is no *copy* to
        // report, and there is still a **release**. `[measured 2026-08-09, boots `fmb1` /
        // `msr1` at `319d29a`]` `cuInit` hangs in `uvm_push_end_and_wait`, and the push it
        // waits on — `channel_init`'s, the FIRST push on every UVM channel — is
        // `SET_OBJECT` plus exactly this shape (`ogkm-580: uvm_channel.c:2518-2572, 1492,
        // 1512-1513, 1043-1051` → `uvm_volta_ce.c:50-70`). So the entire content of that
        // submission decoded to `Opaque`, the whole ring decoded to no launch at all, and
        // the word `uvm_spin_loop` polls could never move.
        //
        // ⊘ It becomes [`PushMethod::CeRelease`] and NOT a zero-length `CeLaunchDma`: the
        // release-only push never writes `OFFSET_OUT_*`, so the `state.latched(subch, 2)?`
        // below would refuse it anyway — and satisfying that with a default would invent a
        // destination address, which is the one thing this codec's refusals exist to stop.
        if flags & submit::ce::LAUNCH_TRANSFER_MASK == submit::ce::LAUNCH_TRANSFER_NONE {
            // A release this codec cannot express refuses the whole launch, exactly as it
            // does for a copy — `ce_completion`'s `None` arm and its reasons are unchanged.
            // ⊘ And `Some(None)` — no transfer AND no release — is a genuine no-op with no
            // fact in it, which stays `Opaque`.
            let completion = Self::ce_completion(state, subch, flags)??;
            return Some(PushMethod::CeRelease {
                completion,
                flush: flags & submit::ce::LAUNCH_FLUSH_ENABLE != 0,
            });
        }
        // ⊘ ONE refusal per fact. These two used to share a line, and collapsing them is
        // what let a whole work kind sit undecoded behind a comment about pitch: a
        // multi-line launch is a *geometry* this codec does not model, a remap-enabled one
        // is a *work kind* it now does.
        if flags & submit::ce::LAUNCH_MULTI_LINE_ENABLE != 0 {
            return None;
        }
        // ★ The work kind, and with it the unit `LINE_LENGTH_IN` counts. See
        // `remap_fill`: under `REMAP_ENABLE` the length is in ELEMENTS, and an element is
        // `COMPONENT_SIZE × NUM_DST_COMPONENTS` bytes (`ogkm-580: uvm_maxwell_ce.c:359,
        // :371`). A decoder that read it as bytes would under-report a 4-byte fill's
        // extent by 4× — silently, and in the direction that leaves guest memory
        // un-written rather than faulting.
        let (work, element) = if flags & submit::ce::LAUNCH_REMAP_ENABLE != 0 {
            let (pattern, element) = Self::remap_fill(state, subch)?;
            (CeWork::Fill { pattern }, element)
        } else {
            (CeWork::Copy, 1)
        };
        let up = |v: u32| u64::from(v & submit::ce::OFFSET_UPPER_MASK) << 32;
        // ⊘ A fill has NO SOURCE OPERAND, and RM proves it: `channelPushMemoryProperties`
        // pushes `OFFSET_IN_UPPER/_LOWER` only on the `bCeMemcopy` arm (`ogkm-580:
        // channel_utils.c:1036-1067`), so a memset's source slots are **never latched**.
        // Requiring them would refuse every fill the driver sends; reporting whatever a
        // *previous* copy on this subchannel left in them would report a stale address as
        // the fill's source. Zero, explicitly, matching `CeLaunchDma::src`'s own docs
        // (*"meaningless for Scrub/Fill"*).
        let src = match work {
            CeWork::Copy => up(state.latched(subch, 0)?) | u64::from(state.latched(subch, 1)?),
            CeWork::Scrub | CeWork::Fill { .. } => 0,
        };
        let dst = up(state.latched(subch, 2)?) | u64::from(state.latched(subch, 3)?);
        // ★ `LINE_COUNT` is deliberately NOT required. With `MULTI_LINE_ENABLE_FALSE` the
        // engine ignores it, so demanding it would refuse a copy real hardware performs —
        // and `rm::ce_pushbuffer` writes it only because it shares a header with
        // `LINE_LENGTH_IN`.
        let len = u64::from(state.latched(subch, 4)?) * element;
        // See the type docs: the absolute-address phase downstream and the engine's
        // transfer-relative phase coincide exactly on an element-aligned destination.
        if !dst.is_multiple_of(element) {
            return None;
        }
        // ★★★ E10e — the engine-class completion, decoded HERE because it is part of this
        // launch and not a method beside it. A release this codec cannot express refuses the
        // whole launch; see `ce_completion`.
        let completion = Self::ce_completion(state, subch, flags)?;
        // ★ Non-`?`: the phys-mode registers are NOT required operands. An operand may be
        // physical and never have written its target (the engine then reads the reset
        // value, `LOCAL_FB`), so demanding it would refuse a copy real hardware performs —
        // the opposite discipline from the offsets, which the copy cannot address without.
        Some(PushMethod::CeLaunchDma {
            dst: GpuVa(dst),
            src: GpuVa(src),
            len,
            dst_is_virtual: flags & submit::ce::LAUNCH_DST_PHYSICAL == 0,
            src_is_virtual: flags & submit::ce::LAUNCH_SRC_PHYSICAL == 0,
            dst_target: Self::phys_target(state.latched(subch, 7)),
            src_target: Self::phys_target(state.latched(subch, 6)),
            work,
            completion,
        })
    }

    /// Decode the four-word `MEM_OP_A..D` run, or `None` if it is not a PDB-targeted
    /// TLB invalidate.
    fn tlb_invalidate(args: &[u32]) -> Option<PushMethod> {
        let [a, _b, c, d] = *args.first_chunk::<4>()?;
        let op = d >> submit::fifo::MEM_OP_D_OPERATION_SHIFT;
        if op != submit::fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE
            && op != submit::fifo::MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED
        {
            // MEMBAR, the L2 operations and ACCESS_COUNTER_CLR share this method run.
            return None;
        }
        if c & submit::fifo::MEM_OP_C_PDB_ALL != 0 {
            // `PDB_ALL` names no page directory; there is no `pdb` to report.
            return None;
        }
        let pdb = u64::from(c & submit::fifo::MEM_OP_C_PDB_ADDR_LO_MASK)
            | (u64::from(d & submit::fifo::MEM_OP_D_PDB_ADDR_HI_MASK) << 32);
        Some(PushMethod::TlbInvalidate {
            pdb: Pdb(pdb),
            membar: a & submit::fifo::MEM_OP_A_SYSMEMBAR_EN != 0,
        })
    }
}

impl PushbufferAbi for Ga10xPushbuffer {
    /// The header's own argument count, per `SEC_OP`.
    ///
    /// ⊘ `0` for the two encodings `NVC56F_DMA_SEC_OP` enumerates without a format. That
    /// is the trait's documented refusal — the core then advances past just the header —
    /// and it is the *only* place this codec can desynchronise, which is why every other
    /// form, including the legacy `GRP0`/`GRP2` ones this port models nothing of, is
    /// sized rather than refused.
    fn method_len(&self, header: u32) -> usize {
        submit::method_header_decode(header).map_or(0, |h| h.arg_words)
    }

    /// One `(header, args)` pair → a core fact, or [`PushMethod::Opaque`].
    ///
    /// See the type docs for the three facts this decodes, and for why `LAUNCH_DMA` is
    /// **not** one of them.
    fn decode_method(&self, header: u32, args: &[u32]) -> PushMethod {
        let Some(h) = submit::method_header_decode(header) else {
            return PushMethod::Opaque;
        };
        // ★★ Only the incrementing framing, and only an argument list of exactly the
        // length the header declared. A short list is a run the range cut in half; a
        // non-incrementing run at the same address is a different operation.
        if h.form != submit::MethodForm::Incrementing || args.len() != h.arg_words {
            return PushMethod::Opaque;
        }
        let decoded = match (h.method, h.arg_words) {
            (submit::SET_OBJECT, 1) => Some(PushMethod::SetObject {
                class: ClassId(args[0] & submit::SET_OBJECT_NVCLASS_MASK),
            }),
            (submit::fifo::SEM_ADDR_LO, 5) => Self::sem_release(args),
            (submit::fifo::MEM_OP_A, 4) => Self::tlb_invalidate(args),
            _ => None,
        };
        decoded.unwrap_or(PushMethod::Opaque)
    }

    /// The ring's GPFIFO entries, 8 bytes each, little-endian.
    ///
    /// Two refusals, both of them whole-ring or whole-entry rather than partial: a ring
    /// that is not a whole number of entries yields **nothing**, and a control entry
    /// (`LENGTH == 0`) contributes **nothing**. See the type docs.
    ///
    /// # ★★★ MEASURED 2026-08-02 — the address is a GPU VA and it is **NOT** a GPA
    ///
    /// Two boots at rev `c93930d` on vast `46529600` (RTX 3060 / GA106, guest driver
    /// 580.159.04 open, stock), evidence in `docs/reference/bench_evidence/`:
    ///
    /// | boot | guest RAM | `gpFifoOffset` the guest declared |
    /// |---|---|---|
    /// | `c93930d_run_e5ring1_*` | 8 GiB (`e820` usable to `0x2_7fff_ffff`) | `0x1_2006_4000` |
    /// | `c93930d_run_e5ring2g_*` | 2 GiB (`e820` usable to `0x7ffd_bfff`, **nothing above 4 GiB**) | `0x1_2006_4000` |
    ///
    /// **Byte-identical across a 4× change in guest RAM, and at 2 GiB it names memory the
    /// guest does not have.** So it is not derived from the guest's physical layout: it is a
    /// virtual address out of the guest's own VA heap.
    ///
    /// ⊘ **And the 8 GiB row is the dangerous one.** There, `0x1_2006_4000` *is* a legal
    /// guest-physical address, so `Vmm::gpa_read` **succeeds** and returns whatever guest RAM
    /// happens to live there. The failure is not a fault; it is bytes.
    ///
    /// `gpFifoOffset` is the ring base rather than an entry's `GET`, and they are the same
    /// kind of address in the same allocation: the driver sets
    /// `gpFifoOffset = pChannel->pbGpuVA + pChannel->channelPbSize`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:1232`) and
    /// writes `get = pChannel->pbGpuVA + gpOffset` into `GP_ENTRY0_GET`/`GP_ENTRY1_GET_HI`
    /// (`:1871-1879`), with `pbGpuVA` the `dmaOffset` an `NV04_MAP_MEMORY_DMA` returned
    /// (`:842`). `ogkm-580: ctrl2080fifo.h:809` calls the field *"Gpfifo Virtual Offset"*.
    ///
    /// ## ★★★ CLOSED — this returns a GPU **VA**, and it is now typed as one
    ///
    /// `[src]` The address decoded out of `GP_ENTRY0_GET` / `GP_ENTRY1_GET_HI` is a **GPU
    /// virtual address in the channel's address space**, not a guest-physical one: UVM
    /// computes `pushbuffer_va = uvm_pushbuffer_get_gpu_va_for_push(pushbuffer, push)` and
    /// hands it to `set_gpfifo_entry` unchanged (`ogkm-580:
    /// kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`), and
    /// [`kayfabe_abi::submit::gp_entry_decode`] names its own field `gpu_va`.
    ///
    /// This codec is still **not** the place to translate — inventing a translation here
    /// would be the fabricated-pointer failure this module refuses everywhere else, and
    /// the walk needs a `Vas` no [`PushbufferAbi`] method is handed. What changed is one
    /// level up: [`PushRange::va`] is a [`GpuVa`], and `kayfabe_fwd::read_pushbuffer`
    /// resolves it through the issuing channel's address table before a guest byte is
    /// fetched (`docs/design/execution_plane_increments.md` §8.2.3).
    fn gpfifo_entries(&self, ring: &[u8]) -> Vec<PushRange> {
        if ring.is_empty() || !ring.len().is_multiple_of(submit::GP_ENTRY_SIZE as usize) {
            return Vec::new();
        }
        ring.chunks_exact(submit::GP_ENTRY_SIZE as usize)
            .filter_map(|e| {
                let raw = u64::from_le_bytes(e.try_into().expect("8 bytes"));
                submit::gp_entry_decode(raw).map(|d| PushRange {
                    va: GpuVa(d.gpu_va),
                    len: d.len_bytes,
                })
            })
            .collect()
    }

    /// ★★★ **E5 — the run-aware decode, and the CE `LAUNCH_DMA` operands with it.**
    ///
    /// This is the entry point the owner's 2026-08-02 ruling
    /// (`execution_plane_increments.md` §8.2.1) chose over the two alternatives, and it is
    /// what makes [`PushMethod::CeLaunchDma`] producible at all: a real
    /// `AMPERE_DMA_COPY_B` copy is **five separate method runs** and `LAUNCH_DMA` carries
    /// none of its operands, so a per-method decoder is not merely inconvenient here —
    /// it is *structurally incapable*.
    ///
    /// `state` is the channel's own engine method state ([`MethodState`]); the operands
    /// are latched into it as the runs arrive and `LAUNCH_DMA` fires what has
    /// accumulated. That is the engine's behaviour, mirrored: **following the protocol,
    /// not working around it.**
    ///
    /// # ★★ The three things this walk does, in order
    ///
    /// 1. **`SET_OBJECT` binds a subchannel** — and clears its slots, because a method
    ///    address means whatever the bound class says it means.
    /// 2. **An incrementing run latches** every argument whose *address* this codec
    ///    mirrors a register for, at `method + 4·i`. Written as an address walk rather
    ///    than as a match on the exact runs `rm::ce_pushbuffer` happens to emit — a real
    ///    driver is free to split them differently and the engine would not care.
    /// 3. **`LAUNCH_DMA` fires**, if and only if everything it needs is there.
    ///
    /// # ⊘ What does NOT latch, and why that is a refusal rather than a gap
    ///
    /// Only [`submit::MethodForm::Incrementing`] runs whose argument count matches their
    /// header exactly. A `NON_INC` run writes `count` words to **one** register (last
    /// wins), `ONE_INC` writes the first at `method` and the rest at `method + 4`, and
    /// `IMMD_DATA` carries its datum in the header. Each is a *different* register-write
    /// pattern, this codec models none of them, and modelling one wrongly would put a
    /// number in an operand slot — where it is invisible until it becomes a destination
    /// address. A launch whose operands never arrived does not fire; it stays
    /// [`PushMethod::Opaque`]. Every real writer in reach (`NV_PUSH_nU`, UVM, and
    /// `rm::ce_pushbuffer`) uses `SEC_OP_INC_METHOD`.
    ///
    /// # ⊘ And the slots are NOT cleared when a launch fires
    ///
    /// Because the engine does not clear them. A second `LAUNCH_DMA` with no intervening
    /// operand run re-runs the same copy on real silicon, so it decodes to the same
    /// `CeLaunchDma` here. Bounded by `kayfabe_fwd`'s `MAX_PUSH_TOTAL_BYTES` and
    /// `MAX_CE_SPANS_PER_PARSE`, neither of which this changes.
    fn decode_run(&self, state: &mut MethodState, run: &[(u32, Vec<u32>)]) -> Vec<PushMethod> {
        run.iter()
            .map(|(header, args)| {
                let Some(h) = submit::method_header_decode(*header) else {
                    return PushMethod::Opaque;
                };
                if h.form != submit::MethodForm::Incrementing || args.len() != h.arg_words {
                    // ⊘ Not latched, and not decoded either — `decode_method` refuses the
                    // same two shapes, so this is one rule stated once rather than twice.
                    return self.decode_method(*header, args);
                }
                let subch = h.subchannel as usize;
                let mut fired = None;
                for (i, &arg) in args.iter().enumerate() {
                    // `method + 4·i`: an incrementing header walks consecutive method
                    // ADDRESSES, and the class headers name methods in bytes.
                    let addr = h.method.wrapping_add(4 * (i as u32));
                    if addr == submit::SET_OBJECT {
                        // ★ Binding CLEARS this subchannel — see `MethodState::bind_object`.
                        state.bind_object(subch, ClassId(arg & submit::SET_OBJECT_NVCLASS_MASK));
                    } else if addr == submit::ce::LAUNCH_DMA {
                        // ★ Evaluated where it arrives, not after the loop: a run may
                        // legally carry operand writes *and* the launch, and the launch
                        // must see the state as of its own position in the stream.
                        fired = Ga10xPushbuffer::ce_launch(state, subch, arg);
                    } else if let Some(slot) = Ga10xPushbuffer::ce_slot(addr) {
                        state.latch(subch, slot, arg);
                    }
                }
                // ★ The single-method facts `decode_method` already decoded keep their
                // decode: `SET_OBJECT` is reported as well as bound, and the host-FIFO
                // semaphore and TLB-invalidate runs are untouched by the accumulator
                // (their method addresses are the CHANNEL class's, 0x28 and 0x5c, not the
                // engine object's).
                //
                // ⊘ A fired launch wins, and one `(header, args)` pair still yields one
                // `PushMethod` — so the pathological run that is long enough to cover BOTH
                // `SET_OBJECT` (0x0) and `LAUNCH_DMA` (0x300), 193 arguments on, reports
                // the launch and not the bind. The bind still HAPPENED (the loop above
                // applied it in address order, which is what the engine does); what is
                // lost is only the routing-confirmation report, which no core code acts
                // on. Named rather than left as a surprise; nothing a real writer emits is
                // shaped like that.
                fired.unwrap_or_else(|| self.decode_method(*header, args))
            })
            .collect()
    }
}

/// A pushbuffer codec that recognises no method — kept as the vocabulary's own *"this
/// generation has no method ABI"*.
///
/// ⚠ **No longer what [`Ga10xArch`] answers with**: `E4` built the real one
/// ([`Ga10xPushbuffer`]). Kept for [`UnbuiltGmmu`]'s reason.
#[derive(Debug, Default)]
pub struct UnbuiltPushbuffer;

impl PushbufferAbi for UnbuiltPushbuffer {
    /// `0` argument words. ⚠ Combined with [`Self::gpfifo_entries`] returning nothing,
    /// no method stream is ever walked, so this cannot advance a parser wrongly — it is
    /// unreachable rather than merely conservative.
    fn method_len(&self, _header: u32) -> usize {
        0
    }

    /// [`PushMethod::Opaque`] — the vocabulary's own *"a method this codec does not
    /// recognise"*. Every method is one, which is true.
    fn decode_method(&self, _header: u32, _args: &[u32]) -> PushMethod {
        PushMethod::Opaque
    }

    /// No entries. A GPFIFO ring this codec is shown yields no ranges, so nothing
    /// downstream ever receives a `PushRange` derived from a format nobody validated.
    fn gpfifo_entries(&self, _ring: &[u8]) -> Vec<PushRange> {
        Vec::new()
    }
}

kayfabe_util::assert_send_sync!(
    Ga10xArch,
    Ga10xGmmu,
    Ga10xUserd,
    Ga10xPushbuffer,
    UnbuiltGmmu,
    UnbuiltUserd,
    UnbuiltPushbuffer
);
