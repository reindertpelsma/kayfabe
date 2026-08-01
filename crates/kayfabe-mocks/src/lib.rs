//! # kayfabe-mocks — the deterministic, in-process fake GPU harness
//!
//! One mock per port (`mode2_rust_testing_strategy.md` §4), so the logic core runs
//! **end-to-end without a GPU, a hypervisor, or an OS**:
//!
//! - [`MockArch`] — a complete fake GPU generation ("Mockingbird"). Its encodings are
//!   **deliberately not NVIDIA's**: any core code that secretly assumes a real bit
//!   layout fails these tests. It is also the standing proof of the anti-duplication
//!   property: it implements `Arch` exactly the way a real `impl Arch for Ampere`
//!   would, with zero core edits.
//! - [`MockVmm`] — a scripted hypervisor: sparse guest RAM as byte maps, recorded
//!   irqs/slots/traps, and a **virtual clock advanced explicitly by the test**
//!   (determinism is load-bearing; no real timers exist anywhere).
//! - [`MockRmBackend`] / [`MockIsolate`] / [`MockIsolateFactory`] — a fake host RM
//!   per isolate: records every verb, hands out synthetic handles from a
//!   **per-isolate namespace** (so cross-isolate handle reach is detectable), and
//!   can be scripted to fail (`fail_next`) for negative paths.
//!
//! All mocks are pure in-memory state machines: no files, no sockets, no wall clock.
//!
//! ## ★★ Modelling RM's blocking semantics (`host_execution_plane.md` §2)
//!
//! The isolate's defining hazard is **semantic, not temporal**: RM serialises every
//! ioctl-reachable path on the per-client WRITE lock and waits **uninterruptibly**, so a
//! wedged verb puts every sibling of that client into D state. A mock that returns
//! promptly by construction cannot express it, which is why **R1**, **F1** and
//! **I-NOAMP** had never been tested against the hazard they exist for.
//!
//! Three pieces model it, all deterministic (no sleeps — timing jitter in a fixture
//! manufactures flakes and does not model serialisation anyway):
//!
//! 1. [`ClientLock`] — one lock per isolate (= per RM client). Every verb takes it for
//!    its whole duration, so a second verb on the same client **blocks**, while verbs on
//!    other clients proceed. Enabled by [`RmRecorder::serialize_per_client`].
//! 2. [`VerbHold`] parked with `interruptible == false` (via
//!    [`RmRecorder::never_cancels`]) — an **uninterruptible hold**: the verb does not
//!    return until the test releases it, and a cancellation request lands on it with no
//!    effect, which is observable ([`VerbHold::cancel_request_seen`] says the request
//!    arrived, [`RmRecorder::cancels_observed`] stays empty).
//! 3. [`RmRecorder::parked_verbs`] — the ranked-lock mask the park was entered with, so
//!    "no lock is held **across** the wedge" is read off the witness rather than
//!    inspected.
//!
//! Randomised delay is available as **secondary** fuzz only, behind `KAYFABE_SLOW=1`
//! ([`slow_fuzz_enabled`]); no invariant rests on it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use kayfabe_arch::{
    Aperture, Arch, CeWork, DoorbellTarget, GmmuFmt, GmmuVersion, LevelShift, ObjectKind, PageSize,
    PdeEdge, PteDecode, PushMethod, PushRange, PushbufferAbi, UserdModel,
};
use kayfabe_isolate::{
    CancelHandle, CancelReason, CancelSink, CeSource, CeSubCopy, ExportRequest, ExportSource,
    ExportedBacking, HostHandle, Isolate, IsolateFactory, IsolateId, RmBackend, RmError, Txn,
    Worker, WorkerId,
};
use kayfabe_util::Instant;
use kayfabe_vmm::{
    BarId, CoreEvent, FbMeta, GuestRamMap, HostRegion, IrqSpec, Present, PresentError, Prot,
    RamHandle, RamRegionId, RegionKind, SlotId, SurfaceHandle, TrapMode, Vblank, Vmm, VmmError,
};

// ---------------------------------------------------------------------------------
// Bounded termination — the one watchdog every test file shares
// ---------------------------------------------------------------------------------

pub mod watchdog;
pub use watchdog::{WATCHDOG_ENV, WatchdogGuard, watchdog};

// ---------------------------------------------------------------------------------
// MockArch — the "Mockingbird" generation
// ---------------------------------------------------------------------------------

/// Fake class-ID plan of the Mockingbird generation. Values are arbitrary and
/// deliberately unlike any real NVIDIA class id.
pub mod mock_classes {
    use kayfabe_arch::ids::ClassId;

    /// Client root.
    pub const CLIENT: ClassId = ClassId(0xF001);
    /// Device.
    pub const DEVICE: ClassId = ClassId(0xF002);
    /// Subdevice.
    pub const SUBDEVICE: ClassId = ClassId(0xF003);
    /// VASpace.
    pub const VASPACE: ClassId = ClassId(0xF010);
    /// TSG / channel group.
    pub const TSG: ClassId = ClassId(0xF020);
    /// CtxShare / subcontext.
    pub const CTXSHARE: ClassId = ClassId(0xF021);
    /// GR GPFIFO channel.
    pub const CHANNEL_GR: ClassId = ClassId(0xF030);
    /// CE GPFIFO channel.
    pub const CHANNEL_CE: ClassId = ClassId(0xF031);
    /// Compute engine object (GR-compute context).
    pub const COMPUTE: ClassId = ClassId(0xF040);
    /// Copy-engine object.
    pub const DMA_COPY: ClassId = ClassId(0xF041);
    /// Graphics engine object (GR-graphics context; routes scanout to `Present`).
    pub const GRAPHICS: ClassId = ClassId(0xF042);
    /// NVENC encoder object / session.
    pub const NVENC: ClassId = ClassId(0xF043);
    /// Plain memory object (`NV01_MEMORY_*` / `NV_MEMORY_VIRTUAL` shaped).
    pub const MEMORY: ClassId = ClassId(0xF050);
    /// Os-event / notifier object (`NV01_EVENT` shaped).
    pub const EVENT: ClassId = ClassId(0xF051);
}

/// A complete fake GPU generation. See crate docs.
#[derive(Debug, Default)]
pub struct MockArch {
    mmu: MockGmmuFmt,
    userd: MockUserd,
    pushbuffer: MockPushbuffer,
}

impl MockArch {
    /// New Mockingbird arch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inverse of [`Arch::vchid_from_userd_flags`] — lets tests declare a channel
    /// with a chosen vChid through the *encoded* form, like a real guest would.
    #[must_use]
    pub fn userd_flags_for(vchid: VChid) -> u32 {
        // Fake packing: vchid in bits [18:7], marker bits low.
        (u32::from(vchid.0 & 0xfff) << 7) | 0b101
    }

    /// Inverse of [`Arch::decode_doorbell`] — build a valid Mockingbird token.
    #[must_use]
    pub fn token_for(vchid: VChid) -> u64 {
        0xD000_0000_0000_0000 | (u64::from(vchid.0 & 0xfff) << 9)
    }
}

impl Arch for MockArch {
    fn name(&self) -> &'static str {
        "mockingbird"
    }

    fn classify(&self, class: ClassId) -> ObjectKind {
        use mock_classes as c;
        match class {
            c::CLIENT => ObjectKind::Client,
            c::DEVICE => ObjectKind::Device,
            c::SUBDEVICE => ObjectKind::Subdevice,
            c::VASPACE => ObjectKind::VaSpace,
            c::TSG => ObjectKind::Tsg,
            c::CTXSHARE => ObjectKind::CtxShare,
            // A GR-class channel is GrCompute until an engine object refines it
            // (graphics/NVENC arrive as engine objects on a GR channel).
            c::CHANNEL_GR => ObjectKind::Channel {
                engine: EngineKind::GrCompute,
            },
            c::CHANNEL_CE => ObjectKind::Channel {
                engine: EngineKind::Ce,
            },
            c::COMPUTE => ObjectKind::EngineObject {
                engine: EngineKind::GrCompute,
            },
            c::DMA_COPY => ObjectKind::EngineObject {
                engine: EngineKind::Ce,
            },
            c::GRAPHICS => ObjectKind::EngineObject {
                engine: EngineKind::GrGraphics,
            },
            c::NVENC => ObjectKind::EngineObject {
                engine: EngineKind::NvEnc,
            },
            c::MEMORY => ObjectKind::Memory,
            c::EVENT => ObjectKind::Event,
            _ => ObjectKind::Unknown,
        }
    }

    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        VChid(((flags >> 7) & 0xfff) as u16)
    }

    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        // Malformed unless the fake marker byte is present (hostile-bytes posture).
        if token >> 56 != 0xD0 {
            return None;
        }
        Some(DoorbellTarget {
            vchid: VChid(((token >> 9) & 0xfff) as u16),
            // ⊘ Mockingbird has ONE runlist, and this constant is the whole reason
            // `MockArch` could never have caught a GA10x mis-decode: the invented token
            // has no runlist field to get wrong. See `Ga10xArch::decode_doorbell`.
            runlist: kayfabe_arch::ids::RunlistId(0),
        })
    }

    fn mmu(&self) -> &dyn GmmuFmt {
        &self.mmu
    }

    fn userd(&self) -> &dyn UserdModel {
        &self.userd
    }

    // `engine_of_object` is the provided derivation from `classify` — one class
    // table, so the engine mapping cannot drift from the graph's classification.

    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        // Mockingbird's Case-2 (GSP-internal, ack-only) control set.
        matches!(cmd, mock_ctrl::PROMOTE_CTX | mock_ctrl::GET_CTX_BUFFER_INFO)
    }

    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        &self.pushbuffer
    }
}

// ---------------------------------------------------------------------------------
// WireClassArch — MockArch, plus NVIDIA's REAL class ids, for `classify` ONLY
// ---------------------------------------------------------------------------------

/// [`MockArch`] with one seam replaced: [`Arch::classify`] also answers NVIDIA's real
/// class ids.
///
/// ## Why it has to exist
///
/// [`MockArch`]'s class plan is deliberately unlike NVIDIA's — *"any core code that
/// secretly assumes a real bit layout fails these tests"* — and that is right. But it
/// makes the mock unusable for a test driven from **wire bytes**: `NV01_ROOT` is `0x0`,
/// which `MockArch`'s `classify` answers [`ObjectKind::Unknown`] for, so a graph fed from
/// a real `GSP_RM_ALLOC` would declare no namespace at all. `gsp_core_bridge.md` §6's B1
/// row says *"constructs a real `Gpu` (mock arch/isolate)"* without noticing the gap.
///
/// ## ★ Why overriding **only** `classify` is sound, and not laziness
///
/// `classify` is the **only** [`Arch`] seam whose argument comes off the GSP wire on this
/// path. `RmEvent::Alloc::class` is the one field `kayfabe_rmrpc::translate` copies
/// verbatim out of `rpc_gsp_rm_alloc_v03_00.hClass`; every other seam
/// ([`Arch::vchid_from_userd_flags`], [`Arch::decode_doorbell`], [`Arch::mmu`],
/// [`Arch::userd`], [`Arch::is_case2_control`], [`Arch::pushbuffer`]) consumes a value
/// that no wire-bytes test supplies in NVIDIA's encoding. Overriding any of them would
/// import a *real* bit layout into the mock world and retire the standing property above
/// in exchange for nothing.
///
/// ## ★★ The fall-through is load-bearing
///
/// An id this shim does not name is answered by [`MockArch`], so a graph built from
/// `mock_classes` and a graph built from wire class ids classify **identically** under
/// one arch. That is what makes the bridge's strongest oracle expressible at all:
/// `Boundaries(RpcScript) == Boundaries(Scenario)`, where the script declares
/// `ClassId(NV01_ROOT)` and the hand-written reference scenario declares
/// [`mock_classes::CLIENT`], and the projection must not be able to tell.
///
/// The two id spaces do not collide — `mock_classes` is `0xF0xx` and the three ids below
/// are `0x0`/`0x41`/`0x80` — so this **adds** ids and shadows none. A future class needs
/// an arm here *and* a `kayfabe-abi` constant; per that crate's consumer-first rule,
/// neither lands without the other.
#[derive(Debug, Default)]
pub struct WireClassArch(MockArch);

impl WireClassArch {
    /// A shim over a fresh [`MockArch`].
    #[must_use]
    pub fn new() -> WireClassArch {
        WireClassArch(MockArch::new())
    }

    /// The wrapped mock, for the seams a test drives directly.
    #[must_use]
    pub fn mock(&self) -> &MockArch {
        &self.0
    }
}

impl Arch for WireClassArch {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    /// NVIDIA's real ids first, then [`MockArch`]'s plan.
    ///
    /// The constants are `kayfabe-abi`'s (decision #2's quarantine: every NVIDIA value
    /// lives there, once). `NV01_ROOT` and `NV01_ROOT_CLIENT` are **one resource kind** —
    /// RM's own `is_client_root_class` says so — which is why both map to
    /// [`ObjectKind::Client`] rather than the newer spelling getting an arm of its own.
    ///
    /// ## ★★ The channel arm is where a real `Arch` runs out of information
    ///
    /// There is exactly ONE GPFIFO channel class per architecture. A CUDA process's GR
    /// channel and its CE channel are both `AMPERE_CHANNEL_GPFIFO_A`, and what separates
    /// them is `NV_CHANNEL_ALLOC_PARAMS.engineType` — a *params* fact, which
    /// `kayfabe_core::rmgraph::RmEvent::Alloc` has nowhere to carry. So `classify` cannot
    /// answer it, and does not pretend to: the class maps to
    /// [`EngineKind::GrCompute`] and the CE channel becomes a CE channel only when its
    /// `AMPERE_DMA_COPY_B` engine object arrives and `kayfabe_core::project`'s refinement
    /// pass rewrites it. That is [`MockArch`]'s documented behaviour too (*"a GR-class
    /// channel is GrCompute until an engine object refines it"*) — the mock was right
    /// about the shape before the wire ids showed up to confirm it.
    fn classify(&self, class: ClassId) -> ObjectKind {
        use kayfabe_abi::generated::classes as nv;
        match class.0 {
            nv::NV01_ROOT | nv::NV01_ROOT_CLIENT => ObjectKind::Client,
            nv::NV01_DEVICE_0 => ObjectKind::Device,
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
            _ => self.0.classify(class),
        }
    }

    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        self.0.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.0.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.0.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.0.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.0.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.0.pushbuffer()
    }
}

/// Mockingbird Case-2 (GSP-internal, ack-only) control commands. Deliberately-fake
/// values; a real arch sources these from its Axis-A tables.
pub mod mock_ctrl {
    use kayfabe_arch::ids::ControlCmd;

    /// `PROMOTE_CTX`-shaped: the host already promoted its own GR ctx (Case-1).
    pub const PROMOTE_CTX: ControlCmd = ControlCmd(0x2080_012b);
    /// `GET_CTX_BUFFER_INFO`-shaped: re-derived host-side, ack-only.
    pub const GET_CTX_BUFFER_INFO: ControlCmd = ControlCmd(0x2080_1219);
    /// A forwardable (Case-1) control — NOT ack-only (used to prove the split).
    pub const FORWARDABLE: ControlCmd = ControlCmd(0x0000_1234);
}

/// The Mockingbird pushbuffer/method ABI. Fake, self-consistent encodings so the ONE
/// core parser can be driven without a GPU. A "method word" is a `(header, args)`
/// pair: the header's high byte is the opcode; args carry addresses/payloads.
#[derive(Debug, Default)]
pub struct MockPushbuffer;

/// Mockingbird method opcodes (high byte of the header word). Fake values.
pub mod mock_method {
    /// `SET_OBJECT`: arg[0] = engine-object class id.
    pub const SET_OBJECT: u8 = 0xA0;
    /// CE `LAUNCH_DMA`: args = [dst_lo, dst_hi, len,
    /// flags(bit0 = dst_is_virtual, bit1 = src_is_virtual, bits[3:2] = work kind),
    /// src_lo, src_hi, fill_pattern]. A short arg list decodes the missing words as zero (decode is
    /// total on hostile input), so the flag layout puts the two operand-form bits where
    /// a truncated stream reads them as "physical", never as a virtual address into
    /// somebody's table.
    pub const CE_LAUNCH_DMA: u8 = 0xB0;
    /// `SEM_RELEASE`: args = [addr_lo, addr_hi, payload_lo, payload_hi].
    pub const SEM_RELEASE: u8 = 0xC0;
    /// `MMU_TLB_INVALIDATE`: args = [pdb_lo, pdb_hi, membar(bit0)].
    pub const TLB_INVALIDATE: u8 = 0xD0;
}

impl MockPushbuffer {
    /// Build one method word: `(header, args)`. Test helper to script pushbuffers.
    #[must_use]
    pub fn method(opcode: u8, args: &[u32]) -> (u32, Vec<u32>) {
        ((u32::from(opcode) << 24) | args.len() as u32, args.to_vec())
    }

    /// Encode a `SET_OBJECT` for `class`.
    #[must_use]
    pub fn set_object(class: ClassId) -> (u32, Vec<u32>) {
        Self::method(mock_method::SET_OBJECT, &[class.0])
    }

    /// Encode a plain CE `LAUNCH_DMA` copy to `dst` for `len` bytes, from an
    /// **untracked virtual source** (`GpuVa(0)`, bound in no test's table).
    ///
    /// Kept at its original three-argument shape on purpose: every existing caller is
    /// asserting something about the *destination*, and a source it never named should
    /// not silently become a physical operand — which is what a widened signature
    /// defaulting to `false` would have done, flipping the C's execute predicate
    /// (`src_phys`) for the whole suite. Callers that care use
    /// [`MockPushbuffer::ce_launch_dma_full`].
    #[must_use]
    pub fn ce_launch_dma(dst: u64, len: u64, dst_is_virtual: bool) -> (u32, Vec<u32>) {
        Self::ce_launch_dma_full(dst, dst_is_virtual, GpuVa(0), true, len, CeWork::Copy)
    }

    /// Encode a CE `LAUNCH_DMA` naming **both** operands and the work kind — the inputs
    /// the C's execute predicate reads (`C: nvkvm_gpu_emul.c:6310`).
    #[must_use]
    pub fn ce_launch_dma_full(
        dst: u64,
        dst_is_virtual: bool,
        src: GpuVa,
        src_is_virtual: bool,
        len: u64,
        work: CeWork,
    ) -> (u32, Vec<u32>) {
        let (work_bits, pattern): (u32, u32) = match work {
            CeWork::Copy => (0, 0),
            CeWork::Scrub => (1, 0),
            CeWork::Fill { pattern } => (2, pattern),
        };
        Self::method(
            mock_method::CE_LAUNCH_DMA,
            &[
                dst as u32,
                (dst >> 32) as u32,
                len as u32,
                u32::from(dst_is_virtual) | (u32::from(src_is_virtual) << 1) | (work_bits << 2),
                src.0 as u32,
                (src.0 >> 32) as u32,
                pattern,
            ],
        )
    }

    /// Encode a `SEM_RELEASE` of `addr` to `payload`.
    #[must_use]
    pub fn sem_release(addr: u64, payload: u64) -> (u32, Vec<u32>) {
        Self::method(
            mock_method::SEM_RELEASE,
            &[
                addr as u32,
                (addr >> 32) as u32,
                payload as u32,
                (payload >> 32) as u32,
            ],
        )
    }

    /// Encode an `MMU_TLB_INVALIDATE` of `pdb`.
    #[must_use]
    pub fn tlb_invalidate(pdb: u64, membar: bool) -> (u32, Vec<u32>) {
        Self::method(
            mock_method::TLB_INVALIDATE,
            &[pdb as u32, (pdb >> 32) as u32, u32::from(membar)],
        )
    }
}

impl PushbufferAbi for MockPushbuffer {
    fn method_len(&self, header: u32) -> usize {
        // Fake convention: the header's low 16 bits are the arg-word count (set by
        // `method`). Capped so a hostile header cannot request an unbounded read.
        (header & 0xffff).min(16) as usize
    }

    fn decode_method(&self, header: u32, args: &[u32]) -> PushMethod {
        let lo64 = |i: usize| u64::from(*args.get(i).unwrap_or(&0));
        let pair = |i: usize| lo64(i) | (lo64(i + 1) << 32);
        match (header >> 24) as u8 {
            mock_method::SET_OBJECT => PushMethod::SetObject {
                class: ClassId(args.first().copied().unwrap_or(0)),
            },
            mock_method::CE_LAUNCH_DMA => PushMethod::CeLaunchDma {
                dst: GpuVa(pair(0)),
                src: GpuVa(pair(4)),
                len: lo64(2),
                dst_is_virtual: lo64(3) & 1 != 0,
                src_is_virtual: lo64(3) & 2 != 0,
                // An unknown kind decodes as a plain copy — never guessed into a scrub,
                // which is a *destroying* operation (trap-min: no guessed semantics).
                work: match (lo64(3) >> 2) & 3 {
                    1 => CeWork::Scrub,
                    2 => CeWork::Fill {
                        pattern: lo64(6) as u32,
                    },
                    _ => CeWork::Copy,
                },
            },
            mock_method::SEM_RELEASE => PushMethod::SemRelease {
                addr: GpuVa(pair(0)),
                payload: pair(2),
            },
            mock_method::TLB_INVALIDATE => PushMethod::TlbInvalidate {
                pdb: Pdb(pair(0)),
                membar: lo64(2) & 1 != 0,
            },
            // Anything else is opaque — passed through, acted on by no core code.
            _ => PushMethod::Opaque,
        }
    }

    fn gpfifo_entries(&self, ring: &[u8]) -> Vec<PushRange> {
        // Fake GPFIFO: 16-byte entries, each [gpa: u64 LE, len: u64 LE]. A truncated
        // tail is ignored (a hostile ring must never panic — decode is total).
        ring.chunks_exact(16)
            .map(|e| {
                let gpa = u64::from_le_bytes(e[0..8].try_into().expect("8 bytes"));
                let len = u64::from_le_bytes(e[8..16].try_into().expect("8 bytes"));
                PushRange { gpa, len }
            })
            .collect()
    }
}

/// Fake MMU format: **real VER2 geometry, fake entry encoding**
/// (bit0 = valid, bit1 = leaf, bit2 = sysmem aperture, bit3 = "the big-page sub-table"
/// on a directory entry, phys in bits [51:12]).
///
/// ★ The split is deliberate and is what makes this double useful for the page-table
/// decoder: **the geometry is not fake.** [`MockGmmuFmt::level_shift`] is the C's own
/// table transcribed value-for-value (`C: nvkvm_gpu_emul.c:8706-8708`), because the
/// decoder's whole job is to turn *(table page, level)* into *(virtual address, leaf)*
/// and a made-up stride would make every test about that a test about nothing. Only the
/// **bit layout** of an entry is invented, and that is the half a decoder is
/// architecturally forbidden to know anyway (`PteDecode`'s no-raw-bits discipline).
#[derive(Debug, Default)]
pub struct MockGmmuFmt;

/// Mockingbird leaf sizes (includes a huge leaf so the #13 "every real page size"
/// discipline is exercised by construction).
///
/// ★ These are exactly `1 << shift` of [`MOCK_LEVELS`] rows 4, 5, 3 and 2 — the four
/// levels at which the measured regime can put a leaf. `MockGmmuFmt::decode_entry` sizes
/// a leaf from its level for that reason, so a leaf found at a level with **no**
/// corresponding page size (rows 0 and 1) produces a size that is not in this list —
/// which is precisely the un-enumerated size the walker must fault on rather than drop.
pub const MOCK_PAGE_SIZES: [PageSize; 4] = [
    PageSize(0x1000),
    PageSize(0x10000),
    PageSize(0x200000),
    PageSize(0x2000_0000),
];

/// ★★★ **The C's level table, transcribed** (`C: nvkvm_gpu_emul.c:8706-8708`):
/// `{47,512},{38,512},{29,512},{21,256},{12,512},{16,32}`.
///
/// Row `n` is the C's `m2_cpt` level `n`: 0..=2 are the three 8-byte page directories,
/// 3 is the dual-entry directory (16-byte slots, 256 of them), and 4 / 5 are the **two
/// alternative leaf tables** under it — small pages and big pages. 4 and 5 are siblings,
/// not a sequence, which is why an edge into one of them carries its own
/// [`PteDecode::Pde::child_level`] and the walker never increments.
pub const MOCK_LEVELS: [LevelShift; 6] = [
    LevelShift {
        shift: 47,
        entries: 512,
    },
    LevelShift {
        shift: 38,
        entries: 512,
    },
    LevelShift {
        shift: 29,
        entries: 512,
    },
    LevelShift {
        shift: 21,
        entries: 256,
    },
    LevelShift {
        shift: 12,
        entries: 512,
    },
    LevelShift {
        shift: 16,
        entries: 32,
    },
];

/// The level whose slots are 16 bytes wide (the dual-entry directory).
pub const MOCK_DUAL_LEVEL: u8 = 3;

impl MockGmmuFmt {
    /// Encode a fake leaf entry (test helper; inverse of `decode_entry`).
    #[must_use]
    pub fn encode_leaf(phys: u64, sysmem: bool) -> u128 {
        u128::from((phys & 0x000f_ffff_ffff_f000) | 0b011 | if sysmem { 0b100 } else { 0 })
    }

    /// Encode a fake directory entry pointing at `next`.
    ///
    /// `big` is the mock's one-bit stand-in for the dual slot's second half: set, an
    /// entry at [`MOCK_DUAL_LEVEL`] names the **big-page** table (level 5, 32 entries,
    /// 64 KiB stride) instead of the small-page one (level 4, 512 entries, 4 KiB). It
    /// exists so `child_level` has a live value other than `level + 1`; a field whose
    /// only observed value is the default is a field no test can be wrong about.
    #[must_use]
    pub fn encode_pde(next: u64, sysmem: bool, big: bool) -> u128 {
        u128::from(
            (next & 0x000f_ffff_ffff_f000)
                | 0b001
                | if sysmem { 0b100 } else { 0 }
                | if big { 0b1000 } else { 0 },
        )
    }

    /// ★★ Encode a **DUAL** directory entry at [`MOCK_DUAL_LEVEL`] — one 16-byte slot
    /// naming a small-page table **and** a big-page table.
    ///
    /// The real thing is a 16-byte `DUAL_PDE` whose two halves are two independent
    /// sub-table pointers (`ogkm-580:
    /// src/common/inc/swref/published/pascal/gp100/dev_mmu.h:112`). The mock keeps the
    /// same shape at the same width: the low 64 bits are the small-page edge and the
    /// high 64 bits are the big-page edge, either of which may be zero for "this half
    /// names nothing".
    #[must_use]
    pub fn encode_dual_pde(small: u64, big: u64) -> u128 {
        let half = |p: u64, b: bool| {
            if p == 0 {
                0u128
            } else {
                Self::encode_pde(p, false, b)
            }
        };
        half(small, false) | (half(big, true) << 64)
    }

    /// Encode a fake **sparse** entry — the third state (`PteDecode::Sparse`).
    ///
    /// Distinguishable from invalid (all-zero, what `MMU_WALK_FILL_INVALID` writes) and
    /// from a leaf, which is the entire point of the variant.
    #[must_use]
    pub const fn encode_sparse() -> u128 {
        0b1_0000
    }
}

impl GmmuFmt for MockGmmuFmt {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }
    fn page_sizes(&self) -> &[PageSize] {
        &MOCK_PAGE_SIZES
    }
    fn entry_size(&self, level: u8) -> u8 {
        // The dual-entry directory's slots are 16 bytes wide; every other level's are 8.
        // `256 * 16` and `512 * 8` are both exactly one 4 KiB page, which is the fact
        // that makes the geometry self-consistent.
        if level == MOCK_DUAL_LEVEL { 16 } else { 8 }
    }
    fn levels(&self) -> u8 {
        5
    }
    fn level_shift(&self, level: u8) -> Option<LevelShift> {
        MOCK_LEVELS.get(usize::from(level)).copied()
    }
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        let hi = (raw >> 64) as u64;
        let raw = raw as u64;
        // Sparse is checked FIRST and on its own bit: it is neither valid nor absent, and
        // a decoder that reaches the valid-bit test before it can only answer one of the
        // two things sparse is not (`reachability_on_transition.md` §3.6).
        if raw & 0b1_0000 != 0 {
            return PteDecode::Sparse;
        }
        if raw & 0b1 == 0 {
            return PteDecode::Invalid;
        }
        let phys = raw & 0x000f_ffff_ffff_f000;
        let aperture = if raw & 0b100 != 0 {
            Aperture::SysmemCoherent
        } else {
            Aperture::Vidmem
        };
        if raw & 0b10 != 0 {
            // A leaf at level L maps exactly the bytes L's entries stride — that is what
            // a leaf at that level MEANS, and it is why the 512 MiB leaf lives at the
            // 29-bit row. Deliberately NOT clamped into `MOCK_PAGE_SIZES`: a leaf at a
            // level the regime has no page size for must reach the walker as the size it
            // literally claims, so the "un-enumerated size is a loud fault" rule has
            // something to fire on.
            let size = self.level_shift(level).map_or(PageSize(0), |g| {
                PageSize(1u64.checked_shl(u32::from(g.shift)).unwrap_or(0))
            });
            PteDecode::Leaf {
                phys,
                aperture,
                size,
                read_only: false,
            }
        } else {
            let edge = |w: u64| PdeEdge {
                next: w & 0x000f_ffff_ffff_f000,
                aperture: if w & 0b100 != 0 {
                    Aperture::SysmemCoherent
                } else {
                    Aperture::Vidmem
                },
                child_level: if level == MOCK_DUAL_LEVEL && w & 0b1000 != 0 {
                    5
                } else {
                    level + 1
                },
            };
            PteDecode::Pde {
                edge: edge(raw),
                // ★ The dual slot's SECOND half — present only at the 16-byte level, and
                // only when that half names something. Everywhere else the high 64 bits
                // of the `u128` are padding the walker zero-filled, so answering `Some`
                // from them would invent an edge out of a slot that is 8 bytes wide.
                also: if level == MOCK_DUAL_LEVEL && hi != 0 && hi & 0b1 != 0 {
                    Some(edge(hi))
                } else {
                    None
                },
            }
        }
    }
}

/// Fake USERD geometry.
#[derive(Debug, Default)]
pub struct MockUserd;

impl UserdModel for MockUserd {
    fn userd_size(&self) -> u64 {
        0x400
    }
    fn gp_get_offset(&self) -> u64 {
        0x110
    }
    fn gp_put_offset(&self) -> u64 {
        0x118
    }
}

// ---------------------------------------------------------------------------------
// MockVmm — the scripted hypervisor with a virtual clock
// ---------------------------------------------------------------------------------

/// A recorded memslot installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotRecord {
    /// Guest-physical start.
    pub gpa: u64,
    /// Length.
    pub len: u64,
    /// Backing named by the core.
    pub backing: HostRegion,
    /// Protection / overlay mode.
    pub prot: Prot,
    /// True if installed via `map_read_native`.
    pub read_native: bool,
}

/// A refused guest-physical access, recorded so a test can assert the instrument was
/// **reached** rather than merely silent (`testing_doctrine.md` §1 rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefusedAccess {
    /// The address the access started at.
    pub gpa: u64,
    /// Its length in bytes.
    pub len: u64,
    /// `true` for a write, `false` for a read.
    pub write: bool,
    /// The exact refusal.
    pub err: VmmErrorKind,
}

/// Discriminator for [`RefusedAccess::err`] (a `Copy` shadow of the `VmmError` variants
/// this path can produce, so a recorded refusal is comparable without cloning payloads
/// that are already in `gpa`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VmmErrorKind {
    /// The range resolved to a device register window.
    NonRamGpa,
    /// The range resolved to nothing, or left its region, or was un-formable.
    BadGpa,
}

/// The scripted hypervisor (testing strategy §4). Guest RAM is a sparse byte map;
/// time only moves when the test calls [`MockVmm::advance`].
///
/// ## ★ The guest-physical region map (2026-07-27)
///
/// Before this existed the mock could not express the hazard `l1_os_shell.md` §10.1
/// item 6 names at all: **every** GPA was RAM, including addresses that on a real
/// backend are device registers, and `gpa_read` had no failing arm whatsoever. A test
/// that "proved" the core refuses a device-aimed descriptor would have been a green
/// instrument on a path the harness could not construct — `testing_doctrine.md` §1.
///
/// So [`MockVmm::new`] starts as [`GuestRamMap::all_ram`] — the *historical* semantics,
/// spelled out rather than implied — and a test **narrows** it with
/// [`MockVmm::declare_device_mmio`] / [`MockVmm::declare_unbacked`].
#[derive(Debug)]
pub struct MockVmm {
    ram: BTreeMap<u64, u8>,
    /// The guest-physical region map every `gpa_read`/`gpa_write` is proven against.
    regions: GuestRamMap,
    /// Every refused guest-physical access, in order (assert the hazard was reached).
    pub refused: Vec<RefusedAccess>,
    /// Installed slots by id (public for assertions).
    pub slots: BTreeMap<SlotId, SlotRecord>,
    /// Every `raise_irq` in order (assert completions were delivered).
    pub irqs: Vec<IrqSpec>,
    /// Every `set_trap` registration in order.
    pub traps: Vec<(BarId, Range<u64>, TrapMode)>,
    /// Every `export_ram` call.
    pub exports: Vec<Option<Range<u64>>>,
    /// ★ The SHARED deadline queue (`l1_os_shell.md` §6.4): one implementation in the
    /// port crate, used verbatim by the mock and by the real backend. A backend that
    /// re-implemented it could order timers differently from the one every deterministic
    /// test was written against.
    deferred: kayfabe_vmm::DeferQueue,
    now: Instant,
    next_slot: u64,
}

/// The region id [`MockVmm::new`]'s default all-RAM declaration uses.
pub const MOCK_RAM_REGION: RamRegionId = RamRegionId(0);

// ★ Hand-written rather than derived: a DERIVED `Default` would produce an EMPTY
// region map, in which every GPA refuses. Two constructors that disagree about whether
// guest memory exists is exactly the "green instrument" shape this map was added to
// remove, so `default()` is `new()` and there is no second answer.
impl Default for MockVmm {
    fn default() -> Self {
        Self::new()
    }
}

impl MockVmm {
    /// Fresh VMM at virtual time zero, with the whole guest-physical space declared
    /// RAM (the mock's historical semantics — narrow it with
    /// [`MockVmm::declare_device_mmio`] / [`MockVmm::declare_unbacked`]).
    #[must_use]
    pub fn new() -> Self {
        Self {
            ram: BTreeMap::new(),
            regions: GuestRamMap::all_ram(MOCK_RAM_REGION),
            refused: Vec::new(),
            slots: BTreeMap::new(),
            irqs: Vec::new(),
            traps: Vec::new(),
            exports: Vec::new(),
            deferred: kayfabe_vmm::DeferQueue::new(),
            now: Instant::ZERO,
            next_slot: 0,
        }
    }

    /// ★ Declare `range` to be a **device register window** — another emulated device's
    /// BAR, the platform's MMIO, one of our own trapped BARs. Every `gpa_read` /
    /// `gpa_write` touching it refuses with [`VmmError::NonRamGpa`].
    ///
    /// This is the only way to model the `l1_os_shell.md` §10.1 item 6 hazard: on a
    /// QEMU backend the *same* entry point that memcpys RAM takes the VMM's global lock
    /// for one of these (`[src] v10.2.0 system/physmem.c:3250`, `:3347` →
    /// `:3196-3209`), which is why the port refuses instead of serving.
    pub fn declare_device_mmio(&mut self, range: Range<u64>) {
        self.regions
            .declare(
                // A distinct id per declared window, so a test that asserts a resolved
                // span cannot confuse two of them. The top bit keeps device ids clear
                // of `MOCK_RAM_REGION`.
                RamRegionId((1u64 << 63) | range.start),
                RegionKind::Device,
                range.start,
                range.end - range.start,
            )
            .expect("a device window inside the 64-bit space");
    }

    /// Declare `range` to be backed by nothing at all (a hole in the flat view).
    /// Accesses refuse with [`VmmError::BadGpa`] — the near neighbour of
    /// [`VmmError::NonRamGpa`] that must never start reporting as it.
    pub fn declare_unbacked(&mut self, range: Range<u64>) {
        self.regions.undeclare(range.start, range.end - range.start);
    }

    /// Read-only view of the region map (for tests that assert the resolver directly).
    #[must_use]
    pub fn regions(&self) -> &GuestRamMap {
        &self.regions
    }

    /// Prove a guest-physical range is RAM, recording the refusal if it is not — the
    /// mock's instance of the one central check ([`GuestRamMap::resolve`]).
    fn prove_ram(&mut self, gpa: u64, len: u64, write: bool) -> Result<(), VmmError> {
        match self.regions.resolve(gpa, len) {
            Ok(_) => Ok(()),
            Err(e) => {
                let kind = match e {
                    VmmError::NonRamGpa { .. } => VmmErrorKind::NonRamGpa,
                    _ => VmmErrorKind::BadGpa,
                };
                self.refused.push(RefusedAccess {
                    gpa,
                    len,
                    write,
                    err: kind,
                });
                Err(e)
            }
        }
    }

    /// Advance the virtual clock by `d`, returning every deferred [`CoreEvent`]
    /// that became due, in order. The test then feeds them to the core — exactly
    /// what a real adapter's serialized executor would do.
    pub fn advance(&mut self, d: Duration) -> Vec<CoreEvent> {
        self.now = self.now.advanced(d);
        self.deferred.due(self.now)
    }

    /// Test helper: read back guest RAM (missing bytes read as 0).
    #[must_use]
    pub fn ram_read(&self, gpa: u64, len: usize) -> Vec<u8> {
        (0..len as u64)
            .map(|i| *self.ram.get(&(gpa + i)).unwrap_or(&0))
            .collect()
    }
}

impl Vmm for MockVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        // ★ PROVE FIRST. A hostile GPFIFO entry names this `gpa`, so the range is
        // proven to lie in one declared RAM region before a single byte is touched —
        // the port contract on `Vmm::gpa_read`. This also subsumes the old `checked_add`
        // guard: a range near `u64::MAX` that would wrap is `BadGpa` rather than a
        // wrap-around read of sparse zeros (which no real adapter could serve).
        self.prove_ram(gpa, buf.len() as u64, false)?;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = gpa
                .checked_add(i as u64)
                .and_then(|a| self.ram.get(&a))
                .copied()
                .unwrap_or(0);
        }
        Ok(())
    }

    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        self.prove_ram(gpa, buf.len() as u64, true)?;
        for (i, &b) in buf.iter().enumerate() {
            if let Some(a) = gpa.checked_add(i as u64) {
                self.ram.insert(a, b);
            }
        }
        Ok(())
    }

    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        self.slots.insert(
            id,
            SlotRecord {
                gpa,
                len,
                backing,
                prot,
                read_native: false,
            },
        );
        Ok(id)
    }

    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        self.slots
            .remove(&slot)
            .map(|_| ())
            .ok_or(VmmError::BadSlot(slot))
    }

    fn set_trap(&mut self, bar: BarId, range: Range<u64>, mode: TrapMode) -> Result<(), VmmError> {
        self.traps.push((bar, range, mode));
        Ok(())
    }

    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        self.irqs.push(irq);
        Ok(())
    }

    fn export_ram(&mut self, slice: Option<Range<u64>>) -> Result<RamHandle, VmmError> {
        self.exports.push(slice.clone());
        Ok(RamHandle {
            token: self.exports.len() as u64,
            covers: slice,
        })
    }

    fn defer(&mut self, after: Duration, event: CoreEvent) {
        self.deferred.push(self.now, after, event);
    }

    fn now(&self) -> Instant {
        self.now
    }

    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        self.slots.insert(
            id,
            SlotRecord {
                gpa,
                len,
                backing,
                prot: Prot::ReadOnly,
                read_native: true,
            },
        );
        if let Some(r) = write_trap {
            self.traps.push((BarId::Bar0, r, TrapMode::WriteOnly));
        }
        Ok(id)
    }
}

// ---------------------------------------------------------------------------------
// MockRmBackend / MockIsolate — a fake host RM with per-isolate namespaces
// ---------------------------------------------------------------------------------

/// One recorded RM verb (the forwarded-op log the tests assert on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RmVerb {
    /// Generic alloc.
    Alloc {
        /// Parent handle.
        parent: HostHandle,
        /// Class id.
        class: ClassId,
        /// Returned handle.
        handle: HostHandle,
    },
    /// Intent: an engine object allocated on a host channel (the Case-1 forward).
    AllocEngineObject {
        /// The host channel it was allocated on.
        chan: HostHandle,
        /// The engine-object class.
        class: ClassId,
        /// Returned handle.
        handle: HostHandle,
    },
    /// Intent: host VAS allocated.
    AllocVaSpace {
        /// Returned handle.
        handle: HostHandle,
    },
    /// Intent: host sysmem allocated.
    AllocSysmem {
        /// Returned handle.
        handle: HostHandle,
        /// Requested length.
        len: u64,
    },
    /// Intent: host channel allocated on a host VAS, on a declared engine/runlist.
    AllocChannel {
        /// The host VAS.
        vas: HostHandle,
        /// The engine/runlist the channel was declared for (GR-1: the wrong-runlist
        /// class is pinned by asserting THIS field, per channel).
        engine: EngineKind,
        /// Returned channel handle.
        handle: HostHandle,
        /// Returned host work-submit token.
        token: u64,
    },
    /// Intent: channel made runnable.
    Schedule {
        /// The channel.
        chan: HostHandle,
    },
    /// Host GPU VA mapped.
    MapGpuVa {
        /// The host VAS mapped into.
        vas: HostHandle,
        /// The memory object.
        memory: HostHandle,
        /// Length.
        len: u64,
        /// Returned host GPU VA.
        va: u64,
    },
    /// Host GPU VA unmapped.
    UnmapGpuVa {
        /// The host VAS.
        vas: HostHandle,
        /// The VA.
        va: u64,
    },
    /// Doorbell rung with a host token.
    RingDoorbell {
        /// The token.
        token: u64,
    },
    /// ★★★ #102 stage C2 — ONE sub-copy of a partitioned copy-engine request, as it was
    /// actually performed. The `by` field is what makes "this range went to real
    /// hardware and that one did not" an assertable fact rather than an inference.
    CeCopy {
        /// The host VAS the addresses live in.
        vas: HostHandle,
        /// The sub-copy, exactly as the plan built it.
        sub: CeSubCopy,
    },
    /// ★★★ #102 stage C3 — one read of the fabricated aperture, and **whether it was
    /// covered**. The `covered` field is recorded because "the walker faulted" and "the
    /// walker was never asked" are two different failures and a log of reads alone
    /// cannot tell them apart.
    FbRead {
        /// The physical address read.
        phys: u64,
        /// How many bytes were asked for.
        len: u64,
        /// Whether a declared aperture extent covered the range.
        covered: bool,
    },
    /// Intent: a host memory object exported as a presentable surface (the display
    /// seam's producer half, GR-2b — the isolate-side PRIME export).
    ExportSurface {
        /// The host memory object (render target) that was exported.
        memory: HostHandle,
        /// The minted surface token.
        surface: SurfaceHandle,
    },
    /// ★★★ Intent: a backing exported to the VMM for a guest memslot — decision (b)
    /// (`RmBackend::export_backing`).
    ///
    /// Records the SOURCE, because the whole point of the verb is that its two sources
    /// have opposite outcomes and a log that recorded only "an export happened" could not
    /// tell a fabricated backing from a refused device one.
    ExportBacking {
        /// Which class of bytes was asked for.
        source: ExportSource,
        /// How many bytes.
        len: u64,
        /// The token minted, or `None` when the request was refused.
        token: Option<u64>,
    },
    /// Object freed.
    Free {
        /// The handle.
        obj: HostHandle,
    },
    /// Control issued.
    Control {
        /// Target object.
        obj: HostHandle,
        /// Command.
        cmd: ControlCmd,
    },
}

/// Which verb a [`HoldSpec`] selects. Coarser than [`RmVerb`] (no payload), because
/// a hold is armed *before* the verb runs and can only match on identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerbKind {
    /// [`RmBackend::alloc`].
    Alloc,
    /// [`RmBackend::alloc_vaspace`].
    AllocVaSpace,
    /// [`RmBackend::alloc_sysmem`].
    AllocSysmem,
    /// [`RmBackend::alloc_channel`].
    AllocChannel,
    /// [`RmBackend::alloc_engine_object`].
    AllocEngineObject,
    /// [`RmBackend::schedule`].
    Schedule,
    /// [`RmBackend::free`].
    Free,
    /// [`RmBackend::control`].
    Control,
    /// [`RmBackend::map_gpu_va`].
    MapGpuVa,
    /// [`RmBackend::unmap_gpu_va`].
    UnmapGpuVa,
    /// [`RmBackend::ring_doorbell`].
    RingDoorbell,
    /// [`RmBackend::ce_copy`].
    CeCopy,
    /// [`RmBackend::fb_read`].
    FbRead,
    /// [`RmBackend::export_surface`].
    ExportSurface,
    /// [`RmBackend::export_backing`].
    ExportBacking,
}

/// Which verb occurrence a hold latches onto. Every field is optional; `None`
/// matches anything, so a test can hold "verb X of proc A's worker 0" precisely, or
/// "the next map on any isolate" loosely.
///
/// ★ N3: there is no separate `gpu` field. The target rides inside [`IsolateId`], so a
/// spec that named a proc and a GPU which disagreed with the isolate it was aimed at is
/// no longer expressible — it used to be, and it silently matched nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoldSpec {
    /// Restrict to this isolate session — i.e. to one `(proc, GPU)`.
    pub isolate: Option<IsolateId>,
    /// Restrict to this pool worker slot.
    pub worker: Option<WorkerId>,
    /// Restrict to this verb kind.
    pub verb: Option<VerbKind>,
}

impl HoldSpec {
    /// Hold `verb` on `(isolate, worker)` — the precise form stage 4's mean test
    /// is built on ("hold verb X of proc A's worker 0, let everything else run").
    #[must_use]
    pub fn exact(isolate: IsolateId, worker: WorkerId, verb: VerbKind) -> Self {
        HoldSpec {
            isolate: Some(isolate),
            worker: Some(worker),
            verb: Some(verb),
        }
    }

    /// Hold the next `verb` on `isolate`, whichever worker takes it.
    #[must_use]
    pub fn on_isolate(isolate: IsolateId, verb: VerbKind) -> Self {
        HoldSpec {
            isolate: Some(isolate),
            verb: Some(verb),
            ..Self::default()
        }
    }

    fn matches(&self, id: IsolateId, worker: WorkerId, verb: VerbKind) -> bool {
        self.isolate.is_none_or(|x| x == id)
            && self.worker.is_none_or(|x| x == worker)
            && self.verb.is_none_or(|x| x == verb)
    }
}

/// ★ A scripted **holdable verb**: the latch a test uses to hold one verb *pending*
/// and release it explicitly — no sleeps, no timing, progress-only assertions.
///
/// This is the mechanism the #37 intra-proc invariant is proven with: hold thread A's
/// verb, prove thread B of the SAME proc completes an independent op end to end while
/// it is held, then release and check A's commit. Everything about it is edge-driven
/// ([`VerbHold::wait_until_pending`] blocks until the verb genuinely entered the
/// backend), so a test never has to guess how long "concurrently" takes.
#[derive(Debug)]
pub struct VerbHold {
    state: Mutex<HoldState>,
    cv: std::sync::Condvar,
}

#[derive(Debug, Default)]
struct HoldState {
    entered: bool,
    released: bool,
    /// ★ §7.2 — an out-of-band break signal landed on the worker parked here. Lives in
    /// the HOLD's state, not the cell's, so the park's wait predicate is over exactly one
    /// mutex and the two structures are never held at once (see [`MockCancelSink`]).
    cancelled: Option<CancelReason>,
    /// ★ §7.5 — the requester was abandoned: the reply is never coming.
    abandoned: bool,
}

impl VerbHold {
    /// A fresh, un-entered, un-released hold.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(VerbHold {
            state: Mutex::new(HoldState::default()),
            cv: std::sync::Condvar::new(),
        })
    }

    /// True once a verb has actually entered this hold and is parked in it.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        let g = self.state.lock().expect("hold");
        g.entered && !g.released
    }

    /// Block **this** (test) thread until the held verb has entered the backend —
    /// the progress edge that replaces a sleep.
    ///
    /// ★★ **Bounded, and the bound is a HANG BEING CONVERTED INTO A FAILURE.**
    ///
    /// This was an unbounded `Condvar::wait`, and it is the single commonest way a test
    /// in this workspace wedges: fifty-two call sites arm a hold and then block here, so a
    /// verb that never reaches the backend — because the routing changed, because the
    /// worker died, because a mutant broke dispatch — parks the test thread forever. The
    /// only thing that ever ended it was the file's watchdog, whole minutes later, killing
    /// the entire binary with a `SIGABRT` that named nothing.
    ///
    /// The timeout is **not** the synchronisation mechanism: when the verb arrives, the
    /// condvar returns immediately and the deadline is never consulted. It is only ever
    /// reached by a verb that is genuinely not coming, and it then fails **on the test's
    /// own thread**, with the hold's state, so libtest reports one failed test instead of a
    /// dead process. [`WATCHDOG_ENV`](crate::watchdog::WATCHDOG_ENV) raises it for
    /// sanitizer runs, which is also every file's watchdog knob, so one variable still
    /// covers a slow run.
    ///
    /// # Panics
    /// If the verb has not entered within the bound — deliberately, and by name.
    pub fn wait_until_pending(&self) {
        /// Long enough that reaching it means the verb is not coming at all: the mock
        /// backend does no I/O, so entry is a function call away once the verb is issued.
        const NOT_COMING: Duration = Duration::from_secs(30);

        let bound = crate::watchdog::limit_or_env(NOT_COMING);
        let started = std::time::Instant::now();
        let mut g = self.state.lock().expect("hold");
        while !g.entered {
            let left = match bound.checked_sub(started.elapsed()) {
                Some(d) if !d.is_zero() => d,
                _ => panic!(
                    "★★ HELD VERB NEVER ENTERED THE BACKEND within {bound:?}. This is a \
                     nontermination being reported instead of hung on: the test armed a \
                     hold and blocked for the verb to reach it, and no verb arrived — so \
                     it was never issued, or it was routed somewhere else, or the thread \
                     that should have issued it is itself blocked. Hold state: \
                     entered={} released={} cancelled={:?} abandoned={}. Threads:\n{}",
                    g.entered,
                    g.released,
                    g.cancelled,
                    g.abandoned,
                    crate::watchdog::thread_report(),
                ),
            };
            let (ng, _) = self.cv.wait_timeout(g, left).expect("hold");
            g = ng;
        }
    }

    /// ★★ The break signal that **reached** this parked verb, if any — regardless of
    /// whether the verb honoured it.
    ///
    /// This is the half that makes "the wedged verb was not cancelled" a positive
    /// assertion instead of a vacuous one. `RmRecorder::cancels_observed` staying empty
    /// is equally true of a cancel that was never delivered; pairing it with *this*
    /// says the request arrived at the wait and the wait did not break — which is the
    /// D-state shape, not a broken seam.
    #[must_use]
    pub fn cancel_request_seen(&self) -> Option<CancelReason> {
        self.state.lock().expect("hold").cancelled
    }

    /// True once §7.5's abandon reached this parked verb.
    #[must_use]
    pub fn abandon_seen(&self) -> bool {
        self.state.lock().expect("hold").abandoned
    }

    /// Release the held verb. Idempotent.
    pub fn release(&self) {
        let mut g = self.state.lock().expect("hold");
        g.released = true;
        self.cv.notify_all();
    }

    /// The backend side: mark entered, wake any waiter, and park until released **or
    /// until the worker's out-of-band cancel seam fires** (§7.2/§7.5).
    ///
    /// Returns what the park woke on:
    /// - `Ok(())` — released normally (or the cancel was one the verb ignores).
    /// - `Err(Interrupted)` — a break signal landed and this verb kind observes it.
    /// - `Err(Wedged)` — §7.5's abandon: the requester was released with no reply.
    ///
    /// ★ **`interruptible` is the modelled fact, not a convenience.** RM's waits are
    /// mostly *not* interruptible — the API lock is a `down_write`, the GSP RPC busy-polls
    /// with no signal check (`l1_os_shell.md` §7.9, §12.26) — so a mock in which every
    /// cancel lands would make the whole D-state escape untestable and would prove a
    /// property the host does not have.
    fn enter_and_park(&self, interruptible: bool) -> Result<(), RmError> {
        let mut g = self.state.lock().expect("hold");
        g.entered = true;
        self.cv.notify_all();
        loop {
            if g.abandoned {
                return Err(RmError::Wedged);
            }
            if interruptible && g.cancelled.is_some() {
                return Err(RmError::Interrupted);
            }
            if g.released {
                return Ok(());
            }
            g = self.cv.wait(g).expect("hold");
        }
    }

    /// The cancel side: record the break signal and wake the parked verb. Takes only
    /// this hold's own mutex, and is called with the cell's mutex already **dropped**.
    fn signal(&self, cancelled: Option<CancelReason>, abandoned: bool) {
        let mut g = self.state.lock().expect("hold");
        if let Some(r) = cancelled {
            g.cancelled = Some(r);
        }
        g.abandoned |= abandoned;
        self.cv.notify_all();
    }
}

// ---------------------------------------------------------------------------------
// ★★ The cancel seam (`l1_os_shell.md` §7.1–§7.5)
// ---------------------------------------------------------------------------------

/// One recorded cancellation event — the **cancel observer** the mean run asserts on.
///
/// Two shapes, deliberately separate, because they answer different questions and
/// conflating them is how a cancellation test passes for the wrong reason:
/// *was the signal delivered* (did it name a live txn) versus *did the verb observe it*
/// (did the host wait actually break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelEvent {
    /// The isolate whose slot was signalled.
    pub isolate: IsolateId,
    /// The pool slot.
    pub worker: WorkerId,
    /// The transaction the request named.
    pub txn: Txn,
    /// The reason, or `None` for §7.5's abandon.
    pub reason: Option<CancelReason>,
    /// Whether the txn was still current. `false` = the verb finished first (§7.3's
    /// fourth row) and the request was dropped — a normal outcome, never a failure.
    pub delivered: bool,
}

/// Per-slot cancel state. One mutex; nothing else is ever held with it.
#[derive(Debug, Default)]
struct CancelCell {
    /// This checkout's txn. Monotonic per slot and never reused.
    txn: u64,
    /// True between `checkout` and `checkin` — a cancel outside that window names a
    /// stale txn by definition.
    in_flight: bool,
    /// A delivered-but-not-yet-observed break signal.
    armed: Option<CancelReason>,
    /// A delivered abandon (§7.5).
    abandoned: bool,
    /// What the verb actually observed this checkout (cleared at the next one).
    observed: Option<CancelReason>,
    /// The hold this slot's verb is currently parked in, if any.
    parked: Option<Arc<VerbHold>>,
}

/// ★ The mock's [`CancelSink`]: one per pool slot, shared by the slot's
/// [`kayfabe_isolate::CancelHandle`] and its [`MockRmBackend`].
///
/// It is the *whole* of §7.2's out-of-band path in miniature — a control pipe and a
/// `tgkill` become a flag and a condvar notify — and it deliberately keeps the two
/// properties the C paid for:
///
/// 1. **The stale-txn drop.** `deliver` arms nothing unless the txn is the one currently
///    in flight, so a cancel that races its own verb's completion cannot land on an
///    unrelated later operation.
/// 2. **Armed for exactly ONE ioctl** (§7.2 refinement 4). Observing the signal DISARMS
///    it, so the unwind's own `free`s are not themselves interrupted — without this a
///    cancelled chain leaks everything it had allocated, which is the failure the
///    conservation ledger would report as UNACCOUNTED.
#[derive(Debug)]
pub struct MockCancelSink {
    isolate: IsolateId,
    worker: WorkerId,
    st: Mutex<CancelCell>,
    recorder: SharedRecorder,
}

impl MockCancelSink {
    fn new(isolate: IsolateId, worker: WorkerId, recorder: SharedRecorder) -> Arc<Self> {
        Arc::new(MockCancelSink {
            isolate,
            worker,
            st: Mutex::new(CancelCell::default()),
            recorder,
        })
    }

    /// A fresh checkout: mint the txn, clear every per-checkout fact.
    fn begin(&self, txn: u64) {
        let mut g = self.st.lock().expect("cancel cell");
        g.txn = txn;
        g.in_flight = true;
        g.armed = None;
        g.abandoned = false;
        g.observed = None;
        g.parked = None;
    }

    /// Check-in: the txn is over, so any later request naming it is stale.
    fn end(&self) {
        let mut g = self.st.lock().expect("cancel cell");
        g.in_flight = false;
        g.armed = None;
        g.parked = None;
    }

    /// The txn currently in flight, if any (the handle mint reads this).
    fn current_txn(&self) -> Option<Txn> {
        let g = self.st.lock().expect("cancel cell");
        g.in_flight.then_some(Txn(g.txn))
    }

    /// Publish the hold a verb is about to park in (if any), and answer whether a signal
    /// is **already** waiting — `(armed break signal, abandoned)`.
    ///
    /// The two happen in one critical section so the signal cannot slip between them:
    /// a `deliver` that runs before this call is seen in the returned state, and one that
    /// runs after it finds `parked` published and wakes the hold.
    fn park_in_and_peek(&self, hold: Option<&Arc<VerbHold>>) -> (Option<CancelReason>, bool) {
        let mut g = self.st.lock().expect("cancel cell");
        g.parked = hold.map(Arc::clone);
        (g.armed, g.abandoned)
    }

    /// The break signal currently armed, if any.
    fn armed_reason(&self) -> Option<CancelReason> {
        self.st.lock().expect("cancel cell").armed
    }

    /// ★★ **SPEND the signal — "armed for exactly ONE ioctl" (§7.2 refinement 4), for the
    /// verb that did NOT observe it.**
    ///
    /// [`MockCancelSink::observe`] disarms on the interruptible path. This is the other
    /// path, and forgetting it is worse than forgetting that one: a break signal
    /// delivered into an *uninterruptible* wait (the common case — RM's waits mostly are)
    /// would otherwise stay armed past the verb it named and be observed by whatever ran
    /// next. What runs next, on a refused or cancelled op, is **the disposal** — so the
    /// cleanup gets interrupted and the chain's host objects leak, which is precisely the
    /// failure refinement 4 exists to prevent, arriving through the door nobody watched.
    ///
    /// Measured by `cancellation.rs`'s census: the uninterruptible arm reported
    /// `(fired 1, delivered 1, observed 1)` where the design says the verb never observed
    /// anything — the observation belonged to the `unmap`/`free` two steps later.
    fn spend(&self) {
        self.st.lock().expect("cancel cell").armed = None;
    }

    /// A verb observed the break signal: DISARM (refinement 4) and record what it saw.
    fn observe(&self, reason: CancelReason) {
        let mut g = self.st.lock().expect("cancel cell");
        g.armed = None;
        g.observed = Some(reason);
        drop(g);
        self.recorder
            .lock()
            .expect("recorder")
            .cancels_observed
            .push((self.isolate, self.worker, reason));
    }

    /// The shared body of `deliver`/`abandon`: arm under the cell mutex, then — with it
    /// **dropped** — wake whatever hold the verb is parked in.
    fn fire(&self, txn: Txn, reason: Option<CancelReason>) -> bool {
        let (delivered, hold) = {
            let mut g = self.st.lock().expect("cancel cell");
            if !g.in_flight || g.txn != txn.0 {
                (false, None)
            } else {
                match reason {
                    Some(r) => g.armed = Some(r),
                    None => g.abandoned = true,
                }
                (true, g.parked.clone())
            }
        };
        if delivered && let Some(h) = hold {
            h.signal(reason, reason.is_none());
        }
        self.recorder
            .lock()
            .expect("recorder")
            .cancels_delivered
            .push(CancelEvent {
                isolate: self.isolate,
                worker: self.worker,
                txn,
                reason,
                delivered,
            });
        delivered
    }
}

impl CancelSink for MockCancelSink {
    fn deliver(&self, txn: Txn, reason: CancelReason) -> bool {
        self.fire(txn, Some(reason))
    }
    fn abandon(&self, txn: Txn) -> bool {
        self.fire(txn, None)
    }
    fn observed(&self) -> Option<CancelReason> {
        self.st.lock().expect("cancel cell").observed
    }
}

// ---------------------------------------------------------------------------------
// ★★★ ClientLock — RM's per-client serialisation (`host_execution_plane.md` §2)
// ---------------------------------------------------------------------------------

/// ★ One recorded **block**: a verb that had to wait for the client lock, and the verb
/// it waited behind.
///
/// Content, not a count. A counter answers *"was anything serialised"*, which is
/// equally true of a lock that serialises the wrong pair; this names both sides, so
/// "proc A's worker 1 blocked behind proc A's worker 0" and "…behind proc B's" are
/// different values rather than the same `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientWait {
    /// The client (= isolate) whose lock was contended.
    pub isolate: IsolateId,
    /// The pool slot that had to wait.
    pub waiter: WorkerId,
    /// The verb it was trying to issue.
    pub waiter_verb: VerbKind,
    /// The pool slot that held the lock.
    pub holder: WorkerId,
    /// The verb that was holding it.
    pub holder_verb: VerbKind,
}

#[derive(Debug, Default)]
struct ClientLockState {
    held_by: Option<(WorkerId, VerbKind)>,
    queued: Vec<(WorkerId, VerbKind)>,
    waits: Vec<ClientWait>,
}

/// ★★★ **The per-RM-client serialisation lock** — the modelled half of
/// `rm_concurrency_semantics`'s central measured fact.
///
/// Every ioctl-reachable resource-server entry point passes `LOCK_ACCESS_WRITE` for the
/// client lock — eight `_serverLockClientWithLockInfo(…, LOCK_ACCESS_WRITE, …)` sites at
/// BOTH vendored tags, seven of them at identical lines
/// (`ogkm-610:`/`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_server.c:778`,
/// `:1143`, `:1503`, `:1923`, `:2009`, `:2131`, `:2218`; only the last moved,
/// `ogkm-610: :2546` / `ogkm-580: :2468`), and alloc ASSERTS the client is not held for
/// read (`ogkm-610:`/`ogkm-580: :786-788`, byte-identical). Alloc/free additionally take
/// the **global** API lock in write, held across the GSP RPC. **There is no version seam
/// in RM's locking** — which is what makes this model version-independent rather than a
/// 610-shaped guess. So two ioctls on one client are legal and
/// ordinary but **never concurrent**, and a verb that wedges holds the lock the whole
/// time. gVisor's nvproxy corroborates in production: a per-client exclusive mutex held
/// across the host ioctl (`gvisor: pkg/sentry/devices/nvproxy/frontend_unsafe.go:367-381`).
///
/// One lock per [`IsolateId`], because an isolate is one sandboxed host process with one
/// RM client (`l1_concurrency.md` §7.2). Its whole worker pool shares it — which is the
/// point: the pool buys **liveness isolation**, not wire concurrency, and until this
/// existed the mock claimed the latter.
///
/// ## ★ It is deliberately NOT a ranked lock
///
/// It stands in for a lock inside the **host kernel**, on the far side of the isolate
/// port. Our R1/R3 discipline governs locks we own; the host kernel does not participate
/// in it (`l1_os_shell.md` §6.6: *"we do not create this property and we cannot delete
/// it — we can only amplify it"*). Registering it with
/// [`kayfabe_util::lockwitness`] would make every host verb look like an R1 violation of
/// our own rules, which is a category error.
///
/// ## Off by default, and that is a **recorded divergence**, not an oversight
///
/// See [`RmRecorder::serialize_per_client`].
#[derive(Debug)]
pub struct ClientLock {
    isolate: IsolateId,
    st: Mutex<ClientLockState>,
    cv: std::sync::Condvar,
}

impl ClientLock {
    fn new(isolate: IsolateId) -> Arc<Self> {
        Arc::new(ClientLock {
            isolate,
            st: Mutex::new(ClientLockState::default()),
            cv: std::sync::Condvar::new(),
        })
    }

    /// Take the client lock for `(worker, verb)`, blocking while another verb of the
    /// same client holds it. A block is published in [`ClientLock::queued`] **before**
    /// the wait, so a test has a positive edge to synchronise on
    /// ([`ClientLock::wait_until_blocked`]) instead of a sleep.
    fn acquire(&self, worker: WorkerId, verb: VerbKind) {
        let mut g = self.st.lock().expect("client lock");
        if let Some((hw, hv)) = g.held_by {
            g.waits.push(ClientWait {
                isolate: self.isolate,
                waiter: worker,
                waiter_verb: verb,
                holder: hw,
                holder_verb: hv,
            });
            g.queued.push((worker, verb));
            self.cv.notify_all();
            while g.held_by.is_some() {
                g = self.cv.wait(g).expect("client lock");
            }
            let i = g
                .queued
                .iter()
                .position(|&e| e == (worker, verb))
                .expect("this waiter published itself before waiting");
            g.queued.remove(i);
        }
        g.held_by = Some((worker, verb));
    }

    fn release(&self) {
        let mut g = self.st.lock().expect("client lock");
        g.held_by = None;
        self.cv.notify_all();
    }

    /// The `(worker, verb)` currently holding this client's lock, if any.
    #[must_use]
    pub fn held_by(&self) -> Option<(WorkerId, VerbKind)> {
        self.st.lock().expect("client lock").held_by
    }

    /// The `(worker, verb)` pairs blocked on this client right now, in arrival order.
    #[must_use]
    pub fn queued(&self) -> Vec<(WorkerId, VerbKind)> {
        self.st.lock().expect("client lock").queued.clone()
    }

    /// Every block this client's lock has caused, in order — the exact-content census.
    #[must_use]
    pub fn waits(&self) -> Vec<ClientWait> {
        self.st.lock().expect("client lock").waits.clone()
    }

    /// ★ Block **this** (test) thread until at least `n` verbs are queued behind the
    /// holder — the progress EDGE that replaces a sleep, exactly like
    /// [`VerbHold::wait_until_pending`]. Without it, "the sibling is blocked" could only
    /// be probed as an absence at a moment, which is a race.
    pub fn wait_until_blocked(&self, n: usize) {
        let mut g = self.st.lock().expect("client lock");
        while g.queued.len() < n {
            g = self.cv.wait(g).expect("client lock");
        }
    }
}

/// Releases its [`ClientLock`] on drop — including on every early `return Err` inside
/// [`MockRmBackend::gate`], which is why the lock is a guard rather than a pair of calls.
/// `None` when [`RmRecorder::serialize_per_client`] is off.
#[derive(Debug)]
pub struct ClientGuard(Option<Arc<ClientLock>>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if let Some(l) = &self.0 {
            l.release();
        }
    }
}

/// ★ One verb that **parked** in a [`VerbHold`], with the ranked-lock mask its thread
/// held at the moment it parked.
///
/// R1's own assert fires at [`kayfabe_isolate::Worker::execute`]'s *entry*. This is the
/// stronger, different fact the wedge tests need: **nothing is held across the wait**.
/// Read off [`kayfabe_util::lockwitness::held_mask`] by the parking thread itself, so it
/// is a witness rather than an inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParkedVerb {
    /// The isolate whose verb parked.
    pub isolate: IsolateId,
    /// The pool slot.
    pub worker: WorkerId,
    /// The verb kind.
    pub verb: VerbKind,
    /// The parking thread's ranked-lock mask (bit = rank). MUST be 0.
    pub held_lock_mask: u8,
}

/// ★★ A **bystander hit**: a handle from another client's namespace was presented, the
/// raw value was live *here*, and the host served it against the local object.
///
/// See [`MockRmBackend::check`] for why the double now does this instead of refusing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BystanderHit {
    /// The isolate whose connection the verb ran on.
    pub isolate: IsolateId,
    /// The handle the caller presented (carrying the namespace it really belongs to).
    pub presented: HostHandle,
    /// The **local** object the host actually operated on — the bystander.
    pub hit: HostHandle,
}

/// ★ Is the **secondary** randomised-delay fuzz on (`KAYFABE_SLOW=1`)?
///
/// Deliberately secondary and deliberately opt-in: the modelled hazard is serialisation
/// and uninterruptibility, not slowness, and a fixture whose invariants rest on timing
/// manufactures flakes (this project has paid for three — `reactor_os` #73's 1.15 %
/// race, #69's 2-in-26 teardown, and `9254e85`'s 13 silent park points). Nothing in the
/// mock's *semantics* changes when it is on; it only widens the interleavings a
/// multi-threaded test explores.
#[must_use]
pub fn slow_fuzz_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KAYFABE_SLOW").is_ok_and(|v| v == "1"))
}

/// The secondary fuzz itself: a bounded, cheap yield storm whose length comes from a
/// per-thread xorshift. No sleep, no clock, no allocation — and no effect at all unless
/// [`slow_fuzz_enabled`].
fn slow_fuzz() {
    if !slow_fuzz_enabled() {
        return;
    }
    thread_local! {
        static SEED: std::cell::Cell<u64> = const { std::cell::Cell::new(0x2545_F491_4F6C_DD1D) };
    }
    let n = SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x % 64
    });
    for _ in 0..n {
        std::thread::yield_now();
    }
}

/// Shared recorder: `(isolate, verb)` in global order, so tests can assert both
/// per-isolate behavior and cross-isolate separation. Plus a scriptable failure and
/// the armed [`VerbHold`]s.
#[derive(Debug, Default)]
pub struct RmRecorder {
    /// The global verb log.
    pub log: Vec<(IsolateId, RmVerb)>,
    /// If set, the next verb on ANY isolate fails with this error (then clears).
    pub fail_next: Option<RmError>,
    /// ★ **Standing** per-kind failure injection: every verb of this kind fails with
    /// this error until the entry is removed (`l1_concurrency.md` §12.16).
    ///
    /// [`RmRecorder::fail_next`] is one-shot, which cannot express the case the G4
    /// teardown work needs: a chain whose verb fails **and** whose unwind's own `free`
    /// then fails too — i.e. the only way a `VerbFailure` carries a non-empty
    /// `orphans`. A failing teardown is not an exotic scenario to script; it is the
    /// scenario the residue vocabulary exists for.
    pub fail_kinds: BTreeMap<VerbKind, RmError>,
    /// ★★★ #102 — script the backend into **ignoring the fixed-offset placement**: every
    /// `map_gpu_va` returns `at + this` instead of `at`.
    ///
    /// This models a real, reachable host condition — an RM that does not honour
    /// `DMA_OFFSET_FIXED_TRUE`, or a port that forgot to set it (which is what
    /// `flags: 0, dma_offset: 0` was) — and it is the ONLY lever that makes
    /// [`RmError::PlacementRefused`] fire. A guard nothing can trip is not a guard; this
    /// is how the address-identity check is proven to be live rather than decorative
    /// (`suspect_the_instrument_first`).
    pub placement_drift: Option<u64>,
    /// ★★ **The uninterruptible verbs** (`l1_os_shell.md` §7.9, §12.26): verb kinds whose
    /// host wait a break signal does NOT break.
    ///
    /// This is the modelled half of the fact that makes §7.5 exist at all. RM serialises
    /// every ioctl-reachable path on a `down_write` and busy-polls the GSP RPC with no
    /// signal check, so *"a real RM ioctl is actually interruptible"* is a claim the mock
    /// cannot settle and must therefore be able to **deny**. A cancel aimed at one of
    /// these arms, is delivered, and is simply never observed — which is precisely the
    /// D-state shape the two-stage watchdog escalates out of.
    ///
    /// Empty by default: the ordinary case is that the interrupt lands.
    pub never_cancels: BTreeSet<VerbKind>,
    /// ★ Every cancel/abandon request that was **fired** at a sink, whether or not its
    /// txn was still current. The stale-txn drop is only observable because the dropped
    /// ones are recorded too.
    pub cancels_delivered: Vec<CancelEvent>,
    /// ★ Every cancel a verb actually **observed** — strictly a subset of the delivered
    /// ones, and the two are asserted separately on purpose (see [`CancelEvent`]).
    pub cancels_observed: Vec<(IsolateId, WorkerId, CancelReason)>,
    /// ★★ Verbs that hit an **abandoned** worker (§7.5) — including the one that
    /// discovered the abandon, which is why the correct value is `1` and not `0`.
    ///
    /// It needs its own counter because the outcome is otherwise invisible: a wedged
    /// worker answers every verb with [`RmError::Wedged`], so a chain that *attempts*
    /// its unwind on one leaves byte-identical residue to a chain that correctly does
    /// not. Deleting `Worker::execute`'s wedge short-circuit changed nothing anywhere in
    /// the suite until this existed.
    ///
    /// ★ And it counts the discovering verb **on purpose**. An `== 0` invariant would be
    /// equally true of a counter nobody increments — the vacuity this campaign has now
    /// caught six times. `== 1` fails in both directions: `0` means the instrument is
    /// dead, `> 1` means an unwind ran on a worker that cannot answer.
    ///
    /// It is a real property, not bookkeeping: a real wedged worker's control channel is
    /// gone and its thread is inside the ioctl, so issuing a verb on it is a write to a
    /// dead socket, never a harmless no-op.
    pub verbs_that_hit_an_abandoned_worker: usize,
    /// ★★★ **Serialise every verb of one client behind one lock** — RM's measured
    /// behaviour ([`ClientLock`]). Set it before the first verb; it is read per verb.
    ///
    /// ## Why the default is `false`, stated as the divergence it is
    ///
    /// Turning it on unconditionally **deadlocks `l1_mean`'s composed run**, and that is
    /// the finding rather than a reason to leave it off quietly: the mean run parks two
    /// verbs on the witness's own isolate and then requires three *sibling threads of the
    /// same `Proc`* to complete alloc/map-heavy workloads end to end. Against real RM
    /// they cannot — they are behind the same client's write lock. `l1_os_shell.md`
    /// §6.6 already says so in as many words (*"on real hardware, process A's slow RM
    /// call already makes process B's RM call wait … any claim that our design gives B
    /// independent progress would be claiming something the hardware path does not
    /// have"*), and states I-NOAMP as a **cross-process** obligation; the mean run's
    /// intra-proc progress claim is stronger than the design's, and was passing only
    /// because the double was too polite.
    ///
    /// ## ★ MEASURED, 2026-07-29 — exactly what a `true` default costs
    ///
    /// Forced on for one full `cargo test --workspace --no-fail-fast` run, **twelve**
    /// tests stop terminating and **zero** assertions fail. Every one is a *liveness*
    /// claim, and each names the same premise — *N pool workers of one isolate have N
    /// verbs in flight at once*:
    ///
    /// | test | the claim RM refutes |
    /// |---|---|
    /// | `l1_mean::mean_multiproc_multithread_multigpu_multiworkload` | 3 sibling threads of the witness `Proc` finish while 2 of its verbs are parked |
    /// | `l1_mean::n3_c_…straddler_progresses_under_pending_work_on_both_isolates` | same, per target |
    /// | `l1_mean::a_guest_steering_dma_descriptors_…_under_load` | ditto, DMA arm |
    /// | `l1_mean::a_real_memory_plane_survives_…_under_load` | ditto, KVM arm |
    /// | `l1_mean::trace_arm::the_shared_trace_is_totally_ordered_…` | ditto, trace arm |
    /// | `l1_verb_seam::progress_under_pending_verb_intra_proc` | the #37 invariant, stated directly |
    /// | `l1_verb_seam::poll_and_delivery_need_no_worker_at_full_saturation` | the pool saturated by *concurrent* verbs |
    /// | `retry_ledger::converging_retries_release_every_host_object_they_allocated` | ★ one hold **per pool slot**, all pending at once |
    /// | `retry_ledger::a_retry_whose_replan_diverges_…` | same shape |
    /// | `retry_ledger::the_commit_retry_bound_still_releases_…` | same shape |
    /// | `t0_subset_free::the_drain_never_races_a_verb_in_flight_on_the_same_isolate` | a drain runs while a sibling's verb is in flight |
    /// | `cancellation::every_checked_out_worker_of_a_dying_proc_is_cancelled` | two workers of one proc parked simultaneously |
    ///
    /// **That nothing fails an assertion is the finding's precise shape:** the design is
    /// not *incorrect* under RM's serialisation, it is *slower than the suite assumes*,
    /// and the assumption is load-bearing in twelve places.
    ///
    /// So this stays opt-in until that claim is renegotiated in the doc — the mean test
    /// is not narrowed to accommodate it — and every test that asserts a *blocking*
    /// property turns it on explicitly.
    pub serialize_per_client: bool,
    /// Every isolate's [`ClientLock`], by session — how a test reaches the edge
    /// [`ClientLock::wait_until_blocked`] without owning the factory.
    pub client_locks: BTreeMap<IsolateId, Arc<ClientLock>>,
    /// ★★★ **Every isolate that has been DROPPED, in drop order** (`#130`).
    ///
    /// [`MockIsolateFactory::spawned`] has always recorded the births. Nothing recorded
    /// the deaths, so *"does the isolate die with the device?"* — the owner's
    /// unload-and-reload requirement, and the cross-lifetime hazard the C measured on
    /// real hardware on 2026-06-18 (kernel RM clients hold refcounted references into
    /// **user** memory; the consequence is pinned here by
    /// `tests/tests/cross_proc_lifetime.rs`) — was a question this harness **could not
    /// answer either way**. A property whose instrument
    /// does not exist reads exactly like a property that holds.
    ///
    /// ★ Deaths, not retirements. `Isolate::retire` sets a `bool` and nothing else: it
    /// refuses new checkouts and leaves the sandbox alive, which is correct and is
    /// **not** what a reload needs. Only `Drop` kills — for `HostIsolate` it is
    /// close-fds → `SIGKILL` → blocking `waitpid`
    /// (`kayfabe-isolate-host/src/isolate.rs:810`, `kayfabe-linux-raw/src/spawn_unsafe.rs:980`).
    /// This list is written from the same place, so a test can tell the two apart.
    pub isolates_dropped: Vec<IsolateId>,
    /// ★ Every verb that entered a [`VerbHold`] and the ranked-lock mask it parked with
    /// (see [`ParkedVerb`]).
    pub parked_verbs: Vec<ParkedVerb>,
    /// ★★ Every cross-namespace handle the backend **served** rather than refused — the
    /// bystander hits a real host cannot detect (see [`BystanderHit`]).
    pub bystander_hits: Vec<BystanderHit>,
    /// Armed holds, matched newest-spec-first and consumed one-shot.
    holds: Vec<(HoldSpec, Arc<VerbHold>)>,
    /// ★ Accounting for verbs whose log entries [`RmRecorder::compact`] has already
    /// handed out (`l1_concurrency.md` §12.35). [`RmRecorder::ledger`] starts from this
    /// and folds the remaining [`RmRecorder::log`] on top, so draining the log to bound
    /// memory costs nothing in accounting — which it silently did before.
    carried: HostLedger,
    /// ★★★ #102 stage C2 — **the model GPU memory the two copy engines move bytes
    /// through**, sparse and byte-addressed.
    ///
    /// It exists so the split can be checked the only way that means anything: *the
    /// bytes*. A partition that is off by one page, drops a sub-copy, or gets a
    /// sub-copy's source offset wrong is invisible to any assertion about span COUNTS
    /// and immediately visible here.
    ///
    /// Sparse, and an unwritten byte reads `0` — the C's framebuffer backing has exactly
    /// this shape (*"Our FB backing is sparse-zero (unwritten reads return 0)"*,
    /// `C: nvkvm_gpu_emul.c:6316`), and it is what makes a scrub a no-op there.
    ///
    /// ★ **Device-wide rather than per-isolate, deliberately.** This models memory the
    /// host GPU owns; two isolates copying to the same host address are, on real
    /// hardware, writing the same bytes. It is therefore NOT a blast-radius surface and
    /// no isolation test should read it as one (isolation lives in the handle namespace
    /// and the per-`Vas` host VAS, which are unchanged).
    ce_mem: BTreeMap<u64, u8>,
    /// ★★★ #102 stage C3 — **the extents of the FABRICATED APERTURE** this double models
    /// an isolate mapping of, as `(base, len)` ranges.
    ///
    /// [`RmBackend::fb_read`] serves a range only if one of these covers it whole. That
    /// is the entire reason the field exists: [`RmRecorder::ce_mem`] is sparse-zero, so
    /// without an explicit extent every address in the 64-bit space would read as a page
    /// of zeros and the walker's `MISS = FAULT` arm could **never fire**. A guard that
    /// cannot fire is not a guard, and this one guards the rule the whole stage rests on.
    ///
    /// ★ It is a *fixture knob*, not a design fact. Where the real aperture begins and
    /// how big it is, on a real host, is not written down anywhere in this tree and is
    /// deliberately not invented here — see `HostRmBackend::fb_read`, which refuses for
    /// exactly that reason. Tests declare what they mean to model.
    ///
    /// Empty by default: a double that had not been told it maps anything must not
    /// pretend it does.
    pub fb_aperture: Vec<(u64, u64)>,
}

/// ★ **The acquire/release ledger** — every host resource the mock handed out, and
/// whether it came back (`l1_concurrency.md` §12.16).
///
/// A pure query over [`RmRecorder::log`], so it costs nothing to keep and cannot drift
/// from what actually happened. It is the invariant that generalises G1–G4: *every host
/// object acquired is released exactly once; every GPU mapping made is unmapped exactly
/// once; nothing is released that was never acquired.* A teardown test asserts this
/// balances rather than asserting a hand-picked list of verbs, which is what makes it
/// catch the leak nobody thought to name.
///
/// **What "leaked" means here, precisely.** [`HostLedger::leaked`] is per-**object**
/// leakage: an object whose handle was minted and never individually freed. It is not a
/// claim that host memory is lost forever — an isolate's whole handle namespace dies
/// with its session at the reap, which is the bulk backstop the C also relied on
/// (`C: src/qemu/virtio_nvgpu.c:100-118`, #80). The distinction is the point: bulk
/// disposal at session death is a real disposition, and it is a *different* one from
/// per-object reclaim, so a test that wants the latter must say so.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HostLedger {
    /// Objects minted and never freed, per isolate — the per-object leak set.
    pub leaked: BTreeMap<IsolateId, BTreeSet<HostHandle>>,
    /// `(isolate, handle)` freed more than once.
    pub double_free: Vec<(IsolateId, HostHandle)>,
    /// `(isolate, handle)` freed without ever having been minted in that isolate — a
    /// cross-namespace reach if it ever appears (boundary 2).
    pub free_of_unknown: Vec<(IsolateId, HostHandle)>,
    /// `(isolate, host VAS, host GPU VA)` mapped and never unmapped.
    pub leaked_maps: BTreeMap<IsolateId, BTreeSet<(HostHandle, u64)>>,
    /// `(isolate, host VAS, host GPU VA)` unmapped without a matching map.
    pub unmap_of_unknown: Vec<(IsolateId, HostHandle, u64)>,
}

impl HostLedger {
    /// True if every acquisition was matched by exactly one release, and nothing was
    /// released that was never acquired — objects **and** mappings.
    #[must_use]
    pub fn is_balanced(&self) -> bool {
        self.leaked.values().all(BTreeSet::is_empty)
            && self.leaked_maps.values().all(BTreeSet::is_empty)
            && self.double_free.is_empty()
            && self.free_of_unknown.is_empty()
            && self.unmap_of_unknown.is_empty()
    }

    /// Objects still outstanding on `isolate` (empty set if it never leaked).
    #[must_use]
    pub fn leaked_on(&self, isolate: IsolateId) -> BTreeSet<HostHandle> {
        self.leaked.get(&isolate).cloned().unwrap_or_default()
    }

    /// Total outstanding objects across every isolate.
    #[must_use]
    pub fn leaked_count(&self) -> usize {
        self.leaked.values().map(BTreeSet::len).sum()
    }
}

impl RmRecorder {
    /// ★ Seed the model GPU memory (see [`RmRecorder::ce_mem`]) — a test's way of
    /// putting source bytes somewhere a copy can read them.
    pub fn ce_seed(&mut self, addr: u64, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.ce_mem.insert(addr.wrapping_add(i as u64), *b);
        }
    }

    /// Read `len` bytes of the model GPU memory back. Unwritten bytes read `0`
    /// (sparse-zero, `C: nvkvm_gpu_emul.c:6316`).
    #[must_use]
    pub fn ce_image(&self, addr: u64, len: u64) -> Vec<u8> {
        (0..len)
            .map(|i| self.ce_mem.get(&addr.wrapping_add(i)).copied().unwrap_or(0))
            .collect()
    }

    /// Forget every byte — so two runs of the same copy start from the same state.
    pub fn ce_clear(&mut self) {
        self.ce_mem.clear();
    }

    /// ★ Declare that the modelled isolate maps `[base, base+len)` of the fabricated
    /// aperture (see [`RmRecorder::fb_aperture`]).
    pub fn fb_declare(&mut self, base: u64, len: u64) {
        self.fb_aperture.push((base, len));
    }

    /// Does a declared aperture extent cover `[phys, phys+len)` **whole**?
    ///
    /// Whole, not partially: a half-served page-table page is a page whose tail decodes
    /// as zeros, which is the one answer this seam is forbidden to give.
    #[must_use]
    pub fn fb_covers(&self, phys: u64, len: u64) -> bool {
        let end = u128::from(phys) + u128::from(len);
        self.fb_aperture.iter().any(|&(b, l)| {
            u128::from(b) <= u128::from(phys) && end <= u128::from(b) + u128::from(l)
        })
    }

    /// Forget one range. The narrow form matters for a sweep: re-seeding a whole source
    /// region per execution costs more than the copies under test, and a comparison
    /// harness that dominates its own subject stops being run.
    pub fn ce_clear_range(&mut self, addr: u64, len: u64) {
        for i in 0..len {
            self.ce_mem.remove(&addr.wrapping_add(i));
        }
    }

    /// ★★★ Perform one sub-copy against the model memory.
    ///
    /// **Both engines move the same bytes**, and that is the whole point: §12's split is
    /// about *who is allowed to be pointed at an address*, never about what the copy
    /// means. A model in which the two arms wrote different bytes could not detect a
    /// partition that sent a range to the wrong engine — it would detect only that it
    /// sent it somewhere. The engine is recorded in the verb log instead, so "which
    /// engine" and "which bytes" are two independent assertions.
    ///
    /// ★ **The fill's pattern phase is taken from the ADDRESS, not from the start of the
    /// sub-copy.** That is what makes splitting a fill at an unaligned offset produce
    /// byte-identical results to filling the whole range at once — the compliant path is
    /// the natural one, rather than a rule about where a fill may be cut. Real remap
    /// components are address-indexed for the same reason.
    ///
    /// Read-then-write, so a sub-copy whose operands overlap is at least
    /// self-consistent. Overlap ACROSS sub-copies is genuinely undefined — a real copy
    /// engine gives no such guarantee either — so callers keep source and destination
    /// disjoint.
    fn ce_apply(&mut self, sub: CeSubCopy) {
        match sub.src {
            CeSource::Address(src) => {
                let bytes: Vec<u8> = (0..sub.len)
                    .map(|i| self.ce_mem.get(&src.wrapping_add(i)).copied().unwrap_or(0))
                    .collect();
                for (i, b) in bytes.into_iter().enumerate() {
                    self.ce_mem.insert(sub.dst.wrapping_add(i as u64), b);
                }
            }
            CeSource::Constant(pattern) => {
                let p = pattern.to_le_bytes();
                for i in 0..sub.len {
                    let a = sub.dst.wrapping_add(i);
                    self.ce_mem.insert(a, p[(a % 4) as usize]);
                }
            }
        }
    }

    /// ★ Replay the verb log into an acquire/release [`HostLedger`].
    ///
    /// Handle-minting verbs are acquisitions; [`RmVerb::Free`] is a release;
    /// [`RmVerb::MapGpuVa`]/[`RmVerb::UnmapGpuVa`] are the mapping pair. Only
    /// SUCCESSFUL verbs are in the log (the mock records after the backend succeeds),
    /// which is the right semantics: a failed alloc acquired nothing, and a failed free
    /// released nothing — the latter being exactly why `VerbFailure::orphans` exists.
    #[must_use]
    pub fn ledger(&self) -> HostLedger {
        let mut l = self.carried.clone();
        Self::fold(&mut l, &self.log);
        l
    }

    /// ★ **Take the verb log without losing the ledger** (`l1_concurrency.md` §12.35).
    ///
    /// A long soak drains the global log periodically to bound memory — 16 threads ×
    /// 100 000 ops is a lot of `RmVerb`s — and the obvious way to write that,
    /// `std::mem::take(&mut rec.log)`, **destroys the accounting**: every acquisition in
    /// the drained prefix disappears, so `ledger()` afterwards reports the whole run's
    /// live core state as dangling. The teardown post-condition (§12.35) found exactly
    /// that in `concurrency_stress`, where the drain had been correct-looking and
    /// unexamined since it was written.
    ///
    /// So the drain is a *supported* operation instead of a footgun: fold the prefix into
    /// [`RmRecorder::carried`] first, then hand it out. The invariant is that
    /// `ledger()` is the same value whether or not this was ever called.
    pub fn compact(&mut self) -> Vec<(IsolateId, RmVerb)> {
        let drained = core::mem::take(&mut self.log);
        let mut carried = core::mem::take(&mut self.carried);
        Self::fold(&mut carried, &drained);
        self.carried = carried;
        drained
    }

    /// Fold `entries` into `l` — the shared body of [`RmRecorder::ledger`] and
    /// [`RmRecorder::compact`], so the two can never disagree.
    fn fold(l: &mut HostLedger, entries: &[(IsolateId, RmVerb)]) {
        for (iso, verb) in entries {
            let live = l.leaked.entry(*iso).or_default();
            let minted = match verb {
                RmVerb::Alloc { handle, .. }
                | RmVerb::AllocEngineObject { handle, .. }
                | RmVerb::AllocVaSpace { handle }
                | RmVerb::AllocSysmem { handle, .. }
                | RmVerb::AllocChannel { handle, .. } => Some(*handle),
                _ => None,
            };
            if let Some(h) = minted {
                if !live.insert(h) {
                    l.double_free.push((*iso, h)); // a re-mint of a live handle
                }
                continue;
            }
            match verb {
                RmVerb::Free { obj } => {
                    if !live.remove(obj) {
                        l.free_of_unknown.push((*iso, *obj));
                    }
                }
                RmVerb::MapGpuVa { vas, va, .. } => {
                    l.leaked_maps.entry(*iso).or_default().insert((*vas, *va));
                }
                RmVerb::UnmapGpuVa { vas, va }
                    if !l.leaked_maps.entry(*iso).or_default().remove(&(*vas, *va)) =>
                {
                    l.unmap_of_unknown.push((*iso, *vas, *va));
                }
                _ => {}
            }
        }
    }

    /// Arm a one-shot hold: the first verb matching `spec` parks inside the backend
    /// until the returned [`VerbHold`] is released.
    pub fn hold(&mut self, spec: HoldSpec) -> Arc<VerbHold> {
        let h = VerbHold::new();
        self.holds.push((spec, Arc::clone(&h)));
        h
    }

    /// `id`'s [`ClientLock`] — `None` until its isolate has been spawned.
    #[must_use]
    pub fn client_lock(&self, id: IsolateId) -> Option<Arc<ClientLock>> {
        self.client_locks.get(&id).map(Arc::clone)
    }

    /// Every [`ParkedVerb`] of `id`, in order (assertion helper).
    #[must_use]
    pub fn parked_of(&self, id: IsolateId) -> Vec<ParkedVerb> {
        self.parked_verbs
            .iter()
            .filter(|p| p.isolate == id)
            .copied()
            .collect()
    }

    /// Every verb this isolate issued, in order (assertion helper).
    #[must_use]
    pub fn verbs_of(&self, id: IsolateId) -> Vec<RmVerb> {
        self.log
            .iter()
            .filter(|(i, _)| *i == id)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// Handle to the shared recorder, held by the test.
pub type SharedRecorder = Arc<Mutex<RmRecorder>>;

/// The handle namespace ONE isolate's whole worker pool shares.
///
/// §7.2: the isolate is one sandboxed process per `(Proc, GpuId)` — the RM client and
/// its handle namespace are per-process identities and stay singular. Workers are
/// slots inside it sharing exactly one thing, the RM connection, which is
/// kernel-mediated. So a handle minted on worker 0 must be valid on worker 1, and
/// this `Mutex` is the mock's stand-in for that kernel mediation. It is a plain
/// `std::sync::Mutex`, deliberately outside the rank system: it is not an L1 lock.
#[derive(Debug)]
struct RmNamespace {
    handles: BTreeSet<HostHandle>,
    next: u64,
    next_token: u64,
    /// ★ #102 — GPU VA occupancy **per host VAS**: `vas -> (va -> span)`.
    ///
    /// Replaces the `next_map_page` bump counter. Under address identity the mock no
    /// longer *chooses* addresses, so the only thing left to track is whether the
    /// caller's chosen one is free — and it is free or not *within one VAS*, which is
    /// the separation #14 actually rests on.
    maps: BTreeMap<HostHandle, BTreeMap<u64, u64>>,
    retired: bool,
}

/// A fake host RM connection into one isolate's shared namespace, from ONE pool
/// worker.
///
/// Handle values are namespaced by isolate id (`(id+1) << 32 | n`) so any
/// cross-isolate handle use is *visible* in assertions; validity is still
/// enforced per-isolate ([`RmError::BadHandle`]) — the blast-radius property.
#[derive(Debug)]
pub struct MockRmBackend {
    id: IsolateId,
    /// `id.gpu()`, cached for readability — never an independent fact (N3).
    gpu: GpuId,
    worker: WorkerId,
    /// `id + 1` — the isolate's primary namespace lane (unchanged, so the high bits of
    /// every minted handle/token/VA still read back as the session's ProcId+1).
    idlane: u64,
    recorder: SharedRecorder,
    ns: Arc<Mutex<RmNamespace>>,
    /// This slot's out-of-band cancel seam — the same `Arc` its pool slot's
    /// [`kayfabe_isolate::CancelHandle`] holds (§7.1).
    cancel: Arc<MockCancelSink>,
    /// The RM client lock every worker of this isolate shares ([`ClientLock`]).
    client: Arc<ClientLock>,
}

/// ★★ The part of a mock handle a **real host** would see.
///
/// [`MockRmBackend`] mints `handle_hi() | n`, where the high lanes are pure
/// instrumentation (they make a cross-namespace value *readable* in an assertion) and
/// `n` is the per-client sequence — which every client mints from the same base, exactly
/// as RM does from `RS_CLIENT_HANDLE_BASE` (`ogkm-610:
/// src/nvidia/generated/g_resserv_nvoc.h:173`, `ogkm-580: :188`, same `0xC1D00000`). So
/// masking off the lanes leaves precisely the value a real client would have been handed,
/// and two isolates' `n`-th objects collide — which is the fact
/// [`MockRmBackend::check`] must honour.
pub const HOST_RAW_MASK: u64 = 0x00ff_ffff;

impl MockRmBackend {
    fn namespace(idlane: u64, gpu: GpuId) -> Arc<Mutex<RmNamespace>> {
        Arc::new(Mutex::new(RmNamespace {
            handles: BTreeSet::new(),
            next: 1,
            // Namespaced fake host tokens: primary lane = idlane (so `>>20` still reads
            // ProcId+1); the target GPU is folded into a LOWER field, so two isolates of
            // ONE proc on DIFFERENT GPUs still mint provably-disjoint tokens (MG-5).
            next_token: (idlane << 20) | (u64::from(gpu.0) << 8),
            maps: BTreeMap::new(),
            retired: false,
        }))
    }

    fn new(
        id: IsolateId,
        worker: WorkerId,
        recorder: SharedRecorder,
        ns: Arc<Mutex<RmNamespace>>,
        cancel: Arc<MockCancelSink>,
        client: Arc<ClientLock>,
    ) -> Self {
        MockRmBackend {
            id,
            gpu: id.gpu(),
            worker,
            idlane: u64::from(id.proc()) + 1,
            recorder,
            ns,
            cancel,
            client,
        }
    }

    /// High 40 bits shared by every handle/surface: `idlane` (bits ≥32, so `>>32`
    /// reads ProcId+1) with the target GPU folded into bits [24..32) — distinct per
    /// `(isolate, GPU)`, the MG-5 cross-GPU blast-radius property.
    fn handle_hi(&self) -> u64 {
        (self.idlane << 32) | (u64::from(self.gpu.0) << 24)
    }

    /// Pre-verb gate: take this client's RM lock, park in any armed [`VerbHold`]
    /// matching this `(isolate, gpu, worker, verb)`, then apply `fail_next` / the retired
    /// refusal.
    ///
    /// The hold is looked up and REMOVED under the recorder lock, then parked in with
    /// the recorder lock released — otherwise a held verb would freeze every other
    /// isolate's recording and the "everything else keeps running" property the mean
    /// test asserts would be a lie about the mock, not about the design.
    ///
    /// ## ★★ Order: the CLIENT lock first, then everything else
    ///
    /// RM takes the client write lock at the ioctl's door and only then does the work
    /// that may block, so the wedged verb holds it for the whole wait. Modelling it in
    /// the other order would let a sibling slip past the wedge and would reproduce
    /// exactly the politeness this exists to remove. The returned [`ClientGuard`] is what
    /// releases it — including on every `return Err` below.
    ///
    /// **Lock order is `ClientLock` → `RmRecorder` → `RmNamespace`, never reversed**, and
    /// no two of them are ever held at once except that outer-to-inner nesting.
    fn gate(&mut self, verb: VerbKind) -> Result<ClientGuard, RmError> {
        let serialize = self.recorder.lock().expect("recorder").serialize_per_client;
        let guard = if serialize {
            self.client.acquire(self.worker, verb);
            ClientGuard(Some(Arc::clone(&self.client)))
        } else {
            ClientGuard(None)
        };
        slow_fuzz();
        let (hold, interruptible) = {
            let mut r = self.recorder.lock().expect("recorder");
            let interruptible = !r.never_cancels.contains(&verb);
            let hold = r
                .holds
                .iter()
                .position(|(s, _)| s.matches(self.id, self.worker, verb))
                .map(|i| r.holds.remove(i).1);
            (hold, interruptible)
        };
        // ★ §7.2 — the break signal is checked BEFORE the verb runs and again while it is
        // parked. A cancel that arrived while this slot was between verbs must still land
        // on the next one of the same txn, or "cancellation appears to work and does
        // nothing" for every chain step that is not the one being held.
        match self.cancel.park_in_and_peek(hold.as_ref()) {
            (_, true) => {
                self.recorder
                    .lock()
                    .expect("recorder")
                    .verbs_that_hit_an_abandoned_worker += 1;
                return Err(RmError::Wedged);
            }
            (Some(r), false) if interruptible => {
                self.cancel.observe(r);
                return Err(RmError::Interrupted);
            }
            _ => {}
        }
        if let Some(h) = hold {
            // ★ R1's *other* half, witnessed rather than inspected: the mask this thread
            // is about to spend an unbounded wait holding. `Worker::execute` asserts
            // lock-freedom at its entry; this records it at the WAIT, which is the fact a
            // wedge test needs and the one an entry assert cannot give.
            self.recorder
                .lock()
                .expect("recorder")
                .parked_verbs
                .push(ParkedVerb {
                    isolate: self.id,
                    worker: self.worker,
                    verb,
                    held_lock_mask: kayfabe_util::lockwitness::held_mask(),
                });
            match h.enter_and_park(interruptible) {
                Ok(()) => {}
                Err(RmError::Interrupted) => {
                    let r = self
                        .cancel
                        .armed_reason()
                        .expect("the park woke on a break signal, so one is armed");
                    self.cancel.observe(r);
                    return Err(RmError::Interrupted);
                }
                Err(e) => {
                    if e == RmError::Wedged {
                        // The verb that DISCOVERED the abandon counts too — see the
                        // counter's docs for why an `== 0` invariant would be vacuous.
                        self.recorder
                            .lock()
                            .expect("recorder")
                            .verbs_that_hit_an_abandoned_worker += 1;
                    }
                    return Err(e);
                }
            }
        }
        // ★ §7.2 refinement 4 — this verb is the ONE ioctl the signal was armed for, and
        // it did not break on it. The signal is spent here rather than left to be
        // observed by whatever runs next (which, on a failing op, is the disposal).
        self.cancel.spend();
        if self.ns.lock().expect("ns").retired {
            return Err(RmError::Other(0xdead));
        }
        let mut r = self.recorder.lock().expect("recorder");
        if let Some(&e) = r.fail_kinds.get(&verb) {
            return Err(e);
        }
        if let Some(e) = r.fail_next.take() {
            return Err(e);
        }
        drop(r);
        Ok(guard)
    }

    fn mint(&mut self) -> HostHandle {
        let mut ns = self.ns.lock().expect("ns");
        let h = HostHandle::new(self.id, self.handle_hi() | ns.next);
        ns.next += 1;
        ns.handles.insert(h);
        h
    }

    /// ★★★ Resolve `h` **the way a real host would**, returning the object the verb will
    /// actually operate on.
    ///
    /// ## The lie this used to tell
    ///
    /// It used to answer [`RmError::BadHandle`] for any handle not minted in *this*
    /// isolate — i.e. it validated against the mock's own per-isolate namespace.
    /// [`kayfabe_isolate::HostHandle`]'s own docs say a real host does **not**: RM mints
    /// every client's handles from one `RS_CLIENT_HANDLE_BASE`, so isolate A's `0x…07`
    /// and isolate B's `0x…07` are *both live and unrelated*, and presenting one on the
    /// other's connection does not fault — **it names a different, live object**. A free
    /// would destroy a bystander; an unmap would tear down a bystander's mapping.
    ///
    /// That divergence is not hypothetical: `07da582`'s cross-GPU handle was caught
    /// **only** by this refusal, which the real host does not provide, so every
    /// handle-boundary test that leaned on it was optimistic.
    ///
    /// ## What it does now
    ///
    /// A foreign handle whose **host-visible** value ([`HOST_RAW_MASK`]) is live in this
    /// client is *served*, against the local twin, and recorded as a
    /// [`BystanderHit`]. A value that is live nowhere here is still `BadHandle` — that
    /// arm is real too (RM's `clientGetResource` fails on an unknown handle).
    ///
    /// ## What still catches it, and why that is the right place
    ///
    /// Two instruments, neither of which is the backend's luck:
    ///
    /// - [`kayfabe_isolate::Worker::execute`]'s **foreign-handle gate**, which refuses on
    ///   the recorded provenance carried in the type and runs before any verb;
    /// - [`HostLedger::free_of_unknown`], because a `Free` is logged with the handle as
    ///   *presented* — so an isolate that freed something it never minted still shows up,
    ///   and the local twin it destroyed stays outstanding forever, which is the actual
    ///   damage rather than a proxy for it.
    fn check(&self, h: HostHandle) -> Result<HostHandle, RmError> {
        let twin = {
            let ns = self.ns.lock().expect("ns");
            if ns.handles.contains(&h) {
                return Ok(h);
            }
            ns.handles
                .iter()
                .find(|t| t.raw() & HOST_RAW_MASK == h.raw() & HOST_RAW_MASK)
                .copied()
        };
        match twin {
            Some(hit) => {
                self.recorder
                    .lock()
                    .expect("recorder")
                    .bystander_hits
                    .push(BystanderHit {
                        isolate: self.id,
                        presented: h,
                        hit,
                    });
                Ok(hit)
            }
            None => Err(RmError::BadHandle(h)),
        }
    }

    fn record(&self, verb: RmVerb) {
        self.recorder
            .lock()
            .expect("recorder")
            .log
            .push((self.id, verb));
    }
}

impl RmBackend for MockRmBackend {
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        let _client = self.gate(VerbKind::Alloc)?;
        if parent != HostHandle::NULL {
            self.check(parent)?;
        }
        let handle = self.mint();
        self.record(RmVerb::Alloc {
            parent,
            class,
            handle,
        });
        Ok(handle)
    }

    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError> {
        let _client = self.gate(VerbKind::AllocVaSpace)?;
        let handle = self.mint();
        self.record(RmVerb::AllocVaSpace { handle });
        Ok(handle)
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        let _client = self.gate(VerbKind::AllocSysmem)?;
        let handle = self.mint();
        self.record(RmVerb::AllocSysmem { handle, len });
        Ok(handle)
    }

    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        let _client = self.gate(VerbKind::AllocChannel)?;
        self.check(vas)?;
        let handle = self.mint();
        let token = {
            let mut ns = self.ns.lock().expect("ns");
            let t = ns.next_token;
            ns.next_token += 1;
            t
        };
        self.record(RmVerb::AllocChannel {
            vas,
            engine,
            handle,
            token,
        });
        Ok((handle, token))
    }

    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: ClassId,
        _params: &[u8],
    ) -> Result<HostHandle, RmError> {
        let _client = self.gate(VerbKind::AllocEngineObject)?;
        self.check(chan)?;
        let handle = self.mint();
        self.record(RmVerb::AllocEngineObject {
            chan,
            class,
            handle,
        });
        Ok(handle)
    }

    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::Schedule)?;
        self.check(chan)?;
        self.record(RmVerb::Schedule { chan });
        Ok(())
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::Free)?;
        // ★ The VICTIM, which is `obj` itself in the ordinary case and the local twin on
        // a cross-namespace reach — the bystander a real host would destroy. The log
        // still names the handle as PRESENTED, so `HostLedger::free_of_unknown` sees the
        // reach and the twin stays outstanding: both halves of the damage, separately.
        let victim = self.check(obj)?;
        self.ns.lock().expect("ns").handles.remove(&victim);
        self.record(RmVerb::Free { obj });
        Ok(())
    }

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        _payload: &mut [u8],
    ) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::Control)?;
        self.check(obj)?;
        self.record(RmVerb::Control { obj, cmd });
        Ok(())
    }

    /// ★★★ #102 — the mock honours the requested placement, because the real backend
    /// does (`DMA_OFFSET_FIXED_TRUE`) and a double that chose its own address would be
    /// modelling the bug rather than the driver.
    ///
    /// This used to mint a synthetic VA out of disjoint bit lanes, so every mapping in
    /// the process was distinct by construction. That made the suite's #14 assertions
    /// pass for the wrong reason and hid the whole of `#102`: no test could observe that
    /// a forwarded guest VA had no host mapping at that VA, because *no* host mapping was
    /// ever at a guest VA.
    ///
    /// What the mock still models faithfully is the property the lanes were really for:
    /// **collisions are per host VAS**. Occupancy is tracked per `vas`, so two mappings
    /// of the same range in ONE VAS is a loud [`RmError::NoMemory`] (RM refuses a fixed
    /// map over a live range), while two *different* VASes mapping the *same* guest VA
    /// is entirely fine — which is #14's actual fix and now the arrangement the tests
    /// assert.
    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError> {
        let _client = self.gate(VerbKind::MapGpuVa)?;
        self.check(vas)?;
        self.check(memory)?;
        let span = len.next_multiple_of(0x1000).max(0x1000);
        {
            let mut ns = self.ns.lock().expect("ns");
            let occupied = ns.maps.entry(vas).or_default();
            // A fixed-offset map onto a live range is refused, not silently relocated —
            // relocating is exactly the behaviour `RmError::PlacementRefused` exists to
            // catch one layer up, and RM does not do it.
            let clash = occupied
                .range(..at.0.saturating_add(span))
                .next_back()
                .is_some_and(|(&start, &l)| start.saturating_add(l) > at.0);
            if clash {
                return Err(RmError::NoMemory);
            }
            occupied.insert(at.0, span);
        }
        // The one scripted deviation: a backend that ignored the fixed-offset request
        // and placed the mapping elsewhere. Nothing else in the mock can produce it, and
        // it is the only way to make `Worker::execute`'s identity check fire.
        let drift = self.recorder.lock().expect("recorder").placement_drift;
        let va = at.0 + drift.unwrap_or(0);
        self.record(RmVerb::MapGpuVa {
            vas,
            memory,
            len,
            va,
        });
        Ok(va)
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::UnmapGpuVa)?;
        self.check(vas)?;
        // #102: release the occupancy so a later fixed map at the same VA succeeds —
        // "unmap eager, then rebind" is the address plane's own discipline, and a mock
        // that never freed the range would make every legitimate rebind look like the
        // `ALREADY-MAPPED` collision class.
        self.ns
            .lock()
            .expect("ns")
            .maps
            .entry(vas)
            .or_default()
            .remove(&gpu_va);
        self.record(RmVerb::UnmapGpuVa { vas, va: gpu_va });
        Ok(())
    }

    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::RingDoorbell)?;
        self.record(RmVerb::RingDoorbell { token: host_token });
        Ok(())
    }

    fn ce_copy(&mut self, vas: HostHandle, sub: CeSubCopy) -> Result<(), RmError> {
        let _client = self.gate(VerbKind::CeCopy)?;
        // A copy into a host VAS this isolate does not own is a LOUD BadHandle. Same
        // check as every other verb that names a VAS: the addresses are only meaningful
        // inside it, and per-`Vas` host separation is #14's proven fix.
        self.check(vas)?;
        // ★ A zero-length sub-copy is a PARTITION BUG, not a degenerate case to absorb.
        // `kayfabe-fwd` never emits one; a backend that quietly accepted one would make
        // an off-by-one in the range algebra invisible here and visible only as missing
        // bytes much later.
        if sub.len == 0 {
            return Err(RmError::BadHandle(vas));
        }
        {
            let mut rec = self.recorder.lock().expect("recorder");
            rec.ce_apply(sub);
        }
        self.record(RmVerb::CeCopy { vas, sub });
        Ok(())
    }

    /// ★★★ #102 stage C3 — the fabricated aperture, read back.
    ///
    /// The bytes come from the **same** [`RmRecorder::ce_mem`] that
    /// [`RmBackend::ce_copy`]'s `Ours` arm wrote them into, and that identity is the
    /// whole model: §12.2's ruling is that the content lives in the isolate's mapping of
    /// the fabricated aperture *because we are the ones who wrote it there*. A separate
    /// store here would be modelling a different design.
    ///
    /// Coverage is a *declared* extent ([`RmRecorder::fb_aperture`]), not "anything the
    /// sparse map has a byte for": an aperture covers addresses nobody has written yet,
    /// and a page of genuine zeros inside it is a real answer.
    fn fb_read(&mut self, phys: u64, buf: &mut [u8]) -> Result<bool, RmError> {
        let _client = self.gate(VerbKind::FbRead)?;
        let len = buf.len() as u64;
        let covered = {
            let rec = self.recorder.lock().expect("recorder");
            let covered = rec.fb_covers(phys, len);
            if covered {
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = rec
                        .ce_mem
                        .get(&phys.wrapping_add(i as u64))
                        .copied()
                        .unwrap_or(0);
                }
            }
            covered
        };
        self.record(RmVerb::FbRead { phys, len, covered });
        Ok(covered)
    }

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        let _client = self.gate(VerbKind::ExportSurface)?;
        // An unknown memory object is a LOUD BadHandle — never a silently minted
        // surface (and cross-isolate render targets are refused by the same check).
        self.check(memory)?;
        // Namespaced like host handles (handle_hi | n) so cross-(isolate, GPU)
        // surface use is visible in assertions.
        let n = {
            let mut ns = self.ns.lock().expect("ns");
            let n = ns.next;
            ns.next += 1;
            n
        };
        let surface = SurfaceHandle(self.handle_hi() | n);
        self.record(RmVerb::ExportSurface { memory, surface });
        Ok(surface)
    }

    /// ★★★ Decision (b) in the double — **and the double refuses the device class too.**
    ///
    /// The mock is what the whole core suite runs against, so this is where the boundary
    /// becomes visible to every test that never touches an OS. It refuses
    /// `ExportSource::HostDeviceMemory` for the same reason the real backend does, and it
    /// is deliberately NOT a knob: a mock that could be scripted to succeed there would
    /// let a core-side consumer be written against an outcome no host can produce — the
    /// mock-wall shape recorded in `CLAUDE.md` (749 tests green and 99.2 % mutation, and
    /// making the double honest about ONE property killed 12 of them).
    ///
    /// The token is namespaced like a handle (`handle_hi() | n`), so a backing minted by
    /// proc A's isolate and presented on proc B's is visible in an assertion rather than
    /// plausible.
    fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError> {
        let _client = self.gate(VerbKind::ExportBacking)?;
        if let ExportSource::HostDeviceMemory { memory } = want.source {
            // An unknown object is still a LOUD BadHandle: *"I have never heard of this
            // object"* and *"its bytes are on the card"* are different facts, and a
            // refusal that merged them would answer a typo with an architectural verdict.
            self.check(memory)?;
            self.record(RmVerb::ExportBacking {
                source: want.source,
                len: want.len,
                token: None,
            });
            return Err(RmError::NotExportableAsMemory { memory });
        }
        if want.len == 0 {
            return Err(RmError::NoMemory);
        }
        let n = {
            let mut ns = self.ns.lock().expect("ns");
            let n = ns.next;
            ns.next += 1;
            n
        };
        let token = self.handle_hi() | n;
        self.record(RmVerb::ExportBacking {
            source: want.source,
            len: want.len,
            token: Some(token),
        });
        Ok(ExportedBacking {
            token,
            offset: 0,
            len: want.len,
            prot: want.prot,
        })
    }
}

/// One pool slot's state (`l1_concurrency.md` §7.2/§7.3).
#[derive(Debug)]
enum Slot {
    /// Checked in and available.
    Idle(Worker),
    /// Checked out — its handle is `&mut`-owned by some thread right now.
    Busy,
    /// Died out of band. **Never resurrected** (§7.3): its component is condemned by
    /// the same event, because the guest's published data lived in host memory owned
    /// by this isolate's RM client and died with it.
    Dead,
}

/// A fake per-process isolate holding a **bounded pool** of [`Worker`]s over one
/// shared handle namespace (§7.2).
#[derive(Debug)]
pub struct MockIsolate {
    id: IsolateId,
    slots: Vec<Slot>,
    /// One cancel seam per slot, parallel to `slots` — held here so
    /// [`Isolate::cancel_handle`] can reach it **without** the [`Worker`], which is the
    /// whole of §7.1.
    cancels: Vec<Arc<MockCancelSink>>,
    /// Monotonic per-isolate txn mint. Never reused, so a stale request can never alias
    /// a fresh checkout (§7.7(i)).
    next_txn: u64,
    ns: Arc<Mutex<RmNamespace>>,
    retired: bool,
    /// ★ Held for exactly one reason: to write this isolate's **death** into
    /// [`RmRecorder::isolates_dropped`]. See that field.
    recorder: SharedRecorder,
}

/// ★★★ **The death witness** (`#130`). Without it the harness records births and nothing
/// else, so "the isolate dies with the device" is unfalsifiable in every test that uses
/// these mocks — and unfalsifiable reads as true.
///
/// ★ It models a real event rather than decorating one. The production `HostIsolate` has
/// no `kill()` a caller must remember: its `Drop` closes the worker sockets, sends
/// `SIGKILL` and `waitpid`s, so **being dropped IS the kill**. Recording the drop is
/// therefore recording the kill, and a `Proc` that is leaked instead of reaped leaves a
/// live sandboxed process holding host RM objects — exactly the residue a reloaded device
/// must not reattach to.
impl Drop for MockIsolate {
    fn drop(&mut self) {
        let mut r = self.recorder.lock().unwrap_or_else(|e| e.into_inner());
        r.isolates_dropped.push(self.id);
    }
}

impl Isolate for MockIsolate {
    fn id(&self) -> IsolateId {
        self.id
    }
    fn pool_size(&self) -> usize {
        self.slots.len()
    }
    fn idle_workers(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s, Slot::Idle(_)))
            .count()
    }
    fn checkout(&mut self) -> Option<Worker> {
        if self.retired {
            return None; // a retiring isolate refuses NEW checkouts (§5.4)
        }
        let i = self.slots.iter().position(|s| matches!(s, Slot::Idle(_)))?;
        match std::mem::replace(&mut self.slots[i], Slot::Busy) {
            Slot::Idle(mut w) => {
                // ★ §7.1 — one txn per checkout, minted here and stamped on both halves
                // (the worker that goes out, the cell the CancelHandle reads). A request
                // naming an earlier one is dropped by the sink.
                self.next_txn += 1;
                let txn = self.next_txn;
                self.cancels[i].begin(txn);
                w.begin_txn(Txn(txn));
                Some(w)
            }
            _ => unreachable!("position() selected an Idle slot"),
        }
    }
    fn checkin(&mut self, worker: Worker) {
        let i = worker.id().0 as usize;
        if let Some(c) = self.cancels.get(i) {
            // The txn is over: from here a request naming it is STALE and lands nowhere.
            c.end();
        }
        match self.slots.get(i) {
            // The slot died while its worker was out: the worker handle dies with it.
            Some(Slot::Dead) | None => drop(worker),
            _ => self.slots[i] = Slot::Idle(worker),
        }
    }
    fn checked_out(&self) -> Vec<WorkerId> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Slot::Busy))
            .map(|(i, _)| WorkerId(i as u32))
            .collect()
    }
    fn cancel_handle(&self, worker: WorkerId) -> Option<CancelHandle> {
        // ★ Keyed on "is a txn outstanding", NOT on `Slot::Busy` — and the difference is
        // load-bearing. `worker_died` turns a Busy slot into `Dead` while its requester
        // is still parked inside the verb, and that requester is exactly the one §7.5
        // must release. Keying on the slot enum would make the abandon a silent no-op on
        // the one path that needs it most.
        let i = worker.0 as usize;
        let cell = self.cancels.get(i)?;
        let txn = cell.current_txn()?;
        Some(CancelHandle::new(
            self.id,
            worker,
            txn,
            Arc::clone(cell) as Arc<dyn CancelSink>,
        ))
    }
    fn worker_died(&mut self, worker: WorkerId) -> bool {
        match self.slots.get_mut(worker.0 as usize) {
            Some(slot) => {
                *slot = Slot::Dead;
                true
            }
            None => false,
        }
    }
    fn in_flight(&self) -> usize {
        // ★ G3 (§12.16): counted from `Busy` DIRECTLY, never as
        // `pool_size() - idle_workers()`. A `Dead` slot is neither idle nor in
        // flight and can never become either (§7.3, "no resurrect"), so the
        // subtraction would report a lost worker as a live round trip forever — an
        // isolate that never quiesces, a proc that never reaps, an arena that never
        // recycles. Exactly the trap the quiesce predicate exists to avoid.
        self.slots
            .iter()
            .filter(|s| matches!(s, Slot::Busy))
            .count()
    }
    fn retire(&mut self) {
        self.retired = true;
        self.ns.lock().expect("ns").retired = true;
    }
    fn is_retired(&self) -> bool {
        self.retired
    }

    /// ⊘ **Always `None`, and that is a claim rather than a stub.** A mock isolate is a
    /// *working* isolate — it answers verbs — so it has no refusal to report, and one
    /// invented here would be the mock deciding an E1 assertion's answer for it. The
    /// E1 tests therefore drive the two REAL implementors
    /// (`kayfabe_isolate::StillbornIsolate` and
    /// `kayfabe_isolate_host::HostIsolate`), never this one.
    fn refusal(&self) -> Option<kayfabe_isolate::IsolateRefusal<'_>> {
        None
    }
}

/// Spawns [`MockIsolate`]s that all record into one [`SharedRecorder`].
#[derive(Debug)]
pub struct MockIsolateFactory {
    recorder: SharedRecorder,
    pool_size: usize,
    /// Every spawned session id, in order — and an [`IsolateId`] IS a `(proc, GPU)`
    /// pair, so this is still the isolate-per-`(proc, GPU)` witness it always was (N3).
    pub spawned: Vec<IsolateId>,
}

impl MockIsolateFactory {
    /// Create a factory + the recorder handle the test keeps. Pools are
    /// [`kayfabe_isolate::DEFAULT_POOL_WORKERS`] wide.
    #[must_use]
    pub fn new() -> (Self, SharedRecorder) {
        Self::with_pool_size(kayfabe_isolate::DEFAULT_POOL_WORKERS)
    }

    /// As [`MockIsolateFactory::new`], with an explicit pool width — the knob the
    /// saturation/backpressure tests turn (a pool of 1 is the degenerate
    /// single-in-flight isolate decision #34 originally shipped).
    ///
    /// # Panics
    /// If `pool_size` is zero: an isolate that can never issue a verb is a
    /// configuration error, not a runtime condition.
    #[must_use]
    pub fn with_pool_size(pool_size: usize) -> (Self, SharedRecorder) {
        assert!(pool_size > 0, "an isolate pool needs at least one worker");
        let recorder: SharedRecorder = Arc::default();
        (
            MockIsolateFactory {
                recorder: Arc::clone(&recorder),
                pool_size,
                spawned: Vec::new(),
            },
            recorder,
        )
    }
}

impl Default for MockIsolateFactory {
    fn default() -> Self {
        Self::new().0
    }
}

impl IsolateFactory for MockIsolateFactory {
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate> {
        self.spawned.push(id);
        let ns = MockRmBackend::namespace(u64::from(id.proc()) + 1, id.gpu());
        // ★ ONE RM client lock per isolate, shared by its whole pool — and published on
        // the recorder, which is the handle the test keeps (the factory is moved into
        // `Gpu::realize` and is unreachable afterwards).
        let client = ClientLock::new(id);
        self.recorder
            .lock()
            .expect("recorder")
            .client_locks
            .insert(id, Arc::clone(&client));
        let cancels: Vec<Arc<MockCancelSink>> = (0..self.pool_size)
            .map(|i| MockCancelSink::new(id, WorkerId(i as u32), Arc::clone(&self.recorder)))
            .collect();
        let slots = (0..self.pool_size)
            .map(|i| {
                let w = WorkerId(i as u32);
                Slot::Idle(Worker::with_cancel(
                    id,
                    w,
                    Box::new(MockRmBackend::new(
                        id,
                        w,
                        Arc::clone(&self.recorder),
                        Arc::clone(&ns),
                        Arc::clone(&cancels[i]),
                        Arc::clone(&client),
                    )),
                    Arc::clone(&cancels[i]) as Arc<dyn CancelSink>,
                ))
            })
            .collect();
        Box::new(MockIsolate {
            id,
            slots,
            cancels,
            next_txn: 0,
            ns,
            retired: false,
            recorder: Arc::clone(&self.recorder),
        })
    }
}

// ---------------------------------------------------------------------------------
// MockPresent — a scriptable display/scanout sink (the GR-graphics `Present` seam)
// ---------------------------------------------------------------------------------

/// A fake present sink: records every presented `(buffer, meta)` and hands back a
/// monotonic [`Vblank`], so a GR-graphics scanout route is testable with no display.
/// Can be scripted to fail (`fail_next`) for the negative path.
#[derive(Debug, Default)]
pub struct MockPresent {
    /// Every presented frame in order (assert the scanout surface reached the sink).
    pub presented: Vec<(SurfaceHandle, FbMeta)>,
    /// If set, the next present fails with this error (then clears).
    pub fail_next: Option<PresentError>,
    next_seq: u64,
}

impl MockPresent {
    /// A fresh sink with no frames presented.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Present for MockPresent {
    fn present(&mut self, buffer: SurfaceHandle, meta: FbMeta) -> Result<Vblank, PresentError> {
        if let Some(e) = self.fail_next.take() {
            return Err(e);
        }
        self.presented.push((buffer, meta));
        let seq = self.next_seq;
        self.next_seq += 1;
        Ok(Vblank { seq })
    }
}

// The concurrency contract, compile-time-asserted (decision #17): the mocks must be
// `Send + Sync` like the real adapters they stand in for — the multi-thread stress
// harness (`tests/concurrency_stress.rs`) drives a mock-realized `Gpu` from many
// simulated vCPU threads.
kayfabe_util::assert_send_sync!(
    MockArch,
    MockGmmuFmt,
    MockUserd,
    MockPushbuffer,
    MockVmm,
    MockRmBackend,
    MockIsolate,
    MockIsolateFactory,
    MockPresent,
    RmRecorder,
    SharedRecorder,
    VerbHold,
    VerbKind,
    HoldSpec,
    ClientLock,
    ClientGuard,
    ClientWait,
    ParkedVerb,
    BystanderHit,
    SlotRecord,
    RmVerb,
);

#[cfg(test)]
mod tests {
    use super::*;
    // `CoreEventKind` is no longer part of the mock's own surface (the memory-lock
    // primitive left the `Vmm` trait — `l1_os_shell.md` §6.8); it is still the payload
    // `defer` carries, so the ordering test names it directly.
    use kayfabe_vmm::CoreEventKind;

    /// ⊘⊘ **THIS TEST IS NOT EVIDENCE ABOUT ANY REAL DOORBELL ENCODING, and it never
    /// was.** [`MockArch::token_for`] is the inverse of [`MockArch::decode_doorbell`], so
    /// the expected value is produced by the function under test running backwards — the
    /// shape `never_let_a_test_use_the_thing_under_test_as_its_own_observer` catalogues,
    /// and the shape that let a planted mutation survive on 2026-08-01. It is kept
    /// because a mock's two halves *must* agree for every test that builds a token to
    /// mean anything; it is renamed and annotated so that the agreement is never again
    /// read as a fact about silicon. The real doorbell encoding is settled elsewhere:
    /// `Ga10xArch::decode_doorbell`, `tests/tests/doorbell_token.rs` (real GA106 tokens)
    /// and `tests/tests/worksubmit_token_oracle.rs` (RM's own encoder).
    #[test]
    fn mock_arch_token_roundtrip_is_self_consistent_only() {
        let a = MockArch::new();
        let v = VChid(0x2a);
        assert_eq!(
            a.decode_doorbell(MockArch::token_for(v)),
            Some(DoorbellTarget {
                vchid: v,
                runlist: kayfabe_arch::ids::RunlistId(0)
            })
        );
        assert_eq!(a.vchid_from_userd_flags(MockArch::userd_flags_for(v)), v);
        assert_eq!(
            a.decode_doorbell(0x1234),
            None,
            "hostile token must not decode"
        );
    }

    /// ★★ **Bounded termination for the two tests below that park a real thread.**
    ///
    /// The house rule (`concurrency_stress.rs`'s M4b lesson, and `l1_mean`'s own
    /// `l1_mean::watchdog`-shaped guard): a test that can wedge must
    /// **fail fast and loudly**, never eat the CI timeout. It earned its place
    /// immediately — a bite-check mutant that disabled [`ClientLock`] hung the whole
    /// `kayfabe-mocks` unittest binary forever, with no watchdog anywhere in this crate
    /// to stop it, and stalled the harness rather than reporting CAUGHT.
    ///
    /// ★ Now the crate's own [`crate::watchdog::watchdog`] — this was the tenth copy of
    /// it, and like the other nine it announced its abort with `eprintln!`, which libtest
    /// buffers and `abort()` discards. Its own doc comment above says the guard "earned
    /// its place immediately" when a `ClientLock` mutant hung this binary forever; what
    /// it could not say is that the hang was reported as a bare `SIGABRT` with no text.
    #[must_use]
    fn bounded(test: &'static str) -> crate::watchdog::WatchdogGuard {
        crate::watchdog::watchdog(test, Duration::from_secs(60))
    }

    #[test]
    fn mock_vmm_virtual_clock_orders_deferred_events() {
        let mut vmm = MockVmm::new();
        vmm.defer(
            Duration::from_millis(2),
            CoreEvent::Deferred(CoreEventKind::DeferredReap),
        );
        vmm.defer(
            Duration::from_millis(1),
            CoreEvent::Deferred(CoreEventKind::CompletionRedeliver(
                kayfabe_arch::ids::GpuId::ZERO,
            )),
        );
        assert!(
            vmm.advance(Duration::from_micros(500)).is_empty(),
            "nothing due yet"
        );
        let due = vmm.advance(Duration::from_millis(2));
        assert_eq!(
            due,
            vec![
                CoreEvent::Deferred(CoreEventKind::CompletionRedeliver(
                    kayfabe_arch::ids::GpuId::ZERO
                )),
                CoreEvent::Deferred(CoreEventKind::DeferredReap),
            ],
            "due in deadline order, deterministically"
        );
    }

    /// ★★★ #102 — the mock's placement contract, at scale and at the boundary that
    /// matters.
    ///
    /// This replaces `mock_rm_map_gpu_va_stays_distinct_past_65536_pages`, which pinned
    /// the *old* minting scheme (disjoint bit lanes, distinct-by-construction) after an
    /// M4b wrap bug. That scheme is gone: under address identity the mock does not choose
    /// addresses at all, so "the mock mints distinct VAs" is no longer a property it has
    /// or should have. What replaces it is the property the real driver has and the whole
    /// data plane depends on — **it puts the mapping where it was told** — plus the
    /// collision rule that carries #14: collisions are per host VAS, so the SAME address
    /// in two DIFFERENT VASes is legal and the same address twice in ONE VAS is refused.
    ///
    /// Kept at 70k iterations, deliberately: the replaced test's whole value was crossing
    /// 2^16, and a rewrite that quietly dropped the scale would be narrowing a test to
    /// fit a change.
    #[test]
    fn mock_rm_map_gpu_va_places_exactly_where_asked_and_collides_only_within_one_vas() {
        let (mut f, _rec) = MockIsolateFactory::new();
        let mut iso = f.spawn(IsolateId::new(1, GpuId::ZERO));
        let mut w = iso.checkout().expect("fresh pool has an idle worker");
        w.with_rm(|rm| {
            let vas_a = rm.alloc_vaspace().unwrap();
            let vas_b = rm.alloc_vaspace().unwrap();
            let mem = rm.alloc_sysmem(0x1000).unwrap();
            const BASE: u64 = 0x2_0000_0000;
            for k in 0..70_000u64 {
                let want = GpuVa(BASE + k * 0x1000);
                // The SAME address is mapped into BOTH VASes — the #14 arrangement.
                for vas in [vas_a, vas_b] {
                    assert_eq!(
                        rm.map_gpu_va(vas, mem, 0x1000, want),
                        Ok(want.0),
                        "map #{k} must land exactly at {want:?} in {vas:?}"
                    );
                }
            }
            // A third map over a live range in the SAME VAS is refused, not relocated.
            assert_eq!(
                rm.map_gpu_va(vas_a, mem, 0x1000, GpuVa(BASE)),
                Err(RmError::NoMemory),
                "a fixed map over a live range in one VAS is loud"
            );
            // …and becomes legal again once the range is released (unmap eager, rebind).
            rm.unmap_gpu_va(vas_a, BASE).unwrap();
            assert_eq!(rm.map_gpu_va(vas_a, mem, 0x1000, GpuVa(BASE)), Ok(BASE));
        });
    }

    /// ★ §12.35 — [`RmRecorder::compact`]'s defining invariant: the ledger is the same
    /// value whether or not the log was ever drained.
    ///
    /// This is the regression `concurrency_stress` was silently carrying. Its 16-thread
    /// soak drains the global verb log every 4096 ops to bound memory; with a bare
    /// `mem::take` that deleted every acquisition in the drained prefix, so the whole
    /// run's live host state read back as dangling. Asserting the equality here, on a
    /// scripted three-verb log, is what keeps the drain a supported operation instead of
    /// a footgun that only shows up under a slow-gated stress test.
    #[test]
    fn recorder_compact_preserves_the_ledger_exactly() {
        let (mut f, rec) = MockIsolateFactory::new();
        let mut iso = f.spawn(IsolateId::new(1, GpuId::ZERO));
        let (vas, mem) = iso.checkout().expect("idle worker").with_rm(|rm| {
            let vas = rm.alloc_vaspace().unwrap();
            let mem = rm.alloc_sysmem(0x1000).unwrap();
            let _ = rm
                .map_gpu_va(vas, mem, 0x1000, GpuVa(0x2_0020_0000))
                .unwrap();
            (vas, mem)
        });

        let undrained = rec.lock().expect("recorder").ledger();
        assert_eq!(
            undrained.leaked_on(IsolateId::new(1, GpuId::ZERO)),
            BTreeSet::from([vas, mem]),
            "precondition: both handles are outstanding before any drain"
        );
        assert_eq!(
            undrained.leaked_maps[&IsolateId::new(1, GpuId::ZERO)].len(),
            1,
            "precondition: the one mapping is outstanding too"
        );

        let drained = rec.lock().expect("recorder").compact();
        assert_eq!(
            drained.len(),
            3,
            "compact hands out exactly the drained prefix"
        );
        assert!(
            rec.lock().expect("recorder").log.is_empty(),
            "…and the log really is emptied, so the memory bound still holds"
        );
        assert_eq!(
            rec.lock().expect("recorder").ledger(),
            undrained,
            "★ the ledger is UNCHANGED by the drain — the whole point"
        );

        // And a verb issued AFTER the compaction folds onto the carried accounting
        // rather than replacing it: freeing `mem` must leave exactly `vas` outstanding.
        iso.checkout()
            .expect("idle worker")
            .with_rm(|rm| rm.free(mem))
            .expect("free");
        let after = rec.lock().expect("recorder").ledger();
        assert_eq!(
            after.leaked_on(IsolateId::new(1, GpuId::ZERO)),
            BTreeSet::from([vas]),
            "post-compaction verbs fold ONTO the carried ledger, never over it"
        );
        assert!(
            after.free_of_unknown.is_empty(),
            "the freed handle was known despite its alloc living only in the carried half"
        );
    }

    /// A handle whose raw value is live in **no** client is `BadHandle` — RM's own
    /// `clientGetResource` failure, and the arm that must survive the host-faithful
    /// rewrite of [`MockRmBackend::check`].
    ///
    /// ★ Note precisely what this does and does not show. Isolate B has allocated
    /// nothing, so `ha`'s host-visible value names no object *there* either. It is a test
    /// of the unknown-handle arm, **not** of blast-radius containment — which the mock
    /// deliberately no longer provides, because a real host does not. See
    /// [`a_sibling_clients_live_raw_value_is_served_exactly_as_a_real_host_would`].
    #[test]
    fn mock_rm_handles_are_isolate_scoped() {
        let (mut f, _rec) = MockIsolateFactory::new();
        let mut a = f.spawn(IsolateId::new(1, GpuId::ZERO));
        let mut b = f.spawn(IsolateId::new(2, GpuId::ZERO));
        let ha = a
            .checkout()
            .expect("idle worker")
            .with_rm(|rm| rm.alloc_vaspace().unwrap());
        assert_eq!(
            b.checkout()
                .expect("idle worker")
                .with_rm(|rm| rm.schedule(ha)),
            Err(RmError::BadHandle(ha))
        );
    }

    /// ★★★ **The double's one known LIE, corrected** (`host_execution_plane.md` §2.1).
    ///
    /// `MockRmBackend` used to validate handles against its own per-isolate namespace, so
    /// a cross-namespace handle was *refused by the backend*. [`HostHandle`]'s own docs
    /// say a real host does **not**: every client's handles come from one
    /// `RS_CLIENT_HANDLE_BASE`, so isolate A's n-th object and isolate B's n-th object
    /// wear the same raw value, both live and unrelated. `07da582`'s cross-GPU handle was
    /// caught **only** by that refusal — i.e. by something production does not have.
    ///
    /// Five facts, in the order the damage happens:
    ///
    /// 1. two clients really do mint the same host-visible value;
    /// 2. presenting A's handle on B's connection **succeeds** — no fault, as on a host;
    /// 3. …and it is recorded as the [`BystanderHit`] it is, naming the local victim;
    /// 4. the victim is **destroyed**: B's own handle no longer resolves, which is the
    ///    actual damage rather than a proxy for it;
    /// 5. the [`HostLedger`] still catches it — `free_of_unknown` names the reach and the
    ///    destroyed twin stays outstanding forever. That is the right place for the
    ///    detector: an audit of what happened, not the backend's luck.
    #[test]
    fn a_sibling_clients_live_raw_value_is_served_exactly_as_a_real_host_would() {
        let (mut f, rec) = MockIsolateFactory::new();
        let (ia, ib) = (
            IsolateId::new(1, GpuId::ZERO),
            IsolateId::new(2, GpuId::ZERO),
        );
        let mut a = f.spawn(ia);
        let mut b = f.spawn(ib);
        let ha = a
            .checkout()
            .expect("idle worker")
            .with_rm(|rm| rm.alloc_vaspace().unwrap());
        let hb = b
            .checkout()
            .expect("idle worker")
            .with_rm(|rm| rm.alloc_vaspace().unwrap());

        // ---- (1) one base, two clients, one value.
        assert_ne!(ha, hb, "the recorded PROVENANCE still separates them");
        assert_eq!(
            ha.raw() & HOST_RAW_MASK,
            hb.raw() & HOST_RAW_MASK,
            "★ …but the HOST-VISIBLE value is identical, as RM's single handle base makes it"
        );

        // ---- (2)+(3) B serves A's handle against B's own object.
        assert_eq!(
            b.checkout().expect("idle worker").with_rm(|rm| rm.free(ha)),
            Ok(()),
            "★★ a real host does not fault here — it names a different, LIVE object"
        );
        assert_eq!(
            rec.lock().expect("recorder").bystander_hits,
            vec![BystanderHit {
                isolate: ib,
                presented: ha,
                hit: hb,
            }],
            "★ and the double says which bystander was hit, by name"
        );

        // ---- (4) the bystander really is gone from B's client.
        assert_eq!(
            b.checkout()
                .expect("idle worker")
                .with_rm(|rm| rm.schedule(hb)),
            Err(RmError::BadHandle(hb)),
            "★★ B's OWN object was destroyed by a verb that never named it"
        );

        // ---- (5) the ledger is the detector, and it names both halves of the damage.
        let l = rec.lock().expect("recorder").ledger();
        assert_eq!(
            l.free_of_unknown,
            vec![(ib, ha)],
            "the cross-namespace reach (boundary 2), recorded"
        );
        assert_eq!(
            (l.leaked_on(ia), l.leaked_on(ib)),
            (BTreeSet::from([ha]), BTreeSet::from([hb])),
            "★ A's object is untouched and B's destroyed twin is outstanding forever — \
             nothing will ever free it, because nothing can name it"
        );

        // ---- The unknown-handle arm is untouched: a value live NOWHERE still faults.
        let mut c = f.spawn(IsolateId::new(3, GpuId::ZERO));
        assert_eq!(
            c.checkout()
                .expect("idle worker")
                .with_rm(|rm| rm.schedule(ha)),
            Err(RmError::BadHandle(ha)),
            "a client with no object at that value still refuses — this is not a \
             blanket accept"
        );
    }

    /// ★★★ **The park witness reads the real thread-local, not a constant.**
    ///
    /// Every wedge test asserts [`ParkedVerb::held_lock_mask`] `== 0`, and a field
    /// hard-wired to `0` would satisfy all of them — the exact vacuity this campaign keeps
    /// finding. So this test makes the witness report a **non-zero** mask, which is only
    /// possible if it is genuinely reading [`kayfabe_util::lockwitness::held_mask`].
    ///
    /// ★ And the shape it uses is not artificial. `Worker::with_rm` asserts lock-freedom
    /// at **its entry**; a rank noted *inside* the closure is a lock acquired **past the
    /// door**, which the entry assert cannot see and which is precisely the R1 violation
    /// the park record exists to catch — a thread spending an unbounded host wait with one
    /// of our ranked locks held.
    #[test]
    fn the_park_witness_reads_the_real_thread_local_not_a_constant() {
        let _wd = bounded("the_park_witness_reads_the_real_thread_local_not_a_constant");
        use kayfabe_util::lockwitness;

        let (mut f, rec) = MockIsolateFactory::new();
        let id = IsolateId::new(7, GpuId::ZERO);
        let mut iso = f.spawn(id);
        let hold = rec.lock().expect("recorder").hold(HoldSpec::exact(
            id,
            WorkerId(0),
            VerbKind::AllocVaSpace,
        ));
        let mut w = iso.checkout().expect("idle worker");

        std::thread::scope(|sc| {
            sc.spawn(|| {
                w.with_rm(|rm| {
                    lockwitness::note_acquired(1); // ★ past the entry assert
                    let r = rm.alloc_vaspace();
                    lockwitness::note_released(1);
                    r
                })
                .expect("the verb completes once released");
            });
            hold.wait_until_pending();
            hold.release();
        });

        assert_eq!(
            rec.lock().expect("recorder").parked_of(id),
            vec![ParkedVerb {
                isolate: id,
                worker: WorkerId(0),
                verb: VerbKind::AllocVaSpace,
                held_lock_mask: lockwitness::bit(1),
            }],
            "★★ the park record must carry the mask the PARKING THREAD actually held. A \
             constant `0` passes every `== 0` assertion in the suite and detects nothing"
        );
        assert_eq!(
            lockwitness::held_depth(),
            0,
            "the test thread itself never held one"
        );
    }

    /// ★★ [`ClientLock`]: verbs of ONE client serialise; verbs of DIFFERENT clients do
    /// not. The port-level version of what `l1_mean` asserts through the whole device.
    ///
    /// The edge is [`ClientLock::wait_until_blocked`], never a sleep.
    #[test]
    fn one_clients_verbs_serialise_and_another_clients_do_not() {
        let _wd = bounded("one_clients_verbs_serialise_and_another_clients_do_not");
        let (mut f, rec) = MockIsolateFactory::new();
        rec.lock().expect("recorder").serialize_per_client = true;
        let (ia, ib) = (
            IsolateId::new(1, GpuId::ZERO),
            IsolateId::new(2, GpuId::ZERO),
        );
        let mut a = f.spawn(ia);
        let mut b = f.spawn(ib);
        let hold = rec.lock().expect("recorder").hold(HoldSpec::exact(
            ia,
            WorkerId(0),
            VerbKind::AllocVaSpace,
        ));
        let lock_a = rec
            .lock()
            .expect("recorder")
            .client_lock(ia)
            .expect("spawned");
        let lock_b = rec
            .lock()
            .expect("recorder")
            .client_lock(ib)
            .expect("spawned");

        let mut w0 = a.checkout().expect("slot 0");
        let mut w1 = a.checkout().expect("slot 1");
        std::thread::scope(|sc| {
            sc.spawn(|| w0.with_rm(|rm| rm.alloc_vaspace().expect("released eventually")));
            hold.wait_until_pending();
            assert_eq!(
                lock_a.held_by(),
                Some((WorkerId(0), VerbKind::AllocVaSpace))
            );

            sc.spawn(|| w1.with_rm(|rm| rm.alloc_sysmem(0x1000).expect("released eventually")));
            lock_a.wait_until_blocked(1);
            assert_eq!(
                lock_a.queued(),
                vec![(WorkerId(1), VerbKind::AllocSysmem)],
                "★ a sibling worker of the SAME client is blocked, named exactly"
            );

            // …while the OTHER client runs to completion, on this very thread.
            b.checkout().expect("idle worker").with_rm(|rm| {
                rm.alloc_vaspace()
                    .expect("a different client is unaffected")
            });
            assert_eq!(
                (lock_b.waits(), lock_b.queued()),
                (vec![], vec![]),
                "★★ …and recorded no block of its own — no cross-client amplification"
            );

            assert_eq!(
                lock_a.waits(),
                vec![ClientWait {
                    isolate: ia,
                    waiter: WorkerId(1),
                    waiter_verb: VerbKind::AllocSysmem,
                    holder: WorkerId(0),
                    holder_verb: VerbKind::AllocVaSpace,
                }],
                "exactly one block, naming both sides"
            );
            hold.release();
        });
        assert_eq!(
            lock_a.held_by(),
            None,
            "every guard released its client's lock"
        );
    }
}
