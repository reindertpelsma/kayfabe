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
//! ⊘ **One seam inside the codec still refuses, and it is the interesting one.** A real
//! `LAUNCH_DMA` carries **none** of its operands — they were written by *earlier, separate*
//! method runs — and [`PushbufferAbi::decode_method`] is handed one `(header, args)` pair
//! and nothing else. So [`PushMethod::CeLaunchDma`] is **not decodable at this seam at
//! all**, and `LAUNCH_DMA` is [`PushMethod::Opaque`]. `MockArch` hid that by inventing a
//! one-method encoding that packs both operands, a length and a work kind into a single
//! method's arguments. See [`Ga10xPushbuffer`] for the full statement; closing it is `E5`'s
//! decision, not a detail.
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
    Aperture, Arch, DoorbellTarget, GmmuFmt, GmmuVersion, HostClasses, LevelShift, ObjectKind,
    PageSize, PdeEdge, PteDecode, PushMethod, PushRange, PushbufferAbi, UserdModel,
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
        "GA10x (GA106, object model + GMMU + doorbell + USERD/pushbuffer; CE launch operands unbuilt)"
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
            nv::AMPERE_DMA_COPY_B => ObjectKind::EngineObject {
                engine: EngineKind::Ce,
            },
            _ => ObjectKind::Unknown,
        }
    }

    /// ⊘ **`VChid(0)` for every input, and that is a refusal wearing the only shape this
    /// signature allows.** The real recovery is a GA10x `USERD` flag-field decode that
    /// this port has never validated against silicon — `gsp_core_bridge.md` §6 names
    /// *"that the `userd_flags`→`VChid` recovery matches real silicon"* as exactly what
    /// the stage cannot prove. Collapsing every channel to one vChid makes a **second**
    /// channel collide loudly in the core's `by_vchid` index rather than route silently
    /// to the wrong one; the return type has no `Option`, so a loud collision is the
    /// strongest available statement of "unbuilt".
    ///
    /// ## ★★ What the encoding IS, recorded so the increment starts from a reading
    ///
    /// `[src]` RM splits the chid across two **non-adjacent** subfields with a flag bit
    /// between them — `NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE 10:8`,
    /// `_USERD_INDEX_FIXED 11:11`, `_USERD_INDEX_PAGE_VALUE 20:12` (`ogkm-580:
    /// src/common/sdk/nvidia/inc/alloc/alloc_channel.h:184, 186, 201`) — and fills them
    /// from the chid as `VALUE = ChID % n` / `PAGE_VALUE = ChID / n`, where
    /// `n = NVBIT(DRF_SIZE(_USERD_INDEX_VALUE))` = 8 (`ogkm-580:
    /// src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2793, 2800, 2802`). So the recovery
    /// is `PAGE_VALUE * 8 + VALUE`.
    ///
    /// ⊘ **That reading is not a licence to land it**, and this refusal stays until it is
    /// settled the way `E3`'s doorbell token was: by compiling RM's own writer as an
    /// oracle and differentialling the decode against it over the field space
    /// (`isolate_the_drivers_own_checks`; `worksubmit_token_oracle.rs` is the pattern).
    /// A transcription of the three lines above is exactly the instrument that has been
    /// wrong before. Note also that `kayfabe_mocks::MockArch` — the seam's only
    /// non-refusing implementer — packs the chid into **one** contiguous field, so
    /// nothing in the suite has ever exercised a split one.
    fn vchid_from_userd_flags(&self, _flags: u32) -> VChid {
        VChid(0)
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
/// ⊘ **`LAUNCH_DMA` decodes to [`PushMethod::Opaque`], deliberately, and this is E4's
/// most important finding.** A real `AMPERE_DMA_COPY_B` launch carries **none** of its
/// operands: `OFFSET_IN_UPPER…OFFSET_OUT_LOWER` are a *separate earlier run*,
/// `LINE_LENGTH_IN`/`LINE_COUNT` another, `SET_SEMAPHORE_A/B/PAYLOAD` a third, and
/// `LAUNCH_DMA` itself is one word of flags (see `kayfabe_isolate_host::rm::ce_pushbuffer`,
/// which is the encoder a real GA106 executed at rung R17). [`PushbufferAbi::decode_method`]
/// is **per-method and stateless** — it is handed one `(header, args)` pair and nothing
/// else — so it *cannot* produce a [`PushMethod::CeLaunchDma`] whose `dst`, `src` and
/// `len` are anything but invented.
///
/// `MockArch` hid this: `mock_method::CE_LAUNCH_DMA` packs destination, source, length and
/// work kind into **one** method's arguments, an encoding no NVIDIA chip has. So the seam
/// looked sufficient for as long as the only implementer was the mock.
///
/// ⇒ The choice here is the one this module's header paragraph mandates — *"a plausible
/// answer on any of them is worse than a refusal"*. Filling `CeLaunchDma` from a
/// `LAUNCH_DMA` word alone would hand the address plane a destination the guest never
/// wrote, which is a **silent memory-safety fact about the guest**. Closing this needs a
/// seam that can see a *run* of methods, and that is a decision for `E5`, recorded in
/// `docs/design/execution_plane_increments.md`.
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
    /// ## ★★★ UNRESOLVED — this returns a GPU **VA** into a field the core reads as a GPA
    ///
    /// `[src]` The address decoded out of `GP_ENTRY0_GET` / `GP_ENTRY1_GET_HI` is a **GPU
    /// virtual address in the channel's address space**, not a guest-physical one: UVM
    /// computes `pushbuffer_va = uvm_pushbuffer_get_gpu_va_for_push(pushbuffer, push)` and
    /// hands it to `set_gpfifo_entry` unchanged (`ogkm-580:
    /// kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`), and
    /// [`kayfabe_abi::submit::gp_entry_decode`] names its own field `gpu_va`.
    /// `kayfabe_fwd::read_pushbuffer` then passes it to `Vmm::gpa_read` with no walk.
    ///
    /// This codec is **not** the place to fix that — inventing a translation here would be
    /// the fabricated-pointer failure this module refuses everywhere else, and the walk
    /// needs a `Vas` no [`PushbufferAbi`] method is handed. See [`PushRange`]'s type note
    /// and `docs/design/execution_plane_increments.md`.
    fn gpfifo_entries(&self, ring: &[u8]) -> Vec<PushRange> {
        if ring.is_empty() || !ring.len().is_multiple_of(submit::GP_ENTRY_SIZE as usize) {
            return Vec::new();
        }
        ring.chunks_exact(submit::GP_ENTRY_SIZE as usize)
            .filter_map(|e| {
                let raw = u64::from_le_bytes(e.try_into().expect("8 bytes"));
                submit::gp_entry_decode(raw).map(|d| PushRange {
                    gpa: d.gpu_va,
                    len: d.len_bytes,
                })
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
