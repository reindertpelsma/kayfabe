//! # nvkvm-tests — conformance harness helpers
//!
//! Shared scenario builders for the integration tests under `tests/`. The whole
//! suite runs at unit speed with **no GPU, no hypervisor, no OS** — the payoff of
//! the pure-state-machine core (`mode2_rust_testing_strategy.md` §1).
//!
//! The centerpiece is [`Scenario`]: a small DSL for scripting a guest's RM protocol
//! as abstract `RmEvent`s, with helpers to build the exact shapes the design docs
//! name (a compute process, a UVM dup, two processes with identical VAs/handles).

use nvkvm_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb};
use nvkvm_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use nvkvm_mocks::{MockArch, mock_classes as mc};

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
    /// index (`deviceInstance`) — `None` = single-GPU default (routes to `GpuId::ZERO`);
    /// `Some(i)` = the multi-GPU target `GpuId(i)`.
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
                device_instance,
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
            facts: AllocFacts::default(),
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
    pub gr_vchid: nvkvm_arch::ids::VChid,
    /// CE channel handle.
    pub ce_channel: HObject,
    /// CE channel's vChid.
    pub ce_vchid: nvkvm_arch::ids::VChid,
}

/// The **#14 shape**: identical guest handle values (both procs' GR channel is
/// `0x5c000019`, etc. — round 1), which the tests pair with identical guest VAs and
/// distinct PDBs. vChids differ (E0: fresh per channel-create, zero collisions).
#[must_use]
pub fn identical_handles(gr_vchid: u16, ce_vchid: u16) -> ProcessHandles {
    use nvkvm_arch::ids::VChid;
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
