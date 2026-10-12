//! Worker-only restricted USER ring relay. No vCPU/drainer caller may run a
//! turn: source-view preparation can call RM. Native USER privileges and own
//! mirrored mappings enforce access; scanning is never a completion authority.

use super::*;
use crate::mem::userread::{self, Mapping, Snapshot};
use kf_chan::user_scan::{Fetch, Scan, Trigger};

/// Retry means no native publication/action was attempted. An uncertain write or
/// native doorbell error is always terminal; issued credits are never rolled back.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Retry,
    Refused(String),
}
impl From<String> for Failure {
    fn from(v: String) -> Self {
        Self::Refused(v)
    }
}
impl From<&str> for Failure {
    fn from(v: &str) -> Self {
        Self::Refused(v.into())
    }
}
type Turn<T> = Result<T, Failure>;
fn locked<T>(v: std::sync::TryLockResult<T>, name: &'static str) -> Turn<T> {
    v.map_err(|e| match e {
        std::sync::TryLockError::WouldBlock => Failure::Retry,
        std::sync::TryLockError::Poisoned(_) => name.into(),
    })
}
fn source_error(e: userread::Refusal, phase: &'static str) -> Failure {
    match e {
        userread::Refusal::Busy | userread::Refusal::Changed => Failure::Retry,
        _ => format!("{phase}: {e:?}").into(),
    }
}
fn access_error(e: kf_rm::memory_list::AccessError, phase: &'static str) -> Failure {
    match e {
        kf_rm::memory_list::AccessError::Busy => Failure::Retry,
        _ => format!("{phase}: {e:?}").into(),
    }
}

/// One lifetime selected before any private native publication. Host numbering
/// is admitted only for the first ENG_SW child in this vertical slice.
#[derive(Clone)]
pub(super) struct Software {
    pub guest: u32,
    number: u16,
    pub live: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
}
impl Software {
    fn matches(&self, value: u32) -> bool {
        value == crate::defapi::NV50_DEFERRED_API_CLASS || value == u32::from(self.number)
    }
}
fn same_software(a: &Option<Software>, b: &Option<Software>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(&a.live, &b.live),
        _ => false,
    }
}
/// Classify and patch using ONE immutable channel-owned identity snapshot.
/// The later prefix scan uses the same snapshot; live admission is rechecked
/// before publication and before independently planned software actions.
fn stage_plan(scan: &Scan, software: &Option<Software>, words: &[u32]) -> Turn<(Vec<u32>, bool)> {
    let mut patched = Vec::new();
    patched
        .try_reserve_exact(words.len())
        .map_err(|_| "USER_RING_SCAN_ALLOC")?;
    patched.extend_from_slice(words);
    let mut scan = scan.clone();
    let mut at = 0usize;
    let mut rare = false;
    while let Some(t) = scan
        .segment(words.get(at..).ok_or("USER_RING_SCAN_BOUND")?, |v| {
            software.as_ref().is_some_and(|sw| sw.matches(v))
        })
        .map_err(|e| format!("USER_RING_SCAN {e:?}"))?
    {
        rare = true;
        let start = at.checked_add(t.start).ok_or("USER_RING_SCAN_BOUND")?;
        at = at.checked_add(t.end).ok_or("USER_RING_SCAN_BOUND")?;
        patched
            .get_mut(start..at)
            .ok_or("USER_RING_SCAN_BOUND")?
            .fill(0);
    }
    if rare && !scan.at_boundary() {
        return Err("USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED".into());
    }
    Ok((patched, rare))
}
/// Native fetch credit and private staging retirement are separate. The first
/// representing descriptor carries the guest ring receipt, including a prefix
/// of zero bytes; no later suffix publication repeats it.
fn source_receipt(staged: bool, first: bool, generation: u64, get: u32) -> Option<(u64, u32)> {
    (!staged && first).then_some((generation, get))
}
fn suffix_after_trigger(at: usize, trigger: Trigger, words: usize) -> Turn<(usize, bool)> {
    let next = at
        .checked_add(trigger.end)
        .filter(|n| *n <= words)
        .ok_or("USER_RING_SUFFIX_BOUND")?;
    Ok((next, next == words))
}

fn software_snapshot(me: &ChanPlane, r: &userrelay::Record) -> Turn<Option<Software>> {
    let pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
    let t = pt
        .get(&r.key)
        .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
        .ok_or("USER_RING_ALLOCATION_GONE")?;
    let Some(sw) = &t.user_defapi else {
        return Ok(None);
    };
    if !sw.live.load(Ordering::Acquire) {
        return Err("USER_RING_SOFTWARE_OBJECT_GONE".into());
    }
    if !sw.ready.load(Ordering::Acquire) {
        return Err(Failure::Retry);
    }
    Ok(Some(sw.clone()))
}

pub(super) struct Pump {
    cache: crate::userstore::Cache,
    ram_generation: u64,
    generation: u64,
    source_admitted: bool,
    cursor: u32,
    scan: Scan,
    fetch: Fetch,
    segment: Option<Segment>,
    guest_get: Option<(u64, u32)>,
    pub tick: bool,
    again: bool,
}

struct Segment {
    ring: Option<Mapping<'static>>,
    software: Option<Software>,
    source: Snapshot<'static>,
    pb: Snapshot<'static>,
    entry: u64,
    words: Vec<u32>,
    patched: Vec<u32>,
    at: usize,
    generation: u64,
    guest_get: u32,
    rare: bool,
    staged: bool,
    wait: Option<Wait>,
}
struct Wait {
    witness: u64,
    trigger: Option<Trigger>,
    action: Option<PendingAction>,
}
enum PendingAction {
    Invalidate {
        ticket: u64,
        plan: crate::defapi::Planned,
    },
    Context {
        cell: SwCell,
        plan: crate::defapi::Planned,
    },
}

impl Pump {
    pub(super) fn new(me: &ChanPlane) -> Result<Self, String> {
        Ok(Self {
            cache: crate::userstore::Cache::new(
                me.rm,
                me.store,
                store_read_bound(me.layout.carve()),
            )
            .map_err(|e| format!("USER_RING_CACHE {e:?}"))?,
            ram_generation: me.ram.try_generation().ok_or("USER_RING_RAM_TOPOLOGY")?,
            generation: 1,
            source_admitted: false,
            cursor: 0,
            scan: Scan::default(),
            fetch: Fetch::new(kf_chan::user_storage::ENTRIES).map_err(str::to_owned)?,
            segment: None,
            guest_get: None,
            tick: false,
            again: false,
        })
    }
    pub(super) fn release(&mut self) -> Result<(), String> {
        self.cache
            .release()
            .map_err(|e| format!("USER_RING_SOURCE_CLEANUP {e:?}"))
    }

    fn snapshot(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        va: u64,
        bytes: usize,
    ) -> Turn<Snapshot<'static>> {
        userread::snapshot(
            &r.mirror.rows,
            &r.mirror.log,
            me.ram,
            self.ram_generation,
            va,
            bytes,
            &mut self.cache,
        )
        .map_err(|e| source_error(e, "USER_RING_SOURCE"))
    }

    fn turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Turn<bool> {
        self.tick = false;
        self.again = false;
        let native = r.chan.ok_or("USER_RING_NATIVE_ABSENT")?;
        let get = r
            .lease
            .storage
            .lock()
            .map_err(|_| "USER_RING_STORAGE_POISONED")?
            .gpu_get()?;
        for (generation, get) in self.fetch.observe(get).map_err(str::to_owned)? {
            if generation == self.generation {
                self.guest_get = Some((generation, get));
            }
        }
        if let Some((generation, get)) = self.guest_get {
            if generation == self.generation {
                // Logical memory access revalidates Free/reuse and RAM topology.
                match r.userd.write(
                    me,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_GET,
                    &get.to_le_bytes(),
                ) {
                    Ok(()) => self.guest_get = None,
                    Err(kf_rm::memory_list::AccessError::Busy) => {
                        self.tick = true;
                        return Ok(false);
                    }
                    Err(e) => return Err(format!("USER_RING_GET_WRITE {e:?}").into()),
                }
            } else {
                self.guest_get = None;
            }
        }
        self.tick = self.fetch.fetched() != self.fetch.issued();
        if self.segment.is_none() {
            if me.inbox.walk_busy() {
                me.inbox.wait_walk(r.idx);
                return Ok(false);
            }
            let mut put = [0u8; 4];
            r.userd
                .read(
                    me,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_PUT,
                    &mut put,
                )
                .map_err(|e| access_error(e, "USER_RING_PUT_READ"))?;
            let put = u32::from_le_bytes(put);
            if put >= r.source.1 {
                return Err("USER_RING_PUT_BOUND".into());
            }
            if put == self.cursor {
                return Ok(false);
            }
            // Declaration admission only, not a pin. Native never fetches the
            // guest ring; EACH current GP/PB still has fresh exact source guards.
            let ring = if self.source_admitted {
                None
            } else {
                Some(
                    userread::mapped(
                        &r.mirror.rows,
                        &r.mirror.log,
                        me.ram,
                        self.ram_generation,
                        r.source.0,
                        u64::from(r.source.1) * 8,
                        store_read_bound(me.layout.carve()),
                    )
                    .map_err(|e| source_error(e, "USER_RING_WHOLE_RING"))?,
                )
            };
            let va = u64::from(self.cursor)
                .checked_mul(8)
                .and_then(|off| r.source.0.checked_add(off))
                .ok_or("USER_RING_GP_ADDRESS")?;
            let source = self.snapshot(me, r, va, 8)?;
            let entry = u64::from_le_bytes(
                source
                    .bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "USER_RING_GP_SNAPSHOT_SIZE")?,
            );
            let descriptor = kf_chan::user_stream::inspect_entry(entry, userread::MAX_BYTES as u64)
                .map_err(|e| {
                    format!(
                        "USER_RING_GP_FORMAT {e:?} entry={entry:#018x} source_va={va:#x} cursor={:#x} put={put:#x} entries={}",
                        self.cursor, r.source.1
                    )
                })?;
            let guest_get =
                self.cursor.checked_add(1).ok_or("USER_RING_CURSOR")? & (r.source.1 - 1);
            if matches!(descriptor, kf_chan::user_stream::Source::Nop) {
                if self.fetch.available() == 0 {
                    self.tick = true;
                    return Ok(false);
                }
                self.publish(
                    me,
                    r,
                    None,
                    ring.as_ref(),
                    Some(&source),
                    None,
                    |pump, storage| {
                        let (slot, put) = pump
                            .fetch
                            .publish(Some((pump.generation, guest_get)))
                            .map_err(str::to_owned)?;
                        storage.write_user_gp(slot, entry)?;
                        storage.publish_put(put)
                    },
                )?;
                self.source_admitted = true;
                self.cursor = guest_get;
                self.tick = true;
                self.again = true;
                return Ok(true);
            }
            let kf_chan::user_stream::Source::Segment(descriptor) = descriptor else {
                return Err("USER_RING_GP_FORMAT".into());
            };
            let pb = self.snapshot(
                me,
                r,
                descriptor.gpu_va,
                usize::try_from(descriptor.len_bytes).map_err(|_| "USER_RING_PB_SIZE")?,
            )?;
            let mut words = Vec::new();
            words
                .try_reserve_exact(pb.bytes.len() / 4)
                .map_err(|_| "USER_RING_SCAN_ALLOC")?;
            for word in pb.bytes.chunks_exact(4) {
                words.push(u32::from_le_bytes(
                    word.try_into().map_err(|_| "USER_RING_WORD")?,
                ));
            }
            let software = software_snapshot(me, r)?;
            let (patched, rare) = stage_plan(&self.scan, &software, &words)?;
            self.segment = Some(Segment {
                ring,
                software,
                source,
                pb,
                entry,
                words,
                patched,
                at: 0,
                generation: self.generation,
                guest_get,
                rare,
                staged: false,
                wait: None,
            });
        }
        let mut segment = self.segment.take().ok_or("USER_RING_SEGMENT_ABSENT")?;
        let outcome = self.segment_turn(me, r, native, &mut segment);
        match outcome {
            Ok((submitted, finished)) => {
                if !finished {
                    self.segment = Some(segment);
                    self.tick = true;
                }
                // A CPU-only action/receipt can expose a new source delta too.
                // Preserve a durable worker pass even with no new guest ring.
                self.again = finished || submitted;
                Ok(submitted)
            }
            Err(e) => {
                self.segment = Some(segment);
                Err(e)
            }
        }
    }

    /// No host verb inside any admission guard. The caller holds Record over
    /// the final mapped doorbell too; native Free takes that mutex off-drainer.
    fn publish(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        software: Option<&Option<Software>>,
        ring: Option<&Mapping<'static>>,
        gp: Option<&Snapshot<'static>>,
        pb: Option<&Snapshot<'static>>,
        write: impl FnOnce(
            &mut Self,
            &kf_chan::user_storage::Storage<kf_chan::user_storage::CpuView>,
        ) -> Result<(), String>,
    ) -> Turn<()> {
        if me.inbox.walk_busy() {
            me.inbox.wait_walk(r.idx);
            return Err(Failure::Retry);
        }
        let userd = r.userd.clone();
        userd
            .admit(|| -> Turn<()> {
                let mut pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let t = pt
                    .get_mut(&r.key)
                    .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
                    .ok_or("USER_RING_ALLOCATION_GONE")?;
                if software.is_some_and(|expected| !same_software(expected, &t.user_defapi)) {
                    return Err(Failure::Retry);
                }
                if let Some(sw) = &t.user_defapi {
                    if !sw.live.load(Ordering::Acquire) {
                        return Err("USER_RING_SOFTWARE_OBJECT_GONE".into());
                    }
                    if !sw.ready.load(Ordering::Acquire) {
                        return Err(Failure::Retry);
                    }
                }
                if t.stopped || t.disabled {
                    return Err(Failure::Retry);
                }
                if me.stop.load(Ordering::Acquire) {
                    return Err("USER_RING_PUBLICATION_STOPPED".into());
                }
                let storage = locked(r.lease.storage.try_lock(), "USER_RING_STORAGE_POISONED")?;
                // Exclude first-object admission even on an uncertain native write.
                // A premature/hostile doorbell can only close its own admission.
                let publish = || {
                    me.ram
                        .admit_user_generation(self.ram_generation, || {
                            t.user_published = true;
                            write(self, &storage).map_err(Failure::from)
                        })
                        .map_err(|e| source_error(e, "USER_RING_PUBLICATION_TOPOLOGY"))?
                };
                let sources = || {
                    if let Some(gp) = gp {
                        gp.admit(|| {
                            if let Some(pb) = pb {
                                pb.admit(publish)
                                    .map_err(|e| source_error(e, "USER_RING_PB_ADMISSION"))?
                            } else {
                                publish()
                            }
                        })
                        .map_err(|e| source_error(e, "USER_RING_GP_ADMISSION"))?
                    } else {
                        publish()
                    }
                };
                if let Some(ring) = ring {
                    ring.admit(sources)
                        .map_err(|e| source_error(e, "USER_RING_WHOLE_RING_ADMISSION"))?
                } else {
                    sources()
                }
            })
            .map_err(|e| access_error(e, "USER_RING_USERD_ADMISSION"))??;
        me.rm
            .doorbell(r.chan.ok_or("USER_RING_NATIVE_ABSENT")?.token)
            .map_err(|e| format!("USER_RING_DOORBELL_UNCERTAIN {e:?}").into())
    }

    fn segment_turn(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        _native: kf_host::Channel,
        s: &mut Segment,
    ) -> Turn<(bool, bool)> {
        if let Some(mut wait) = s.wait.take() {
            if self.fetch.fetched() < wait.witness {
                s.wait = Some(wait);
                return Ok((false, false));
            }
            if let Some(trigger) = wait.trigger {
                match self.software(me, r, trigger.call, &mut wait.action) {
                    Ok(true) => {}
                    Ok(false) => {
                        s.wait = Some(wait);
                        return Ok((false, false));
                    }
                    Err(e) => {
                        s.wait = Some(wait);
                        return Err(e);
                    }
                }
                let (at, finished) = suffix_after_trigger(s.at, trigger, s.words.len())?;
                s.at = at;
                // The prefix witness already retired every published byte. A
                // trigger at END has no suffix to consume: EVICT can deschedule
                // the channel, so never append a new receipt it cannot fetch.
                return Ok((false, finished));
            }
            // This receipt retires private staging only. Guest GP_GET follows
            // the first representing descriptor's real fetch, independently.
            return Ok((false, true));
        }
        let mut next = self.scan.clone();
        let trigger = next
            .segment(s.words.get(s.at..).ok_or("USER_RING_SCAN_BOUND")?, |v| {
                s.software.as_ref().is_some_and(|sw| sw.matches(v))
            })
            .map_err(|e| format!("USER_RING_SCAN {e:?}"))?;
        if !s.rare {
            if self.fetch.available() == 0 {
                return Ok((false, false));
            }
            self.publish(
                me,
                r,
                Some(&s.software),
                s.ring.as_ref(),
                Some(&s.source),
                Some(&s.pb),
                |pump, storage| {
                    let (slot, put) = pump
                        .fetch
                        .publish(Some((s.generation, s.guest_get)))
                        .map_err(str::to_owned)?;
                    storage.write_user_gp(slot, s.entry)?;
                    storage.publish_put(put)
                },
            )?;
            self.source_admitted = true;
            self.scan = next;
            self.cursor = s.guest_get;
            self.tick = true;
            return Ok((true, true));
        }
        let end = trigger.map_or(s.words.len(), |t| s.at.saturating_add(t.start));
        let bytes = end
            .checked_sub(s.at)
            .and_then(|n| n.checked_mul(4))
            .ok_or("USER_RING_PREFIX_BOUND")?;
        let credits = if bytes == 0 { 2 } else { 3 };
        if self.fetch.available() < credits {
            return Ok((false, false));
        }
        // Initial staging retains exact source guards. Once staged, the owned
        // snapshot survives the guest freeing/moving its old ring at migration.
        let gp = (!s.staged).then_some(&s.source);
        let pb = (!s.staged).then_some(&s.pb);
        self.publish(
            me,
            r,
            Some(&s.software),
            if s.staged { None } else { s.ring.as_ref() },
            gp,
            pb,
            |pump, storage| {
                if !s.staged {
                    storage.write_authored(0, &s.patched)?;
                }
                if bytes != 0 {
                    let source = source_receipt(s.staged, true, s.generation, s.guest_get);
                    let (slot, _) = pump.fetch.publish(source).map_err(str::to_owned)?;
                    storage.write_gp(
                        slot,
                        (s.at as u64)
                            .checked_mul(4)
                            .ok_or("USER_RING_PREFIX_BOUND")?,
                        bytes as u64,
                        s.entry & (1 << 63) != 0,
                    )?;
                }
                let source = source_receipt(s.staged, bytes == 0, s.generation, s.guest_get);
                let (barrier, _) = pump.fetch.publish(source).map_err(str::to_owned)?;
                storage.write_host_barrier(barrier)?;
                let (witness, put) = pump.fetch.publish(None).map_err(str::to_owned)?;
                storage.write_host_witness(witness)?;
                storage.publish_put(put)
            },
        )?;
        if !s.staged && s.generation == self.generation {
            self.source_admitted = true;
            self.cursor = s.guest_get;
        }
        s.staged = true;
        self.scan = next;
        s.wait = Some(Wait {
            witness: self.fetch.issued(),
            trigger,
            action: None,
        });
        Ok((true, false))
    }

    fn software(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        call: kf_chan::tmode::SwCall,
        pending: &mut Option<PendingAction>,
    ) -> Turn<bool> {
        let reg = me.defapi_reg.get().ok_or("USER_RING_REGISTRY_ABSENT")?;
        if let Some(action) = pending.take() {
            let (plan, result, flushed) = match action {
                PendingAction::Invalidate { ticket, plan } => match me.inbox.split_result(ticket) {
                    Some(result) => (plan, result, true),
                    None => {
                        *pending = Some(PendingAction::Invalidate { ticket, plan });
                        return Ok(false);
                    }
                },
                PendingAction::Context { cell, plan } => {
                    let result = cell.try_lock().ok().and_then(|mut result| result.take());
                    match result {
                        Some(result) => (plan, result.map(|_| ()), false),
                        None => {
                            *pending = Some(PendingAction::Context { cell, plan });
                            return Ok(false);
                        }
                    }
                }
            };
            reg.executed_entry(plan.key, &plan.entry, flushed && result.is_ok());
            result?;
            return Ok(true);
        }
        let sw = software_snapshot(me, r)?
            .filter(|sw| sw.matches(call.value))
            .ok_or("USER_RING_SOFTWARE_OBJECT_GONE")?;
        let handle = sw.guest;
        let key = kf_rm::defapi::ObjKey {
            client: r.key.0,
            object: handle,
        };
        let entry = reg
            .lookup(key, call.data)
            .map_err(|e| format!("USER_RING_UNREGISTERED_API {e:#x}"))?;
        let plan = crate::defapi::Planned {
            key,
            bundle: entry.decoded,
            entry,
        };
        me.defapi_triggers.fetch_add(1, Ordering::Relaxed);
        let outcome = match plan.bundle {
            kf_abi::defapi::Bundle::UpdateChannelInfo(update) => self.migrate(me, r, update),
            kf_abi::defapi::Bundle::InvalidateTlb { .. } => {
                let ticket =
                    me.inbox
                        .request_user_split(r.idx, r.vas_key)
                        .map_err(|e| match e {
                            crate::mem::UserSplitError::Busy => Failure::Retry,
                            crate::mem::UserSplitError::Refused(why) => why.into(),
                        })?;
                *pending = Some(PendingAction::Invalidate { ticket, plan });
                return Ok(false);
            }
            kf_abi::defapi::Bundle::InitializeCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::PromoteCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::EvictCtx {
                chan_client,
                object,
                ..
            } => {
                let pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let own = pt.get(&r.key).is_some_and(|t| {
                    Arc::ptr_eq(&t.allocation, &r.allocation)
                        && chan_client == r.key.0
                        && (object == r.key.1 || t.tsg == Some(object))
                });
                drop(pt);
                if !own {
                    Err("USER_RING_CONTEXT_FOREIGN_TARGET".into())
                } else {
                    let cell = Arc::new(Mutex::new(None));
                    me.user_ring_ctx_act(
                        plan.clone(),
                        cell.clone(),
                        r.idx,
                        r.key,
                        r.allocation.clone(),
                    )?;
                    *pending = Some(PendingAction::Context { cell, plan });
                    return Ok(false);
                }
            }
            _ => Err(format!("USER_RING_DEFERRED_UNSUPPORTED {:?}", plan.bundle).into()),
        };
        if outcome != Err(Failure::Retry) {
            reg.executed_entry(plan.key, &plan.entry, false);
        }
        outcome?;
        Ok(true)
    }

    fn migrate(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        u: kf_abi::defapi::FifoUpdate,
    ) -> Turn<()> {
        if (u.client, u.channel) != r.key
            || u.userd_memory == 0
            || u.userd_offset & 0x1ff != 0
            || u.gpfifo_va & 7 != 0
            || u.entries < 2
            || !u.entries.is_power_of_two()
            || u.entries > 1 << 20
        {
            return Err("USER_RING_MIGRATION_DECLARATION".into());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("USER_RING_SOURCE_SEQUENCE")?;
        let userd = me
            .memory_access
            .lease(u.client, u.userd_memory, u.userd_offset, 0x200)
            .map_err(|e| access_error(e, "USER_RING_MIGRATION_USERD"))?;
        let ring = userread::mapped(
            &r.mirror.rows,
            &r.mirror.log,
            me.ram,
            self.ram_generation,
            u.gpfifo_va,
            u64::from(u.entries) * 8,
            store_read_bound(me.layout.carve()),
        )
        .map_err(|e| source_error(e, "USER_RING_MIGRATION_RING"))?;
        if me.inbox.walk_busy() {
            me.inbox.wait_walk(r.idx);
            return Err(Failure::Retry);
        }
        userd
            .admit(|| -> Turn<()> {
                let mut pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let t = pt
                    .get_mut(&r.key)
                    .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
                    .ok_or("USER_RING_ALLOCATION_GONE")?;
                ring.admit(|| {
                    // DIAGNOSTIC hypothesis: new guest source starts at0. Same-GPU
                    // passive reference sees zero; exact trigger semantics still
                    // require the flag-OFF vertical-slice run. Never reset host GET.
                    r.userd = userrelay::UserdSource::Registered(userd.clone());
                    r.source = (u.gpfifo_va, u.entries);
                    t.ring_at = (u.gpfifo_va, u.entries, None);
                    self.generation = generation;
                    self.source_admitted = false;
                    self.cursor = 0;
                    self.guest_get = None;
                })
                .map_err(|e| source_error(e, "USER_RING_MIGRATION_ADMISSION"))
            })
            .map_err(|e| access_error(e, "USER_RING_MIGRATION_USERD_ADMISSION"))??;
        eprintln!(
            "kf3: USER_RING_MIGRATED {:#x}:{:#x} source={:#x}x{} generation={generation}; private native ring/USERD unchanged; no host migration verb",
            r.key.0, r.key.1, u.gpfifo_va, u.entries
        );
        Ok(())
    }
}

impl ChanPlane {
    pub(super) fn user_ring_software_object(
        &self,
        client: u32,
        parent: u32,
        handle: u32,
        notify_completion: bool,
    ) -> ChanAnswer {
        let key = (client, parent);
        let admission = (|| -> Result<_, String> {
            let mut pt = self.pt.lock().map_err(|_| "USER_RING_PT_POISONED")?;
            let t = pt
                .get_mut(&key)
                .filter(|t| t.user_ring)
                .ok_or("USER_RING_ALLOCATION_GONE")?;
            // Match CPU-RM statement-time numbering even when this alloc refuses.
            let number = t.sw_ids.register().ok_or("USER_RING_SW_NUMBER_EXHAUSTED")?;
            if notify_completion {
                return Err("USER_RING_DEFERRED_NOTIFICATION_UNSUPPORTED".into());
            }
            if t.user_published {
                return Err("USER_RING_LATE_SOFTWARE_OBJECT_UNSUPPORTED".into());
            }
            if number != 1 || t.user_defapi.is_some() {
                return Err("USER_RING_FIRST_SOFTWARE_OBJECT_ONLY".into());
            }
            let sw = Software {
                guest: handle,
                number,
                live: Arc::new(AtomicBool::new(true)),
                ready: Arc::new(AtomicBool::new(false)),
            };
            t.user_defapi = Some(sw.clone());
            Ok((t.allocation.clone(), t.chan, sw))
        })();
        let (allocation, native, sw) = match admission {
            Ok(v) => v,
            Err(why) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_NOT_SUPPORTED,
                    why,
                };
            }
        };
        let d = kf_gsp::Deferred::new();
        let abandoned = sw.live.clone();
        d.on_orphaned(move || {
            abandoned.store(false, Ordering::Release);
        });
        let act: Act = Box::new(move |me| {
            let fail = |why: &str| (NV_ERR_INVALID_STATE, why.to_owned());
            let record = me
                .user_ring_record(native.token)
                .ok_or_else(|| fail("USER_RING_RECORD_GONE"))?;
            // Off-drainer: exclude publication and native Free through this RM verb.
            let r = record
                .lock()
                .map_err(|_| fail("USER_RING_RECORD_POISONED"))?;
            if r.free_attempted
                || r.retired
                || !Arc::ptr_eq(&r.allocation, &allocation)
                || !sw.live.load(Ordering::Acquire)
            {
                return Err(fail("USER_RING_SW_ADMISSION_GONE"));
            }
            let current = |pt: &PtMap| {
                pt.get(&key).is_some_and(|t| {
                    Arc::ptr_eq(&t.allocation, &allocation)
                        && t.chan == native
                        && t.user_defapi
                            .as_ref()
                            .is_some_and(|v| Arc::ptr_eq(&v.live, &sw.live))
                })
            };
            if !me.pt.lock().is_ok_and(|pt| current(&pt)) {
                return Err(fail("USER_RING_SW_ALLOCATION_GONE"));
            }
            // No host deferred entries: native SET_OBJECT/NOP is supported; all
            // intercepted host actions use the separately owned guest registry.
            let host = me.rm.alloc_deferred_api(native).map_err(|e| {
                sw.live.store(false, Ordering::Release);
                (NV_ERR_NOT_SUPPORTED, format!("USER_RING_NATIVE_5080 {e:?}"))
            })?;
            let mut pt = me.pt.lock().map_err(|_| fail("USER_RING_PT_POISONED"))?;
            if !current(&pt) || !sw.live.load(Ordering::Acquire) {
                // Retain the native child under its exact channel until Free.
                return Err(fail("USER_RING_SW_ALLOCATION_GONE_AFTER_NATIVE_ALLOC"));
            }
            pt.get_mut(&key)
                .ok_or_else(|| fail("USER_RING_ALLOCATION_GONE"))?
                .defapi_host = Some((handle, host));
            sw.ready.store(true, Ordering::Release);
            drop(pt);
            drop(r);
            me.user_ring_wake(native.token);
            Ok(format!(
                "USER_RING_5080 admitted own {client:#x}:{handle:#x}, classID{} host {host:#x}; no host registrations",
                sw.number
            ))
        });
        self.defer_cell("USER ring software object", act, d)
    }

    pub(super) fn user_ring_serve(&self, ht: u32) -> Option<bool> {
        let record = self.user_ring_record(ht)?;
        let Ok(mut r) = record.enter() else {
            return Some(false);
        };
        if r.retired || r.free_attempted || r.dead.is_some() {
            record.cancel();
            return Some(false);
        }
        let pt = match self.pt.try_lock() {
            Ok(pt) => pt,
            Err(std::sync::TryLockError::WouldBlock) => return Some(false),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                record.cancel();
                r.dead = Some("USER_RING_PT_POISONED".into());
                self.poisoned.fetch_add(1, Ordering::Relaxed);
                return Some(false);
            }
        };
        let Some(t) = pt
            .get(&r.key)
            .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
        else {
            record.cancel();
            return Some(false);
        };
        if t.stopped || t.disabled {
            return Some(false);
        }
        if self.stop.load(Ordering::Acquire) {
            return Some(false);
        }
        drop(pt);
        record.retry.store(false, Ordering::Release);
        let mut pump = match r.pump.take() {
            Some(p) => p,
            None => return Some(false),
        };
        let outcome = pump.turn(self, &mut r);
        r.pump = Some(pump);
        match outcome {
            Ok(submitted) => {
                // Drain a coalesced source delta without waiting for the park
                // tick. One extra turn may observe a pending native receipt;
                // that turn then parks rather than spinning on the GPU.
                if r.pump.as_ref().is_some_and(|p| p.again) && self.plane.ring_internal(r.idx) {
                    let _ = self.wake.signal();
                }
                Some(submitted)
            }
            Err(Failure::Retry) => {
                if let Some(p) = &mut r.pump {
                    // A changed/contended prepublication snapshot has no native
                    // credit. Re-read it rather than retrying a permanently stale
                    // view. Staged native work and its receipt state stay owned.
                    if p.segment.as_ref().is_some_and(|s| !s.staged) {
                        p.segment = None;
                    }
                    p.tick = true;
                }
                Some(false)
            }
            Err(Failure::Refused(why)) => {
                self.poisoned.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "kf3: USER_RING_REFUSED {:#x}:{:#x} host {ht:#x}: {why}; no completion/reuse credit",
                    r.key.0, r.key.1
                );
                r.dead = Some(why);
                record.cancel();
                Some(false)
            }
        }
    }

    /// Only selects the existing worker's short park interval. No timeout ever
    /// grants a receipt, retires bytes, or releases a stalled action.
    pub(crate) fn user_ring_poll_due(&self) -> bool {
        self.user_rings.try_lock().is_ok_and(|rings| {
            rings.values().any(|record| {
                !record.is_canceled()
                    && (record.pending()
                        || record.try_lock().is_ok_and(|r| {
                            !r.retired
                                && !r.free_attempted
                                && r.dead.is_none()
                                && r.pump.as_ref().is_some_and(|p| p.tick)
                        }))
            })
        })
    }

    /// Worker tick may queue tokens only, never prepare views or scan.
    pub(super) fn user_ring_refresh(&self) {
        let Ok(rings) = self.user_rings.try_lock() else {
            return;
        };
        for record in rings.values() {
            let mut signaled = false;
            record.wake_if(
                |r| {
                    !r.retired
                        && !r.free_attempted
                        && r.dead.is_none()
                        && (record.pending() || r.pump.as_ref().is_some_and(|p| p.tick))
                },
                |idx| signaled = self.plane.ring_internal(idx),
            );
            if signaled {
                let _ = self.wake.signal();
            }
        }
    }

    pub(super) fn user_ring_wake(&self, token: u32) {
        if let Some(record) = self.user_ring_record(token) {
            if !record.request() {
                return;
            }
            record.wake_if(
                |r| !r.retired && !r.free_attempted && r.dead.is_none(),
                |idx| {
                    self.plane.ring_internal(idx);
                },
            );
            // Even when metadata is busy, wake the worker to observe durable
            // demand; never publish a stale guest idx without its guard.
            let _ = self.wake.signal();
        }
    }

    fn user_ring_ctx_act(
        &self,
        plan: crate::defapi::Planned,
        cell: SwCell,
        ring: u32,
        key: (u32, u32),
        allocation: Arc<()>,
    ) -> Result<(), String> {
        let act: Act = Box::new(move |me| {
            let current = me
                .pt
                .lock()
                .ok()
                .and_then(|pt| {
                    pt.get(&key)
                        .map(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &allocation))
                })
                .unwrap_or(false);
            let result = if current {
                me.deferred_ctx(&plan)
            } else {
                Err((
                    NV_ERR_INVALID_STATE,
                    "USER_RING_CONTEXT_ALLOCATION_GONE".into(),
                ))
            };
            if let Ok(mut cell) = cell.lock() {
                *cell = Some(
                    result
                        .clone()
                        .map_err(|(status, why)| format!("{why} ({status:#x})")),
                );
            }
            if me.plane.ring_internal(ring) {
                let _ = me.wake.signal();
            }
            result
        });
        let sent = self.acts.lock().ok().and_then(|acts| {
            acts.as_ref().map(|tx| {
                tx.send((act, kf_gsp::Deferred::new(), "USER ring context"))
                    .is_ok()
            })
        });
        if sent == Some(true) {
            Ok(())
        } else {
            Err("USER_RING_CONTEXT_ACT_UNAVAILABLE".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::method_header_inc as h;
    fn sw(guest: u32, number: u16) -> Software {
        Software {
            guest,
            number,
            live: Arc::new(AtomicBool::new(true)),
            ready: Arc::new(AtomicBool::new(true)),
        }
    }
    #[test]
    fn classification_is_pinned_across_free_reuse_and_uses_admitted_class_id() {
        let software = Some(sw(0x51, 7));
        let words = [h(5, 0, 1).unwrap(), 7, h(5, 0x200, 1).unwrap(), 29, 0];
        let scan = Scan::default();
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        assert_eq!(patched, [words[0], 7, 0, 0, 0]);
        let old = software.as_ref().unwrap();
        old.live.store(false, Ordering::Release);
        let replacement = Some(sw(0x51, 7));
        assert!(!same_software(&software, &replacement));
        assert!(!same_software(&software, &None));
        // Same numeric handles cannot alter this already prepared plan.
        let mut prefix = scan.clone();
        let trigger = prefix.segment(&words, |v| old.matches(v)).unwrap().unwrap();
        assert_eq!((trigger.start, trigger.end, trigger.call.data), (2, 4, 29));
        assert_eq!(
            stage_plan(&scan, &software, &words).unwrap(),
            (patched, true)
        );
        assert!(!old.matches(1));
        assert!(old.matches(crate::defapi::NV50_DEFERRED_API_CLASS));
    }
    #[test]
    fn all_rare_packets_are_nopped_but_the_prefix_scan_stops_at_each_exact_trigger() {
        let software = Some(sw(0x51, 1));
        let words = [
            h(5, 0, 1).unwrap(),
            1,
            h(5, 0x200, 1).unwrap(),
            27,
            0,
            h(5, 0x200, 1).unwrap(),
            28,
            0,
        ];
        let scan = Scan::default();
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        assert_eq!(patched, [words[0], 1, 0, 0, 0, 0, 0, 0]);
        let mut state = scan.clone();
        let a = state
            .segment(&words, |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        let b = state
            .segment(&words[a.end..], |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        assert_eq!((a.start, a.end, b.start, b.end), (2, 4, 1, 3));
        assert_eq!(scan, Scan::default()); // planning never advances submitted state
    }
    #[test]
    fn a_rare_fragmented_suffix_refuses_without_advancing_the_scan() {
        let software = Some(sw(0x51, 1));
        let words = [
            h(5, 0, 1).unwrap(),
            1,
            h(5, 0x200, 1).unwrap(),
            27,
            h(2, 0x300, 2).unwrap(),
            8,
        ];
        let scan = Scan::default();
        assert_eq!(
            stage_plan(&scan, &software, &words),
            Err("USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED".into())
        );
        assert_eq!(scan, Scan::default());
    }
    #[test]
    fn descriptor_fetch_reports_old_source_before_evict_or_migration_and_keeps_staging() {
        let mut fetch = Fetch::new(8).unwrap();
        fetch.publish(source_receipt(false, true, 1, 17)).unwrap();
        fetch.publish(None).unwrap();
        fetch.publish(None).unwrap();
        let witness = fetch.issued();
        assert_eq!(fetch.observe(1).unwrap(), [(1, 17)]);
        assert!(fetch.fetched() < witness); // old source retirement is NOT PB reuse
        assert_eq!(source_receipt(true, true, 1, 17), None);
        fetch.observe(3).unwrap();
        assert_eq!(fetch.fetched(), witness);
        // Migration changes only source generation; private native GET continues.
        fetch.publish(source_receipt(false, true, 2, 1)).unwrap();
        assert_eq!(fetch.observe(4).unwrap(), [(2, 1)]);
    }
    #[test]
    fn evict_at_end_finishes_after_prefix_receipt_without_a_new_native_suffix() {
        let software = Some(sw(0x51, 1));
        let words = [h(5, 0, 1).unwrap(), 1, h(5, 0x200, 1).unwrap(), 27];
        let mut scan = Scan::default();
        let trigger = scan
            .segment(&words, |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        assert_eq!(
            suffix_after_trigger(0, trigger, words.len()),
            Ok((words.len(), true))
        );
        let mut fetch = Fetch::new(8).unwrap();
        let (first, _) = fetch.publish(source_receipt(false, true, 1, 1)).unwrap();
        assert_eq!(first, 0);
        fetch.publish(None).unwrap();
        fetch.publish(None).unwrap();
        fetch.observe(3).unwrap();
        let native_issued = fetch.issued();
        // Completing the CPU action adds no GPU work and needs no post-evict GET.
        assert!(suffix_after_trigger(0, trigger, words.len()).unwrap().1);
        assert_eq!(fetch.issued(), native_issued);
    }
    #[test]
    fn contention_retries_but_poison_revocation_and_uncertain_io_do_not() {
        let lock = Mutex::new(());
        let _guard = lock.lock().unwrap();
        assert!(matches!(
            locked(lock.try_lock(), "POISON"),
            Err(Failure::Retry)
        ));
        assert_eq!(
            source_error(userread::Refusal::Busy, "SOURCE"),
            Failure::Retry
        );
        assert_eq!(
            source_error(userread::Refusal::Changed, "SOURCE"),
            Failure::Retry
        );
        assert_eq!(
            access_error(kf_rm::memory_list::AccessError::Busy, "USERD"),
            Failure::Retry
        );
        assert!(matches!(
            source_error(userread::Refusal::Topology, "SOURCE"),
            Failure::Refused(_)
        ));
        assert!(matches!(
            access_error(kf_rm::memory_list::AccessError::Gone, "USERD"),
            Failure::Refused(_)
        ));
        assert!(matches!(
            Failure::from("NATIVE_DOORBELL_UNCERTAIN"),
            Failure::Refused(_)
        ));
    }
}
