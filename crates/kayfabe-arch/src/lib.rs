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

pub mod gsp;
pub mod ids;

pub use gsp::{GspModel, GspObservation, GspReg, LibosRegionLayout};
use ids::{ClassId, ControlCmd, EngineKind, GpuVa, Pdb, VChid};

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
        /// ★★ **Which level the pointed-at table is** — supplied by the format, never
        /// assumed to be `level + 1` by the walker.
        ///
        /// It is `level + 1` for most edges and it is **not** universally: in the
        /// measured regime the deepest directory's slot names a small-page table *and* a
        /// big-page table, which are two different rows of
        /// [`GmmuFmt::level_shift`]'s table with different strides and different entry
        /// counts (`C: nvkvm_gpu_emul.c:8706-8708`, rows 4 and 5). A walker that
        /// incremented would decode a 32-entry table as a 512-entry one and read 3 840
        /// bytes past its end.
        child_level: u8,
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
    Fill {
        /// The 32-bit pattern, repeated across the destination. Its phase is taken from
        /// the DESTINATION ADDRESS (component `n` of a 4-byte-aligned word), never from
        /// the start of a sub-copy — which is what makes splitting a fill at an
        /// unaligned offset produce the same bytes as filling the range whole.
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
        /// What the engine is being asked to do.
        work: CeWork,
    },
    /// `SEM_RELEASE` / `SET_SEMAPHORE_A/B` + payload / finishPayload: the completion —
    /// a semaphore address in a VAS advanced to `payload`. Extracted for the
    /// completion plane's observe (`execution_plane.md` §2.4).
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

/// One pushbuffer range to walk: a contiguous run of method words a GPFIFO entry
/// points at. The core walks these; the arch iterates the GPFIFO to produce them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushRange {
    /// Guest-physical (or shared) address of the method words.
    pub gpa: u64,
    /// Length of the range in bytes.
    pub len: u64,
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
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid;

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
}

// The concurrency contract, compile-time-asserted (decision #17): every public type
// — and every trait-object seam the core stores or returns — is `Send + Sync`.
kayfabe_util::assert_send_sync!(
    ids::HClient,
    ids::HObject,
    ids::Pdb,
    ids::VChid,
    ids::ClassId,
    ids::GpuVa,
    ids::Gpa,
    ids::GpuId,
    ids::ControlCmd,
    ids::EngineKind,
    ObjectKind,
    DoorbellTarget,
    GmmuVersion,
    PageSize,
    Aperture,
    PteDecode,
    LevelShift,
    PushMethod,
    CeWork,
    PushRange,
    dyn Arch,
    dyn GmmuFmt,
    dyn UserdModel,
    dyn PushbufferAbi,
);
