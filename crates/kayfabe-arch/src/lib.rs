//! # kayfabe-arch — abstract GPU vocabulary + the Axis-B `Arch` trait seams
//!
//! This crate is two things, deliberately together at the bottom of the GPU-domain
//! dependency graph (below `kayfabe-core`, so core/mmu/fwd/completion share one set of
//! newtypes without cycles):
//!
//! 1. **The vocabulary** ([`ids`]): newtypes for every identity the system routes on —
//!    [`ids::HClient`], [`ids::HObject`], [`ids::Pdb`], [`ids::VChid`], [`ids::ClassId`], …
//!    These are *abstract*: no NVIDIA struct layout, no driver-version value, appears
//!    anywhere in this crate (that is Axis A, quarantined to `kayfabe-abi` — design
//!    decision #2, `mode2_abi_agnostic_layer.md`).
//!
//! 2. **The Axis-B seams** ([`Arch`], [`GmmuFmt`], [`UserdModel`]): the *architectural
//!    behavior* a GPU generation defines — token bit-decode, PTE/PDE encoding, USERD
//!    offsets, class-ID recognition (`mode2_abi_agnostic_layer.md` §4.2). The pure-logic
//!    core is written **only** against these traits.
//!
//! ## The anti-C-duplication property (load-bearing, tested)
//!
//! Adding a real architecture (e.g. `Ampere`) is exactly:
//!
//! ```text
//! struct Ampere;              // in an arch-impl crate, NOT in any logic crate
//! impl Arch for Ampere { … }  // + impl GmmuFmt / UserdModel for its regime
//! ```
//!
//! with **zero edits to `kayfabe-core`/`-mmu`/`-fwd`/`-completion`**. The core owns
//! *algorithms* (graph derivation, demux loops, walk logic, completion policy); an
//! `Arch` impl owns *encodings* (which bits mean what). Code that knows both is
//! misplaced (decision #1). The `MockArch` in `kayfabe-mocks` is the first `impl` and
//! the proof of the seam: it uses deliberately fake encodings so any core code that
//! secretly assumes a real NVIDIA encoding fails the mock-driven tests.
//!
//! ## Concurrency (decision #17)
//!
//! The four seams here ([`Arch`], [`GmmuFmt`], [`UserdModel`], [`PushbufferAbi`]) are
//! **`Send + Sync` supertraits**: an implementation is a stateless (or immutable)
//! encoding table, every method takes `&self`, and the composition root stores one
//! `Box<dyn Arch>` inside the `Gpu` that multiple vCPU threads share — so the seam
//! itself must be shareable. An `impl` needing interior mutation would be a design
//! smell (encodings don't change at runtime) and is rejected by the compiler.

pub mod fault;
pub mod gsp;
pub mod ids;

pub use gsp::{
    ArchBootState, BootContext, BootPhase, BootSequence, BootStageDesc, BootStep, BootStepKind,
    BootSteps, GspModel, GspObservation, GspReg, LibosRegionLayout, NoBootSequence, RegWrite,
};
use ids::{ClassId, ControlCmd, EngineKind, GpuVa, Pdb, RunlistId, VChid};

/// ★ **The privilege a client root DECLARES about itself** — the discriminator that
/// decides whether a client belongs to a guest *process* or to the guest *kernel*
/// (decision #14's grouping rule; `l1_concurrency.md` §12.27).
///
/// RM stamps this on the client's own `NV01_ROOT` alloc, at creation time and before
/// any `DUP_OBJECT` can happen: the alloc params carry a `processID` which is the
/// creating client's `ProcID` for a user-privileged client and a reserved
/// kernel sentinel when `privLevel >= RS_PRIV_LEVEL_KERNEL`
/// (the `NV_RM_RPC_ALLOC_SHARE_DEVICE` macro body:
/// `ogkm-610:`/`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` — same path, same
/// lines, byte-identical at both tags; driven from its one call site,
/// `ogkm-610: src/nvidia/src/kernel/gpu/device.c:179-186`, `ogkm-580: :180-187`). It is
/// therefore a **declared protocol fact**, recorded
/// here and never re-derived — the house rule that §12.25's bug came from breaking.
///
/// ★ The three things this is deliberately NOT (all measured on an RTX 3060 / 580.159.04,
/// `l1_concurrency.md` §12.27):
/// - **Not the handle value.** UVM's session client `0xc1d00069` sits numerically
///   *between* two user clients `0xc1d00067`/`0xc1d00068`, sharing
///   `RS_CLIENT_HANDLE_BASE`. `RS_CLIENT_INTERNAL_HANDLE_BASE` (`0xC1E00000`) exists and
///   other kernel clients do use it — UVM's session does not. Keying on the range would
///   mis-file the single most important kernel client in the system.
/// - **Not the process name.** `processName` was empty in every observed record.
/// - **Not an inference from the dup graph.** The dups are what the classification
///   *decides about*; deriving it from them would be circular (and is exactly the
///   collapse this type exists to prevent).
///
/// The numeric sentinel itself is an NVIDIA constant and lives, per the quarantine rule
/// (decision #2), only in `kayfabe-abi` (`kayfabe_abi::client_kind_from_process_id`).
/// Everything above the ABI seam speaks this abstract type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClientKind {
    /// A **user**-privileged client: it declared a real guest process id. Two user
    /// clients are the same `Proc` only if a `DUP_OBJECT` joins them — genuine
    /// sharing, hence one blast radius (decision #14).
    User {
        /// The guest process id the client declared. Recorded because it is the fact
        /// that was declared; grouping deliberately does **not** key on it (sharing,
        /// not co-residency, is what makes one blast radius).
        pid: u32,
    },
    /// A **kernel**-privileged client: RM-internal, owned by the guest *driver*, not by
    /// any one guest process. There is exactly one UVM session client per
    /// `nvidia_uvm` module load and every CUDA process in the guest dups into it, so a
    /// dup *into* one of these is a **reference**, never a merge.
    Kernel,
}

/// What kind of RM object a class ID denotes, as far as the *core* needs to know.
///
/// This is the output of [`Arch::classify`] — the graph *shape* is core/invariant
/// (`mode2_rust_rewrite_architecture.md` §4.3.1a "Arch-invariance"); only the leaf
/// class-ID values change per generation, so recognition is behind the trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// Root client (`RmClientResource`): a handle namespace + access rights.
    /// NOT a process key (values are reused across processes; N per process).
    Client,
    /// `NV01_DEVICE_0`-shaped device node (parent = client).
    Device,
    /// Subdevice node (parent = device).
    Subdevice,
    /// A GPU virtual address space (`FERMI_VASPACE_A`-shaped, parent = device).
    /// ★ THE MEMORY BOUNDARY: once bound, it owns a PDB and the address plane
    /// keys on it (never on `Proc`).
    VaSpace,
    /// A time-slice group / channel group (parent = device). Declares hVASpace + engine.
    Tsg,
    /// A subcontext / context share (parent = TSG). Binds channel ↔ VASpace (VEID).
    CtxShare,
    /// A GPFIFO channel (parent = device or TSG). The exec boundary (vChid).
    Channel {
        /// The [`EngineKind`] the channel's *class* declares (its runlist target).
        /// A GR channel classifies as [`EngineKind::GrCompute`] until an engine
        /// object refines it (graphics/NVENC land as engine objects on a GR-class
        /// channel; the projection derives the refinement — `execution_plane.md`
        /// §2.1/§2.2).
        engine: EngineKind,
    },
    /// An engine object allocated on a channel (compute class, DMA-copy class,
    /// graphics class, NVENC session/encoder class, …). This is what makes its
    /// channel a context of that [`EngineKind`].
    EngineObject {
        /// The engine kind the object makes its channel's context.
        engine: EngineKind,
    },
    /// A memory resource (`NV01_MEMORY_*`, `NV_MEMORY_VIRTUAL`, …): the thing a
    /// `MAP_MEMORY_DMA` maps into a VAS at an offset. First-class because the
    /// address-table's RPC populate source resolves a mapping's `memory` handle to
    /// this node's backing (`execution_plane.md` §1 object-model gap).
    Memory,
    /// An os-event / notifier object (`NV01_EVENT`, semaphore surface): owned by a
    /// client, so completion routing (which os-event → which client/notify-index) is
    /// **graph-derived**, not an opaque id (`execution_plane.md` §1 object-model gap).
    Event,
    /// Semaphore / context-DMA / anything else the graph tracks as a plain node.
    Other,
    /// Not a class this architecture knows. The graph records it as-is (`Unknown` —
    /// a plain node, never re-labelled); callers on stricter paths treat it as a
    /// loud fault, never a guess.
    Unknown,
}

/// Decoded doorbell write (Axis-B seam B6: `token bits → target`).
///
/// E0 (proven on the bench, 2026-07-19): the work-submit token identifies the target
/// channel; the core demuxes on the decoded [`VChid`] — no CPU-state (CR3) read exists
/// anywhere in the design.
///
/// ## ★★★ `runlist` is not decoration — it is the half a decode used to DROP
///
/// This struct used to be `{ vchid }` alone, and on GA10x that is **lossy**: RM's own
/// encoder writes a work-submit token as exactly two fields, `RUNLIST_ID` at 22:16 and
/// `VECTOR` (the chid) at 11:0, and a `{ vchid }` decode throws the first one away.
/// Keeping it is not a routing change (`kayfabe_core`'s exec-plane index is still keyed
/// `(GpuId, VChid)`) — it is a refusal to *silently* discard bits on the one seam where a
/// wrong answer has no second party to notice it (`execution_plane_increments.md` §2.1).
///
/// ⊘ **What the bench measured about the ambiguity, which is the opposite of what an
/// earlier draft of this comment claimed.** `[measured]` RTX 3060 / GA106 / 580.159.04,
/// `docs/reference/bench_evidence/doorbell-census-ba74151.out`:
/// `per_runlist_channel_ram=0`, and six channels held **simultaneously** across four
/// runlists took six **distinct** chids. On this part chids come from one global
/// `CHID_MGR` (`ogkm-580: kernel_fifo.c:1457-1466`), so a chid is device-unique and
/// `(GpuId, VChid)` *is* a channel identity. The earlier draft cited
/// `rm-ladder-419afe8.out:21-25` — where five channels all read chid 7 — as proof of
/// aliasing; that run allocated and freed them **one at a time** and was measuring
/// handle recycling. `doorbell_token_encoding.md` §4 states the scope: the adequacy is a
/// fact about GA106, not about the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellTarget {
    /// The virtual channel ID the token addresses. On GA10x this is the token's
    /// `NV_CTRL_VF_DOORBELL_VECTOR` field, i.e. the chid **within `runlist`**.
    pub vchid: VChid,
    /// The runlist the token addresses.
    ///
    /// An architecture whose token carries no runlist reports [`RunlistId`]`(0)` — the
    /// single-runlist reading — and says so at its `decode_doorbell`.
    pub runlist: RunlistId,
}

/// GMMU format regime (Axis-B spine, `mode2_abi_agnostic_layer.md` §3.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GmmuVersion {
    /// Pascal…Ada: 5–6 levels, 49-bit VA, dual-aperture PTE fields.
    Ver2,
    /// Hopper+: 7 levels, 57-bit VA, PCF, unified addressing.
    Ver3,
}

/// A leaf page size this architecture's MMU can map.
///
/// #13's corollary L3: the walker MUST enumerate **every** real leaf size; a walk
/// hitting an un-enumerated size is a loud fault, never a silent drop (the GA10x
/// PD1 512M-leaf gap cost weeks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageSize(pub u64);

/// Which physical aperture a PTE points into.
///
/// ★★★ **This is half of every physical-address key in the port, and it is not
/// optional** (`gpga_address_space.md`; [`crate::FbWindow`]'s viewer index). A bare
/// address is not an identity: vidmem offset `X` and sysmem offset `X` are different
/// bytes on different devices, and a map keyed on the address alone silently aliases
/// them. That is the identity/uniqueness defect family — the same shape as the measured
/// client-aliasing bug, where an `hClient` was treated as naming a process and two CUDA
/// processes turned out to share one. Key on `(Aperture, address)`, never on `address`.
///
/// ★ `Ord` is derived so the pair can key a `BTreeMap` — i.e. so that the *correct* key
/// is also the *convenient* one. The order itself is the declaration order and carries
/// no meaning; nothing may branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Aperture {
    /// GPU-local video memory (frame buffer).
    Vidmem,
    /// System memory, coherent mapping.
    SysmemCoherent,
    /// System memory, non-coherent mapping.
    SysmemNonCoherent,
    /// Peer GPU memory.
    Peer,
}

/// ★★★ **A framebuffer window — an aperture whose accesses are DEVICE MEMORY, not
/// registers.**
///
/// # Why this is a separate name and not one more unclaimed offset
///
/// `kayfabe_device::plane`'s module docs argue that an unclaimed *register* may read a defaulted zero,
/// because the guest is reading a register a real chip does have and answering zero is a
/// wrong value rather than a fabricated mapping. **That argument does not extend to these
/// three windows**, and conflating them is what this type exists to stop:
///
/// - a write here is a write to the framebuffer, and **a page table lives in the
///   framebuffer**. Dropping it silently is `#13 CE-DROP` with a different transport — the
///   mapping is simply absent later, at an address that names nothing;
/// - a read here is a read of framebuffer *content*, so a defaulted zero is not merely a
///   wrong register value, it is a page of invalid page-table entries — which is a
///   perfectly well-formed answer meaning *"this maps nothing"*. That is the exact shape
///   `kayfabe_mmu::walker::FbRead` refuses to spell as zeros.
///
/// ⊘ **Nothing here serves a byte.** This port models no device memory; it names the
/// absence so that a boot which touches the framebuffer says so in its own counters rather
/// than looking like a boot that touched an unmodelled register.
///
/// # The measurement that made this worth a type (2026-07-31)
///
/// Replaying the committed C reference traces
/// (`nvidia-gpu-passthrough/traces/mode2_c_reference/`) and bucketing every recorded
/// base-address-register write by region: the cold boot `cap1b_coldboot_hermetic_d6`
/// carries **177 856** instance-window writes, **33 978** `PRAMIN` writes and **2**
/// framebuffer-aperture writes; the matmul trace `cap3_matmul_forwarding` carries
/// **214 552 / 33 978 / 1 511** — i.e. **211 836** and **250 041** window accesses in the
/// two runs, **461 877** in total. Under this port every one of them was indistinguishable
/// from an unknown register offset.
///
/// ★ **That census is a test, not a paragraph**:
/// `crates/kayfabe-crec/tests/fb_window_census.rs` re-derives the cold boot's three numbers
/// from the capture this repository carries, **through this classifier**, and fails if they
/// move. It reads `cap1_coldboot_hermetic` rather than `cap1b`, which differs only by 32
/// witnessed continuation-element reads and has the identical window census — checked, not
/// assumed.
/// ★ **Home note.** This type lived in `kayfabe-device` until the GPGA viewer index
/// needed to name a window from the address plane. `kayfabe-trace` depends on
/// `kayfabe-mmu` and `kayfabe-device` depends on `kayfabe-trace`, so `mmu -> device`
/// would have been a cycle; the *name* is architecture vocabulary (which windows a
/// chip has), so it belongs here, while the classifier that decides which window an
/// offset is in stays a chip-row fact in `kayfabe_device::ChipProfile::fb_window`.
/// `kayfabe_device::FbWindow` still resolves — it is a re-export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FbWindow {
    /// The 1 MiB `PRAMIN` window in the register aperture, positioned by
    /// `NV_PBUS_BAR0_WINDOW`. **Untranslated**: the framebuffer address is the window base
    /// plus the offset (`C: src/qemu/nvkvm_gpu_emul.c:904-908`).
    Pramin,
    /// Base-address register `kayfabe_abi::pcibars::bus_bar::FB` — the framebuffer
    /// aperture. GMMU-translated through the `bar1PdeBase` this port itself reports
    /// (`C: :4604`).
    FbAperture,
    /// Base-address register `kayfabe_abi::pcibars::bus_bar::INST` — RM's `BAR2` window.
    /// Identity while the window is bound physical, GMMU-translated through `bar2PdeBase`
    /// afterwards (`C: :6588`).
    InstanceWindow,
}

impl FbWindow {
    /// One clause naming the window, for a diagnostic. Never branched on.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pramin => "the PRAMIN framebuffer window",
            Self::FbAperture => "the framebuffer aperture (BAR1)",
            Self::InstanceWindow => "the instance/BAR2 window",
        }
    }
}

/// ★★ **ONE edge out of a page-directory slot** — where the sub-table is, which
/// aperture it lives in, and what level it is.
///
/// It is a type rather than three fields because a slot can name **two** of them
/// ([`PteDecode::Pde::also`]), and three loose fields cannot be optional together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PdeEdge {
    /// Physical address of the next level.
    pub next: u64,
    /// Aperture the next level lives in.
    pub aperture: Aperture,
    /// ★★ **Which level the pointed-at table is** — supplied by the format, never
    /// assumed to be `level + 1` by the walker.
    ///
    /// It is `level + 1` for most edges and it is **not** universally: in the regime the
    /// C artifact decoded, the deepest directory's slot names a small-page table *and* a
    /// big-page table, which are two different rows of [`GmmuFmt::level_shift`]'s table
    /// with different strides and different entry counts
    /// (`C: nvkvm_gpu_emul.c:8706-8708`, rows 4 and 5). A walker that incremented would
    /// decode a 32-entry table as a 512-entry one and read 3 840 bytes past its end.
    pub child_level: u8,
}

/// A decoded page-table entry, in core terms (no raw bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PteDecode {
    /// Invalid / not present.
    Invalid,
    /// ★★★ **SPARSE — a third state, and conflating it with either neighbour is a
    /// different bug** (`reachability_on_transition.md` §3.6, hole 6).
    ///
    /// Sparse is a distinct fill state with its own templates in the walker RM ships —
    /// `MMU_WALK_FILL_SPARSE`, `ogkm-580:
    /// src/nvidia/src/kernel/gpu/mmu/gmmu_walk.c:904-935`. The guest declares *"there is
    /// deliberately nothing here"*, which is not the same statement as *"nothing has been
    /// written here"*:
    ///
    /// - fold it into [`PteDecode::Leaf`] and a valid→sparse transition **binds** a
    ///   mapping the guest declared backing-free;
    /// - fold it into [`PteDecode::Invalid`] and the declaration disappears, so nothing
    ///   downstream can ever answer a sparse access differently from an unmapped one.
    ///
    /// It contributes **no binding and no edge**. Its whole content is that it is
    /// distinguishable.
    Sparse,
    /// A pointer to a next-level page directory/table — one edge, or two.
    Pde {
        /// The sub-table this slot names.
        edge: PdeEdge,
        /// ★★★ **The SECOND sub-table a dual entry names**, or `None`.
        ///
        /// The deepest directory's slots are 16 bytes wide and name two sub-tables — a
        /// small-page table and a big-page table — not one
        /// (`ogkm-580: src/common/inc/swref/published/pascal/gp100/dev_mmu.h:112`, the
        /// `DUAL_PDE` width; the level table at `ogkm-580:
        /// src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:61-102`). A
        /// decode that can only return one of them drops the other **silently**, which
        /// is `#13`'s exact shape one level up the tree, and the previous shape of this
        /// enum could not express the second edge at all.
        also: Option<PdeEdge>,
    },
    /// A leaf mapping.
    Leaf {
        /// Physical address of the mapped page.
        phys: u64,
        /// Aperture of the mapped page.
        aperture: Aperture,
        /// Leaf page size (must be one of [`GmmuFmt::page_sizes`]).
        size: PageSize,
        /// Guest-visible read-only bit.
        read_only: bool,
    },
}

/// ★★★ **The geometry of ONE page-table level** — how far apart its entries' virtual
/// addresses are, and how many of them a table at that level holds.
///
/// This is the second half of what a page-table decode needs; the first half is
/// [`GmmuFmt::decode_entry`], which says what one entry *means*. Without the geometry a
/// decoder can read an entry but cannot say **which virtual address it describes**, and
/// a leaf with no VA is not a mapping — which is why the C carries exactly this table
/// beside its decoder (`C: nvkvm_gpu_emul.c:8706-8708`).
///
/// ★ **`entries` is a count, not a size.** The two are not interchangeable: the big-page
/// regime's table holds **32** entries in a page that could hold 512, so
/// `page_bytes / entry_size` over-reads it by 3 840 bytes. The count is a fact about the
/// format and it is stated as one.
///
/// ★ The word here was "the *measured* regime", and the 32 is **read**, from the C's own
/// table beside its decoder (`C: nvkvm_gpu_emul.c:8706-8708`) — this port has not decoded
/// a big-page table on hardware and watched the count. The over-read arithmetic is
/// arithmetic and holds either way; what is not established by a reading is that the
/// hardware in front of us is in that regime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelShift {
    /// Virtual-address bit position at which this level's entry index begins. Entry `i`
    /// of a table at this level covers `vabase | (i << shift)`.
    pub shift: u8,
    /// How many entries a table at this level holds. **Not** derived from a page size.
    pub entries: u32,
}

/// Axis-B core: the GMMU format codec + level geometry for one MMU regime.
///
/// The **walk algorithm** lives in `kayfabe-mmu` (core logic, regime-independent);
/// this trait supplies only the per-regime *format*: how many levels, how an entry
/// decodes, which leaf sizes exist. ~two heavy impls (VER2/VER3) with thin per-gen
/// deltas (`mode2_abi_agnostic_layer.md` §4.2). #13 lives or dies here.
///
/// `Send + Sync`: a format codec is immutable shared data (crate docs, decision #17).
pub trait GmmuFmt: Send + Sync {
    /// The format regime.
    fn version(&self) -> GmmuVersion;
    /// Every real leaf page size, ascending. MUST be exhaustive (see [`PageSize`]).
    fn page_sizes(&self) -> &[PageSize];
    /// Size in bytes of one entry at directory level `level` (0 = root).
    fn entry_size(&self, level: u8) -> u8;
    /// Number of levels in a full walk (root..leaf) for this regime.
    fn levels(&self) -> u8;
    /// ★★★ Geometry of level `level` — [`LevelShift`]. `None` for a level this regime
    /// does not have.
    ///
    /// **`None` is a loud answer, not a default.** A decoder handed a level it cannot
    /// size must fault; guessing a stride would place every leaf under it at the wrong
    /// virtual address, and a wrong VA in the address table is a cross-context mapping,
    /// not a cosmetic error. The #13 lesson stated one level up: an un-enumerated size
    /// is a loud fault, never a silent drop.
    ///
    /// ★ The level numbering is the **regime's own**, ascending from the root, and it
    /// may contain more rows than [`GmmuFmt::levels`] counts a single walk to be deep:
    /// where a regime offers two alternative leaf tables (small pages and big pages)
    /// under one directory, those are two rows, reachable only via
    /// [`PteDecode::Pde::child_level`].
    fn level_shift(&self, level: u8) -> Option<LevelShift>;
    /// Decode a raw entry read at `level` into core terms.
    ///
    /// An encoding this regime cannot represent decodes to [`PteDecode::Invalid`];
    /// it must never be *guessed* into a leaf.
    fn decode_entry(&self, level: u8, raw: u128) -> PteDecode;
}

/// Axis-B seam B5: USERD layout accessors + the #11 liveness rule's geometry.
///
/// The core treats USERD as an opaque page with named fields; the per-generation
/// impl knows the offsets. (The #11 USERD-wipe bug is prevented structurally in the
/// core by type-distinguishing pages backing live host objects; this trait only
/// supplies the geometry.)
///
/// `Send + Sync`: geometry is immutable shared data (crate docs, decision #17).
pub trait UserdModel: Send + Sync {
    /// Total USERD size in bytes for one channel.
    fn userd_size(&self) -> u64;
    /// Byte offset of the GP_GET field (host progress pointer) within USERD.
    fn gp_get_offset(&self) -> u64;
    /// Byte offset of the GP_PUT field (guest submit pointer) within USERD.
    fn gp_put_offset(&self) -> u64;
}

/// Where a **physical** copy-engine operand lives — the `SET_{SRC,DST}_PHYS_MODE.TARGET`
/// the engine reads when the operand's `_TYPE` is `_PHYSICAL`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:66-80`).
///
/// ★★★ **This is the residency signal for a physical operand, and nothing else carries
/// it.** A physical CE address is a raw number in one of several apertures whose ranges may
/// collide, so `dst`/`src` alone cannot say whether the bytes are in the emulated
/// framebuffer or in guest sysmem. `memmgrTestCeUtils`' vid→sys copy is exactly this case:
/// the CeUtils pushbuffer emits `_TARGET_LOCAL_FB` for the FB surface and
/// `_TARGET_{COHERENT,NONCOHERENT}_SYSMEM` for the sysmem one
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:629-642, 1069-1082`).
///
/// ⊘ **Meaningful only when the corresponding operand is `_is_virtual == false`.** For a
/// virtual operand the engine consults the MMU, not this register, so the value is carried
/// but not consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysTarget {
    /// `_LOCAL_FB` (0) — device framebuffer. Also the register's reset value, so an
    /// unlatched physical operand is faithfully `LocalFb` (`clc7b5.h:68`).
    LocalFb,
    /// `_COHERENT_SYSMEM` (1) — system memory, coherent (`clc7b5.h:69`).
    CoherentSysmem,
    /// `_NONCOHERENT_SYSMEM` (2) — system memory, non-coherent (`clc7b5.h:70`).
    NonCoherentSysmem,
    /// `_PEERMEM` (3) — a peer GPU's memory (`clc7b5.h:71`). Not reachable by a CPU copy;
    /// carried so the classifier can refuse it by name rather than mistake it for FB.
    Peer,
}

/// ★★★ **Where a CPU-executed copy-engine operand's bytes are READ AND WRITTEN** — the
/// shell-side store a [`crate::PhysTarget`] or a binding's [`Aperture`] resolves to.
///
/// There are exactly two such stores, and they are different number spaces that can collide
/// numerically — which is the whole reason [`crate::PhysTarget`] exists and a raw address is
/// not enough:
///
/// - [`CpuPlane::Fb`] — the emulated framebuffer (`_TARGET_LOCAL_FB`, or an
///   [`Aperture::Vidmem`] binding). Reached in the shell through its `SparseFb`.
///   `[measured 2026-08-08, boot `run_p35_84d857d`]` the walling CeUtils channel's **page
///   directories** are here.
/// - [`CpuPlane::GuestRam`] — guest system memory (`_TARGET_{COHERENT,NONCOHERENT}_SYSMEM`,
///   or a sysmem binding). Reached through the `Vmm`.
///   `[measured 2026-08-08, boot `run_p35_84d857d`]` that channel's **ring, pushbuffer and
///   finishPayload** are all here, so this is the arm the wall actually takes.
///
/// ⊘ Peer memory (`_TARGET_PEERMEM`) is neither and has no `CpuPlane`: an operand naming it
/// is refused by name rather than mistaken for one of these two.
///
/// # ⚠ This says WHERE the CPU reads. It does **not** say who guarantees it stays there.
///
/// That is [`Backing`], and the two are deliberately separate fields of [`Residency`] — see
/// that type for the measurement that makes the split load-bearing rather than tidy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuPlane {
    /// The emulated framebuffer — device-local storage we invented, held in the shell.
    Fb,
    /// Guest system memory — reached through the `Vmm`'s `gpa_read`/`gpa_write`.
    GuestRam,
}

/// ★★★ **AN ADDRESS INSIDE A [`CpuPlane`]** — where a byte actually lives in the store that
/// holds it. Its own type, and that is the entire point.
///
/// # ⊘ Why this is a NEWTYPE and not a second `u64` field
///
/// `[measured 2026-08-08]` two functions in **one module** (`kayfabe_rt::cpu_ce`) disagreed
/// about what the address in a copy-engine sub-copy meant: `execute_ours` wrote the
/// destination plane **at the address as given** — a GPU virtual address for a virtual
/// operand — while `write_completion`, ten lines away, *resolved* the address and wrote at
/// `binding.phys + off`. For a physical operand the two coincide, which is exactly why every
/// test in the tree passed: `cpu_ce_executor.rs`'s cases were all physical, and
/// `ce_representability_split.rs` asserted only split-vs-whole, a property a translating and
/// a non-translating executor satisfy *identically*. The real destination
/// (`memmgrTestCeUtils`' memset, `bUseVasForCeCopy = 1`) is **virtual**, so the data half
/// filled the wrong page while the completion half released a truthful-looking semaphore over
/// it — `#12`'s where-mistake, which cost the C artifact weeks.
///
/// Standing owner directive (`no_real_phys_only_gpga_or_gpa`, 2026-08-05): this is *"the same
/// species as VA≠GPA, whose fix was a **NEWTYPE not a rename**"*. A sibling `u64` beside
/// [`crate::ids::GpuVa`]`.0` leaves the two mutually assignable and the conflation returns the
/// first time somebody is tired; two types make the mix-up a **compile error**.
///
/// # ⚠ It is NOT one number space — it is one number space *per plane*
///
/// A `PlaneAddr` is meaningless without the [`CpuPlane`] it indexes: a framebuffer offset and
/// a guest-physical address collide numerically and name different bytes. That is why it
/// travels inside a [`CpuOperand`] beside its [`Residency`] rather than on its own, and why
/// `PhysTarget`/[`Aperture`] exist at all.
///
/// ⊘ **Not a host-physical address.** Nothing in this port has one, by design
/// (`no_real_phys_only_gpga_or_gpa`): this is a framebuffer offset or a guest-physical
/// address, chosen by the plane.
///
/// # ★ The separation, proved rather than asserted
///
/// A newtype nobody tested is a rename. These four doctests are the mechanical half: each
/// `compile_fail` row is paired with a **compiling twin that differs by exactly one line**, so
/// a row cannot go green because the snippet stopped compiling for an unrelated reason
/// (`should_panic_matches_the_wrong_site`, in its compile-time form).
///
/// A GPU virtual address does not become a plane address:
/// ```compile_fail
/// let va = kayfabe_arch::ids::GpuVa(0x4_2000_0000);
/// let here: kayfabe_arch::PlaneAddr = va;
/// // error[E0308]: mismatched types
/// //   expected struct `PlaneAddr`, found struct `GpuVa`
/// ```
/// …and the twin that does compile, which is the deliberate act of stating which it is:
/// ```
/// let va = kayfabe_arch::ids::GpuVa(0x4_2000_0000);
/// let here = kayfabe_arch::PlaneAddr(va.0);
/// ```
///
/// A plane address does not silently become a bare integer either — which is what stops it
/// being fed back to something that wants a VA:
/// ```compile_fail
/// let here = kayfabe_arch::PlaneAddr(0x2f2c_b004);
/// let raw: u64 = here;
/// // error[E0308]: mismatched types
/// //   expected `u64`, found struct `PlaneAddr`
/// ```
/// ```
/// let here = kayfabe_arch::PlaneAddr(0x2f2c_b004);
/// let raw: u64 = here.0;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneAddr(pub u64);

impl PlaneAddr {
    /// This address advanced by `by` bytes, wrapping — the caller has already bounded the
    /// length, and the store re-validates the result, so a wrap is a refused access rather
    /// than a panic (boundary-1 posture).
    #[must_use]
    pub const fn offset(self, by: u64) -> PlaneAddr {
        PlaneAddr(self.0.wrapping_add(by))
    }
}

impl core::fmt::Display for PlaneAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PlaneAddr({:#x})", self.0)
    }
}

/// ★★★ **A CPU-reachable operand, WHOLE: where its bytes are, who guarantees they stay, and
/// the address they are at in that store.**
///
/// ⊘ **One `Option`, not two.** The place and the address in it are present under exactly the
/// same condition — the operand is ours to touch — and a pair of independently-optional
/// fields is a state where one is `Some` and the other `None`, which has no meaning and which
/// nothing would ever check. Bundling them is what makes *"the plane address is present
/// exactly when the plane is"* a fact about the type rather than a comment about a struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuOperand {
    /// Where the bytes live and who guarantees they stay there.
    pub residency: Residency,
    /// ★★★ The address **in [`Residency::plane`]** — never a GPU virtual address. For a
    /// physical operand it is the address the command named; for a virtual one it is the
    /// resolved binding's address plus the run's offset into it, computed where the binding
    /// is in hand and never recovered downstream.
    pub addr: PlaneAddr,
}

/// ★★★ **WHO GUARANTEES THE BYTES STAY WHERE THEY ARE**, for the duration of an operation.
///
/// # ⚠ Why this is a second field and not a third [`CpuPlane`] variant (owner, 2026-08-08)
///
/// The first attempt at this seam added a *"not ours, refuse"* arm to the plane, and the
/// owner corrected it against the C artifact's own measured design
/// (`/workspace/nvidia-gpu-passthrough/docs/design/mode2_uvm_residency.md`, DECIDED
/// 2026-06-04, and it ran at **host parity** — it does not refuse):
///
/// - a guest **managed** VA is backed by a host `cudaMallocManaged` allocation; **host UVM
///   owns residency and the guest's UVM is an inert fiction**;
/// - the guest is told a *static* residency (`PreferredLocation=CPU` + `AccessedBy=GPU`, the
///   supported fault-free zero-copy configuration), so it never faults at that level;
/// - real migration happens one level **below** the guest's addressing, at GPA→HPA, through
///   MMU-notifier invalidation → `migrate_to_ram`.
///
/// ⇒ **From the executor's seat managed memory is still guest RAM read through the `Vmm`.
/// The plane is unchanged.** What differs is that we do not own or pin it. So the thing that
/// must not be conflated is not *place*, it is **place vs ownership** — and a two-variant
/// plane conflates *"where the CPU reads"* with *"who guarantees it stays there"*.
///
/// ★★ **Load-bearing today, not speculative.** A CPU copy assumes its source and destination
/// are stable for the duration of the copy. Under a host-owned backing they are not.
/// Recording who guarantees stability is exactly the fact that assumption rests on, and a
/// later managed case is then a **new value of this field** rather than a new arm every
/// `match` in the tree has to learn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Backing {
    /// ★ **We guarantee it.** The emulated framebuffer is ours outright; guest RAM is backed
    /// by memory the hypervisor holds for the life of the machine. Neither moves under a
    /// copy in progress, so a CPU copy over it needs no further interlock.
    Stable,
    /// ★★★ **The host driver owns it and may move it under us.** The managed-memory case:
    /// the backing is a host `cudaMallocManaged` allocation whose residency host UVM
    /// decides. ⊘ Not a refusal *in principle* — the C ran this at parity — but this port
    /// has built no interlock, so an executor that meets one today stops **by name** rather
    /// than copying from a page that may migrate mid-copy.
    HostOwned,
}

/// ★★★ **The residency ANSWER: where the bytes are, and who guarantees they stay.**
///
/// ⊘ One value carrying two independent facts, because they *are* independent: managed
/// memory is [`CpuPlane::GuestRam`] with a [`Backing::HostOwned`], and ordinary sysmem is
/// the same plane with a [`Backing::Stable`]. A type that could only say the first half
/// would make those two indistinguishable — which is the conflation this split exists to
/// remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Residency {
    /// Which shell-side store holds the bytes.
    pub plane: CpuPlane,
    /// Who guarantees they stay there for the duration of an operation.
    pub backing: Backing,
}

impl Residency {
    /// A place we own outright — the common answer, and the only one this port can serve.
    #[must_use]
    pub const fn stable(plane: CpuPlane) -> Residency {
        Residency {
            plane,
            backing: Backing::Stable,
        }
    }
}

/// ★★★ **Residency is a QUERY, not a classification performed inline at a call site.**
///
/// # Why a trait for something with one implementation
///
/// Owner directive, 2026-08-08: once residency is a query, a host-owned/managed case becomes
/// an **implementation behind it** and the CPU executor never changes. The executor asks
/// where bytes live and who owns them, and handles the answer; it does not compute one from
/// an address range, because an address range is exactly what stops being sufficient the
/// moment the backing is somebody else's ([`Backing::HostOwned`]).
///
/// ⊘ **And the implementation is meant to be EXTENDED, not paralleled** (same directive):
/// the C's own record is that a managed session *"reuses Mode-1 machinery"* and that the only
/// genuinely new piece is keeping the guest UVM quiescent. A second residency subsystem
/// beside this one would be the parallel path that directive forbids.
///
/// `None` means **there is no CPU plane at all** — peer memory. It is not a soft failure and
/// not a default: the caller turns it into a named refusal.
///
/// `Send + Sync`: an oracle is consulted wherever a submission is parsed, and every method
/// takes `&self` (decision #17's shape for every Axis-B seam).
pub trait ResidencyOracle: Send + Sync {
    /// Where do the bytes of a **physical** operand live, given the `_TARGET` the guest's own
    /// pushbuffer declared beside it and the address itself?
    ///
    /// ⊘ Both arguments, always: an address without its aperture names nothing
    /// (`mode2_forwarding_model.md`). The `_TARGET` is the guest's declaration; the address
    /// is what an implementation that tracks real backings would look up.
    fn residency_of_physical(&self, target: PhysTarget, addr: u64) -> Option<Residency>;

    /// Where do the bytes behind a **resolved binding** live, given the aperture its leaf
    /// declared and the resolved address?
    fn residency_of_aperture(&self, aperture: Aperture, addr: u64) -> Option<Residency>;
}

/// What a copy-engine command asks the engine to **do**, in core terms.
///
/// The C reads this off `LAUNCH_DMA`'s flags as two booleans, `mscrub`
/// (`MEMORY_SCRUB`) and `remap` (`REMAP_ENABLE`, a constant fill), and both appear
/// *negated* in its execute predicate (`C: nvkvm_gpu_emul.c:6310`): a scrub or a fill is
/// never handed to the host copy engine there. Carried as a decoded kind rather than two
/// arch-shaped flag bits, per the no-raw-bits discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeWork {
    /// A source→destination copy.
    Copy,
    /// `MEMORY_SCRUB` — zero the destination; no source operand.
    Scrub,
    /// `REMAP_ENABLE` constant fill — write a repeating pattern; no source operand.
    ///
    /// ★ **The pattern is part of the fact.** The C carries it as `remapA` and writes it
    /// per 32-bit word (`C: nvkvm_gpu_emul.c:6349`); a decode that dropped it could
    /// describe *that* a fill happened and never *what it wrote*, which is not a
    /// forwardable intent. It is also what makes the fill's split-invariance testable at
    /// all: a byte-uniform pattern hides every phase error.
    ///
    /// # ⚠⚠ `[src]` LIMIT — a `u32` cannot express the driver's whole fill vocabulary
    ///
    /// ⊘ A **reading of the driver**, said as one: no boot has sent an 8-byte fill through
    /// this port. What the source establishes is that the shape exists and is emitted by
    /// code compiled for this class; whether this port's guest ever reaches it is not
    /// something a source read can answer.
    ///
    /// `[src]` `ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:379-419` — the period of
    /// a real CE fill is set by `SET_REMAP_COMPONENTS`, and the driver emits **three**
    /// periods on this class: 1 byte (`memset_1`), 4 bytes (`memset_4`) and **8 bytes**
    /// (`memset_8`, `NUM_DST_COMPONENTS_TWO` across `CONST_A`+`CONST_B`). This field's
    /// downstream phase is `pattern[addr % 4]`, so periods 1, 2 and 4 are expressible
    /// exactly and **8 is not**.
    ///
    /// ⊘ `kayfabe_chips::Ga10xPushbuffer` therefore **refuses** an 8-byte-element fill by
    /// name rather than truncating it to its low four bytes — which would write the wrong
    /// half of every other element, silently. Widening this variant (to element bytes plus
    /// an element width) is the fix; until then the refusal is what keeps the gap visible.
    /// ★ Note the C artifact is *positively wrong* here rather than merely narrow: it
    /// writes `remapA` per 32-bit word unconditionally and reads no component map at all,
    /// so it disagrees with hardware on RM's own 1-byte scrub map for every pattern above
    /// `0xFF`.
    Fill {
        /// The 32-bit pattern, repeated across the destination. Its phase is taken from
        /// the DESTINATION ADDRESS (component `n` of a 4-byte-aligned word), never from
        /// the start of a sub-copy — which is what makes splitting a fill at an
        /// unaligned offset produce the same bytes as filling the range whole.
        ///
        /// ⚠ A decoder is responsible for **normalising** its element into this
        /// representation: a 1-byte-element fill of `v` arrives here as
        /// `u32::from_le_bytes([v; 4])`, not as `v`. The absolute-address phase is only
        /// equal to the engine's transfer-relative phase on an element-aligned
        /// destination, which is a condition the decoder checks.
        pattern: u32,
    },
}

/// A pushbuffer method, decoded into **core terms** (no raw bits). The ONE parser
/// (`kayfabe-fwd`) dispatches on this; the [`PushbufferAbi`] produces it. Mirrors
/// [`PteDecode`]'s "no raw bits in the core" discipline (`execution_plane.md` §2.3):
/// the parser decodes *just* these four fact kinds — everything else is
/// [`PushMethod::Opaque`] and passes through untouched (the anti-emulation boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMethod {
    /// `SET_OBJECT`: the subsequent methods target this engine-object class — used to
    /// confirm the channel's [`EngineKind`] (routing only).
    SetObject {
        /// The engine-object class the channel is now bound to.
        class: ClassId,
    },
    /// CE `LAUNCH_DMA` / `MEMSET` / `COPY`: a copy whose destination the address plane
    /// must capture — #13's CE-PT-write capture input when `dst_is_virtual` is false
    /// (a physical PT-page write) or a data copy otherwise.
    ///
    /// ★ **Both operands and the work kind, because two different decisions read them**
    /// (`eight_blockers_resolved.md` §11.5). The C's *execute* predicate is
    /// `!mscrub && !remap && !src_phys && !dst_phys && is_user_ce(chan_client)`
    /// (`C: nvkvm_gpu_emul.c:6310`) — it reads the SOURCE operand's form and the work
    /// kind, neither of which this method used to carry. A decode that drops them cannot
    /// express the C's decision at all, so the port silently answered it with the
    /// destination-only *capture* predicate instead.
    CeLaunchDma {
        /// Destination address (a GPU VA when `dst_is_virtual`, else a physical addr).
        dst: GpuVa,
        /// Source address (a GPU VA when `src_is_virtual`, else a physical addr).
        /// Meaningless for [`CeWork::Scrub`]/[`CeWork::Fill`], which have no source
        /// (`C: :6320` "No src is set.").
        src: GpuVa,
        /// Length in bytes.
        len: u64,
        /// Whether the destination is a virtual (VAS) address vs a physical FB address.
        dst_is_virtual: bool,
        /// Whether the source is a virtual (VAS) address vs a physical FB address.
        src_is_virtual: bool,
        /// The destination's `SET_DST_PHYS_MODE.TARGET` — the residency signal when
        /// `dst_is_virtual == false`. See [`PhysTarget`].
        dst_target: PhysTarget,
        /// The source's `SET_SRC_PHYS_MODE.TARGET` — the residency signal when
        /// `src_is_virtual == false`. Meaningless for [`CeWork::Scrub`]/[`CeWork::Fill`].
        src_target: PhysTarget,
        /// What the engine is being asked to do.
        work: CeWork,
        /// ★★★ **The completion this launch releases — the ENGINE-class semaphore, and it
        /// is part of the launch rather than a method beside it.**
        ///
        /// `LAUNCH_DMA.SEMAPHORE_TYPE` decides whether the engine writes
        /// `SET_SEMAPHORE_PAYLOAD` to `SET_SEMAPHORE_A:B` **after this copy retires**
        /// (`ogkm-580: clc7b5.h:99`). Carrying it inside the launch is what makes *"signal
        /// only after the bytes are in place"* expressible at all: a separate
        /// [`PushMethod::SemRelease`] would be an unordered sibling, and an executor could
        /// serve it first without anything looking wrong.
        ///
        /// `None` for `SEMAPHORE_TYPE_NONE` — a copy that releases nothing, which is most
        /// of them.
        completion: Option<CeCompletion>,
    },
    /// ★★★ **A `LAUNCH_DMA` that transfers NO DATA and exists only to release its
    /// engine-class semaphore** — `DATA_TRANSFER_TYPE == NONE` with a
    /// `SEMAPHORE_TYPE` this codec can express.
    ///
    /// # ⊘ Why this is its own variant and NOT a `CeLaunchDma { len: 0 }`
    ///
    /// Because it has **no operands at all**. `uvm_hal_volta_ce_semaphore_release`
    /// (`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_ce.c:50-70`) pushes exactly four
    /// words — `SET_SEMAPHORE_A`, `_B`, `_PAYLOAD`, then `LAUNCH_DMA` — and **never
    /// writes `OFFSET_OUT_UPPER`/`_LOWER`**. A `CeLaunchDma` needs a latched destination
    /// (that requirement is [`PushMethod::CeLaunchDma`]'s own anti-`unwrap_or_default`
    /// rule) so this shape could not be expressed as one without inventing an address, and
    /// a zero-length copy to an invented destination is precisely the fiction the codec's
    /// refusals exist to prevent.
    ///
    /// # ★★★ Why decoding it is not a forged completion
    ///
    /// A forged completion advances a payload for work that **did not run**. Here the
    /// guest's own encoding says there is no work: `DATA_TRANSFER_TYPE_NONE` moves zero
    /// bytes, so writing the payload is not a claim *about* a copy — it **is** the whole
    /// act the engine was asked to perform. ⊘ An executor must still honour the ordering
    /// rule that governs [`PushMethod::CeLaunchDma::completion`]: releases are written in
    /// submission order, behind any bytes an earlier launch in the same ring owed.
    ///
    /// # ⚠ What it cost to leave this undecoded
    ///
    /// `[measured 2026-08-09, boots `fmb1`/`msr1` at `319d29a`]` `cuInit` hangs in
    /// `uvm_push_end_and_wait` ← `channel_pool_add` ← `uvm_channel_manager_create`. The
    /// push it waits on is `channel_init`'s (`ogkm-580: uvm_channel.c:2518-2572`):
    /// `SET_OBJECT` (`uvm_maxwell_ce.c:29-37`) then `uvm_channel_end_push`
    /// (`uvm_channel.c:1492, 1512-1513`) → `do_semaphore_release` (`:1043-1051`) →
    /// `uvm_hal_volta_ce_semaphore_release`. ⇒ **The entire content of the first push on
    /// every UVM channel is one release-only launch.** Refusing it made the whole
    /// submission decode to nothing, so the word `uvm_spin_loop` polls could never move.
    CeRelease {
        /// The semaphore this launch releases — the same engine-class
        /// `SET_SEMAPHORE_A/B/PAYLOAD` triple [`PushMethod::CeLaunchDma::completion`]
        /// carries, decoded by the same code.
        ///
        /// ⊘ Not `Option`: a launch that transfers nothing **and** releases nothing is a
        /// no-op the codec reports as [`PushMethod::Opaque`], because there is no fact in
        /// it. This variant exists only when there is something to release.
        completion: CeCompletion,
        /// `LAUNCH_DMA.FLUSH_ENABLE` — the engine flushes before writing the payload.
        ///
        /// ★ Carried because UVM sets it deliberately and its own comment says why
        /// (`ogkm-580: uvm_channel.c:1055-1060`: *"that doesn't provide sufficient
        /// ordering guarantees … just always uses a membar sys"*). An executor that
        /// reordered this release ahead of writes it is meant to fence would be correct on
        /// paper and wrong on the machine.
        flush: bool,
    },
    /// `SEM_RELEASE` / `SET_SEMAPHORE_A/B` + payload / finishPayload: the completion —
    /// a semaphore address in a VAS advanced to `payload`. Extracted for the
    /// completion plane's observe (`execution_plane.md` §2.4).
    ///
    /// ⚠ **The HOST-FIFO semaphore**, not the engine-class one — see [`CeCompletion`] for
    /// the four-byte trap that distinction exists to prevent, and [`PushMethod::CeRelease`]
    /// for the engine-class release that carries no copy.
    SemRelease {
        /// The semaphore address (in the channel's VAS).
        addr: GpuVa,
        /// The payload the semaphore is released to.
        payload: u64,
    },
    /// `MEM_OP_A/C/D` with `OPERATION = MMU_TLB_INVALIDATE`: the invalidate transport —
    /// carries the invalidated PDB and whether a membar (hard barrier) is required.
    TlbInvalidate {
        /// The page-directory base whose TLB is invalidated.
        pdb: Pdb,
        /// True if a membar must be honored before the parser advances.
        membar: bool,
    },
    /// Any method this arch does not model — passed through verbatim, acted on by no
    /// core code (trap-min, decision #6). NEVER guessed into one of the above.
    Opaque,
}

/// ★★★ **A copy engine's own completion semaphore** — the word the guest polls while it
/// waits for a CE operation, and the fact that was **undecodable in this port until E10e**.
///
/// # ⚠ THE MEASUREMENT THAT MAKES THIS LOAD-BEARING, and it is a four-byte trap
///
/// `[src]` `ogkm-580: channel_utils.c:645, 671-673` (the scrub block) and `:832, 838-840`
/// (the general CeUtils block): RM releases its **finishPayload** — the word
/// `channelWaitForFinishPayload` spins on — through the **engine class**
/// (`NVC8B5_SET_SEMAPHORE_A/B/PAYLOAD` + `LAUNCH_DMA.SEMAPHORE_TYPE`), at
/// `pbGpuVA + finishPayloadOffset`.
///
/// At the **bottom** of the very same pushbuffer block (`:698-746`) it *also* releases a
/// **host-FIFO** semaphore (`NVC86F_SEM_EXECUTE` / `NV906F_SEMAPHORED`) at
/// `pbGpuVA + semaOffset` — a different word, four bytes lower
/// (`finishPayloadOffset = semaOffset + CHANNEL_HOST_SEMAPHORE_SIZE`, `:250`), meaning
/// *"HOST has read all the methods"* and **not** *"the copy finished"*.
///
/// ⊘ [`PushMethod::SemRelease`] decodes only the **host** one. `[measured 2026-08-08, boot
/// `run_p35_84d857d`]` on the walling CeUtils channel those two words are guest RAM
/// `0x2f2c_b000` (host) and `0x2f2c_b004` (finishPayload), and the guest's own probe printed
/// `semaVA=0x42006c004 semaOffset=0x6c000`. ⇒ **an executor that fed the releases it could
/// already decode into a completion write would advance `…b000`, log a completion, and leave
/// the guest spinning on `…b004`.** That is `#12`'s where-mistake displaced by four bytes,
/// and it is why this type exists rather than a reuse of `SemRelease`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeCompletion {
    /// The semaphore address, in the same address space as the launch's operands —
    /// `SET_SEMAPHORE_A` (bits `48:32`) over `SET_SEMAPHORE_B` (bits `31:0`).
    ///
    /// ⚠ `A` is the **HIGH** half. The host-FIFO run next door is `SEM_ADDR_LO` then
    /// `_HI`, i.e. the opposite order, and both are five/three-word incrementing runs of
    /// consecutive methods — so a decoder written from one and applied to the other builds
    /// a plausible address out of the halves swapped.
    pub addr: GpuVa,
    /// The payload the engine writes there when the copy retires — `SET_SEMAPHORE_PAYLOAD`,
    /// zero-extended from its 32 bits.
    ///
    /// ⊘ 32 bits and not 64: `SET_SEMAPHORE_PAYLOAD_UPPER` (`clc7b5.h`, method `0x24C`) is a
    /// separate register this codec does not latch, and it is only consulted by the
    /// four-word release — which is refused rather than decoded, so the upper half can never
    /// be silently needed.
    pub payload: u64,
}

/// One pushbuffer range to walk: a contiguous run of method words a GPFIFO entry
/// points at. The core walks these; the arch iterates the GPFIFO to produce them.
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
/// ⊘ **Latent, not live**, which is the only reason this is a recorded defect and not an
/// incident: nothing in the shipped device path reads a pushbuffer.
/// `kayfabe_rt::device::SharedDevice::parse_pushbuffer` is the one in-lock-legal entry
/// point and `kayfabe-qemu-raw`/`kayfabe-shell` never call it, and the same two boots
/// report `doorbells: 0 arrived` across boot + `modprobe` + device open — the guest never
/// submits work at this wall.
///
/// ⊘ **There is no second number to compare against, and that is a finding too.** The
/// binding that would resolve this VA is a `MAP_MEMORY_DMA`, and that RPC is a HAL stub on
/// every GSP-client part, so it never reaches this port (`kayfabe_rmrpc` crate docs §2.7).
/// Resolving a range therefore needs a GMMU walk through the issuing channel's PDB — the
/// machinery `kayfabe_device`'s BAR2 aperture already has — not a table lookup.
///
/// # ★★★ CLOSED, §8.2.3 — the field is a [`GpuVa`] and the consumer TRANSLATES
///
/// `[src]` A GA10x GPFIFO entry's address is `NVC56F_GP_ENTRY0_GET 31:2` +
/// `NVC56F_GP_ENTRY1_GET_HI 7:0` (`ogkm-580:
/// src/common/sdk/nvidia/inc/class/clc56f.h:270, 272`), and what the driver puts there is
/// a **GPU virtual address in the channel's own address space**: UVM computes
/// `pushbuffer_va = uvm_pushbuffer_get_gpu_va_for_push(...)` and hands exactly that to
/// `set_gpfifo_entry` (`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`).
/// `kayfabe_abi::submit::gp_entry_decode` names its own field `gpu_va` for that reason.
///
/// The field used to be `pub gpa: u64` and `kayfabe_fwd::read_pushbuffer` passed it
/// straight to `kayfabe_vmm::Vmm::gpa_read` with no walk. **A GPA-typed field holding a
/// VA was the whole defect**, so the type is what changed: this is a [`GpuVa`], and
/// `kayfabe_fwd::read_pushbuffer` now resolves it through the issuing channel's
/// `kayfabe_mmu::AddressTable` before any guest byte is fetched — MISS = FAULT.
///
/// # ⊘ Why this is a newtype and NOT an `enum { Gpa(u64), GpuVa(GpuVa) }`
///
/// An enum would let an architecture *declare* that its GPFIFO entry names a
/// guest-physical address, and that is the exact hole this change closes — a `Gpa` arm is
/// a supported way back into the untranslated read. Nothing on this wire produces one:
/// `pChannel->pbGpuVA` is assigned **unconditionally** from the `dmaOffset` an
/// `NV04_MAP_MEMORY_DMA` returned (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:842`), every entry is
/// `pbGpuVA + gpOffset` (`:1871-1879`), and the control field is *"Gpfifo Virtual
/// Offset"* (`ogkm-580: ctrl2080fifo.h:809`). An arch that genuinely had a physical
/// GPFIFO would need a new seam and a measurement, not a variant nobody checked.
///
/// ★ The `kayfabe_mocks::MockPushbuffer` entry names a VA too, **now**. It used to store a
/// genuine GPA — an encoding no NVIDIA chip has — which is why no mock-driven test could
/// ever see the question (`mock_fidelity_both_directions`, fourth instance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushRange {
    /// **The GPU virtual address of the method words, in the ISSUING CHANNEL's address
    /// space.** Not a guest-physical address; see the type note.
    pub va: GpuVa,
    /// Length of the range in bytes.
    pub len: u64,
}

/// The number of engine-object subchannels one channel has —
/// `NVC56F_NUMBER_OF_SUBCHANNELS` (`ogkm-580:
/// src/common/sdk/nvidia/inc/class/clc56f.h:67`), mirrored here rather than imported
/// because `kayfabe-arch` is the vocabulary crate and depends on no ABI.
pub const SUBCHANNELS: usize = 8;

/// How many method slots [`MethodState`] holds per subchannel.
///
/// ⚠ **This is the bound, and it is the whole of the bound.** It is not tuned to a
/// workload and it is not a cache size: it is how many *engine registers* an
/// architecture's decoder is allowed to mirror. A guest that sends a billion methods
/// rewrites these same slots a billion times and allocates nothing.
pub const METHOD_SLOTS: usize = 16;

/// ★★★ **The engine method state one channel holds BETWEEN runs** — the accumulator the
/// owner's 2026-08-02 ruling (`execution_plane_increments.md` §8.2.1) requires, and the
/// reason it may exist at all.
///
/// # Why state, when `decode_method` was stateless
///
/// > *"stateless isn't a requirement, it should be following the protocol; if the
/// > protocol holds a state to emulate an action, so may you."*
///
/// A real `AMPERE_DMA_COPY_B` copy is **five separate method runs**, and `LAUNCH_DMA`
/// carries none of its operands: `OFFSET_IN_UPPER…OFFSET_OUT_LOWER` are one earlier run,
/// `LINE_LENGTH_IN`/`LINE_COUNT` another, and `LAUNCH_DMA` is a header plus one word of
/// flags. The engine holds the operands in its own registers and `LAUNCH_DMA` fires what
/// has accumulated. **Mirroring that is following the protocol**, not working around it;
/// a seam that cannot hold state cannot express what the hardware does, which is what
/// `E4` discovered and refused rather than guessed at.
///
/// ★★ And the state is not an optimisation — it is the **classification buffer**. At the
/// moment the methods arrive we do not know the op's *disposition*: whether it is one we
/// must emulate (a privileged kernel operation — the page-table write) or one we
/// translate and replay on the host GPU. That question has no answer until the op is
/// complete, so the operands have to be held somewhere until it is.
///
/// ⊘ **A partial run is NOT meaningless.** Engine method state persists across GPFIFO
/// entries on real hardware, so a run split across two entries is *incomplete state
/// awaiting more methods* — the C's "a partial run means nothing" was a workaround, and
/// repeating it would have made this port diverge from the thing it emulates.
///
/// # ★★★ The cost this replaces it with: state a guest drives is state a guest can abuse
///
/// So it is bounded the way the hardware bounds it, and the bound is **structural rather
/// than checked**:
///
/// - **Finite.** [`SUBCHANNELS`] × [`METHOD_SLOTS`] `u32`s, in a fixed array. No `Vec`, no
///   map, no key the guest supplies. There is no input that makes this bigger, so there is
///   no limit to enforce and no limit to get wrong.
/// - **Per-channel.** It lives on the channel, because engine method state does.
/// - **Reset exactly where the engine resets.** [`MethodState::bind_object`] clears the
///   subchannel it rebinds — a `SET_OBJECT` puts a *different class* on that subchannel,
///   and method `0x400` on the new class is not method `0x400` on the old one, so
///   carrying operands across a rebind would attribute one object's registers to another.
///   The whole state dies with the channel, which is the other reset the hardware has.
///
/// # ⊘ Opaque to the core, and to every architecture but the one that wrote it
///
/// The core carries this value, resets it and hands it back; it reads **no field**. Slot
/// numbering is an architecture's private business — a method address *is* a register
/// index — so nothing here is named for a GA10x method, and an architecture that models a
/// different engine uses the same slots for its own.
///
/// Every accessor is **total**: an out-of-range subchannel or slot writes nothing and
/// reads `None`. A hostile header naming subchannel 7 (the highest `SUBCHANNEL` encodes)
/// is in range by construction, and an implementation that computed a wider index gets a
/// refusal rather than a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodState {
    /// Latched values, per subchannel.
    slots: [[u32; METHOD_SLOTS]; SUBCHANNELS],
    /// Which slots have been *written*, per subchannel — one bit each.
    ///
    /// ★★ Not a sentinel value, a **presence bitmap**, and the difference is the refusal
    /// posture: `0` is a legal destination offset, a legal line length and a legal set of
    /// flags, so "never written" and "written as zero" have to be distinguishable. A
    /// decoder that could not tell them apart would fire a `CeLaunchDma` with a defaulted
    /// destination, which is the invented answer this whole module exists to refuse.
    written: [u32; SUBCHANNELS],
    /// The engine-object class bound to each subchannel by `SET_OBJECT`, if any.
    object: [Option<ClassId>; SUBCHANNELS],
}

impl Default for MethodState {
    fn default() -> Self {
        MethodState::new()
    }
}

impl MethodState {
    /// A channel whose engine has been reset: nothing bound, nothing latched.
    #[must_use]
    pub const fn new() -> MethodState {
        MethodState {
            slots: [[0; METHOD_SLOTS]; SUBCHANNELS],
            written: [0; SUBCHANNELS],
            object: [None; SUBCHANNELS],
        }
    }

    /// Bind `class` to `subch` — and **clear that subchannel's latched slots**, because a
    /// rebind changes what its method addresses mean. See the type docs.
    ///
    /// Out-of-range `subch`: nothing happens (total).
    pub fn bind_object(&mut self, subch: usize, class: ClassId) {
        if subch >= SUBCHANNELS {
            return;
        }
        self.object[subch] = Some(class);
        self.written[subch] = 0;
        self.slots[subch] = [0; METHOD_SLOTS];
    }

    /// The class currently bound to `subch`, or `None` if nothing has bound one.
    #[must_use]
    pub fn object(&self, subch: usize) -> Option<ClassId> {
        self.object.get(subch).copied().flatten()
    }

    /// ★★★ **May a codec read `subch`'s methods as `class`'s?** The predicate to ask before
    /// decoding a method address, and it has exactly two ways to answer yes.
    ///
    /// # ⚠ THE MEASUREMENT THAT FORCED THIS, and it is UVM's own encoder
    ///
    /// `[src] ogkm-580`: UVM binds the copy engine and issues its copy-engine methods on
    /// **two different subchannels**, deliberately.
    ///
    /// - `uvm_hal_maxwell_ce_init` (`kernel-open/nvidia-uvm/uvm_maxwell_ce.c:29-37`) is
    ///   `NV_PUSH_1U(B06F, SET_OBJECT, ceClass)`, and `NV_PUSH_1U` takes its subchannel
    ///   from `UVM_SUBCHANNEL_ ## class` (`uvm_push_macros.h:223-225`), so this is
    ///   `UVM_SUBCHANNEL_B06F` = `UVM_SUBCHANNEL_HOST` = **0** (`:82, :101`). The function's
    ///   own comment says so: *"Notably this sends SET_OBJECT with the CE class on
    ///   subchannel 0 instead of the recommended by HW subchannel 4."*
    /// - Every CE method — `SET_SEMAPHORE_A/B/PAYLOAD`, `LAUNCH_DMA`, the offsets — goes
    ///   out as `NV_PUSH_nU(C3B5, …)`, i.e. `UVM_SUBCHANNEL_C3B5` = `UVM_SUBCHANNEL_CE` =
    ///   `NVA06F_SUBCHANNEL_COPY_ENGINE` = **4** (`:85, :109`; `cla06fsubch.h:30`).
    ///
    /// ⇒ A codec that required the class on the **same** subchannel refused *every method
    /// UVM ever emits*. RM's own `channel_utils.c` binds and fires on one subchannel, which
    /// is why the CeUtils path worked and hid this completely.
    ///
    /// # ⊘⊘ The fallback is TWO conditions, and the second was added because a property
    /// refuted the first — in the same hour, before any boot
    ///
    /// The wider rule *"any **unbound** subchannel, on a channel that bound `class`
    /// somewhere"* was written first. `tests/tests/pushbuffer_ga10x_hostile.rs`'s
    /// `a_hostile_method_stream_never_fires_a_copy_it_did_not_write` refuted it immediately:
    ///
    /// ```text
    /// subchannel 6: codec said Some(CeLaunchDma { dst: GpuVa(279548103369567), … }),
    ///   the stream wrote None — the accumulator and the guest disagree about what was
    ///   submitted
    /// ```
    ///
    /// A compute or graphics object's method addresses collide with the copy engine's, so
    /// *"unbound"* alone lets a stream that never described a copy fire one. ⇒ the unbound
    /// arm is additionally restricted to the **architecturally fixed** subchannel for that
    /// class (`fixed_subch`; for the copy engine `NVA06F_SUBCHANNEL_COPY_ENGINE = 4`,
    /// `ogkm-580: cla06fsubch.h:30`) — the one subchannel hardware itself assigns, and the
    /// one UVM fires on.
    ///
    /// Both halves of the protection are therefore intact: a subchannel bound to some
    /// *other* class answers `false`, and an unbound subchannel that is not the fixed one
    /// answers `false`. What is inferred is only what the guest already said — it named
    /// `class` on this channel — plus one sourced hardware convention.
    ///
    /// ⊘ Deliberately **not** *"default to the channel's engine kind"*: that would be a fact
    /// from the object model reaching into the arch, and it would fire on a channel whose
    /// engine object was allocated but whose pushbuffer never bound anything — an inference
    /// about hardware we have not measured.
    ///
    /// `fixed_subch` is a parameter rather than a constant because *which* subchannel is
    /// fixed for *which* class is a per-generation encoding, and this type is Axis-B's
    /// generation-free half.
    #[must_use]
    pub fn subchannel_speaks(&self, subch: usize, class: ClassId, fixed_subch: usize) -> bool {
        match self.object(subch) {
            Some(bound) => bound == class,
            // Unbound: the fixed subchannel for this class, AND the guest named the class
            // on this channel. Neither condition alone is enough — see the docs above.
            None => {
                subch < SUBCHANNELS && subch == fixed_subch && self.object.contains(&Some(class))
            }
        }
    }

    /// Latch `value` into `slot` of `subch`. Out of range: nothing happens (total).
    pub fn latch(&mut self, subch: usize, slot: usize, value: u32) {
        if subch >= SUBCHANNELS || slot >= METHOD_SLOTS {
            return;
        }
        self.slots[subch][slot] = value;
        self.written[subch] |= 1 << slot;
    }

    /// The value latched in `slot` of `subch`, or `None` if nothing ever wrote it.
    #[must_use]
    pub fn latched(&self, subch: usize, slot: usize) -> Option<u32> {
        if subch >= SUBCHANNELS || slot >= METHOD_SLOTS {
            return None;
        }
        (self.written[subch] & (1 << slot) != 0).then(|| self.slots[subch][slot])
    }

    /// Reset the whole engine — every subchannel, every slot, every binding.
    pub fn reset(&mut self) {
        *self = MethodState::new();
    }
}

/// Axis-B seam: the pushbuffer / method + engine encodings for one GPU generation
/// (`execution_plane.md` §3.1). The *decode logic* (walk GPFIFO → walk methods →
/// dispatch on [`PushMethod`]) is **core**; this trait supplies only the per-arch
/// *encodings* (how a raw method word decodes, which method ID is `SEM_RELEASE`, the
/// GPFIFO entry stride/format).
///
/// `Send + Sync`: a method codec is immutable shared data (crate docs, decision #17).
pub trait PushbufferAbi: Send + Sync {
    /// Number of 32-bit **argument** words that follow a method `header` word (the
    /// method's count field). The core parser uses this to advance the method stream;
    /// the *meaning* of the args is [`PushbufferAbi::decode_method`]'s job. A header
    /// this arch cannot size returns `0` (the core then advances past just the header,
    /// so a hostile stream cannot desync the parser into an unbounded read).
    fn method_len(&self, header: u32) -> usize;

    /// Decode one method word (`header` + its trailing `args`) into core terms.
    /// Anything this arch does not model → [`PushMethod::Opaque`] (never guessed).
    fn decode_method(&self, header: u32, args: &[u32]) -> PushMethod;

    /// The GPFIFO entries of a pushbuffer `ring` (entry stride/format per arch),
    /// each pointing at a [`PushRange`] of method words to walk.
    fn gpfifo_entries(&self, ring: &[u8]) -> Vec<PushRange>;

    /// ★★★ **Decode a RUN of methods — the protocol's own unit** (`E5`, the owner's
    /// ruling at `execution_plane_increments.md` §8.2.1).
    ///
    /// `state` is the channel's engine method state, carried **in** and updated **in
    /// place**: an architecture whose engine assembles an operation out of several method
    /// runs latches the operands here and fires when the run that fires it arrives. See
    /// [`MethodState`] for why the state exists, and for the bound that makes it safe.
    ///
    /// ## The default is the old behaviour, exactly
    ///
    /// A per-method map onto [`PushbufferAbi::decode_method`], ignoring `state`. So an
    /// architecture that models no multi-run operation — every mock, and every generation
    /// this port has not built a codec for — is unchanged by this method existing, which
    /// is the whole reason it is a *provided* method and not a required one.
    ///
    /// ## ⊘ What an implementation may NOT do
    ///
    /// Fire an operation whose operands were never latched. An incomplete run is
    /// **un-fired state awaiting more methods**, and must decode to
    /// [`PushMethod::Opaque`] — never to a modelled fact with defaulted fields. That is
    /// `E4`'s posture unchanged (*"a plausible answer is worse than a refusal"*), and it
    /// is the property the seam is most likely to lose: the operands are *right there* in
    /// a struct with a `Default`.
    fn decode_run(&self, state: &mut MethodState, run: &[(u32, Vec<u32>)]) -> Vec<PushMethod> {
        let _ = state;
        run.iter()
            .map(|(header, args)| self.decode_method(*header, args))
            .collect()
    }
}

/// # The Axis-B architecture trait — one impl per GPU generation
///
/// Everything a GPU generation *behaves like*, in terms the core understands
/// (`mode2_abi_agnostic_layer.md` §4.2). The pure-logic crates
/// (`kayfabe-core`/`-mmu`/`-fwd`/`-completion`) program **only** against this trait —
/// no `if arch == X` exists anywhere in a logic crate (enforced by review + the
/// grep gate, testing strategy §7 Tier 2).
///
/// Object-safe: the composition root holds `Box<dyn Arch>` selected once at device
/// realize (`mode2_abi_agnostic_layer.md` §4.3). `Send + Sync` supertrait: that box
/// lives inside the `Gpu` that vCPU threads share, and an arch is an immutable
/// encoding table — every method takes `&self` (crate docs, decision #17).
///
/// **A real implementer's checklist** (`impl Arch for Ampere`, zero core edits):
/// classify the generation's class-ID set (sourced from the Axis-A codegen tables in
/// `kayfabe-abi`), decode the work-submit token and channel USERD flags, and supply the
/// generation's `GmmuFmt`/`UserdModel`. That is the whole surface.
pub trait Arch: Send + Sync {
    /// Human-readable generation name (trace/diagnostics only — the core must
    /// never branch on it).
    fn name(&self) -> &'static str;

    /// Class-ID recognition (B9): map this generation's class IDs to the
    /// arch-invariant graph vocabulary. Unknown classes return
    /// [`ObjectKind::Unknown`] — recorded, never guessed at.
    fn classify(&self, class: ClassId) -> ObjectKind;

    /// Recover a channel's virtual channel ID ([`VChid`]) from the opaque
    /// USERD/flags word its alloc params declared (the open-driver
    /// `kernel_channel.c` USERD_INDEX recovery, generalized as a seam).
    ///
    /// ## ★★ `None` means "these flags name no channel", and it is RM's own answer
    ///
    /// The GA10x reader has **three** shapes and only one of them yields a chid
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:456-476`):
    /// `_PAGE_FIXED` false leaves the chid to the allocator, and `_PAGE_FIXED` with
    /// `_FIXED` both true is refused by RM with `NV_ERR_INVALID_STATE`. A bare [`VChid`]
    /// return had to answer *something* for those two, and boundary-1 posture is that
    /// guest bytes are hostile — a guest can set those bits, so inventing a channel number
    /// for a word that names none is the same silent mis-route [`Arch::decode_doorbell`]
    /// returns `Option` to avoid. The caller turns it into a named refusal
    /// (`kayfabe_core::ProjectionError::UnnamedVchid`).
    fn vchid_from_userd_flags(&self, flags: u32) -> Option<VChid>;

    /// Decode a doorbell/work-submit token written to the usermode region (B6).
    /// Returns `None` for a token this generation considers malformed — the
    /// caller faults loudly (boundary-1 posture: guest bytes are hostile).
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget>;

    /// The MMU format regime for this generation (B1–B4).
    fn mmu(&self) -> &dyn GmmuFmt;

    /// The USERD geometry for this generation (B5).
    fn userd(&self) -> &dyn UserdModel;

    /// Which engine (if any) an *object* class denotes — the §2.1 [`EngineKind`]
    /// mapping (a compute/graphics/CE/NVENC object makes its channel that kind of
    /// context). `None` for a class that is not an engine object.
    ///
    /// Provided: derived from [`Arch::classify`] — there is ONE class table per
    /// arch, so the engine mapping cannot drift from the graph's classification.
    fn engine_of_object(&self, class: ClassId) -> Option<EngineKind> {
        match self.classify(class) {
            ObjectKind::EngineObject { engine } => Some(engine),
            _ => None,
        }
    }

    /// Is this control a **Case-2** GSP-internal / ROUTE_TO_PHYSICAL control with no
    /// unprivileged userspace equivalent (`PROMOTE_CTX`, `GET_CTX_BUFFER_INFO`, …)?
    /// Its effect is already achieved by Case-1 forwarding, so the core ACKs it and
    /// does nothing on the host (`execution_plane.md` §2.5). Replaying one on an
    /// unprivileged isolate is a "wrong layer" error, never a privilege gain.
    fn is_case2_control(&self, cmd: ControlCmd) -> bool;

    /// The pushbuffer / method ABI for this generation (the ONE parser's encodings).
    fn pushbuffer(&self) -> &dyn PushbufferAbi;

    /// The GSP register model for this generation ([`gsp`], `mode2_gsp_port_plan.md`
    /// §3.5), or `None` for an architecture that has not modelled one.
    ///
    /// ★ **`None` is not a fallback.** The faked-GSP FSM refuses with a named fault when
    /// it gets one, exactly as `MISS = FAULT` requires — there is no default register
    /// model to silently serve zeros. The provided `None` exists so that adding this
    /// seam did not invalidate every existing `impl Arch` (`MockArch` and, later, the
    /// arch-impl crates that do not drive a GSP at all); an architecture that boots a
    /// GSP overrides it.
    fn gsp(&self) -> Option<&dyn GspModel> {
        None
    }

    /// The class ids this generation's **host forwarding path** allocates on a real host
    /// GPU ([`HostClasses`]), or `None` for an architecture with no host-forwarding
    /// profile.
    ///
    /// ★ **`None` is a refusal, exactly as [`Arch::gsp`]'s is.** An arch that has never
    /// been driven against host silicon says so, and the host adapter fails by name
    /// rather than falling back to somebody else's class ids. `MockArch` returns `None`
    /// on purpose: three invented class numbers are precisely the residue this seam
    /// exists to stop, and a mock that answered would let a wrong id reach a real
    /// `NV_ESC_RM_ALLOC` while every test stayed green.
    fn host_classes(&self) -> Option<&dyn HostClasses> {
        None
    }
}

/// The GPFIFO **channel** class, tagged with its role.
///
/// # ★★★ The ROLE is a TYPE, not a convention (`#166`)
///
/// This type and its two siblings ([`UsermodeClass`], [`CeObjectClass`]) exist for one
/// reason, and it is the reason [`HostClasses`] exists at all, one level down.
///
/// `#156` pinned the three host class *values* nine ways. It pinned **which role each
/// call site asks for** zero ways, and a bite campaign measured exactly that: swap
/// `classes.usermode()` for `classes.gpfifo_channel()` at the doorbell-window allocation
/// and **0 of 3** such swaps turned anything red. Both sides of every swap were a bare
/// [`ids::ClassId`], so nothing — not rustc, not a test — could tell *"the number is
/// right"* from *"the number is right **for this hole**"*.
///
/// That is the worst shape a defect can have here, because [`HostClasses`]' own
/// §*Why the wrong answer is SILENT* is about these same three ids: a Hopper host
/// **serves** two of the three wrong picks. A wrong role would have been accepted by a
/// real board with no error, no Xid and no diagnostic.
///
/// So the three roles are three distinct types. A call site that asks for the wrong one
/// **no longer compiles**, which makes the check total: rustc quantifies over every call
/// site in the workspace, so there is no list to keep current and a call site added
/// tomorrow is checked tomorrow. (`gates_quantified_over_a_list`: *derive the universe
/// instead of listing it* — rustc is the strongest derivation available.)
///
/// ⊘ **What this does NOT do**, stated so nothing reads it as more. It refuses a role
/// swap *at a call site*. It does not stop a body that already holds a
/// `&dyn HostClasses` from unwrapping a different role by hand — [`Self::channel_id`]
/// and its siblings exist, and must, because a `u32` has to reach the
/// `NV_ESC_RM_ALLOC` eventually. Two things contain that residue: the unwraps are
/// deliberately **role-named rather than uniform** (there is no `.id()`, no `Deref`, no
/// `From<ChannelClass> for ClassId`), so writing one spells the role out loud; and
/// `tests/tests/host_class_role_wiring.rs` pins the complete **derived** set of unwrap
/// sites in the tree, so a new one is a red test rather than a silent widening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelClass(ClassId);

impl ChannelClass {
    /// Tag a class id as the channel role.
    #[must_use]
    pub const fn new(id: ClassId) -> Self {
        Self(id)
    }

    /// Untag — **the one escape**, and it names the role so that using it for anything
    /// else is a lie a reader can see.
    #[must_use]
    pub const fn channel_id(self) -> ClassId {
        self.0
    }
}

/// The **usermode** class whose 64 KiB CPU mapping is the doorbell window, tagged with
/// its role — see [`ChannelClass`] for why the role is a type (`#166`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UsermodeClass(ClassId);

impl UsermodeClass {
    /// Tag a class id as the usermode role.
    #[must_use]
    pub const fn new(id: ClassId) -> Self {
        Self(id)
    }

    /// Untag — see [`ChannelClass::channel_id`].
    #[must_use]
    pub const fn usermode_id(self) -> ClassId {
        self.0
    }
}

/// The copy-engine **object** class, tagged with its role — see [`ChannelClass`] for why
/// the role is a type (`#166`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CeObjectClass(ClassId);

impl CeObjectClass {
    /// Tag a class id as the CE-object role.
    #[must_use]
    pub const fn new(id: ClassId) -> Self {
        Self(id)
    }

    /// Untag — see [`ChannelClass::channel_id`].
    #[must_use]
    pub const fn ce_object_id(self) -> ClassId {
        self.0
    }
}

/// # `HostClasses` — the class ids the HOST forwarding path allocates
///
/// Three NVIDIA class ids that the unprivileged host isolate passes to a real
/// `NV_ESC_RM_ALLOC` on a real board, and that **change with the host GPU's
/// generation**. Everything else the isolate allocates does not (see below).
///
/// ## Why this is a sub-trait and not three more methods on [`Arch`]
///
/// Because it answers a different question about a different GPU. Every existing [`Arch`]
/// method describes the *guest's emulated* GPU — how to classify a class id the guest
/// sent us, how to decode a doorbell token the guest wrote, what the guest's PTEs look
/// like. These three describe what **we** must say to the *host's* driver when we forward
/// that intent. The two GPUs are the same generation only by coincidence of a bench.
///
/// Splitting it buys three things a flat trait cannot:
///
/// 1. **A refusal that costs nothing.** `Option<&dyn HostClasses>` lets an arch with no
///    host profile decline. Flat methods would force `MockArch` and every future
///    fixture arch to return *some* class id, and there is no honest one to return.
/// 2. **One grep for the whole host-class axis.** `impl HostClasses` is the complete
///    list of places a host class id is decided; `rg 'fn ce_object'` finds every one.
/// 3. **It does not grow [`Arch`]'s object-safe surface for consumers who never forward.**
///    `kayfabe-core`/`-mmu`/`-fwd` hold `dyn Arch` and issue no host ioctls at all.
///
/// ## ⊘ What is deliberately NOT here, and why the set is exactly three
///
/// The rule is **what the name MEANS, not whether it contains a generation word**.
/// Measured against NVIDIA's own per-chip tables in `ogkm-580`'s
/// `src/nvidia/generated/g_gpu_class_list.c`:
///
/// | class | GA106 | AD106 | GH100 | verdict |
/// |---|---|---|---|---|
/// | `FERMI_VASPACE_A` | `:1124` | `:1748` | `:2001` | **invariant** — permanent name |
/// | `FERMI_CONTEXT_SHARE_A` | `:1122` | `:1746` | `:1999` | **invariant** |
/// | `KEPLER_CHANNEL_GROUP_A` | `:1134` | `:1758` | `:2031` | **invariant** |
/// | channel / usermode / CE object | Ampere ids | Ampere ids | **Hopper ids** | **varies** |
///
/// The first three carry a generation word in the name and are the *current* class on
/// every part from Fermi/Kepler to Blackwell. Putting them behind a seam would double
/// the surface to describe zero variation — the "counted address space, not risk" error.
///
/// Two more host-path values were checked for variation and found **invariant**, so they
/// are not seams either:
///
/// - the doorbell register offset and window size. `clc461/561/661/761.h` each define a
///   class id and nothing else; every USERMODE class from Volta to Blackwell inherits
///   `NVC361_NOTIFY_CHANNEL_PENDING` = `0x90` and `NVC361_NV_USERMODE__SIZE` = 65536
///   (`ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:29-33`).
/// - every copy-engine method the host CE path emits. `clc8b5.h` is a delta header and
///   redefines none of `OFFSET_IN/OUT_*`, `LINE_LENGTH_IN`, `LINE_COUNT`,
///   `SET_SEMAPHORE_*` or `LAUNCH_DMA` to a different offset than `clc7b5.h`.
///
/// ## ★★ Why the wrong answer is SILENT, which is what makes this a seam
///
/// A Hopper host's class list contains `AMPERE_CHANNEL_GPFIFO_A` **and**
/// `AMPERE_USERMODE_A` (`g_gpu_class_list.c:1996`, `:1997`), and RM has a live
/// `CliGetChannelClassInfo` arm for the former (`kernel_channel.c:1588-1594`). So two of
/// the three wrong picks are **served, not refused** — the channel comes back with
/// `NVC56F` notifier geometry on a part that wants `NVC86F`. Only `AMPERE_DMA_COPY_B` is
/// absent from GH100 and fails loudly. A silent mis-route on a forwarding path is exactly
/// the failure mode this port spends its effort making loud.
///
/// ## The rule NVIDIA's own client uses, and what this trait is relative to it
///
/// RM's own UVM-facing client does not hardcode per-chip either: `findDeviceClasses`
/// reads `NV0080_CTRL_CMD_GPU_GET_CLASSLIST` off the live device and takes the
/// **numerically largest** member of each family — `isClassHost`, `isClassCE`,
/// `isClassCompute` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8630-8699`,
/// families at `:8543-8601`). An implementation of this trait is a **compile-time
/// statement of what that runtime query would return** for one generation. It is
/// therefore checkable without a GPU — and it is *not* a substitute for the query. See
/// the crate-level note in `kayfabe-chips` for the increment that would replace it.
pub trait HostClasses: Send + Sync + core::fmt::Debug {
    /// Human-readable profile name (diagnostics only — nothing may branch on it).
    fn name(&self) -> &'static str;

    /// The GPFIFO **channel** class: the object a host channel is allocated as, under a
    /// TSG. There is exactly one per generation — a GR channel and a CE channel are the
    /// same class and differ only by `NV_CHANNEL_ALLOC_PARAMS.engineType`.
    fn gpfifo_channel(&self) -> ChannelClass;

    /// The **usermode** class whose 64 KiB CPU mapping is the doorbell window, allocated
    /// under the subdevice.
    fn usermode(&self) -> UsermodeClass;

    /// The copy-engine **object** class, allocated under a CE channel — and the same
    /// number the pushbuffer's `SET_OBJECT` carries in its `NVCLASS` field.
    fn ce_object(&self) -> CeObjectClass;
}

// The concurrency contract, compile-time-asserted (decision #17): every public type
// — and every trait-object seam the core stores or returns — is `Send + Sync`.
kayfabe_util::assert_send_sync!(
    ids::HClient,
    ids::HObject,
    ids::Pdb,
    ids::VChid,
    ids::RunlistId,
    ids::ClassId,
    ids::GpuVa,
    ids::Gpa,
    ids::GpuId,
    ids::ControlCmd,
    ids::EngineKind,
    ChannelClass,
    UsermodeClass,
    CeObjectClass,
    ObjectKind,
    DoorbellTarget,
    GmmuVersion,
    PageSize,
    Aperture,
    FbWindow,
    PteDecode,
    PdeEdge,
    LevelShift,
    PushMethod,
    CeWork,
    PhysTarget,
    CpuPlane,
    Backing,
    Residency,
    PushRange,
    MethodState,
    dyn Arch,
    dyn ResidencyOracle,
    dyn HostClasses,
    dyn GmmuFmt,
    dyn UserdModel,
    dyn PushbufferAbi,
);
