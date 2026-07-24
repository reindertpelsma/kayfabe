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
    /// process holds several clients (compute + UVM). Grouping into a `Proc` is a
    /// projection of the RM graph's DUP edges, never of this value.
    HClient(u32)
);

id_newtype!(
    /// An RM object handle, scoped to one client's namespace. Two processes routinely
    /// present *identical* `HObject` values (#14 round 1: both GR channels were
    /// `0x5c000019`), so an `HObject` is meaningless without its owning [`HClient`] —
    /// see `nvkvm_core`'s `NodeKey`.
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
    /// An RM class ID. The *values* are per-generation/per-version (Axis A, codegen'd
    /// in `nvkvm-abi`); the core only ever passes them to `Arch::classify`.
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
    /// An RM control-command identifier (`GSP_RM_CONTROL` cmd). Values are per-version
    /// (Axis A, codegen'd in `nvkvm-abi`); the core only ever passes them to
    /// [`crate::Arch::is_case2_control`] and the host backend. Lives here (not in
    /// `nvkvm-isolate`) so the `Arch` seam can name it without a dependency cycle.
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
