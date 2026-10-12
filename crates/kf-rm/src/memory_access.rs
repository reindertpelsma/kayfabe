//! Worker access to live, validated MemoryList descriptors.
//!
//! The live RM graph remains the only handle/lifetime authority. No descriptor
//! cache or captured address table is maintained. Reads/writes are bounded by
//! MAX_ACCESS and the acquired logical span, serialized with graph mutation and
//! driver-chain replacement. Worker lock contention returns Busy; no RM verb,
//! GPU work or completion is executed here. Linux gains no GPU mapping.
//! Callers must not retain shared channel metadata guards (e.g. pt) across
//! Access calls or retry Busy under them: deferred-API validation takes graph
//! then channel. A private worker-state mutex is allowed only when no graph,
//! RAM, channel-metadata or drainer path acquires it; Busy immediately unwinds.

use crate::memory_list::{MAX_ACCESS, lookup};
use crate::rmgraph::{NodeKey, ResourceLifetime, RmEvent};
use crate::rmrpc::{GraphObjects, ObjectsRefusal, PageDirStatement, RmObjects};
use kf_arch::ids::{HClient, HObject};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak};

type Graph = Mutex<GraphObjects>;

/// Typed worker refusals. Busy is retryable; no refusal grants reuse credit.
pub use crate::memory_list::AccessError as Refusal;

fn try_lock<T>(lock: &Mutex<T>) -> Result<MutexGuard<'_, T>, Refusal> {
    lock.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => Refusal::Busy,
        TryLockError::Poisoned(_) => Refusal::Poisoned,
    })
}

/// One VM's current object-seat graph. Weak ownership cannot extend a chain's life.
#[derive(Debug, Default)]
pub struct Access {
    current: Mutex<Weak<Graph>>,
}
impl Access {
    /// Acquire bounded logical bytes of a live, RAM-backed MemoryList handle.
    /// The caller must separately validate channel ownership, USERD alignment
    /// and stream-position migration semantics. Acquisition reads no contents.
    pub fn lease(
        self: &Arc<Self>,
        client: u32,
        handle: u32,
        offset: u64,
        length: u64,
    ) -> Result<Lease, Refusal> {
        if length == 0 || length > MAX_ACCESS as u64 || offset.checked_add(length).is_none() {
            return Err(Refusal::Bounds);
        }
        let current = try_lock(&self.current)?;
        let graph = current.upgrade().ok_or(Refusal::Gone)?;
        let objects = try_lock(&graph)?;
        let key = NodeKey::new(HClient(client), HObject(handle));
        // Initial USERD adapter admits original allocations only. Resource
        // identity alone would let a free/re-dup alias resurrect an old handle.
        objects.graph.allocated_node(key).ok_or(Refusal::Gone)?;
        let descriptor = lookup(&objects.graph, key).ok_or(Refusal::Gone)?;
        descriptor
            .access(offset, length as usize)
            .ok_or(Refusal::Bounds)?;
        let lifetime = objects.graph.lifetime_of(key).ok_or(Refusal::Gone)?;
        Ok(Lease {
            access: Arc::downgrade(self),
            graph: Arc::downgrade(&graph),
            key,
            lifetime,
            offset,
            length,
        })
    }
}

/// Exact resource and graph identity plus a bounded logical subspan.
#[derive(Debug, Clone)]
pub struct Lease {
    access: Weak<Access>,
    graph: Weak<Graph>,
    key: NodeKey,
    lifetime: ResourceLifetime,
    offset: u64,
    length: u64,
}
impl Lease {
    fn at(&self, offset: u64, bytes: usize) -> Result<u64, Refusal> {
        if bytes > MAX_ACCESS
            || offset
                .checked_add(bytes as u64)
                .is_none_or(|end| end > self.length)
        {
            return Err(Refusal::Bounds);
        }
        self.offset.checked_add(offset).ok_or(Refusal::Bounds)
    }

    fn with_graph<T>(
        &self,
        f: impl FnOnce(&GraphObjects) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        let access = self.access.upgrade().ok_or(Refusal::Gone)?;
        let current = try_lock(&access.current)?;
        if !Weak::ptr_eq(&current, &self.graph) {
            return Err(Refusal::Gone);
        }
        let graph = current.upgrade().ok_or(Refusal::Gone)?;
        let objects = try_lock(&graph)?;
        if objects.graph.allocated_node(self.key).is_none()
            || objects.graph.lifetime_of(self.key) != Some(self.lifetime)
            || lookup(&objects.graph, self.key).is_none()
        {
            return Err(Refusal::Gone);
        }
        // Both guards remain live through the bounded CPU copy. Free/reuse or
        // replacing the current graph cannot interleave with that copy.
        f(&objects)
    }

    /// Cross-check a channel's already-resolved physical USERD descriptor against
    /// this exact logical span. No pointer escapes and current RAM authority is
    /// checked without reading contents. Subsequent accesses still revalidate.
    pub fn matches_ram_location(&self, base: u64) -> Result<bool, Refusal> {
        self.with_graph(|objects| {
            let descriptor = lookup(&objects.graph, self.key).ok_or(Refusal::Gone)?;
            let length = usize::try_from(self.length).map_err(|_| Refusal::Bounds)?;
            let matches = descriptor.ram_location(self.offset, length) == Some(base);
            objects.try_read_memory_list(self.key, self.offset, &mut [])?;
            Ok(matches)
        })
    }

    /// Keep exact graph/resource authority through a bounded CPU publication.
    /// Current RAM topology is checked first; the caller must retain its birth
    /// topology and source guards through publication too. One global order:
    /// graph/access -> channel metadata -> source rows -> RAM -> private PUT.
    /// No RM verb, wait/retry, nested Access call or GPU-work completion is
    /// allowed in the callback. The authored doorbell follows after all guards.
    /// This checks identity, not channel ownership or cursor/migration semantics.
    pub fn admit<T>(&self, publish: impl FnOnce() -> T) -> Result<T, Refusal> {
        self.with_graph(|objects| {
            objects.try_read_memory_list(self.key, self.offset, &mut [])?;
            Ok(publish())
        })
    }

    /// Read only within this exact live lease and current RAM generation.
    pub fn read(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Refusal> {
        let at = self.at(offset, bytes.len())?;
        self.with_graph(|objects| objects.try_read_memory_list(self.key, at, bytes))
    }
    /// Write only within this exact live lease and current RAM generation.
    /// This is a bounded guest-memory access, never a GPU completion primitive.
    pub fn write(&self, offset: u64, bytes: &[u8]) -> Result<(), Refusal> {
        let at = self.at(offset, bytes.len())?;
        self.with_graph(|objects| objects.try_write_memory_list(self.key, at, bytes))
    }
}

/// Object-seat wrapper sharing its live graph with the worker access seam.
/// GraphObjects is host-free; no method may acquire a host RM/API lock here.
pub struct SharedObjects {
    graph: Arc<Graph>,
}
impl SharedObjects {
    /// Install a fresh graph. Existing leases fail against its distinct Arc
    /// identity even if resource ordinals and all numeric handles restart.
    pub fn new(objects: GraphObjects, access: &Arc<Access>) -> Self {
        let graph = Arc::new(Mutex::new(objects));
        if let Ok(mut current) = access.current.lock() {
            *current = Arc::downgrade(&graph);
        }
        // Poisoned access remains poisoned: workers refuse, rather than keep
        // using a prior graph. RPC graph operations retain their normal policy.
        Self { graph }
    }

    fn objects(&self) -> Result<MutexGuard<'_, GraphObjects>, ObjectsRefusal> {
        self.graph.lock().map_err(|_| ObjectsRefusal::NotModelled {
            what: "shared object graph poisoned",
        })
    }
}
impl RmObjects for SharedObjects {
    fn memory_list(
        &mut self,
        d: kf_abi::memory_list::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        self.objects()?.memory_list(d)
    }
    fn software_runlist_probe(
        &mut self,
        d: crate::sw_runlist_probe::Declaration,
    ) -> Result<RmEvent, ObjectsRefusal> {
        self.objects()?.software_runlist_probe(d)
    }
    fn apply(&mut self, ev: RmEvent, params: &[u8]) -> Result<(), ObjectsRefusal> {
        self.objects()?.apply(ev, params)
    }
    fn page_dir(&mut self, st: PageDirStatement) -> Result<(), ObjectsRefusal> {
        self.objects()?.page_dir(st)
    }
    fn deferred_api(
        &mut self,
        abi: &kf_abi::defapi::DefApiAbi,
        client: u32,
        object: u32,
        ctl: kf_abi::defapi::Control,
        params: &[u8],
    ) -> Option<u32> {
        match self.objects() {
            Ok(mut objects) => objects.deferred_api(abi, client, object, ctl, params),
            Err(_) => Some(kf_abi::NV_ERR_INVALID_STATE),
        }
    }
    fn is_subdevice(&self, client: u32, object: u32) -> bool {
        self.objects()
            .is_ok_and(|objects| objects.is_subdevice(client, object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_contention_is_busy_instead_of_waiting_for_object_seat() {
        let access = Arc::new(Access::default());
        let shared = SharedObjects::new(GraphObjects::new(kf_chip::Family::Ada), &access);
        let graph_guard = shared.graph.lock().unwrap();
        assert_eq!(access.lease(1, 4, 0, 4).err(), Some(Refusal::Busy));
        drop(graph_guard);
        let access_guard = access.current.lock().unwrap();
        assert_eq!(access.lease(1, 4, 0, 4).err(), Some(Refusal::Busy));
        drop(access_guard);
        assert_eq!(access.lease(1, 4, 0, 4).err(), Some(Refusal::Gone));
    }
}
