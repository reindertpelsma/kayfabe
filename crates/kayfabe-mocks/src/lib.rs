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

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use kayfabe_arch::{
    Aperture, Arch, DoorbellTarget, GmmuFmt, GmmuVersion, ObjectKind, PageSize, PteDecode,
    PushMethod, PushRange, PushbufferAbi, UserdModel,
};
use kayfabe_isolate::{
    CancelHandle, CancelReason, CancelSink, HostHandle, Isolate, IsolateFactory, IsolateId,
    RmBackend, RmError, Txn, Worker, WorkerId,
};
use kayfabe_util::Instant;
use kayfabe_vmm::{
    BarId, CoreEvent, FbMeta, GuestRamMap, HostRegion, IrqSpec, Present, PresentError, Prot,
    RamHandle, RamRegionId, RegionKind, SlotId, SurfaceHandle, TrapMode, Vblank, Vmm, VmmError,
};

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
    /// CE `LAUNCH_DMA`: args = [dst_lo, dst_hi, len, flags(bit0 = dst_is_virtual)].
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

    /// Encode a CE `LAUNCH_DMA` to `dst` for `len` bytes.
    #[must_use]
    pub fn ce_launch_dma(dst: u64, len: u64, dst_is_virtual: bool) -> (u32, Vec<u32>) {
        Self::method(
            mock_method::CE_LAUNCH_DMA,
            &[
                dst as u32,
                (dst >> 32) as u32,
                len as u32,
                u32::from(dst_is_virtual),
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
                len: lo64(2),
                dst_is_virtual: lo64(3) & 1 != 0,
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

/// Fake MMU format: VER2-shaped geometry, fake entry encoding
/// (bit0 = valid, bit1 = leaf, bit2 = sysmem aperture, phys in bits [51:12]).
#[derive(Debug, Default)]
pub struct MockGmmuFmt;

/// Mockingbird leaf sizes (includes a huge leaf so the #13 "every real page size"
/// discipline is exercised by construction).
pub const MOCK_PAGE_SIZES: [PageSize; 4] = [
    PageSize(0x1000),
    PageSize(0x10000),
    PageSize(0x200000),
    PageSize(0x2000_0000),
];

impl MockGmmuFmt {
    /// Encode a fake leaf entry (test helper; inverse of `decode_entry`).
    #[must_use]
    pub fn encode_leaf(phys: u64, sysmem: bool) -> u128 {
        u128::from((phys & 0x000f_ffff_ffff_f000) | 0b011 | if sysmem { 0b100 } else { 0 })
    }
}

impl GmmuFmt for MockGmmuFmt {
    fn version(&self) -> GmmuVersion {
        GmmuVersion::Ver2
    }
    fn page_sizes(&self) -> &[PageSize] {
        &MOCK_PAGE_SIZES
    }
    fn entry_size(&self, _level: u8) -> u8 {
        8
    }
    fn levels(&self) -> u8 {
        5
    }
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode {
        let raw = raw as u64;
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
            // Fake rule: leaf size depends on the level it appears at.
            let size = MOCK_PAGE_SIZES[usize::from(4u8.saturating_sub(level).min(3))];
            PteDecode::Leaf {
                phys,
                aperture,
                size,
                read_only: false,
            }
        } else {
            PteDecode::Pde {
                next: phys,
                aperture,
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
    /// Intent: a host memory object exported as a presentable surface (the display
    /// seam's producer half, GR-2b — the isolate-side PRIME export).
    ExportSurface {
        /// The host memory object (render target) that was exported.
        memory: HostHandle,
        /// The minted surface token.
        surface: SurfaceHandle,
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
    /// [`RmBackend::export_surface`].
    ExportSurface,
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
    pub fn wait_until_pending(&self) {
        let mut g = self.state.lock().expect("hold");
        while !g.entered {
            g = self.cv.wait(g).expect("hold");
        }
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
    /// Armed holds, matched newest-spec-first and consumed one-shot.
    holds: Vec<(HoldSpec, Arc<VerbHold>)>,
    /// ★ Accounting for verbs whose log entries [`RmRecorder::compact`] has already
    /// handed out (`l1_concurrency.md` §12.35). [`RmRecorder::ledger`] starts from this
    /// and folds the remaining [`RmRecorder::log`] on top, so draining the log to bound
    /// memory costs nothing in accounting — which it silently did before.
    carried: HostLedger,
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
    next_map_page: u64,
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
}

impl MockRmBackend {
    fn namespace(idlane: u64, gpu: GpuId) -> Arc<Mutex<RmNamespace>> {
        Arc::new(Mutex::new(RmNamespace {
            handles: BTreeSet::new(),
            next: 1,
            // Namespaced fake host tokens: primary lane = idlane (so `>>20` still reads
            // ProcId+1); the target GPU is folded into a LOWER field, so two isolates of
            // ONE proc on DIFFERENT GPUs still mint provably-disjoint tokens (MG-5).
            next_token: (idlane << 20) | (u64::from(gpu.0) << 8),
            next_map_page: 0,
            retired: false,
        }))
    }

    fn new(
        id: IsolateId,
        worker: WorkerId,
        recorder: SharedRecorder,
        ns: Arc<Mutex<RmNamespace>>,
        cancel: Arc<MockCancelSink>,
    ) -> Self {
        MockRmBackend {
            id,
            gpu: id.gpu(),
            worker,
            idlane: u64::from(id.proc()) + 1,
            recorder,
            ns,
            cancel,
        }
    }

    /// High 40 bits shared by every handle/surface: `idlane` (bits ≥32, so `>>32`
    /// reads ProcId+1) with the target GPU folded into bits [24..32) — distinct per
    /// `(isolate, GPU)`, the MG-5 cross-GPU blast-radius property.
    fn handle_hi(&self) -> u64 {
        (self.idlane << 32) | (u64::from(self.gpu.0) << 24)
    }

    /// Pre-verb gate: park in any armed [`VerbHold`] matching this
    /// `(isolate, gpu, worker, verb)`, then apply `fail_next` / the retired refusal.
    ///
    /// The hold is looked up and REMOVED under the recorder lock, then parked in with
    /// the recorder lock released — otherwise a held verb would freeze every other
    /// isolate's recording and the "everything else keeps running" property the mean
    /// test asserts would be a lie about the mock, not about the design.
    fn gate(&mut self, verb: VerbKind) -> Result<(), RmError> {
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
        Ok(())
    }

    fn mint(&mut self) -> HostHandle {
        let mut ns = self.ns.lock().expect("ns");
        let h = HostHandle::new(self.id, self.handle_hi() | ns.next);
        ns.next += 1;
        ns.handles.insert(h);
        h
    }

    fn check(&self, h: HostHandle) -> Result<(), RmError> {
        if self.ns.lock().expect("ns").handles.contains(&h) {
            Ok(())
        } else {
            Err(RmError::BadHandle(h))
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
        self.gate(VerbKind::Alloc)?;
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
        self.gate(VerbKind::AllocVaSpace)?;
        let handle = self.mint();
        self.record(RmVerb::AllocVaSpace { handle });
        Ok(handle)
    }

    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError> {
        self.gate(VerbKind::AllocSysmem)?;
        let handle = self.mint();
        self.record(RmVerb::AllocSysmem { handle, len });
        Ok(handle)
    }

    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError> {
        self.gate(VerbKind::AllocChannel)?;
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
        self.gate(VerbKind::AllocEngineObject)?;
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
        self.gate(VerbKind::Schedule)?;
        self.check(chan)?;
        self.record(RmVerb::Schedule { chan });
        Ok(())
    }

    fn free(&mut self, obj: HostHandle) -> Result<(), RmError> {
        self.gate(VerbKind::Free)?;
        self.check(obj)?;
        self.ns.lock().expect("ns").handles.remove(&obj);
        self.record(RmVerb::Free { obj });
        Ok(())
    }

    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        _payload: &mut [u8],
    ) -> Result<(), RmError> {
        self.gate(VerbKind::Control)?;
        self.check(obj)?;
        self.record(RmVerb::Control { obj, cmd });
        Ok(())
    }

    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
    ) -> Result<u64, RmError> {
        self.gate(VerbKind::MapGpuVa)?;
        self.check(vas)?;
        self.check(memory)?;
        // Fake placement with strictly disjoint fields, so every minted VA is
        // distinct by construction and its provenance is readable off the bits:
        //   [46..] base | [40..46) isolate lane | [32..40) VAS lane | [12..32) page
        // The page counter is shared per ISOLATE (all its workers mint from one
        // namespace, exactly as one RM client would), capped LOUDLY before it could
        // bleed into the VAS lane. (The old scheme OR-ed the lane at bit 28 over a
        // bump counter, which wrapped into duplicates after 2^16 pages — caught by
        // the M4b concurrency stress.)
        let page = {
            let mut ns = self.ns.lock().expect("ns");
            assert!(
                ns.next_map_page < 1 << 20,
                "MockRmBackend VA lane exhausted (2^20 pages mapped on isolate {:?} gpu {:?})",
                self.id,
                self.gpu
            );
            let p = ns.next_map_page;
            ns.next_map_page += len.next_multiple_of(0x1000) >> 12;
            p
        };
        let va = 0x4000_0000_0000
            + (self.idlane << 40)
            + (u64::from(self.gpu.0) << 47)
            + ((vas.raw() & 0xff) << 32)
            + (page << 12);
        self.record(RmVerb::MapGpuVa {
            vas,
            memory,
            len,
            va,
        });
        Ok(va)
    }

    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError> {
        self.gate(VerbKind::UnmapGpuVa)?;
        self.check(vas)?;
        self.record(RmVerb::UnmapGpuVa { vas, va: gpu_va });
        Ok(())
    }

    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError> {
        self.gate(VerbKind::RingDoorbell)?;
        self.record(RmVerb::RingDoorbell { token: host_token });
        Ok(())
    }

    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError> {
        self.gate(VerbKind::ExportSurface)?;
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

    #[test]
    fn mock_arch_token_roundtrip_and_malformed_rejected() {
        let a = MockArch::new();
        let v = VChid(0x2a);
        assert_eq!(
            a.decode_doorbell(MockArch::token_for(v)),
            Some(DoorbellTarget { vchid: v })
        );
        assert_eq!(a.vchid_from_userd_flags(MockArch::userd_flags_for(v)), v);
        assert_eq!(
            a.decode_doorbell(0x1234),
            None,
            "hostile token must not decode"
        );
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

    /// Regression (found by the M4b concurrency stress): the old VA minting OR-ed a
    /// per-VAS lane at bit 28 over a page-bump counter, so after 2^16 single-page
    /// maps the counter wrapped into the lane and duplicate host VAs came out. The
    /// mock must mint distinct VAs well past that boundary, across multiple VASes.
    #[test]
    fn mock_rm_map_gpu_va_stays_distinct_past_65536_pages() {
        let (mut f, _rec) = MockIsolateFactory::new();
        let mut iso = f.spawn(IsolateId::new(1, GpuId::ZERO));
        let mut w = iso.checkout().expect("fresh pool has an idle worker");
        w.with_rm(|rm| {
            let vas_a = rm.alloc_vaspace().unwrap();
            let vas_b = rm.alloc_vaspace().unwrap();
            let mem = rm.alloc_sysmem(0x1000).unwrap();
            let mut seen = std::collections::BTreeSet::new();
            for k in 0..70_000u64 {
                let vas = if k % 2 == 0 { vas_a } else { vas_b };
                let va = rm.map_gpu_va(vas, mem, 0x1000).unwrap();
                assert!(seen.insert(va), "duplicate host VA {va:#x} at map #{k}");
            }
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
            let _ = rm.map_gpu_va(vas, mem, 0x1000).unwrap();
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

    #[test]
    fn mock_rm_handles_are_isolate_scoped() {
        let (mut f, _rec) = MockIsolateFactory::new();
        let mut a = f.spawn(IsolateId::new(1, GpuId::ZERO));
        let mut b = f.spawn(IsolateId::new(2, GpuId::ZERO));
        let ha = a
            .checkout()
            .expect("idle worker")
            .with_rm(|rm| rm.alloc_vaspace().unwrap());
        // Using isolate A's handle on isolate B is refused: blast-radius containment.
        assert_eq!(
            b.checkout()
                .expect("idle worker")
                .with_rm(|rm| rm.schedule(ha)),
            Err(RmError::BadHandle(ha))
        );
    }
}
