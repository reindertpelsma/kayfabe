//! # nvkvm-arch — abstract GPU vocabulary + the Axis-B `Arch` trait seams
//!
//! This crate is two things, deliberately together at the bottom of the GPU-domain
//! dependency graph (below `nvkvm-core`, so core/mmu/fwd/completion share one set of
//! newtypes without cycles):
//!
//! 1. **The vocabulary** ([`ids`]): newtypes for every identity the system routes on —
//!    [`ids::HClient`], [`ids::HObject`], [`ids::Pdb`], [`ids::VChid`], [`ids::ClassId`], …
//!    These are *abstract*: no NVIDIA struct layout, no driver-version value, appears
//!    anywhere in this crate (that is Axis A, quarantined to `nvkvm-abi` — design
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
//! with **zero edits to `nvkvm-core`/`-mmu`/`-fwd`/`-completion`**. The core owns
//! *algorithms* (graph derivation, demux loops, walk logic, completion policy); an
//! `Arch` impl owns *encodings* (which bits mean what). Code that knows both is
//! misplaced (decision #1). The `MockArch` in `nvkvm-mocks` is the first `impl` and
//! the proof of the seam: it uses deliberately fake encodings so any core code that
//! secretly assumes a real NVIDIA encoding fails the mock-driven tests.

pub mod ids;

use ids::{ClassId, EngineClass, VChid};

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
        /// Engine class the channel targets (GR/CE/…).
        engine: EngineClass,
    },
    /// An engine object allocated on a channel (compute class, DMA-copy class, …).
    EngineObject {
        /// Engine class the object targets.
        engine: EngineClass,
    },
    /// Memory / semaphore / event / anything else the graph tracks as a plain node.
    Other,
    /// Not a class this architecture knows. The graph records it as [`ObjectKind::Other`]
    /// but callers on stricter paths must treat it as a loud fault, never guess.
    Unknown,
}

/// Decoded doorbell write (Axis-B seam B6: `token bits → target`).
///
/// E0 (proven on the bench, 2026-07-19): the work-submit token identifies the target
/// channel; the core demuxes on the decoded [`VChid`] — no CPU-state (CR3) read exists
/// anywhere in the design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellTarget {
    /// The virtual channel ID the token addresses.
    pub vchid: VChid,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// A decoded page-table entry, in core terms (no raw bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PteDecode {
    /// Invalid / not present.
    Invalid,
    /// A pointer to a next-level page directory/table.
    Pde {
        /// Physical address of the next level.
        next: u64,
        /// Aperture the next level lives in.
        aperture: Aperture,
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

/// Axis-B core: the GMMU format codec + level geometry for one MMU regime.
///
/// The **walk algorithm** lives in `nvkvm-mmu` (core logic, regime-independent);
/// this trait supplies only the per-regime *format*: how many levels, how an entry
/// decodes, which leaf sizes exist. ~two heavy impls (VER2/VER3) with thin per-gen
/// deltas (`mode2_abi_agnostic_layer.md` §4.2). #13 lives or dies here.
pub trait GmmuFmt {
    /// The format regime.
    fn version(&self) -> GmmuVersion;
    /// Every real leaf page size, ascending. MUST be exhaustive (see [`PageSize`]).
    fn page_sizes(&self) -> &[PageSize];
    /// Size in bytes of one entry at directory level `level` (0 = root).
    fn entry_size(&self, level: u8) -> u8;
    /// Number of levels in a full walk (root..leaf) for this regime.
    fn levels(&self) -> u8;
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
pub trait UserdModel {
    /// Total USERD size in bytes for one channel.
    fn userd_size(&self) -> u64;
    /// Byte offset of the GP_GET field (host progress pointer) within USERD.
    fn gp_get_offset(&self) -> u64;
    /// Byte offset of the GP_PUT field (guest submit pointer) within USERD.
    fn gp_put_offset(&self) -> u64;
}

/// # The Axis-B architecture trait — one impl per GPU generation
///
/// Everything a GPU generation *behaves like*, in terms the core understands
/// (`mode2_abi_agnostic_layer.md` §4.2). The pure-logic crates
/// (`nvkvm-core`/`-mmu`/`-fwd`/`-completion`) program **only** against this trait —
/// no `if arch == X` exists anywhere in a logic crate (enforced by review + the
/// grep gate, testing strategy §7 Tier 2).
///
/// Object-safe: the composition root holds `Box<dyn Arch>` selected once at device
/// realize (`mode2_abi_agnostic_layer.md` §4.3).
///
/// **A real implementer's checklist** (`impl Arch for Ampere`, zero core edits):
/// classify the generation's class-ID set (sourced from the Axis-A codegen tables in
/// `nvkvm-abi`), decode the work-submit token and channel USERD flags, and supply the
/// generation's `GmmuFmt`/`UserdModel`. That is the whole surface.
pub trait Arch {
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
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid;

    /// Decode a doorbell/work-submit token written to the usermode region (B6).
    /// Returns `None` for a token this generation considers malformed — the
    /// caller faults loudly (boundary-1 posture: guest bytes are hostile).
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget>;

    /// The MMU format regime for this generation (B1–B4).
    fn mmu(&self) -> &dyn GmmuFmt;

    /// The USERD geometry for this generation (B5).
    fn userd(&self) -> &dyn UserdModel;
}
