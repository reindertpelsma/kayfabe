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
//!   a wire-bytes test needed the real ids — but it is in `kayfabe-mocks`, whose manifest
//!   says *"Test-only; never a production dependency"*, and it does not name
//!   `NV20_SUBDEVICE_0` either.
//! - **the data-plane seams.** `MockArch`'s `mmu()`, `userd()` and `pushbuffer()` answer
//!   with made-up geometry and a made-up doorbell encoding. In a *test* that is the point.
//!   In the product it is the measured "mock wall" in its worst form: a plausible answer
//!   on the one axis where a wrong answer is a silent memory-safety fact about the guest.
//!
//! ## ★★ So the data plane REFUSES here, and that is the accurate statement
//!
//! GA10x's real GMMU format, USERD model and pushbuffer codec are **not built** — the
//! data plane is the stage blocked on an owner decision, and nothing in this port has
//! ever walked a page table for a real guest. [`UnbuiltGmmu`], [`UnbuiltUserd`] and
//! [`UnbuiltPushbuffer`] say so in the vocabulary the core already reads: zero levels,
//! no page sizes, `Invalid` for every entry, `Opaque` for every method, no GPFIFO
//! entries. A walker that reaches one **misses**, and a miss is a fault
//! (`mode2_address_table.md`: *"the table IS the guest's TLB; miss = fault"*).
//!
//! ⊘ The one thing that must never happen here is a *plausible* answer. Refusing is not a
//! placeholder for the real codec — it is what keeps "the data plane is unbuilt" a fact
//! the system can state instead of a fact it can only discover by corrupting a guest.
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
use kayfabe_arch::gsp::GspModel;
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, VChid};
use kayfabe_arch::{
    Arch, DoorbellTarget, GmmuFmt, GmmuVersion, LevelShift, ObjectKind, PageSize, PteDecode,
    PushMethod, PushRange, PushbufferAbi, UserdModel,
};

/// The GA10x architecture, as the port ships it: a **real** class table over a
/// **refusing** data plane. See the module docs.
#[derive(Debug, Default)]
pub struct Ga10xArch {
    mmu: UnbuiltGmmu,
    userd: UnbuiltUserd,
    push: UnbuiltPushbuffer,
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
        "GA10x (GA106, object model only — data plane unbuilt)"
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
    fn vchid_from_userd_flags(&self, _flags: u32) -> VChid {
        VChid(0)
    }

    /// `None` — no doorbell token decodes. There is no execution plane here to route one
    /// to, and `Option` means this seam can say so exactly.
    fn decode_doorbell(&self, _token: u64) -> Option<DoorbellTarget> {
        None
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
}

/// A GMMU format that decodes **nothing** — GA10x's real one is unbuilt.
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

/// A USERD model with no geometry — GA10x's real one is unbuilt.
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

/// A pushbuffer codec that recognises no method — GA10x's real one is unbuilt.
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

kayfabe_util::assert_send_sync!(Ga10xArch, UnbuiltGmmu, UnbuiltUserd, UnbuiltPushbuffer);
