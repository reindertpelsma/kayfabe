//! Worker-only restricted USER ring relay. No vCPU/drainer caller may run a
//! turn: source-view preparation can call RM. Native USER privileges and own
//! mirrored mappings enforce access; scanning is never a completion authority.

use super::*;
use kf_chan::user_scan::{Fetch, Scan, Trigger};
use crate::mem::userread::{self, Snapshot};

pub(super) struct Pump {
    cache: crate::userstore::Cache,
    ram_generation: u64,
    generation: u64,
    cursor: u32,
    scan: Scan,
    fetch: Fetch,
    segment: Option<Segment>,
    guest_get: Option<(u64, u32)>,
    pub tick: bool,
}

struct Segment {
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
    Invalidate { ticket: u64, plan: crate::defapi::Planned },
    Context { cell: SwCell, plan: crate::defapi::Planned },
}

impl Pump {
    pub(super) fn new(me: &ChanPlane) -> Result<Self, String> {
        Ok(Self {
            cache: crate::userstore::Cache::new(me.rm, me.store,
                store_read_bound(me.layout.carve())).map_err(|e| format!("USER_RING_CACHE {e:?}"))?,
            ram_generation: me.ram.try_generation().ok_or("USER_RING_RAM_TOPOLOGY")?,
            generation: 1, cursor: 0, scan: Scan::default(),
            fetch: Fetch::new(kf_chan::user_storage::ENTRIES).map_err(str::to_owned)?,
            segment: None, guest_get: None, tick: false,
        })
    }
    pub(super) fn release(&mut self) -> Result<(), String> {
        self.cache.release().map_err(|e| format!("USER_RING_SOURCE_CLEANUP {e:?}"))
    }

    fn snapshot(&mut self, me: &ChanPlane, r: &userrelay::Record, va: u64,
        bytes: usize) -> Result<Snapshot<'static>, String> {
        userread::snapshot(&r.mirror.rows, &r.mirror.log, me.ram,
            self.ram_generation, va, bytes, &mut self.cache)
            .map_err(|e| format!("USER_RING_SOURCE {va:#x}+{bytes:#x}: {e:?}"))
    }

    fn turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Result<bool, String> {
        self.tick = false;
        let native = r.chan.ok_or("USER_RING_NATIVE_ABSENT")?;
        let get = r.lease.storage.lock().map_err(|_| "USER_RING_STORAGE_POISONED")?
            .gpu_get()?;
        for (generation, get) in self.fetch.observe(get).map_err(str::to_owned)? {
            if generation == self.generation { self.guest_get = Some((generation, get)); }
        }
        if let Some((generation, get)) = self.guest_get {
            if generation == self.generation {
                // Logical memory access revalidates Free/reuse and RAM topology.
                match r.userd.write(kf_abi::submit::USERD_GP_GET, &get.to_le_bytes()) {
                    Ok(()) => self.guest_get = None,
                    Err(kf_rm::memory_list::AccessError::Busy) => { self.tick = true; return Ok(false); }
                    Err(e) => return Err(format!("USER_RING_GET_WRITE {e:?}")),
                }
            } else { self.guest_get = None; }
        }
        self.tick = self.fetch.fetched() != self.fetch.issued();
        if self.segment.is_none() {
            if me.inbox.walk_busy() { me.inbox.wait_walk(r.idx); return Ok(false); }
            let mut put = [0u8; 4];
            r.userd.read(kf_abi::submit::USERD_GP_PUT, &mut put)
                .map_err(|e| format!("USER_RING_PUT_READ {e:?}"))?;
            let put = u32::from_le_bytes(put);
            if put >= r.source.1 { return Err("USER_RING_PUT_BOUND".into()); }
            if put == self.cursor { return Ok(false); }
            let va = u64::from(self.cursor).checked_mul(8)
                .and_then(|off| r.source.0.checked_add(off)).ok_or("USER_RING_GP_ADDRESS")?;
            let source = self.snapshot(me, r, va, 8)?;
            let entry = u64::from_le_bytes(source.bytes.as_slice().try_into()
                .map_err(|_| "USER_RING_GP_SNAPSHOT_SIZE")?);
            let descriptor = kf_chan::user_stream::inspect_entry(entry, userread::MAX_BYTES as u64)
                .map_err(|e| format!("USER_RING_GP_FORMAT {e:?}"))?;
            let guest_get = self.cursor.checked_add(1).ok_or("USER_RING_CURSOR")? & (r.source.1 - 1);
            if matches!(descriptor, kf_chan::user_stream::Source::Nop) {
                if self.fetch.available() == 0 { self.tick = true; return Ok(false); }
                self.publish(me, r, Some(&source), None, |pump, storage| {
                    let (slot, put) = pump.fetch.publish(Some((pump.generation, guest_get)))
                        .map_err(str::to_owned)?;
                    storage.write_user_gp(slot, entry)?; storage.publish_put(put)
                })?;
                self.cursor = guest_get; self.tick = true; return Ok(true);
            }
            let kf_chan::user_stream::Source::Segment(descriptor) = descriptor else {
                return Err("USER_RING_GP_FORMAT".into());
            };
            let pb = self.snapshot(me, r, descriptor.gpu_va,
                usize::try_from(descriptor.len_bytes).map_err(|_| "USER_RING_PB_SIZE")?)?;
            let mut words = Vec::new();
            words.try_reserve_exact(pb.bytes.len() / 4).map_err(|_| "USER_RING_SCAN_ALLOC")?;
            for word in pb.bytes.chunks_exact(4) {
                words.push(u32::from_le_bytes(word.try_into().map_err(|_| "USER_RING_WORD")?));
            }
            let mut patched = Vec::new();
            patched.try_reserve_exact(words.len()).map_err(|_| "USER_RING_SCAN_ALLOC")?;
            patched.extend_from_slice(&words);
            let mut scan = self.scan.clone();
            let mut at = 0usize;
            let mut rare = false;
            while let Some(t) = scan.segment(words.get(at..).ok_or("USER_RING_SCAN_BOUND")?,
                |v| software_handle(me, r, v).is_some())
                .map_err(|e| format!("USER_RING_SCAN {e:?}"))? {
                rare = true;
                let start = at.checked_add(t.start).ok_or("USER_RING_SCAN_BOUND")?;
                at = at.checked_add(t.end).ok_or("USER_RING_SCAN_BOUND")?;
                patched.get_mut(start..at).ok_or("USER_RING_SCAN_BOUND")?.fill(0);
            }
            if rare && !scan.at_boundary() {
                return Err("USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED".into());
            }
            self.segment = Some(Segment { source, pb, entry, words, patched, at: 0,
                generation: self.generation, guest_get, rare, staged: false, wait: None });
        }
        let mut segment = self.segment.take().ok_or("USER_RING_SEGMENT_ABSENT")?;
        let outcome = self.segment_turn(me, r, native, &mut segment);
        match outcome {
            Ok((submitted, finished)) => {
                if !finished { self.segment = Some(segment); self.tick = true; }
                Ok(submitted)
            }
            Err(e) => { self.segment = Some(segment); Err(e) }
        }
    }

    /// No host verb inside any admission guard. The caller holds Record over
    /// the final mapped doorbell too; native Free takes that mutex off-drainer.
    fn publish(&mut self, me: &ChanPlane, r: &userrelay::Record,
        gp: Option<&Snapshot<'static>>, pb: Option<&Snapshot<'static>>,
        write: impl FnOnce(&mut Self, &kf_chan::user_storage::Storage<kf_chan::user_storage::CpuView>)
            -> Result<(), String>) -> Result<(), String> {
        if me.inbox.walk_busy() { return Err("USER_RING_PUBLICATION_WALK_BUSY".into()); }
        let userd = r.userd.clone();
        userd.admit(|| -> Result<(), String> {
            let pt = me.pt.try_lock().map_err(|_| "USER_RING_PT_BUSY")?;
            let t = pt.get(&r.key).filter(|t| t.user_ring
                && Arc::ptr_eq(&t.allocation, &r.allocation)).ok_or("USER_RING_ALLOCATION_GONE")?;
            if t.stopped || t.disabled || me.stop.load(Ordering::Acquire) {
                return Err("USER_RING_PUBLICATION_STOPPED".into());
            }
            let storage = r.lease.storage.try_lock().map_err(|_| "USER_RING_STORAGE_BUSY")?;
            let publish = || write(self, &storage);
            if let Some(gp) = gp {
                gp.admit(|| if let Some(pb) = pb { pb.admit(publish)
                    .map_err(|e| format!("USER_RING_PB_ADMISSION {e:?}"))? } else { publish() })
                    .map_err(|e| format!("USER_RING_GP_ADMISSION {e:?}"))?
            } else { publish() }
        }).map_err(|e| format!("USER_RING_USERD_ADMISSION {e:?}"))??;
        me.rm.doorbell(r.chan.ok_or("USER_RING_NATIVE_ABSENT")?.token)
            .map_err(|e| format!("USER_RING_DOORBELL_UNCERTAIN {e:?}"))
    }

    fn segment_turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record,
        _native: kf_host::Channel, s: &mut Segment) -> Result<(bool, bool), String> {
        if let Some(mut wait) = s.wait.take() {
            if self.fetch.fetched() < wait.witness {
                s.wait = Some(wait); return Ok((false, false));
            }
            if let Some(trigger) = wait.trigger {
                if !self.software(me, r, trigger.call, &mut wait.action)? {
                    s.wait = Some(wait); return Ok((false, false));
                }
                s.at = s.at.checked_add(trigger.end).ok_or("USER_RING_SUFFIX_BOUND")?;
                return Ok((false, false));
            }
            if s.generation == self.generation {
                self.cursor = s.guest_get;
                self.guest_get = Some((s.generation, s.guest_get));
            }
            return Ok((false, true));
        }
        let mut next = self.scan.clone();
        let trigger = next.segment(s.words.get(s.at..).ok_or("USER_RING_SCAN_BOUND")?,
            |v| software_handle(me, r, v).is_some())
            .map_err(|e| format!("USER_RING_SCAN {e:?}"))?;
        if !s.rare {
            if self.fetch.available() == 0 { return Ok((false, false)); }
            self.publish(me, r, Some(&s.source), Some(&s.pb), |pump, storage| {
                let (slot, put) = pump.fetch.publish(Some((s.generation, s.guest_get)))
                    .map_err(str::to_owned)?;
                storage.write_user_gp(slot, s.entry)?; storage.publish_put(put)
            })?;
            self.scan = next; self.cursor = s.guest_get; self.tick = true;
            return Ok((true, true));
        }
        let end = trigger.map_or(s.words.len(), |t| s.at.saturating_add(t.start));
        let bytes = end.checked_sub(s.at).and_then(|n| n.checked_mul(4))
            .ok_or("USER_RING_PREFIX_BOUND")?;
        let credits = if bytes == 0 { 2 } else { 3 };
        if self.fetch.available() < credits { return Ok((false, false)); }
        // Initial staging retains exact source guards. Once staged, the owned
        // snapshot survives the guest freeing/moving its old ring at migration.
        let gp = (!s.staged).then_some(&s.source);
        let pb = (!s.staged).then_some(&s.pb);
        self.publish(me, r, gp, pb, |pump, storage| {
            if !s.staged { storage.write_authored(0, &s.patched)?; }
            if bytes != 0 {
                let (slot, _) = pump.fetch.publish(None).map_err(str::to_owned)?;
                storage.write_gp(slot, (s.at as u64).checked_mul(4).ok_or("USER_RING_PREFIX_BOUND")?,
                    bytes as u64, s.entry & (1 << 63) != 0)?;
            }
            let (barrier, _) = pump.fetch.publish(None).map_err(str::to_owned)?;
            storage.write_host_barrier(barrier)?;
            let (witness, put) = pump.fetch.publish(None).map_err(str::to_owned)?;
            storage.write_host_witness(witness)?; storage.publish_put(put)
        })?;
        s.staged = true; self.scan = next;
        s.wait = Some(Wait { witness: self.fetch.issued(), trigger, action: None });
        Ok((true, false))
    }

    fn software(&mut self, me: &ChanPlane, r: &mut userrelay::Record, call: kf_chan::tmode::SwCall,
        pending: &mut Option<PendingAction>) -> Result<bool, String> {
        let reg = me.defapi_reg.get().ok_or("USER_RING_REGISTRY_ABSENT")?;
        if let Some(action) = pending.take() {
            let (plan, result, flushed) = match action {
                PendingAction::Invalidate { ticket, plan } => match me.inbox.split_result(ticket) {
                    Some(result) => (plan, result, true),
                    None => { *pending = Some(PendingAction::Invalidate { ticket, plan }); return Ok(false); }
                },
                PendingAction::Context { cell, plan } => {
                    let result = cell.try_lock().ok().and_then(|mut result| result.take());
                    match result {
                        Some(result) => (plan, result.map(|_| ()), false),
                        None => { *pending = Some(PendingAction::Context { cell, plan }); return Ok(false); }
                    }
                }
            };
            reg.executed_entry(plan.key, &plan.entry, flushed && result.is_ok());
            result?; return Ok(true);
        }
        let handle = software_handle(me, r, call.value).ok_or("USER_RING_SOFTWARE_OBJECT_GONE")?;
        let key = kf_rm::defapi::ObjKey { client: r.key.0, object: handle };
        let entry = reg.lookup(key, call.data).map_err(|e| format!("USER_RING_UNREGISTERED_API {e:#x}"))?;
        let plan = crate::defapi::Planned { key, bundle: entry.decoded, entry };
        me.defapi_triggers.fetch_add(1, Ordering::Relaxed);
        let outcome = match plan.bundle {
            kf_abi::defapi::Bundle::UpdateChannelInfo(update) => self.migrate(me, r, update),
            kf_abi::defapi::Bundle::InvalidateTlb { .. } => {
                let ticket = me.inbox.request_user_split(r.idx, r.vas_key).map_err(str::to_owned)?;
                *pending = Some(PendingAction::Invalidate { ticket, plan }); return Ok(false);
            }
            kf_abi::defapi::Bundle::InitializeCtx { chan_client, object, .. }
            | kf_abi::defapi::Bundle::PromoteCtx { chan_client, object, .. }
            | kf_abi::defapi::Bundle::EvictCtx { chan_client, object, .. } => {
                let own = me.pt.try_lock().ok().and_then(|pt| pt.get(&r.key).map(|t|
                    Arc::ptr_eq(&t.allocation, &r.allocation) && chan_client == r.key.0
                    && (object == r.key.1 || t.tsg == Some(object)))).unwrap_or(false);
                if !own { Err("USER_RING_CONTEXT_FOREIGN_TARGET".into()) }
                else {
                    let cell = Arc::new(Mutex::new(None));
                    me.user_ring_ctx_act(plan.clone(), cell.clone(), r.idx, r.key, r.allocation.clone())?;
                    *pending = Some(PendingAction::Context { cell, plan }); return Ok(false);
                }
            }
            _ => Err(format!("USER_RING_DEFERRED_UNSUPPORTED {:?}", plan.bundle)),
        };
        reg.executed_entry(plan.key, &plan.entry, false);
        outcome?; Ok(true)
    }

    fn migrate(&mut self, me: &ChanPlane, r: &mut userrelay::Record,
        u: kf_abi::defapi::FifoUpdate) -> Result<(), String> {
        if (u.client, u.channel) != r.key || u.userd_memory == 0
            || u.userd_offset & 0x1ff != 0 || u.gpfifo_va & 7 != 0
            || u.entries < 2 || !u.entries.is_power_of_two() || u.entries > 1 << 20 {
            return Err("USER_RING_MIGRATION_DECLARATION".into());
        }
        let generation = self.generation.checked_add(1).ok_or("USER_RING_SOURCE_SEQUENCE")?;
        let userd = me.memory_access.lease(u.client, u.userd_memory, u.userd_offset, 0x200)
            .map_err(|e| format!("USER_RING_MIGRATION_USERD {e:?}"))?;
        let ring = userread::mapped(&r.mirror.rows, &r.mirror.log, me.ram,
            self.ram_generation, u.gpfifo_va, u64::from(u.entries) * 8,
            store_read_bound(me.layout.carve())).map_err(|e| format!("USER_RING_MIGRATION_RING {e:?}"))?;
        if me.inbox.walk_busy() { return Err("USER_RING_MIGRATION_WALK_BUSY".into()); }
        userd.admit(|| -> Result<(), String> {
            let mut pt = me.pt.try_lock().map_err(|_| "USER_RING_PT_BUSY")?;
            let t = pt.get_mut(&r.key).filter(|t| t.user_ring
                && Arc::ptr_eq(&t.allocation, &r.allocation)).ok_or("USER_RING_ALLOCATION_GONE")?;
            ring.admit(|| {
                // DIAGNOSTIC hypothesis: new guest source starts at0. Same-GPU
                // passive reference sees zero; exact trigger semantics still
                // require the flag-OFF vertical-slice run. Never reset host GET.
                r.userd = userd.clone(); r.source = (u.gpfifo_va, u.entries);
                t.ring_at = (u.gpfifo_va, u.entries, None);
                self.generation = generation; self.cursor = 0; self.guest_get = None;
            }).map_err(|e| format!("USER_RING_MIGRATION_ADMISSION {e:?}"))
        }).map_err(|e| format!("USER_RING_MIGRATION_USERD_ADMISSION {e:?}"))??;
        eprintln!("kf3: USER_RING_MIGRATED {:#x}:{:#x} source={:#x}x{} generation={generation}; private native ring/USERD unchanged; no host migration verb",
            r.key.0, r.key.1, u.gpfifo_va, u.entries);
        Ok(())
    }
}

fn software_handle(me: &ChanPlane, r: &userrelay::Record, value: u32) -> Option<u32> {
    let pt = me.pt.try_lock().ok()?;
    let t = pt.get(&r.key).filter(|t| t.user_ring
        && Arc::ptr_eq(&t.allocation, &r.allocation))?;
    let (guest, _) = t.defapi_host?;
    (value == crate::defapi::NV50_DEFERRED_API_CLASS || value == 1).then_some(guest)
}

impl ChanPlane {
    pub(super) fn user_ring_serve(&self, ht: u32) -> Option<bool> {
        let record = self.user_ring_record(ht)?;
        let Ok(mut record) = record.try_lock() else { return Some(false) };
        if record.retired || record.free_attempted || record.dead.is_some() { return Some(false); }
        let active = self.pt.try_lock().ok().and_then(|pt| pt.get(&record.key).map(|t|
            t.user_ring && Arc::ptr_eq(&t.allocation, &record.allocation)
            && !t.stopped && !t.disabled)).unwrap_or(false);
        if !active || self.stop.load(Ordering::Acquire) { return Some(false); }
        let mut pump = record.pump.take()?;
        let outcome = pump.turn(self, &mut record);
        record.pump = Some(pump);
        match outcome {
            Ok(submitted) => Some(submitted),
            Err(why) => {
                self.poisoned.fetch_add(1, Ordering::Relaxed);
                eprintln!("kf3: USER_RING_REFUSED {:#x}:{:#x} host {ht:#x}: {why}; no completion/reuse credit",
                    record.key.0, record.key.1);
                record.dead = Some(why); Some(false)
            }
        }
    }

    /// Drainer/worker tick may queue tokens only, never prepare views or scan.
    pub(super) fn user_ring_refresh(&self) {
        let Ok(rings) = self.user_rings.try_lock() else { return };
        for record in rings.values() {
            let Ok(record) = record.try_lock() else { continue };
            if !record.retired && record.dead.is_none()
                && record.pump.as_ref().is_some_and(|p| p.tick)
                && self.plane.ring_internal(record.idx) { let _ = self.wake.signal(); }
        }
    }

    fn user_ring_ctx_act(&self, plan: crate::defapi::Planned, cell: SwCell, ring: u32,
        key: (u32, u32), allocation: Arc<()>) -> Result<(), String> {
        let act: Act = Box::new(move |me| {
            let current = me.pt.lock().ok().and_then(|pt| pt.get(&key).map(|t|
                t.user_ring && Arc::ptr_eq(&t.allocation, &allocation))).unwrap_or(false);
            let result = if current { me.deferred_ctx(&plan) }
                else { Err((NV_ERR_INVALID_STATE, "USER_RING_CONTEXT_ALLOCATION_GONE".into())) };
            if let Ok(mut cell) = cell.lock() {
                *cell = Some(result.clone().map_err(|(status, why)| format!("{why} ({status:#x})")));
            }
            if me.plane.ring_internal(ring) { let _ = me.wake.signal(); }
            result
        });
        let sent = self.acts.lock().ok().and_then(|acts| acts.as_ref().map(|tx|
            tx.send((act, kf_gsp::Deferred::new(), "USER ring context")).is_ok()));
        if sent == Some(true) { Ok(()) } else { Err("USER_RING_CONTEXT_ACT_UNAVAILABLE".into()) }
    }
}
