//! # kayfabe-tests — conformance harness helpers
//!
//! Shared scenario builders for the integration tests under `tests/`. The whole
//! suite runs at unit speed with **no GPU, no hypervisor, no OS** — the payoff of
//! the pure-state-machine core (`mode2_rust_testing_strategy.md` §1).
//!
//! The centerpiece is [`Scenario`]: a small DSL for scripting a guest's RM protocol
//! as abstract `RmEvent`s, with helpers to build the exact shapes the design docs
//! name (a compute process, a UVM dup, two processes with identical VAs/handles).

pub mod teardown;

pub use teardown::{Guarded, ResidueClaim, TeardownView, audit_teardown, unpublish_and_release};

use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use kayfabe_mocks::{MockArch, mock_classes as mc};
use std::sync::OnceLock;

/// ★ §12.27 — the alloc facts of a **user** client root: it declares a guest process
/// id, so it groups with other user clients through `DUP_OBJECT` (decision #14).
///
/// There is deliberately no `AllocFacts::default()` shortcut for a client root: the
/// declared [`ClientKind`] is **required** (`RmGraphError::UndeclaredClientKind`), for
/// the same reason `Scenario::compute_process_on_gpu` never emits an un-instanced
/// `Device` — a real one cannot exist, so modelling one models a shape the protocol does
/// not produce. The pid is derived from the client handle only so scripted scenarios get
/// distinct, stable values; nothing groups on it.
#[must_use]
pub fn user_client(client: HClient) -> AllocFacts {
    AllocFacts {
        client_kind: Some(ClientKind::User { pid: client.0 }),
        ..Default::default()
    }
}

/// ★ §12.27 — the alloc facts of a **kernel** client root (UVM's one session client,
/// RM's internal clients): `processID == KERNEL_PID`. A dup INTO one of these is a
/// reference, never a merge, and the client itself belongs to the system component.
#[must_use]
pub fn kernel_client() -> AllocFacts {
    AllocFacts {
        client_kind: Some(ClientKind::Kernel),
        ..Default::default()
    }
}

/// # Slow-test gate — `KAYFABE_SLOW=1`
///
/// ONE environment variable gates the suite's slow tests. Membership was
/// *measured*, not guessed (2026-07-25, debug, 16-way box): the pushbuffer
/// proptest fuzz (73 s) and the 16-thread stress soak (≈20 s) were the only
/// tests over ~3 s — together ~85% of the suite's wall clock — so they are the
/// only two gated. (The formerly-`#[ignore]`d 20k-token soak measured ~0.6 s
/// and now simply always runs.)
///
/// ★ Two more joined them with G10 (`l1_concurrency.md` §12.22): the
/// `g10_*_is_capped_*` pair drives a device-global list to
/// `MAX_CONDEMNED_COMPONENTS` / `MAX_RETIRED_PROCS`, which is ~2 s each and is
/// exactly the "walk a guest-reachable bound to its cap" shape this gate exists
/// for. Measured 2026-07-25: they were 3.7 s of the fast path's 23.5 s.
///
/// ```sh
/// cargo test --workspace                    # fast path: slow tests skip, loudly
/// KAYFABE_SLOW=1 cargo test --workspace     # everything runs
/// ```
///
/// Resolution: `KAYFABE_SLOW` set to anything non-empty except `0` = enabled;
/// unset/empty/`0` = disabled. Resolved once per process ([`OnceLock`]), so a
/// mid-run `set_var` cannot make half a binary's tests disagree about the policy.
///
/// This is env-only by design: Rust's libtest cannot take custom CLI flags the
/// way Go's `go test -args` can (unknown flags are a hard error), so an env var
/// is the only channel that reaches every test binary uniformly. A gated test
/// uses [`skip_slow!`], which returns early AND prints the exact variable to
/// set — a skipped test must tell the reader how to run it, which is the whole
/// advantage over an invisible `#[ignore]`.
#[must_use]
pub fn slow_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KAYFABE_SLOW").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Skip the enclosing `#[test]` unless [`slow_enabled`] — `skip_slow!("test_name")`.
///
/// Prints the skip line straight to stderr (deliberately bypassing libtest's
/// output capture, which would otherwise swallow it on the passing path) so the
/// gated test is visibly skipped, with the exact command to run it, instead of
/// silently reporting `ok` having done nothing.
#[macro_export]
macro_rules! skip_slow {
    ($name:expr) => {
        if !$crate::slow_enabled() {
            use ::std::io::Write as _;
            let _ = writeln!(
                ::std::io::stderr(),
                "SKIPPED (slow): {} — set KAYFABE_SLOW=1 to run it (KAYFABE_SLOW=1 cargo test --workspace)",
                $name
            );
            return;
        }
    };
}

// =================================================================================
// ★ The conservation ledger's counterpart: what core state still legitimately OWNS
// (`l1_os_shell.md` §7.8; L1-M2 stage M2-a)
// =================================================================================

/// Every host object reachable from one [`kayfabe_core::gpu::Proc`]'s state — its
/// `Vas`es' host VASes, the host memory behind every published binding, and its
/// channels' host objects.
///
/// This is the other half of the ledger assertion, and the half that makes it a
/// statement about *correctness* rather than about verb arithmetic: an object the mock
/// minted is legitimate iff the core can still name it. Anything outstanding that is
/// **not** in this set is a leak by definition — nothing will ever free it, because
/// nothing can address it. `l1_os_shell.md` §7.8 states the conservation invariant;
/// this function is the `Reachable(core state)` side of the `Outstanding(ledger) ==
/// Reachable(core state)` set equality that `retry_ledger.rs` proved out and that
/// `l1_mean.rs` now runs over the whole composed run.
///
/// Lives here rather than in one test file because two suites now assert against it
/// (`retry_ledger.rs`, `l1_mean.rs`) and a second copy is how the two drift — the same
/// reasoning `Gpu::sync_proc_to_boundary` is extracted under.
#[must_use]
pub fn reachable_objects(
    proc: &kayfabe_core::gpu::Proc,
) -> std::collections::BTreeSet<kayfabe_isolate::HostHandle> {
    let mut live = std::collections::BTreeSet::new();
    for vas in proc.vases.values() {
        live.extend(vas.host_vas);
        for (_va, _len, binding) in vas.table.iter() {
            live.extend(binding.host_memory());
        }
    }
    for chan in proc.channels.values() {
        live.extend(chan.host_channel);
        live.extend(chan.host_engine_objects.values().copied());
    }
    live
}

/// Every host GPU mapping reachable from one [`kayfabe_core::gpu::Proc`]'s state, as
/// the ledger keys them: `(host VAS, host GPU VA)`. A published binding is mapped in
/// its OWN `Vas`'s host VAS (the per-`Vas` #14 fix), so the pair is derivable without
/// consulting the verb log.
#[must_use]
pub fn reachable_maps(
    proc: &kayfabe_core::gpu::Proc,
) -> std::collections::BTreeSet<(kayfabe_isolate::HostHandle, u64)> {
    let mut live = std::collections::BTreeSet::new();
    for vas in proc.vases.values() {
        let Some(host_vas) = vas.host_vas else {
            continue;
        };
        for (_va, _len, binding) in vas.table.iter() {
            if let Some(host_va) = binding.host_va() {
                live.insert((host_vas, host_va));
            }
        }
    }
    live
}

// =================================================================================
// ★★ §12.38 — LEGAL protocol orderings: the one ordering the guest's RM imposes
// =================================================================================

/// Does this event **declare a client namespace's root** (`NV01_ROOT` — a `CLIENT`-classed
/// alloc whose parent is itself)? The single ordering constraint `DUP_OBJECT` is subject
/// to is stated against this.
#[must_use]
pub fn declares_client_root(ev: RmEvent) -> Option<HClient> {
    match ev {
        RmEvent::Alloc {
            client,
            parent,
            handle,
            class,
            ..
        } if class == mc::CLIENT && parent == handle => Some(client),
        _ => None,
    }
}

/// Every client namespace `ev` requires to **already exist** — the mirror of
/// `RmGraph::undeclared_namespace` (`l1_concurrency.md` §12.38). Empty for a client-root
/// alloc (it creates the namespace) and for the teardown verbs (`Free`/`Unmap`, which
/// tolerate a namespace that has already died).
#[must_use]
pub fn namespaces_required(ev: RmEvent) -> Vec<HClient> {
    match ev {
        RmEvent::Alloc { class, .. } if class == mc::CLIENT => Vec::new(),
        RmEvent::Alloc { client, .. }
        | RmEvent::SetPageDir { client, .. }
        | RmEvent::MapMemoryDma { client, .. } => vec![client],
        RmEvent::Dup { src, dst } => vec![dst.client, src.client],
        RmEvent::Unmap { .. } | RmEvent::Free { .. } => Vec::new(),
    }
}

/// ★★ **Reorder an arbitrary event sequence into the nearest LEGAL protocol order**
/// (`l1_concurrency.md` §12.38).
///
/// Decision #4 is order-independence over **legal protocol facts**, not over every order
/// a `Vec` can express. RM resolves `hClient` in the client database as the *first* thing
/// every ioctl-reachable entry point does — alloc (`ogkm
/// src/nvidia/src/libraries/resserv/src/rs_server.c:778`), dup (`:1674`, **both** ends),
/// control (`:1503`), inter-map (`:2218`) — answering `NV_ERR_INVALID_OBJECT_HANDLE`
/// (`:3486-3487`, `:3547-3550`) or, one line later in `clientValidate`,
/// `NV_ERR_INVALID_CLIENT` (`rmapi/client.c:782`). So an event naming a namespace that has
/// never declared a root is a request RM refuses — and which the guest's own RM therefore
/// never emits. Shuffling one into existence and then demanding an identical end state
/// would be demanding order-independence over a trace that cannot occur; modelling exactly
/// the ordering the hardware forbids is what the squat vulnerability *was*.
///
/// So permutation properties are stated over the **linear extensions** of that one partial
/// order. This is a *stable* topological pass: it keeps the caller's order wherever it is
/// already legal and defers only the events that would violate it, so every rotation,
/// reverse and interleave still produces a genuinely different order — just never an
/// impossible one. The refusal of an illegal order is a separate, named assertion
/// (`RmGraphError::UndeclaredClient`), not something this function hides.
///
/// Note what it does **not** reorder: an object may still arrive before its parent, a
/// `SET_PAGE_DIRECTORY` before its VASpace, a `MAP_MEMORY_DMA` before either end, and a
/// `DUP_OBJECT` before its **source object**. Those are object-level facts that may
/// genuinely never have reached the wire (only 25 of 82 measured dups do), so they are
/// DEFER-for-observation and stay unordered here — which is the point of stating the
/// partial order at the level of the client root, the one fact always observed.
///
/// # Panics
/// If the sequence names a namespace that is *never* declared — that is not a legal trace
/// and no ordering of it is one, so silently dropping it would be the same class of
/// mistake this function exists to avoid.
#[must_use]
pub fn legal_order(events: &[RmEvent]) -> Vec<RmEvent> {
    let mut declared: std::collections::BTreeSet<HClient> = std::collections::BTreeSet::new();
    let mut pending: Vec<RmEvent> = events.to_vec();
    let mut out: Vec<RmEvent> = Vec::with_capacity(events.len());
    while !pending.is_empty() {
        let before = out.len();
        let mut deferred: Vec<RmEvent> = Vec::new();
        for &ev in &pending {
            if namespaces_required(ev)
                .into_iter()
                .all(|c| declared.contains(&c))
            {
                if let Some(c) = declares_client_root(ev) {
                    declared.insert(c);
                }
                out.push(ev);
            } else {
                deferred.push(ev);
            }
        }
        assert!(
            out.len() > before,
            "this event set names a client namespace that is never declared — not a legal \
             protocol trace, so no ordering of it is one either"
        );
        pending = deferred;
    }
    out
}

/// A scripted sequence of RM protocol events, plus the identities it introduced —
/// enough to drive `Gpu::apply` and then assert on derived boundaries.
#[derive(Debug, Clone, Default)]
pub struct Scenario {
    /// The events, in the order they were scripted (tests also shuffle these).
    pub events: Vec<RmEvent>,
}

impl Scenario {
    /// Empty scenario.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a raw event.
    pub fn push(&mut self, ev: RmEvent) -> &mut Self {
        self.events.push(ev);
        self
    }

    /// Build one CUDA-process-shaped subgraph in `client`:
    /// client → device → {compute VASpace(+PDB), TSG(bound to that VAS),
    /// GR channel(vchid), CE channel(vchid)}. Returns the client's VASpace node key.
    ///
    /// `handles` lets two processes deliberately reuse **identical** handle values
    /// (the #14 shape) — the graph keys nodes by `(client, handle)`, so identical
    /// handles across clients are correctly distinct.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_process(&mut self, client: HClient, pdb: Pdb, h: ProcessHandles) -> NodeKey {
        self.compute_process_on_gpu(client, pdb, h, None)
    }

    /// Like [`Scenario::compute_process`] but declares the `Device`'s physical-GPU
    /// index (`deviceInstance`) — `None` = the single-GPU default, which still **declares
    /// instance 0**; `Some(i)` = the multi-GPU target `GpuId(i)`.
    ///
    /// ★ G9 (`l1_concurrency.md` §12.21): the helper never emits a `Device` with an
    /// *undeclared* instance, because a real one cannot exist — `deviceId` is a required
    /// field of `NV0080_ALLOC_PARAMETERS`, so the ABI layer always observes it. A Device
    /// with no declared instance is now **unroutable** in the core (it used to silently
    /// become GPU 0), so modelling one here would be modelling a shape the protocol does
    /// not produce.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_process_on_gpu(
        &mut self,
        client: HClient,
        pdb: Pdb,
        h: ProcessHandles,
        device_instance: Option<u32>,
    ) -> NodeKey {
        let dev = h.device;
        let vas = h.vaspace;
        self.push(RmEvent::Alloc {
            client,
            parent: h.client_root,
            handle: h.client_root,
            class: mc::CLIENT,
            facts: user_client(client),
        });
        self.push(RmEvent::Alloc {
            client,
            parent: h.client_root,
            handle: dev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(device_instance.unwrap_or(0)),
                ..Default::default()
            },
        });
        self.push(RmEvent::Alloc {
            client,
            parent: dev,
            handle: vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        });
        self.push(RmEvent::SetPageDir {
            client,
            vaspace: vas,
            pdb,
        });
        self.push(RmEvent::Alloc {
            client,
            parent: dev,
            handle: h.tsg,
            class: mc::TSG,
            facts: AllocFacts {
                h_vaspace: Some(vas),
                ..Default::default()
            },
        });
        self.push(RmEvent::Alloc {
            client,
            parent: h.tsg,
            handle: h.gr_channel,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(vas),
                userd_flags: MockArch::userd_flags_for(h.gr_vchid),
                ..Default::default()
            },
        });
        self.push(RmEvent::Alloc {
            client,
            parent: h.tsg,
            handle: h.ce_channel,
            class: mc::CHANNEL_CE,
            facts: AllocFacts {
                h_vaspace: Some(vas),
                userd_flags: MockArch::userd_flags_for(h.ce_vchid),
                ..Default::default()
            },
        });
        NodeKey::new(client, vas)
    }

    /// Allocate a MEMORY object with a declared physical backing, under `parent`.
    pub fn memory(
        &mut self,
        client: HClient,
        parent: HObject,
        handle: HObject,
        phys: u64,
    ) -> &mut Self {
        self.push(RmEvent::Alloc {
            client,
            parent,
            handle,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(phys),
                ..Default::default()
            },
        })
    }

    /// Allocate an EVENT (os-event / notifier) object under `parent`.
    pub fn event(&mut self, client: HClient, parent: HObject, handle: HObject) -> &mut Self {
        self.push(RmEvent::Alloc {
            client,
            parent,
            handle,
            class: mc::EVENT,
            facts: AllocFacts::default(),
        })
    }

    /// Map `memory` into `vaspace` at `va` for `len` bytes (offset 0).
    pub fn map(
        &mut self,
        client: HClient,
        vaspace: HObject,
        memory: HObject,
        va: GpuVa,
        len: u64,
    ) -> &mut Self {
        self.push(RmEvent::MapMemoryDma {
            client,
            vaspace,
            memory,
            va,
            offset: 0,
            len,
        })
    }

    /// ★ §12.27 — model **UVM** aliasing a compute VASpace into its session client via
    /// `DUP_OBJECT`, plus a VASpace of UVM's own. The session client is a **kernel**
    /// client, which is what it is on real hardware: `nvUvmInterfaceSessionCreate` runs
    /// once per `nvidia_uvm` module load, so ONE client is the destination of every
    /// guest process's dups. Returns UVM's own VASpace node key.
    ///
    /// This edge is therefore a **reference, not a merge**: calling it for two different
    /// compute processes leaves them two separate `Proc`s (the measurement's shape). Use
    /// [`Scenario::peer_dup`] when a test wants the *merging* edge.
    #[allow(clippy::too_many_arguments)]
    pub fn uvm_dup(
        &mut self,
        uvm_client: HClient,
        uvm_root: HObject,
        uvm_dev: HObject,
        uvm_vas: HObject,
        uvm_pdb: Pdb,
        alias_handle: HObject,
        compute_vas: NodeKey,
    ) -> NodeKey {
        self.dup_into(
            kernel_client(),
            uvm_client,
            uvm_root,
            uvm_dev,
            uvm_vas,
            uvm_pdb,
            alias_handle,
            compute_vas,
        )
    }

    /// ★ §12.27 — the **merging** cross-client edge: a second *user* client dups
    /// another user client's VASpace. That is genuine sharing between two guest
    /// processes, so it is one blast radius = one `Proc` (decision #14), and it is the
    /// shape the `LateMerge` guard exists for.
    ///
    /// Shape-identical to [`Scenario::uvm_dup`] on purpose: the ONLY difference between
    /// a reference and a merge is the declared [`ClientKind`] of the destination client,
    /// which is exactly the claim §12.27 makes.
    #[allow(clippy::too_many_arguments)]
    pub fn peer_dup(
        &mut self,
        peer_client: HClient,
        peer_root: HObject,
        peer_dev: HObject,
        peer_vas: HObject,
        peer_pdb: Pdb,
        alias_handle: HObject,
        compute_vas: NodeKey,
    ) -> NodeKey {
        self.dup_into(
            user_client(peer_client),
            peer_client,
            peer_root,
            peer_dev,
            peer_vas,
            peer_pdb,
            alias_handle,
            compute_vas,
        )
    }

    /// Shared body of [`Scenario::uvm_dup`] / [`Scenario::peer_dup`]: a client with its
    /// own device + VASpace + PDB, which then dups `compute_vas` into itself.
    #[allow(clippy::too_many_arguments)]
    fn dup_into(
        &mut self,
        root_facts: AllocFacts,
        client: HClient,
        root: HObject,
        dev: HObject,
        vas: HObject,
        pdb: Pdb,
        alias_handle: HObject,
        compute_vas: NodeKey,
    ) -> NodeKey {
        self.push(RmEvent::Alloc {
            client,
            parent: root,
            handle: root,
            class: mc::CLIENT,
            facts: root_facts,
        });
        self.push(RmEvent::Alloc {
            client,
            parent: root,
            handle: dev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        });
        self.push(RmEvent::Alloc {
            client,
            parent: dev,
            handle: vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        });
        self.push(RmEvent::SetPageDir {
            client,
            vaspace: vas,
            pdb,
        });
        // The cross-client transfer edge: alias the compute VASpace into this client.
        self.push(RmEvent::Dup {
            src: compute_vas,
            dst: NodeKey::new(client, alias_handle),
        });
        NodeKey::new(client, vas)
    }
}

/// The handle set for one [`Scenario::compute_process`]. Group these so two
/// processes can be built with either identical or distinct handles.
#[derive(Debug, Clone, Copy)]
pub struct ProcessHandles {
    /// Client root handle.
    pub client_root: HObject,
    /// Device handle.
    pub device: HObject,
    /// VASpace handle.
    pub vaspace: HObject,
    /// TSG handle.
    pub tsg: HObject,
    /// GR channel handle.
    pub gr_channel: HObject,
    /// GR channel's vChid.
    pub gr_vchid: kayfabe_arch::ids::VChid,
    /// CE channel handle.
    pub ce_channel: HObject,
    /// CE channel's vChid.
    pub ce_vchid: kayfabe_arch::ids::VChid,
}

/// The **#14 shape**: identical guest handle values (both procs' GR channel is
/// `0x5c000019`, etc. — round 1), which the tests pair with identical guest VAs and
/// distinct PDBs. vChids differ (E0: fresh per channel-create, zero collisions).
#[must_use]
pub fn identical_handles(gr_vchid: u16, ce_vchid: u16) -> ProcessHandles {
    use kayfabe_arch::ids::VChid;
    ProcessHandles {
        client_root: HObject(0x5c00_0000),
        device: HObject(0x5c00_0001),
        vaspace: HObject(0x5c00_0010),
        tsg: HObject(0x5c00_0012),
        gr_channel: HObject(0x5c00_0019),
        gr_vchid: VChid(gr_vchid),
        ce_channel: HObject(0x5c00_001a),
        ce_vchid: VChid(ce_vchid),
    }
}

/// The compute class id in the mock arch (re-export for tests).
pub const COMPUTE_CLASS: ClassId = mc::COMPUTE;

/// A minimal client-root alloc event (parent == handle, class = CLIENT). Used by
/// the fuzz + weird-order tests that build graphs object-by-object.
#[must_use]
pub fn client_root(client: HClient) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent: HObject(client.0),
        handle: HObject(client.0),
        class: mc::CLIENT,
        facts: user_client(client),
    }
}

/// A minimal **kernel** client-root alloc event (§12.27) — the UVM-session shape: a
/// client every guest process dups into, which merges with nobody and belongs to the
/// system component.
#[must_use]
pub fn kernel_client_root(client: HClient) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent: HObject(client.0),
        handle: HObject(client.0),
        class: mc::CLIENT,
        facts: kernel_client(),
    }
}
