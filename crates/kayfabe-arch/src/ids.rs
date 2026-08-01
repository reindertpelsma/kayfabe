//! Domain identity newtypes — the vocabulary every logic crate shares.
//!
//! Rule L7 (`mode2_rust_rewrite_architecture.md` Part 2): *key every table by the
//! identity the hardware uses* — PDB for address spaces, vChid for channels — never by
//! a driver-visible handle that can be reused, shared, or absent. The newtypes below
//! make "which identity keys this table" a compile-time property: a `HashMap<Pdb, _>`
//! cannot be accidentally indexed by a client handle.
//!
//! All values are abstract `u32`/`u64` wrappers; nothing here encodes an NVIDIA
//! layout or constant.

macro_rules! id_newtype {
    ($(#[$doc:meta])* $name:ident($inner:ty)) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $inner);

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($name), "({:#x})"), self.0)
            }
        }
    };
}

id_newtype!(
    /// An RM client handle (`hClient`): a **handle namespace + access rights**.
    /// Explicitly NOT a process key — values are reused across guest processes and a
    /// process may hold several clients. Grouping into a `Proc` is a projection of the
    /// RM graph's DUP edges **between declared user clients**, never of this value:
    /// ★ §12.27, measured — the one UVM session client's handle (`0xc1d00069`) sits
    /// numerically *between* two guest processes' clients (`0xc1d00067`/`0xc1d00068`),
    /// so no range test and no ordering of this value can tell them apart.
    HClient(u32)
);

id_newtype!(
    /// An RM object handle, scoped to one client's namespace. Two processes routinely
    /// present *identical* `HObject` values (#14 round 1: both GR channels were
    /// `0x5c000019`), so an `HObject` is meaningless without its owning [`HClient`] —
    /// see `kayfabe_core`'s `NodeKey`.
    HObject(u32)
);

id_newtype!(
    /// A page-directory base — "the GPU's CR3". THE data-plane identity: the GMMU keys
    /// page tables by PDB, so the address table keys by PDB (per-`Vas`), and #14's
    /// identical-VA collision is impossible across distinct PDBs by construction.
    Pdb(u64)
);

id_newtype!(
    /// A virtual channel ID, recovered from channel-alloc flags / doorbell tokens.
    /// THE exec-plane identity (experiment E0: one vChid per channel, zero collisions).
    VChid(u16)
);

id_newtype!(
    /// A **runlist** id — the *other half* of a GA10x work-submit token, and the half a
    /// `{ vchid }`-only [`crate::DoorbellTarget`] used to drop on the floor.
    ///
    /// Seven bits wide on GA10x (`NV_CTRL_VF_DOORBELL_RUNLIST_ID 22:16`,
    /// `ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_ctrl.h:27`), which is
    /// RM's own encoder's number and not a reading of it — see
    /// `tests/tests/worksubmit_token_oracle.rs`.
    ///
    /// ## ⊘ A correction, kept because it is the more useful half of the record
    ///
    /// This newtype was introduced with a doc comment asserting that
    /// `docs/reference/bench_evidence/rm-ladder-419afe8.out:21-25` **measured** five
    /// copy-engine channels holding chid 7 on four different runlists, and therefore that
    /// `(GpuId, VChid)` could not be a channel identity. That reading was wrong: those
    /// five channels were allocated and freed **one at a time**, so RM handed the same
    /// recycled chid back each time. The archive shows serial reuse, not simultaneity, and
    /// nothing in it distinguishes the two.
    ///
    /// The census taken to settle it holds the channels **at once**
    /// (`doorbell-census-ba74151.out`) and measures the opposite:
    /// `per_runlist_channel_ram=0`, six live channels across four runlists taking six
    /// **distinct** chids from one global heap. On GA106 a chid *is* device-unique, so
    /// `(GpuId, VChid)` is adequate here — and would not be on a part where that flag is
    /// 1. `doorbell_token_encoding.md` §4 carries the scope.
    RunlistId(u16)
);

id_newtype!(
    /// An RM class ID. The *values* are per-generation/per-version (Axis A, codegen'd
    /// in `kayfabe-abi`); the core only ever passes them to `Arch::classify`.
    ClassId(u32)
);

id_newtype!(
    /// A guest GPU virtual address. Kept distinct from guest-physical ([`Gpa`]) and
    /// host addresses so a translation step can never be skipped silently.
    GpuVa(u64)
);

id_newtype!(
    /// A guest-physical address (GPA).
    Gpa(u64)
);

id_newtype!(
    /// A **routable GPU target** — the multi-GPU axis (`multi_gpu_and_mig.md`).
    ///
    /// Derived from a `Device`'s declared `deviceInstance` (a protocol fact), never
    /// guessed. Deliberately a *target*, not "physical device node": a future MIG
    /// partition-target is another value of this same identity (the accommodation),
    /// with zero re-keying.
    ///
    /// ★ `Pdb` and `VChid` are **per-GPU namespaces** (a PDB is a per-GPU FB address,
    /// a vChid a per-GPU runlist index): two GPUs legally present identical values,
    /// so every routing table keys on `(GpuId, Pdb)` / `(GpuId, VChid)` — the #14
    /// disjoint-by-key-construction lesson lifted onto the GPU axis.
    GpuId(u32)
);

impl GpuId {
    /// The single-target default (the N=1 device realized via `Gpu::new`).
    pub const ZERO: GpuId = GpuId(0);
}

id_newtype!(
    /// An RM control-command identifier (`GSP_RM_CONTROL` cmd). Values are per-version
    /// (Axis A, codegen'd in `kayfabe-abi`); the core only ever passes them to
    /// [`crate::Arch::is_case2_control`] and the host backend. Lives here (not in
    /// `kayfabe-isolate`) so the `Arch` seam can name it without a dependency cycle.
    ControlCmd(u32)
);

/// The execution-plane routing tag for a channel or engine object
/// (`execution_plane.md` §2.1) — THE one engine vocabulary of the core.
///
/// A **routing tag, not a `dyn Engine`**: the engines do not have divergent *core*
/// behavior — each gets a Case-1 alloc forwarded, its pushbuffer decoded by the same
/// loop, and signals via its completion arm. Their differences are entirely
/// *encodings* (class IDs, method IDs, sema offsets), which live behind the
/// [`crate::Arch`] seams. So the core programs against this small enum; a new engine
/// for an existing arch is a new arm + the arch's class-ID/method rows, **zero core
/// edits**. (The coarse `EngineClass{Gr,Ce,Other}` this replaced could not tell
/// NVENC from GR-compute at the `Channel` — routing and completion-arm selection
/// key on THIS enum, at the channel, not just at parse.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    /// GR engine running a compute context (the CUDA/LLM/PyTorch path, incl. the
    /// Tensor-Core path — a path *within* GR, not a separate engine).
    GrCompute,
    /// GR engine running a graphics context (raster; its scanout routes to `Present`).
    GrGraphics,
    /// Copy engine (CE) — the copy IS the workload; also the #13 PT-write data plane.
    Ce,
    /// Video encode engine + session (NVENC).
    NvEnc,
    /// Video decode engine (NVDEC) — an honest gap, unproven; named for completeness.
    NvDec,
    /// An engine the core routes but does not interpret.
    Other,
}
