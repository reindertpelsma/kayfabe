//! # kayfabe-tests — conformance harness helpers
//!
//! Shared scenario builders for the integration tests under `tests/`. The whole
//! suite runs at unit speed with **no GPU, no hypervisor, no OS** — the payoff of
//! the pure-state-machine core (`mode2_rust_testing_strategy.md` §1).
//!
//! The centerpiece is [`Scenario`]: a small DSL for scripting a guest's RM protocol
//! as abstract `RmEvent`s, with helpers to build the exact shapes the design docs
//! name (a compute process, a UVM dup, two processes with identical VAs/handles).

use kayfabe_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use kayfabe_mocks::{MockArch, mock_classes as mc};
use std::sync::OnceLock;

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
            facts: AllocFacts::default(),
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

    /// Model UVM aliasing a compute VASpace into its own client via `DUP_OBJECT`,
    /// plus a UVM VASpace of its own (the "one Proc, several Vas" case). Returns the
    /// UVM client's own VASpace node key.
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
        self.push(RmEvent::Alloc {
            client: uvm_client,
            parent: uvm_root,
            handle: uvm_root,
            class: mc::CLIENT,
            facts: AllocFacts::default(),
        });
        self.push(RmEvent::Alloc {
            client: uvm_client,
            parent: uvm_root,
            handle: uvm_dev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        });
        self.push(RmEvent::Alloc {
            client: uvm_client,
            parent: uvm_dev,
            handle: uvm_vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        });
        self.push(RmEvent::SetPageDir {
            client: uvm_client,
            vaspace: uvm_vas,
            pdb: uvm_pdb,
        });
        // The cross-client transfer edge: alias the compute VASpace into UVM's client.
        self.push(RmEvent::Dup {
            src: compute_vas,
            dst: NodeKey::new(uvm_client, alias_handle),
        });
        NodeKey::new(uvm_client, uvm_vas)
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
        facts: AllocFacts::default(),
    }
}
