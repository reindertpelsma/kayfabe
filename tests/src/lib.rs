//! # kayfabe-tests — conformance harness helpers
//!
//! Shared scenario builders for the integration tests under `tests/`. The whole
//! suite runs at unit speed with **no GPU, no hypervisor, no OS** — the payoff of
//! the pure-state-machine core (`mode2_rust_testing_strategy.md` §1).
//!
//! The centerpiece is [`Scenario`]: a small DSL for scripting a guest's RM protocol
//! as abstract `RmEvent`s, with helpers to build the exact shapes the design docs
//! name (a compute process, a UVM dup, two processes with identical VAs/handles).

pub mod gspworld;
pub mod guest;
pub mod rpctrace;
pub mod rpcwire;
pub mod teardown;

pub use guest::{DeviceTally, DoorbellDevice, Lane as DoorbellLane, probe_loop_image};
pub use teardown::{Guarded, ResidueClaim, TeardownView, audit_teardown, unpublish_and_release};

use kayfabe_arch::ClientKind;
use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use kayfabe_mocks::{MockArch, MockVmm, mock_classes as mc};
use kayfabe_vmm::{
    BarId, CoreEvent, HostRegion, IrqSpec, Prot, RamHandle, SlotId, TrapMode, Vmm, VmmError,
};
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
        live.extend(guest_ram_pins(vas).map(|p| p.memory));
    }
    for chan in proc.channels.values() {
        live.extend(chan.host_channel);
        live.extend(chan.host_engine_objects.values().copied());
    }
    live
}

/// ★★★★★ **§16.80** — the **memory-shaped** half of [`reachable_objects`]: host VASes and
/// the host memory bound into them, and nothing channel-shaped.
///
/// # ⊘ Why this split had to exist, and why nothing needed it until now
///
/// [`reachable_objects`] has always had a channel arm, and until the Case-1 engine-object
/// forward acquired a production caller **nothing could populate it**:
/// `Channel::host_channel` and `Channel::host_engine_objects` are written only by
/// `kayfabe_fwd::commit_engine_object` and `commit_doorbell`. So every assertion written
/// over `reachable_objects` was, in fact, an assertion over host *memory* — two of its four
/// terms were structurally empty.
///
/// ⚠ `[measured 2026-08-10]` wiring the forward made them live and turned
/// `the_rpc_bridge_survives_two_interleaved_guest_streams_under_mean_device_load` red, on a
/// message whose own words are *"host **memory** RM says is live must not be taken away"* —
/// while what had actually gone was a **channel**, whose client root the guest itself freed
/// and which no kernel alias held. The code was right; the instrument quantified over more
/// than its sentence did. `a_correct_capture_can_answer_the_wrong_question`.
///
/// ⇒ Assert over the half the sentence names, and assert the other half **separately** —
/// the channel objects going away is a real property that used to be unobservable.
#[must_use]
pub fn reachable_memory(
    proc: &kayfabe_core::gpu::Proc,
) -> std::collections::BTreeSet<kayfabe_isolate::HostHandle> {
    let mut live = std::collections::BTreeSet::new();
    for vas in proc.vases.values() {
        live.extend(vas.host_vas);
        for (_va, _len, binding) in vas.table.iter() {
            live.extend(binding.host_memory());
        }
        live.extend(guest_ram_pins(vas).map(|p| p.memory));
    }
    live
}

/// ★★★ **The guest-RAM pins are core-reachable objects too, and the harness had to be
/// TOLD** (`guest_ram_crossing.md` §5.8).
///
/// ⚠ `[measured 2026-08-10]` the first green `pin_guest_ram` test failed its **teardown
/// post-condition**, not its own assertions: `★★ UNACCOUNTED … 1 mapping(s) are OUTSTANDING
/// and NOTHING can name them`. The record was in core state the whole time
/// ([`kayfabe_core::gpu::Vas::guest_ram_pins`]); these three enumerations simply did not
/// read it, because every host mapping this port had ever made lived on a
/// [`kayfabe_mmu::Binding`] and they were written to walk the address table.
///
/// ★ That is the harness working, not failing: an object nothing enumerates is exactly an
/// object nothing will free, and the ledger said so on the first run rather than on some
/// later one. ⊘ The wrong fix would have been a `ResidueClaim` — declaring the leak instead
/// of accounting for it.
fn guest_ram_pins(
    vas: &kayfabe_core::gpu::Vas,
) -> impl Iterator<Item = &kayfabe_core::gpu::GuestRamPin> {
    vas.guest_ram_pins.values()
}

/// The **channel-shaped** half of [`reachable_objects`] — see [`reachable_memory`] for the
/// split and for the measurement that forced it.
#[must_use]
pub fn reachable_channel_objects(
    proc: &kayfabe_core::gpu::Proc,
) -> std::collections::BTreeSet<kayfabe_isolate::HostHandle> {
    let mut live = std::collections::BTreeSet::new();
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
        // ★★★ A pinned guest range is mapped in this `Vas`'s host VAS exactly like a
        // published one — the difference is whose bytes are under it, which is not a fact
        // the mapping ledger keys on.
        live.extend(guest_ram_pins(vas).map(|p| (host_vas, p.host_va)));
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
            // ⊘ Test helpers publish a vidmem root: the default the PRODUCTION path must
            // never assume. Stated so a reader does not take it as the general case.
            pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
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
                error_notifier: Some(ErrorNotifier::Sysmem {
                    gpa: notifier_gpa(h.gr_vchid),
                }),
                // ★ w393 — a real user channel always declares its GPFIFO ring, and a
                // `Passthrough` channel is now BORN at this alloc over that ring
                // (`birth_passthrough_channels`); without it there is nothing to adopt and
                // no `ours` fallback for the kind.
                gp_fifo_ring: Some(ring_fact_for(h.gr_vchid)),
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
                error_notifier: Some(ErrorNotifier::Sysmem {
                    gpa: notifier_gpa(h.ce_vchid),
                }),
                gp_fifo_ring: Some(ring_fact_for(h.ce_vchid)),
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
            // ⊘ Test helpers publish a vidmem root: the default the PRODUCTION path must
            // never assume. Stated so a reader does not take it as the general case.
            pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
        });
        // The cross-client transfer edge: alias the compute VASpace into this client.
        self.push(RmEvent::Dup {
            src: compute_vas,
            dst: NodeKey::new(client, alias_handle),
        });
        NodeKey::new(client, vas)
    }
}

/// ★★★ A **GA10x-class** single-channel process — the same shape
/// [`Scenario::compute_process`] builds, but declared with NVIDIA's own class ids so it
/// materializes against `kayfabe_chips::Ga10xArch` rather than against `MockArch`.
///
/// Two deliberate narrowings, both forced by the shipped GA10x arch rather than chosen:
///
/// - **One channel, not two.** `Ga10xArch::vchid_for` answers `VChid(0)` for every input
///   until the USERD flag-field decode is settled against silicon, so a second channel
///   collides loudly in the core's `by_vchid` index. One channel is what this arch can
///   currently express; asking for two would be testing the refusal, not the process.
/// - **`AMPERE_CHANNEL_GPFIFO_A` only**, which `Ga10xArch::classify` reads as a
///   `GrCompute` channel — there is no separate CE channel class on this part.
///
/// Returns the channel's [`NodeKey`].
pub fn ga10x_process(s: &mut Scenario, client: HClient, pdb: Pdb, base: u32) -> NodeKey {
    use kayfabe_abi::generated::classes as nv;
    let h = |off: u32| HObject(base + off);
    let (root, dev, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));
    s.push(RmEvent::Alloc {
        client,
        parent: root,
        handle: root,
        class: ClassId(nv::NV01_ROOT),
        facts: user_client(client),
    });
    s.push(RmEvent::Alloc {
        client,
        parent: root,
        handle: dev,
        class: ClassId(nv::NV01_DEVICE_0),
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client,
        parent: dev,
        handle: vas,
        class: ClassId(nv::FERMI_VASPACE_A),
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client,
        vaspace: vas,
        pdb,
        // ⊘ Test helpers publish a vidmem root: the default the PRODUCTION path must
        // never assume. Stated so a reader does not take it as the general case.
        pdb_aperture: Some(kayfabe_arch::Aperture::Vidmem),
    });
    s.push(RmEvent::Alloc {
        client,
        parent: dev,
        handle: tsg,
        class: ClassId(nv::KEPLER_CHANNEL_GROUP_A),
        facts: AllocFacts {
            h_vaspace: Some(vas),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client,
        parent: tsg,
        handle: chan,
        class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
        facts: AllocFacts {
            h_vaspace: Some(vas),
            // ★ The `NVOS04_FLAGS` word CPU-RM writes for chid 0 — the same chid the
            // notifier below is placed for. `AllocFacts::default()` (a zero word) names NO
            // channel at all: RM's own reader leaves the chid to the allocator when
            // `_PAGE_FIXED` is clear, so `Arch::vchid_from_userd_flags` answers `None` and
            // the projection refuses by name. `MockArch::userd_flags_for` is the encoder
            // because it is the one differentialled against NVIDIA's OWN compiled writer
            // (`tests/tests/userd_chid_oracle.rs`); a literal here would drift silently.
            userd_flags: kayfabe_mocks::MockArch::userd_flags_for(kayfabe_arch::ids::VChid(0)),
            error_notifier: Some(ErrorNotifier::Sysmem {
                gpa: notifier_gpa(kayfabe_arch::ids::VChid(0)),
            }),
            ..Default::default()
        },
    });
    NodeKey::new(client, chan)
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

/// ★ Where a scripted channel declares its **error notifier** — the guest-physical
/// address `kayfabe_abi::notifier` would have decoded out of `errorNotifierMem`.
///
/// Keyed on the vChid rather than on the client, because a scenario's whole point is that
/// two procs share handle *values*: a notifier keyed on a handle would collide across the
/// #14 shape and hide exactly the attribution error those tests exist to catch. Real
/// guests allocate the notifier per channel too (`ogkm-580:
/// src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:5887-5933`).
#[must_use]
pub fn notifier_gpa(vchid: kayfabe_arch::ids::VChid) -> u64 {
    0x7000_0000 + (u64::from(vchid.0) << 8)
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

// =================================================================================
// ★★ `SharedVmm` — one guest memory, many threads, the adapter's OWN leaf lock
// =================================================================================

/// A [`MockVmm`] behind an [`Arc`] + [`Mutex`], usable as a `Vmm` from many threads at
/// once over **one** guest memory.
///
/// # Why this exists rather than a per-thread `MockVmm`
///
/// `Vmm` methods take `&mut self`, so a per-thread mock gives every vCPU thread its own
/// private guest RAM — which is not a hypervisor, and in particular cannot compose "one
/// proc aims a descriptor at MMIO" with "another proc reads a legitimate pushbuffer from
/// the same region map". This wrapper is the realistic shape: guest memory and the
/// guest-physical region map are shared, and each access takes the adapter's own lock
/// around them.
///
/// # ★ And that lock is the one `kayfabe_vmm::GuestRamMap` says must exist
///
/// `gpa_read`/`gpa_write` are **in-lock legal** (`l1_os_shell.md` §6.1), so this mutex is
/// acquired with rank 0 (and sometimes rank 1) already held. It is therefore a **leaf**:
/// we construct it, it acquires nothing beneath itself, and its critical section is a
/// bounded memcpy with no syscall and no wait on a peer. It is deliberately NOT a
/// [`kayfabe_rt::LockRank`] participant — the rank ladder is the core's, and an adapter's
/// internal leaf sits below all of it. Closing the foreign-lock hazard by introducing an
/// *unrankable* lock of our own would have traded one invisible inversion for another;
/// this one is visible, ours, and terminal.
///
/// # ★ It also WITNESSES the ranked-lock depth at each guest-memory access
///
/// The whole hazard is *"an in-lock-legal accessor takes a guest-chosen address"*, so a
/// test of the refusal proves nothing unless the access really did happen with one of
/// our ranked locks held. Without this witness the suite could be green with the read
/// running lock-free — a green instrument on an unexercised path, one level up
/// (`docs/design/testing_doctrine.md` §1).
#[derive(Debug, Clone)]
pub struct SharedVmm {
    inner: std::sync::Arc<std::sync::Mutex<MockVmm>>,
    /// Max `kayfabe_rt::lock::held_depth()` observed at a `gpa_read`/`gpa_write`.
    max_depth: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Min of the same, initialised to `u32::MAX` so "never accessed" is detectable.
    min_depth: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

// ★ Hand-written, and the bite-check is why. A DERIVED `Default` gives `min_depth = 0`
// (`Arc<AtomicU32>::default()`), which makes the lower half of `lock_depth_span`
// vacuously satisfied: a witness reporting a CONSTANT still passes `(0, 1)`. That
// non-biting neuter (N12) is the finding — `testing_doctrine.md` §1 rule 3, "bite-check
// the instrument, not only the fix".
impl Default for SharedVmm {
    fn default() -> Self {
        Self::new(MockVmm::new())
    }
}

impl SharedVmm {
    /// Wrap a scripted [`MockVmm`].
    #[must_use]
    pub fn new(vmm: MockVmm) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(vmm)),
            max_depth: std::sync::Arc::default(),
            min_depth: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(u32::MAX)),
        }
    }

    /// Run `f` against the underlying mock (scripting and assertions).
    pub fn with<R>(&self, f: impl FnOnce(&mut MockVmm) -> R) -> R {
        f(&mut self
            .inner
            .lock()
            .expect("guest-memory leaf lock is never poisoned"))
    }

    /// `(min, max)` ranked-lock nesting observed across every guest-memory access.
    ///
    /// **Both halves are load-bearing.** `max == 0` means every access was lock-free —
    /// the in-lock hazard was never exercised and any refusal test above it is a fact
    /// about a different path. `min == u32::MAX` means no access happened at all. And a
    /// witness that reported a *constant* would fail one of the two, which is what makes
    /// the pair a causality claim rather than a coincidence.
    #[must_use]
    pub fn lock_depth_span(&self) -> (u32, u32) {
        use std::sync::atomic::Ordering::SeqCst;
        (self.min_depth.load(SeqCst), self.max_depth.load(SeqCst))
    }

    /// Record the ranked-lock depth of the calling thread at an access.
    fn witness_depth(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        let d = kayfabe_rt::lock::held_depth();
        self.max_depth.fetch_max(d, SeqCst);
        self.min_depth.fetch_min(d, SeqCst);
    }
}

impl Vmm for SharedVmm {
    fn gpa_read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), VmmError> {
        self.witness_depth();
        self.with(|v| v.gpa_read(gpa, buf))
    }
    fn gpa_write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), VmmError> {
        self.witness_depth();
        self.with(|v| v.gpa_write(gpa, buf))
    }
    fn map_guest(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        prot: Prot,
    ) -> Result<SlotId, VmmError> {
        self.with(|v| v.map_guest(gpa, len, backing, prot))
    }
    fn unmap_guest(&mut self, slot: SlotId) -> Result<(), VmmError> {
        self.with(|v| v.unmap_guest(slot))
    }
    fn set_trap(
        &mut self,
        bar: BarId,
        range: core::ops::Range<u64>,
        mode: TrapMode,
    ) -> Result<(), VmmError> {
        self.with(|v| v.set_trap(bar, range, mode))
    }
    fn raise_irq(&mut self, irq: IrqSpec) -> Result<(), VmmError> {
        self.with(|v| v.raise_irq(irq))
    }
    fn export_ram(&mut self, slice: Option<core::ops::Range<u64>>) -> Result<RamHandle, VmmError> {
        self.with(|v| v.export_ram(slice))
    }
    fn defer(&mut self, after: core::time::Duration, event: CoreEvent) {
        self.with(|v| v.defer(after, event));
    }
    fn now(&self) -> kayfabe_util::Instant {
        self.inner
            .lock()
            .expect("guest-memory leaf lock is never poisoned")
            .now()
    }
    fn map_read_native(
        &mut self,
        gpa: u64,
        len: u64,
        backing: HostRegion,
        write_trap: Option<core::ops::Range<u64>>,
    ) -> Result<SlotId, VmmError> {
        self.with(|v| v.map_read_native(gpa, len, backing, write_trap))
    }
}

/// Lay out a GPFIFO ring naming one range `[va, va+len)`, **without** writing anything
/// into guest memory and **without** binding anything — the hostile shape: the guest
/// points a descriptor wherever it likes.
///
/// ⊘ The address is a GPU **virtual** address (`kayfabe_arch::PushRange::va`), like every
/// real GPFIFO entry's. Unless [`bind_ring`] has bound it, `kayfabe_fwd::read_pushbuffer`
/// will refuse it as an address-table MISS before any guest byte is read.
#[must_use]
pub fn gpfifo_ring(va: u64, len: u64) -> Vec<u8> {
    let mut ring = Vec::new();
    ring.extend_from_slice(&va.to_le_bytes());
    ring.extend_from_slice(&len.to_le_bytes());
    ring
}

/// ★★★ The GPU virtual address a test names a pushbuffer at, given the guest-physical
/// address its bytes actually live at.
///
/// **The bias is the point, and it must never be zero.** A fixture that maps a ring's VA
/// onto the identical GPA cannot tell a translating `read_pushbuffer` from the
/// untranslated one it replaces — that is precisely the encoding
/// `kayfabe_mocks::MockPushbuffer` used to bake in, and it hid a wrong-bytes read for the
/// whole life of the seam (`mock_fidelity_both_directions`,
/// `execution_plane_increments.md` §8.2.3). With a bias, a regression to reading
/// `PushRange::va` raw lands on unmapped guest memory and the test goes red.
///
/// ★ **512 GiB, and the value is constrained rather than picked.** It must be above every
/// GPA any fixture here uses (so a VA is never a plausible GPA), 4-byte aligned (a GA10x
/// `GP_ENTRY0_GET` is bits 31:2), and small enough that `bias + gpa` still fits the **40
/// address bits** a real GPFIFO entry has (`GP_ENTRY0_GET` + `GP_ENTRY1_GET_HI 7:0`,
/// `ogkm-580: clc56f.h:270, 272`). 2^39 satisfies all three, so one fixture vocabulary
/// serves both the mock codec and the real one.
pub const PB_VA_BIAS: u64 = 0x80_0000_0000;

/// The VA a pushbuffer whose bytes live at `gpa` is named at. See [`PB_VA_BIAS`].
#[must_use]
pub fn pb_va(gpa: u64) -> GpuVa {
    GpuVa(PB_VA_BIAS.wrapping_add(gpa))
}

/// ★★★ Bind `[va, va+len)` → guest-physical `gpa` in the address table of the VAS that
/// channel `cid` of proc `pid` issues on — i.e. do for a fixture exactly what the guest's
/// own `NV04_MAP_MEMORY_DMA` of its pushbuffer does for a real channel (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:842`).
///
/// Idempotent: re-binding the same range to the same physical address is a no-op, and
/// re-binding it elsewhere replaces it, so a test may script several rings at one address.
///
/// # Panics
/// If `va == gpa` (see [`PB_VA_BIAS`] — such a fixture asserts nothing about translation),
/// if the proc/channel/VAS does not exist, or if the bind is refused.
pub fn bind_ring_at(
    gpu: &mut kayfabe_core::gpu::Gpu,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
    va: GpuVa,
    gpa: u64,
    len: u64,
) {
    let proc = gpu.procs.get_mut(&pid).expect("the proc is live");
    let chan = proc.channels.get(&cid).expect("the channel exists");
    let (cgpu, cpdb) = (chan.gpu, chan.vas_pdb.expect("the channel declares a VAS"));
    let vas = proc.vas_by_pdb_mut(cgpu, cpdb).expect("the VAS exists");
    bind_ring_in(vas, va, gpa, len);
}

/// [`bind_ring_at`] against a [`kayfabe_core::gpu::Vas`] already in hand.
///
/// # Panics
/// If `va == gpa`, or if the bind is refused.
pub fn bind_ring_in(vas: &mut kayfabe_core::gpu::Vas, va: GpuVa, gpa: u64, len: u64) {
    assert_ne!(
        va.0, gpa,
        "an identity ring binding cannot distinguish a translated read from an \
         untranslated one — use `pb_va(gpa)`"
    );
    // ⊘ w555 — a fixture binds into a space it has just declared, so the base is present.
    // `expect` rather than a default: a rootless space here would mean the fixture never
    // declared one, and binding under `Pdb(0)` would silently reintroduce the merge.
    let pdb = vas.pdb.expect("the fixture's VAS declares a page-directory base");
    if let Some((start, l, b)) = vas.table.binding_at(va)
        && start == va.0
        && l == len
        && b.phys() == gpa
    {
        return; // already exactly this binding
    }
    // ★ Clear EVERY overlapping binding, not just one covering the start address. Two
    // fixture ranges that straddle each other (a workload probing `X` and `X - 8`) leave
    // the second bind refused as an `Overlap` if only the start's binding is dropped —
    // and a fixture that silently fails to bind is a test asserting about a MISS.
    let stale: Vec<u64> = vas
        .table
        .iter()
        .filter(|(s, l, _)| *s < va.0.saturating_add(len) && va.0 < s.saturating_add(*l))
        .map(|(s, _, _)| s)
        .collect();
    for s in stale {
        vas.table.unbind(GpuVa(s));
    }
    vas.table
        .bind(
            pdb,
            va,
            len,
            kayfabe_mmu::Binding::declared_by_guest(gpa, kayfabe_arch::Aperture::SysmemCoherent)
                .expect("the fixture declares a kind the guest can declare"),
        )
        .expect("the ring range binds");
}

/// Decode a mock-format `ring` (16-byte entries, `[va u64 LE, len u64 LE]`) into the
/// `(va, gpa, len)` triples [`script_ring_via`] implied — i.e. undo [`pb_va`].
///
/// # Panics
/// If an entry names a VA below [`PB_VA_BIAS`], which means the fixture did not build it
/// with [`pb_va`] and the caller must bind it explicitly instead.
#[must_use]
pub fn ring_bindings(ring: &[u8]) -> Vec<(GpuVa, u64, u64)> {
    ring.chunks_exact(16)
        .map(|e| {
            let va = u64::from_le_bytes(e[0..8].try_into().expect("8 bytes"));
            let len = u64::from_le_bytes(e[8..16].try_into().expect("8 bytes"));
            assert!(
                va >= PB_VA_BIAS,
                "ring entry at VA {va:#x} was not built with `pb_va`; bind it explicitly"
            );
            (GpuVa(va), va - PB_VA_BIAS, len)
        })
        .collect()
}

/// ★ Bind every range a [`script_ring_via`]-built `ring` names, on the VAS channel `cid`
/// of proc `pid` issues on. The one-call form of "the guest mapped its pushbuffer before
/// naming it in a GPFIFO entry".
///
/// # Panics
/// As [`bind_ring_at`] and [`ring_bindings`].
pub fn bind_ring(
    gpu: &mut kayfabe_core::gpu::Gpu,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
    ring: &[u8],
) {
    for (va, gpa, len) in ring_bindings(ring) {
        bind_ring_at(gpu, pid, cid, va, gpa, len);
    }
}

/// [`bind_ring`] through a [`kayfabe_rt::device::SharedDevice`] — the locked shell's own accessor,
/// so an L1 test binds through the same rank-1 path everything else mutates a proc with.
///
/// # Panics
/// If the proc is gone, or as [`bind_ring_in`] / [`ring_bindings`].
pub fn bind_ring_dev(
    dev: &kayfabe_rt::device::SharedDevice,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
    ring: &[u8],
) {
    let binds = ring_bindings(ring);
    dev.with_proc_mut(pid, |proc| {
        let chan = proc.channels.get(&cid).expect("the channel exists");
        let (cgpu, cpdb) = (chan.gpu, chan.vas_pdb.expect("the channel declares a VAS"));
        let vas = proc.vas_by_pdb_mut(cgpu, cpdb).expect("the VAS exists");
        for (va, gpa, len) in binds {
            bind_ring_in(vas, va, gpa, len);
        }
    })
    .expect("the proc is live");
}

/// Script `methods` into guest RAM at `gpa` through `vmm` and return the GPFIFO ring
/// naming them **by GPU virtual address** ([`pb_va`]) — the legitimate shape.
///
/// ⚠ The ring is not readable until the VA is bound: [`bind_ring`] / [`bind_ring_dev`].
pub fn script_ring(vmm: &SharedVmm, gpa: u64, methods: &[(u32, Vec<u32>)]) -> Vec<u8> {
    script_ring_via(&mut vmm.clone(), gpa, methods)
}

/// ★ [`script_ring`] over **any** backend — the mock harness's `SharedVmm` or the real
/// `kayfabe_vmm_kvm::KvmVmm`.
///
/// Written as one function over `&mut dyn Vmm` rather than duplicated per backend, and
/// that is the portability contract being *used* rather than asserted: a workload that
/// compiles against the port compiles against every implementation of it, so the mean run
/// can drive real KVM memslots through the same script that drives a `BTreeMap`.
///
/// # Panics
/// If the range is not proven RAM by the backend — a scripting call that lands on a
/// device window is a bug in the test, and a silent one would make every assertion below
/// it a fact about an unwritten buffer.
pub fn script_ring_via(vmm: &mut dyn Vmm, gpa: u64, methods: &[(u32, Vec<u32>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (h, args) in methods {
        bytes.extend_from_slice(&h.to_le_bytes());
        for a in args {
            bytes.extend_from_slice(&a.to_le_bytes());
        }
    }
    // ★ Through the PORT (`Vmm::gpa_write`), not through a backend-specific back door —
    // scripting guest memory is a guest-physical access like any other, it is proven RAM
    // like any other, and it is the one in this harness that runs with NO ranked lock
    // held. That is what makes every lock-depth span's LOWER bound a real observation.
    vmm.gpa_write(gpa, &bytes)
        .expect("scripting a legitimate pushbuffer into guest RAM");
    // ★ The entry names the VA, not the GPA — a real GPFIFO entry holds `pbGpuVA +
    // gpOffset` (`ogkm-580: mem_utils_gm107.c:1871-1879`). The bias is what makes a
    // regression to the untranslated read observable; see `PB_VA_BIAS`.
    gpfifo_ring(pb_va(gpa).0, bytes.len() as u64)
}

/// ★★★ **#177** — the guest's own `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`, applied to every
/// channel this [`kayfabe_core::gpu::Gpu`] currently holds. Returns how many channels it
/// declared, so a caller can assert it declared *something*.
///
/// # Why this helper exists, and why it is not a bypass
///
/// `kayfabe_fwd::plan_doorbell` refuses a channel the guest never asked us to schedule
/// (`FwdFault::NotScheduled`). That gate is what makes serving `0xa06f0103` a **performed
/// transition** rather than a word — see `kayfabe_core::gpu::ExecPlane::requested`.
///
/// A real guest always schedules before it rings; the harnesses in this workspace mostly
/// did not, because until #177 nothing made them. So this is the step those tests were
/// **missing**, restored in one place rather than open-coded 45 times. ⊘ It is emphatically
/// **not** a way around the gate: it goes through the same `exec.requested` set the control
/// writes, it is called by tests whose subject is something else entirely, and the gate's
/// own behaviour is asserted in `tests/tests/gpfifo_schedule.rs` — which does *not* call
/// this — including that a channel it has not been called for is refused by name.
pub fn guest_schedules_every_channel(gpu: &mut kayfabe_core::gpu::Gpu) -> usize {
    let mut n = 0;
    for proc in core::iter::once(&mut gpu.system).chain(gpu.procs.values_mut()) {
        for &cid in proc.chan_ids.values() {
            proc.exec.requested.insert(cid);
            n += 1;
        }
    }
    n
}

// =================================================================================
// ★★★★★ w393 — BIRTH-AT-ALLOC: the fixture's guest-userspace channels are born the way
// production births them, BEFORE any doorbell
// =================================================================================

/// How much guest RAM a fixture's isolates can see, when it opens the guest-RAM door
/// ([`kayfabe_mocks::MockIsolateFactory::with_guest_ram`]). Large enough that every
/// [`ring_grant_for`] offset lands inside it.
pub const GUEST_RAM_BYTES: u64 = 0x2_0000_0000;

/// One page: the extent every conventional fixture ring is bound and pinned at.
pub const RING_BYTES: u64 = 4096;

/// The `gpFifoEntries` a conventional fixture channel declares — one page of the mock
/// codec's 16-byte entries (`script_ring_via`).
pub const RING_ENTRIES: u32 = (RING_BYTES / 16) as u32;

/// Where the guest-RAM block appears in the hypervisor's own file offsets. ⊘ Deliberately
/// NOT identity with the GPA (`tests/tests/guest_ram_pin.rs` on why: identity holds on the
/// bench's `-m 2048` and breaks at `-m 8G`, and a fixture that made them equal could not
/// tell a correct chain from one that re-derived the offset from the address).
pub const GUEST_RAM_FILE_BASE: u64 = 0x1_0000_0000;

/// The guest-physical page a conventional fixture channel's GPFIFO ring lives in — keyed
/// on the vChid for the same reason [`notifier_gpa`] is: two procs in the #14 shape share
/// handle *values*, so a ring keyed on a handle would alias across them.
#[must_use]
pub fn ring_gpa_for(vchid: kayfabe_arch::ids::VChid) -> u64 {
    0x5100_0000 + (u64::from(vchid.0) << 12)
}

/// The guest VA that channel's ring is declared at (`gpFifoOffset`) — biased by
/// [`PB_VA_BIAS`], so a VA is never a plausible GPA.
#[must_use]
pub fn ring_va_for(vchid: kayfabe_arch::ids::VChid) -> GpuVa {
    pb_va(ring_gpa_for(vchid))
}

/// The declared `gp_fifo_ring` fact for a conventional fixture channel.
#[must_use]
pub fn ring_fact_for(vchid: kayfabe_arch::ids::VChid) -> kayfabe_core::rmgraph::GpFifoRing {
    kayfabe_core::rmgraph::GpFifoRing {
        va: ring_va_for(vchid).0,
        entries: RING_ENTRIES,
    }
}

/// The VMM's grant for `len` bytes of guest RAM at guest-physical `gpa` — what a
/// hypervisor would state for a ring page, minted the only way a grant can be.
#[must_use]
pub fn ring_grant_for(gpa: u64, len: u64) -> kayfabe_isolate::GuestRamGrant {
    kayfabe_isolate::GuestRamGrant::originated_by_the_vmm(
        GUEST_RAM_FILE_BASE + gpa,
        len,
        Prot::ReadWrite,
    )
}

/// What [`birth_passthrough_channels`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BornChannels {
    /// Every `Passthrough` channel born by this call, in `(proc, chan)` order.
    pub born: Vec<(kayfabe_core::ProcId, kayfabe_core::ChanId)>,
    /// ⊘ `Passthrough` channels that name NO VAS yet — a birth needs a host VAS to place
    /// the ring in, so these stay un-born, **by name**, until the guest's `SetPageDir`
    /// lands and the caller births again. Reported, never silently skipped.
    pub no_vas: Vec<(kayfabe_core::ProcId, kayfabe_core::ChanId)>,
}

/// One un-born `Passthrough` channel's coordinates, collected under one borrow so the
/// acts below can take `&mut`.
struct UnbornPassthrough {
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
    key: kayfabe_core::rmgraph::ResourceKey,
    gpu: kayfabe_arch::ids::GpuId,
    pdb: Option<Pdb>,
    vchid: kayfabe_arch::ids::VChid,
}

fn unborn_passthrough(proc: &kayfabe_core::gpu::Proc) -> Vec<UnbornPassthrough> {
    proc.channels
        .values()
        .filter(|c| {
            c.kind == kayfabe_core::channel_kind::GuestChannelKind::Passthrough
                && c.host_channel.is_none()
        })
        .map(|c| UnbornPassthrough {
            pid: proc.id,
            cid: c.id,
            key: c.key,
            gpu: c.gpu,
            pdb: c.vas_pdb,
            vchid: c.vchid,
        })
        .collect()
}

/// The ring's `(start, len, gpa)` in `vas`'s table — bound by the fixture at the
/// conventional GPA if the guest has not bound it already. ★ An existing binding is kept
/// (a fixture that scripted its own ring through [`bind_ring_at`] keeps its bytes).
fn ring_binding_in(
    vas: &mut kayfabe_core::gpu::Vas,
    ring_va: GpuVa,
    vchid: kayfabe_arch::ids::VChid,
) -> (u64, u64, u64) {
    if let Some((start, len, b)) = vas.table.binding_at(ring_va) {
        return (start, len, b.phys());
    }
    let gpa = ring_gpa_for(vchid);
    // ⊘ w555 — a fixture binds into a space it has just declared, so the base is present.
    // `expect` rather than a default: a rootless space here would mean the fixture never
    // declared one, and binding under `Pdb(0)` would silently reintroduce the merge.
    let pdb = vas.pdb.expect("the fixture's VAS declares a page-directory base");
    vas.table
        .bind(
            pdb,
            ring_va,
            RING_BYTES,
            kayfabe_mmu::Binding::declared_by_guest(gpa, kayfabe_arch::Aperture::SysmemCoherent)
                .expect("the fixture declares a kind the guest can declare"),
        )
        .expect("the ring page binds");
    (ring_va.0, RING_BYTES, gpa)
}

fn ring_declared_by(
    node: Option<&kayfabe_core::rmgraph::RmNode>,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
) -> GpuVa {
    let ring = node.and_then(|n| n.facts.gp_fifo_ring).unwrap_or_else(|| {
        panic!(
            "★ w393 — {pid:?}/{cid:?} is a Passthrough channel that DECLARED NO GPFIFO RING, so \
             nothing can birth it: a passthrough birth adopts the guest's ring at the channel \
             alloc and there is no `ours` fallback for this kind. Declare one on the channel's \
             `AllocFacts::gp_fifo_ring` (`Scenario::compute_process` does; `ring_fact_for` is \
             the conventional value)."
        )
    });
    GpuVa(ring.va)
}

fn expect_pinned(out: Result<kayfabe_fwd::GuestRamPinned, kayfabe_fwd::FwdFault>, va: GpuVa) {
    match out {
        Ok(_) => {}
        Err(kayfabe_fwd::FwdFault::Rm {
            err: kayfabe_isolate::RmError::GuestRamUnavailable,
            ..
        }) => panic!(
            "★ w393 — the ring page at {va:?} cannot be pinned because this fixture's isolates \
             have NO guest-RAM door: build the factory with \
             `.with_guest_ram(kayfabe_tests::GUEST_RAM_BYTES)`. A passthrough birth adopts the \
             guest's OWN ring, and the mock refuses to describe guest pages it was never told \
             exist (the same refusal the real path makes without `memory-backend-memfd,share=on`)."
        ),
        Err(e) => panic!("★ w393 — the ring page at {va:?} did not pin: {e:?}"),
    }
}

fn expect_born(
    out: Result<kayfabe_fwd::ChannelBirthOutcome, kayfabe_fwd::FwdFault>,
    pid: kayfabe_core::ProcId,
    cid: kayfabe_core::ChanId,
) {
    match out {
        Ok(kayfabe_fwd::ChannelBirthOutcome::Born { .. }) => {}
        other => panic!(
            "★ w393 — {pid:?}/{cid:?} was not born at its channel alloc: {other:?}. The ring \
             page was pinned (GuestPhysDma + SoleBacking, the guest's own bytes) just before \
             this, so a refusal here is the birth chain's, not the fixture's."
        ),
    }
}

/// ★★★★★ **w393 — birth every un-born `Passthrough` channel the way production now does:
/// at the guest's own channel alloc, over the guest's own ring, BEFORE any doorbell.**
///
/// # Why this helper exists, and why it is not a bypass
///
/// A doorbell used to birth a user-proc's host channel lazily, over a ring of ours. That
/// was wrong for the kind (`Passthrough` ⇒ the guest drives its own ring and hardware
/// writes `GP_GET`; `RingSource::Ours` is illegal there) and, worse, adopting the guest's
/// USERD at a doorbell birth **destroys the cursor that rang it** — host RM accepts a
/// caller-supplied USERD and then zeroes all 512 bytes (`[measured w233]`, real GA106).
/// So `kayfabe_fwd::plan_doorbell` now refuses a `Passthrough` doorbell on an un-born
/// channel **by name** (`FwdFault::PassthroughDoorbellBirth`), and the birth lives at the
/// alloc (`kayfabe_fwd::plan_channel_birth`), where the guest has not yet written a cursor.
///
/// This is the step every doorbell-ringing harness in the workspace was missing once the
/// birth moved — restored in one place, through the **production chain** and nothing else:
///
/// 1. the guest's own ring page is bound in the channel's VAS (the fixture stands in for
///    the populate pass, as `tests/tests/guest_ram_pin.rs` does);
/// 2. it is **pinned** through [`kayfabe_fwd::pin_guest_ram`] — an `OS_DESCRIPTOR` over the
///    guest's own pages, the shape `GuestRing::memory` calls production, which is what makes
///    the binding *"the guest's own bytes"* (`GuestPhysDma` + `SoleBacking`) and therefore
///    adoptable;
/// 3. the channel is born through [`kayfabe_fwd::birth_channel`], which adopts that ring
///    (and, when reachable, the USERD) or refuses by name. ⊘ No fallback to our ring exists
///    on that chain — `VerbPlan::ChannelBirth::adopt` is not an `Option`.
///
/// `Emulated` channels are untouched: their doorbell birth over our ring is still the
/// design, and `kayfabe_fwd::plan_channel_birth` skips them by name.
///
/// # Panics
/// If a `Passthrough` channel declares no ring, its pin is refused, or its birth is
/// refused — each with the cause named. A channel that names no VAS is reported in
/// [`BornChannels::no_vas`] rather than panicked on: its birth is legitimately later.
pub fn birth_passthrough_channels(gpu: &mut kayfabe_core::gpu::Gpu) -> BornChannels {
    let mut out = BornChannels::default();
    let work: Vec<UnbornPassthrough> = gpu.procs.values().flat_map(unborn_passthrough).collect();
    for u in work {
        let Some(pdb) = u.pdb else {
            out.no_vas.push((u.pid, u.cid));
            continue;
        };
        let ring_va = ring_declared_by(gpu.spine.rmgraph.node_of_resource(u.key), u.pid, u.cid);
        let proc = gpu.procs.get_mut(&u.pid).expect("the proc is live");
        let vas = proc
            .vas_by_pdb_mut(u.gpu, pdb)
            .expect("the channel's VAS exists");
        let (start, len, gpa) = ring_binding_in(vas, ring_va, u.vchid);
        expect_pinned(
            kayfabe_fwd::pin_guest_ram(proc, u.gpu, pdb, GpuVa(start), ring_grant_for(gpa, len)),
            ring_va,
        );
        expect_born(
            kayfabe_fwd::birth_channel(gpu, u.key.origin.client, u.key.origin.handle, None),
            u.pid,
            u.cid,
        );
        out.born.push((u.pid, u.cid));
    }
    out
}

/// One un-born `Passthrough` channel whose ring page [`pin_passthrough_rings_dev`] has
/// pinned — everything a birth needs, and the identity a test races the birth on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedRing {
    /// The owning proc.
    pub pid: kayfabe_core::ProcId,
    /// The channel.
    pub cid: kayfabe_core::ChanId,
    /// The guest's `hClient` — `SharedDevice::birth_channel_by_handle`'s first key.
    pub client: HClient,
    /// The guest's channel handle — its second key.
    pub channel: HObject,
}

/// ★ **Steps 1–2 of [`birth_passthrough_channels`] against a live
/// [`kayfabe_rt::device::SharedDevice`]** — bind (if the guest has not) and PIN every
/// un-born `Passthrough` channel's ring page through the device's own `pin_guest_ram`
/// verb, one rank-1 lock at a time — and hand back what is now ready to be born.
///
/// Split from [`birth_passthrough_channels_dev`] so a test can put the BIRTH itself in
/// flight (hold its `AllocChannel`, move the world under it) — the R5 canaries — with the
/// pin, which is not the verb under study, already done.
///
/// # Panics
/// As [`birth_passthrough_channels`]. A channel that names no VAS is skipped and reported
/// in the returned pair's second element rather than panicked on.
pub fn pin_passthrough_rings_dev(
    dev: &kayfabe_rt::device::SharedDevice,
) -> (
    Vec<PinnedRing>,
    Vec<(kayfabe_core::ProcId, kayfabe_core::ChanId)>,
) {
    use kayfabe_core::gpu::Gpu;
    let mut ready = Vec::new();
    let mut no_vas = Vec::new();
    let mut work: Vec<UnbornPassthrough> = Vec::new();
    for pid in dev.live_pids() {
        if pid == Gpu::SYSTEM_PROC {
            continue;
        }
        if let Some(mut w) = dev.with_proc(pid, unborn_passthrough) {
            work.append(&mut w);
        }
    }
    for u in work {
        let Some(pdb) = u.pdb else {
            no_vas.push((u.pid, u.cid));
            continue;
        };
        let ring_va =
            dev.with_spine(|s| ring_declared_by(s.rmgraph.node_of_resource(u.key), u.pid, u.cid));
        let (start, len, gpa) = dev
            .with_proc_mut(u.pid, |p| {
                let vas = p
                    .vas_by_pdb_mut(u.gpu, pdb)
                    .expect("the channel's VAS exists");
                ring_binding_in(vas, ring_va, u.vchid)
            })
            .expect("the proc is live");
        expect_pinned(
            dev.pin_guest_ram(u.gpu, pdb, GpuVa(start), ring_grant_for(gpa, len)),
            ring_va,
        );
        ready.push(PinnedRing {
            pid: u.pid,
            cid: u.cid,
            client: u.key.origin.client,
            channel: u.key.origin.handle,
        });
    }
    (ready, no_vas)
}

/// [`birth_passthrough_channels`] against a live [`kayfabe_rt::device::SharedDevice`] —
/// [`pin_passthrough_rings_dev`], then the device's own `birth_channel_by_handle` for each
/// pinned ring. Also drains the device's own birth latch (`SharedDevice::apply` latches
/// every channel alloc, as the shim's register-write tail would), so a fixture that
/// allocates a proc *after* wrapping sees the same outcome.
///
/// # Panics
/// As [`birth_passthrough_channels`].
pub fn birth_passthrough_channels_dev(dev: &kayfabe_rt::device::SharedDevice) -> BornChannels {
    let (ready, no_vas) = pin_passthrough_rings_dev(dev);
    let mut out = BornChannels {
        born: Vec::new(),
        no_vas,
    };
    for r in ready {
        expect_born(
            dev.birth_channel_by_handle(r.client, r.channel, None),
            r.pid,
            r.cid,
        );
        out.born.push((r.pid, r.cid));
    }
    // The latch `SharedDevice::apply` filled for these allocs: every row is now
    // `AlreadyBorn` (or `Emulated`), and draining it here keeps the device in the state the
    // shim's tail would leave it in.
    let _ = dev.run_pending_channel_births(&[]);
    out
}
