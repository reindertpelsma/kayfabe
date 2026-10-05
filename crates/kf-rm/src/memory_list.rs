//! Default-off SYSRAM MemoryList registration: a typed view of existing guest RAM.
//! There is no new backing allocation, host RM action, scheduler or completion.

use crate::rmgraph::{AllocFacts, NodeKey, ResourceKey, RmEvent, RmGraph};
use crate::rmrpc::ObjectsRefusal;
use kf_arch::{
    ObjectKind,
    ids::{ClassId, HClient, HObject},
};
use std::sync::Arc;

/// A device DMA range under the realize-time no-vIOMMU posture; never a host address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamSpan {
    /// Guest page base.
    pub base: u64,
    /// Whole page-rounded byte extent.
    pub length: u64,
}
/// Maximum copied bytes per future CPU access; independent of guest descriptor size.
pub const MAX_ACCESS: usize = 4096;

/// Trusted RAM authority supplied by the VMM. Registration never touches contents.
/// Implementations must validate writable real RAM, revoke generations on topology
/// changes, and hold the registration guard across every bounded copy. A generation
/// of zero is invalid and exhausted counters must never wrap or revive old tokens.
pub trait GuestRamAuthority: core::fmt::Debug + Send + Sync {
    /// Validate the whole span and return its current nonzero topology generation.
    fn validate(&self, span: RamSpan) -> Option<u64>;
    /// Read a bounded offset in the same currently registered span/generation.
    fn read(&self, span: RamSpan, generation: u64, offset: u64, bytes: &mut [u8]) -> bool;
    /// Write a bounded offset in the same currently registered span/generation.
    fn write(&self, span: RamSpan, generation: u64, offset: u64, bytes: &[u8]) -> bool;
}

/// A registered descriptor; only this module can mint its facts after RAM validation.
/// No raw region, HVA, memfd offset, or independent handle lifetime is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredMemory {
    declaration: kf_abi::memory_list::Declaration,
    generation: u64,
    parent: ResourceKey,
    device: ResourceKey,
    parent_lifetime: crate::rmgraph::ResourceLifetime,
    device_lifetime: crate::rmgraph::ResourceLifetime,
}
impl RegisteredMemory {
    fn span(self) -> RamSpan {
        RamSpan {
            base: self.declaration.page_base,
            length: self.declaration.span,
        }
    }
    fn access(self, offset: u64, bytes: usize) -> Option<u64> {
        if bytes > MAX_ACCESS || offset.checked_add(bytes as u64)? > self.declaration.length {
            return None;
        }
        offset.checked_add(u64::from(self.declaration.adjustment))
    }
}

fn refuse(what: &'static str) -> ObjectsRefusal {
    ObjectsRefusal::NotModelled { what }
}

pub(crate) fn allocate(
    graph: &mut RmGraph,
    ram: Option<&Arc<dyn GuestRamAuthority>>,
    d: kf_abi::memory_list::Declaration,
) -> Result<RmEvent, ObjectsRefusal> {
    let ram = ram.ok_or_else(|| refuse("memory-list: no guest RAM authority"))?;
    let client = HClient(d.client);
    let parent_key = NodeKey::new(client, HObject(d.parent));
    let reject_parent = || refuse("memory-list: missing/wrong/foreign parent");
    let parent = *graph.allocated_node(parent_key).ok_or_else(reject_parent)?;
    let dev = match parent.kind {
        ObjectKind::Device => parent,
        ObjectKind::Subdevice => *graph
            .allocated_node(NodeKey::new(client, parent.parent))
            .filter(|n| n.kind == ObjectKind::Device)
            .ok_or_else(reject_parent)?,
        _ => return Err(reject_parent()),
    };
    let root = NodeKey::new(client, dev.parent);
    if !graph
        .allocated_node(root)
        .is_some_and(|n| n.kind == ObjectKind::Client && n.key == root)
        || graph.gpu_of(parent_key).is_none()
        || graph.gpu_of(parent_key) != graph.gpu_of(dev.key)
    {
        return Err(reject_parent());
    }
    let parent_lifetime = graph.lifetime_of(parent_key).ok_or_else(reject_parent)?;
    let device_lifetime = graph.lifetime_of(dev.key).ok_or_else(reject_parent)?;
    let span = RamSpan {
        base: d.page_base,
        length: d.span,
    };
    let generation = ram
        .validate(span)
        .filter(|g| *g != 0)
        .ok_or_else(|| refuse("memory-list: range is not current writable guest RAM"))?;
    let event = RmEvent::Alloc {
        client,
        parent: HObject(d.parent),
        handle: HObject(d.handle),
        class: ClassId(d.class),
        facts: AllocFacts {
            guest_memory_list: Some(RegisteredMemory {
                declaration: d,
                generation,
                parent: parent.id(),
                device: dev.id(),
                parent_lifetime,
                device_lifetime,
            }),
            ..Default::default()
        },
    };
    graph.apply(event).map_err(ObjectsRefusal::Graph)?;
    Ok(event)
}

fn lookup(graph: &RmGraph, key: NodeKey) -> Option<RegisteredMemory> {
    let n = graph.node(key)?;
    if n.kind != ObjectKind::GuestMemoryList {
        return None;
    }
    let memory = n.facts.guest_memory_list?;
    // A dup can keep a resource alive, but it cannot resurrect its destroyed parent
    // or substitute a new Device at a recycled numeric handle.
    if !graph.lifetime_is_live(memory.parent_lifetime)
        || !graph.lifetime_is_live(memory.device_lifetime)
    {
        return None;
    }
    Some(memory)
}

/// Future-consumer seam: resolve the LIVE graph handle then revalidate bounded RAM
/// access. No currently admitted RPC consumer calls this method.
pub(crate) fn read(
    graph: &RmGraph,
    ram: Option<&Arc<dyn GuestRamAuthority>>,
    key: NodeKey,
    offset: u64,
    bytes: &mut [u8],
) -> bool {
    let Some((ram, d)) = ram.zip(lookup(graph, key)) else {
        return false;
    };
    let Some(at) = d.access(offset, bytes.len()) else {
        return false;
    };
    ram.read(d.span(), d.generation, at, bytes)
}
/// The write counterpart, with the same live graph and RAM generation checks.
pub(crate) fn write(
    graph: &RmGraph,
    ram: Option<&Arc<dyn GuestRamAuthority>>,
    key: NodeKey,
    offset: u64,
    bytes: &[u8],
) -> bool {
    let Some((ram, d)) = ram.zip(lookup(graph, key)) else {
        return false;
    };
    let Some(at) = d.access(offset, bytes.len()) else {
        return false;
    };
    ram.write(d.span(), d.generation, at, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rmrpc::{GraphObjects, RmObjects};
    use kf_arch::ClientKind;
    use std::sync::Mutex;
    #[derive(Debug)]
    struct Ram(Mutex<(u64, Vec<u8>, usize)>);
    impl GuestRamAuthority for Ram {
        fn validate(&self, s: RamSpan) -> Option<u64> {
            let g = self.0.lock().unwrap();
            (s.base >= 0x2000
                && s.length > 0
                && s.base.checked_add(s.length).is_some_and(|e| e <= 0xa000))
            .then_some(g.0)
        }
        fn read(&self, s: RamSpan, generation: u64, offset: u64, bytes: &mut [u8]) -> bool {
            let mut g = self.0.lock().unwrap();
            g.2 += 1;
            if g.0 != generation
                || bytes.len() > MAX_ACCESS
                || !offset
                    .checked_add(bytes.len() as u64)
                    .is_some_and(|e| e <= s.length)
            {
                return false;
            }
            let at = (s.base - 0x2000 + offset) as usize;
            bytes.copy_from_slice(&g.1[at..at + bytes.len()]);
            true
        }
        fn write(&self, s: RamSpan, generation: u64, offset: u64, bytes: &[u8]) -> bool {
            let mut g = self.0.lock().unwrap();
            g.2 += 1;
            if g.0 != generation
                || bytes.len() > MAX_ACCESS
                || !offset
                    .checked_add(bytes.len() as u64)
                    .is_some_and(|e| e <= s.length)
            {
                return false;
            }
            let at = (s.base - 0x2000 + offset) as usize;
            g.1[at..at + bytes.len()].copy_from_slice(bytes);
            true
        }
    }
    fn key(client: u32, handle: u32) -> NodeKey {
        NodeKey::new(HClient(client), HObject(handle))
    }
    fn event(client: u32, parent: u32, handle: u32, class: u32, facts: AllocFacts) -> RmEvent {
        RmEvent::Alloc {
            client: HClient(client),
            parent: HObject(parent),
            handle: HObject(handle),
            class: ClassId(class),
            facts,
        }
    }
    fn namespace(graph: &mut RmGraph, c: u32) {
        for (parent, handle, class, facts) in [
            (
                c,
                c,
                0x41,
                AllocFacts {
                    client_kind: Some(ClientKind::User { pid: c }),
                    ..Default::default()
                },
            ),
            (
                c,
                2,
                0x80,
                AllocFacts {
                    device_instance: Some(0),
                    ..Default::default()
                },
            ),
            (2, 3, 0x2080, AllocFacts::default()),
        ] {
            graph.apply(event(c, parent, handle, class, facts)).unwrap();
        }
    }
    fn fixture(
        family: kf_chip::Family,
    ) -> (GraphObjects, Arc<Ram>, kf_abi::memory_list::Declaration) {
        let ram = Arc::new(Ram(Mutex::new((1, vec![0x5a; 0x8000], 0))));
        let mut objects = GraphObjects::new(family).with_guest_ram(ram.clone());
        namespace(&mut objects.graph, 1);
        let d = kf_abi::memory_list::Declaration {
            client: 1,
            parent: 2,
            handle: 4,
            class: 0x81,
            flags: 0x48002000,
            page_base: 0x2000,
            span: 0x8000,
            adjustment: 7,
            length: 0x7000,
        };
        (objects, ram, d)
    }
    #[test]
    fn memory_list_registration_reads_nothing_and_access_has_logical_bounds() {
        for family in [
            kf_chip::Family::Turing,
            kf_chip::Family::Ampere,
            kf_chip::Family::Ada,
            kf_chip::Family::Hopper,
            kf_chip::Family::Blackwell,
        ] {
            let (mut objects, ram, d) = fixture(family);
            objects.memory_list(d).unwrap();
            assert_eq!(ram.0.lock().unwrap().2, 0);
            assert_eq!(
                objects.graph.node(key(1, 4)).unwrap().kind,
                ObjectKind::GuestMemoryList
            );
            assert!(objects.write_memory_list(key(1, 4), 0, &[1, 2]));
            assert_eq!(
                &ram.0.lock().unwrap().1[..10],
                &[0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 1, 2, 0x5a]
            );
            assert!(objects.write_memory_list(key(1, 4), d.length - 1, &[3]));
            assert!(!objects.write_memory_list(key(1, 4), d.length, &[4]));
            assert!(!objects.write_memory_list(key(1, 4), u64::MAX, &[4]));
            assert!(!objects.write_memory_list(key(1, 4), 0, &vec![0; MAX_ACCESS + 1]));
            let mut bytes = [0; 2];
            assert!(objects.read_memory_list(key(1, 4), 0, &mut bytes));
            assert_eq!(bytes, [1, 2]);
            assert_eq!(ram.0.lock().unwrap().1[(7 + d.length) as usize], 0x5a);
        }
    }
    #[test]
    fn memory_list_device_and_subdevice_free_and_epoch_fail_closed() {
        for parent in [2, 3] {
            for free in [4, parent, 1] {
                let (mut objects, ram, mut d) = fixture(kf_chip::Family::Ampere);
                d.parent = parent;
                objects.memory_list(d).unwrap();
                let n = *objects.graph.node(key(1, 4)).unwrap();
                objects.memory_list(d).unwrap();
                assert_eq!(objects.graph.node(key(1, 4)), Some(&n));
                for changed in [
                    kf_abi::memory_list::Declaration { flags: 0, ..d },
                    kf_abi::memory_list::Declaration { length: 1, ..d },
                    kf_abi::memory_list::Declaration {
                        page_base: 0x3000,
                        span: 0x7000,
                        ..d
                    },
                ] {
                    assert!(objects.memory_list(changed).is_err());
                }
                objects
                    .graph
                    .apply(RmEvent::Free {
                        client: HClient(1),
                        handle: HObject(free),
                    })
                    .unwrap();
                assert!(!objects.write_memory_list(key(1, 4), 0, &[8]));
                assert_eq!(ram.0.lock().unwrap().2, 0);
            }
        }
        let (mut objects, ram, d) = fixture(kf_chip::Family::Ampere);
        objects.memory_list(d).unwrap();
        ram.0.lock().unwrap().0 += 1;
        assert!(!objects.write_memory_list(key(1, 4), 0, &[8]));
        assert!(
            objects.memory_list(d).is_err(),
            "no silent topology rebind on retry"
        );
    }
    #[test]
    fn memory_list_wrong_parent_alias_range_and_other_class_leave_graph_unchanged() {
        let (mut objects, _, d) = fixture(kf_chip::Family::Ampere);
        for parent in [0, 1, 99] {
            assert!(
                objects
                    .memory_list(kf_abi::memory_list::Declaration { parent, ..d })
                    .is_err()
            );
        }
        for (page_base, span) in [(0, 4096), (0x2000, 0x8001), (u64::MAX, 1), (0x2000, 0)] {
            assert!(
                objects
                    .memory_list(kf_abi::memory_list::Declaration {
                        page_base,
                        span,
                        ..d
                    })
                    .is_err()
            );
        }
        namespace(&mut objects.graph, 10);
        objects
            .graph
            .apply(RmEvent::Dup {
                src: key(10, 2),
                dst: key(1, 7),
            })
            .unwrap();
        assert!(
            objects
                .memory_list(kf_abi::memory_list::Declaration { parent: 7, ..d })
                .is_err()
        );
        assert_eq!(objects.graph.nodes().count(), 6);
        objects
            .graph
            .apply(event(1, 2, 4, 0x81, AllocFacts::default()))
            .unwrap();
        assert!(objects.memory_list(d).is_err());
        assert!(
            !objects.write_memory_list(key(1, 4), 0, &[1]),
            "fn103-like declaration has no RAM authority facts"
        );
    }
    #[test]
    fn memory_list_alias_and_recycled_parent_incarnations_do_not_revive_access() {
        let (mut objects, _, d) = fixture(kf_chip::Family::Ampere);
        namespace(&mut objects.graph, 10);
        objects.memory_list(d).unwrap();
        objects
            .graph
            .apply(RmEvent::Dup {
                src: key(1, 4),
                dst: key(10, 8),
            })
            .unwrap();
        objects
            .graph
            .apply(RmEvent::Free {
                client: HClient(1),
                handle: HObject(4),
            })
            .unwrap();
        assert!(!objects.write_memory_list(key(1, 4), 0, &[1]));
        assert!(objects.write_memory_list(key(10, 8), 0, &[1]));
        objects
            .graph
            .apply(RmEvent::Free {
                client: HClient(1),
                handle: HObject(2),
            })
            .unwrap();
        assert!(!objects.write_memory_list(key(10, 8), 0, &[1]));
        objects
            .graph
            .apply(event(
                1,
                1,
                2,
                0x80,
                AllocFacts {
                    device_instance: Some(0),
                    ..Default::default()
                },
            ))
            .unwrap();
        assert!(
            !objects.write_memory_list(key(10, 8), 0, &[1]),
            "new parent cannot revive old resource identity"
        );
        objects
            .graph
            .apply(RmEvent::Free {
                client: HClient(10),
                handle: HObject(8),
            })
            .unwrap();
        assert!(!objects.write_memory_list(key(10, 8), 0, &[1]));
    }
    #[test]
    fn memory_list_no_ram_authority_cannot_create_a_descriptor() {
        let (_, _, d) = fixture(kf_chip::Family::Ampere);
        let mut objects = GraphObjects::new(kf_chip::Family::Ampere);
        namespace(&mut objects.graph, 1);
        assert!(objects.memory_list(d).is_err());
        assert!(objects.graph.node(key(1, 4)).is_none());
    }

    #[test]
    fn memory_list_quota_failure_has_no_resource_or_access_side_effect() {
        let (mut objects, ram, d) = fixture(kf_chip::Family::Ampere);
        for handle in 5..=crate::rmgraph::MAX_LIVE_HANDLES as u32 + 1 {
            objects
                .graph
                .apply(event(1, 2, handle, 0x1234, AllocFacts::default()))
                .unwrap();
        }
        assert!(objects.memory_list(d).is_err());
        assert!(objects.graph.node(key(1, 4)).is_none());
        assert_eq!(ram.0.lock().unwrap().2, 0);
    }
}
