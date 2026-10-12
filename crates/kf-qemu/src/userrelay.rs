//! Experimental birth-time Windows USER ring ownership (§AI).
//!
//! No Linux allocation can enter this route without a WindowsUserRingPermit.
//! Storage is created by the VA worker; native birth/free runs on the act thread.
//! The private mutex is never acquired by the drainer or a vCPU. The publisher
//! must take it before any graph admission and retain it through the doorbell;
//! native free takes the same mutex AFTER statement-time channel removal.

use super::*;

pub(super) fn enabled() -> bool {
    // Deliberately not activatable, including by an environment variable.
    // Birth quota, initialization, notifier and submit/migration admission must
    // be complete and reviewed before this draft can affect a guest.
    false
}

/// Kept even after statement-time Free; only confirmed native free retires it.
pub(super) struct Record {
    pub allocation: Arc<()>,
    pub quota: Arc<super::userquota::NativeQuota>,
    pub key: (u32, u32),
    pub idx: u32,
    pub chan: Option<kf_host::Channel>,
    pub lease: Arc<crate::userva::Lease>,
    pub mirror: Mirror,
    pub userd: kf_rm::memory_access::Lease,
    pub source: (u64, u32),
    pub retired: bool,
    pub free_attempted: bool,
    pub dead: Option<String>,
}

/// A count is released on dropped/canceled NEVER-BORN acts only. Once native
/// birth succeeds, explicit confirmed native free is required, even on an error.
struct NeverBorn {
    live: Arc<AtomicU64>,
    twin: Option<Arc<crate::twin::TwinState>>,
    native_attempted: bool,
    quota: Option<Arc<super::userquota::NativeQuota>>,
}
impl Drop for NeverBorn {
    fn drop(&mut self) {
        if !self.native_attempted {
            if let Some(q) = &self.quota { let _ = q.release_confirmed(); }
            self.live.fetch_sub(1, Ordering::AcqRel);
            if let Some(t) = &self.twin {
                t.user_done();
            }
        }
    }
}

struct Birth {
    a: ChannelAlloc,
    mirror: Mirror,
    idx: u32,
    pending: Arc<crate::userbirth::Birth>,
    d: kf_gsp::Deferred,
    count: NeverBorn,
    orphaned: Arc<AtomicBool>,
}
impl Birth {
    fn act(self) -> Act {
        Box::new(move |me| self.run(me))
    }
    fn run(mut self, me: &ChanPlane) -> Result<String, (u32, String)> {
        let fail = |why: String| (NV_ERR_INVALID_STATE, why);
        if self.orphaned.load(Ordering::Acquire) {
            return Err(fail("USER_RING_BIRTH_ORPHANED".into()));
        }
        let lease = match me.user_births.poll(&self.pending).map_err(fail)? {
            None => {
                // Scheduler spacing only; it grants no success/reuse credit.
                let due = std::time::Instant::now()
                    .checked_add(std::time::Duration::from_millis(1))
                    .unwrap_or_else(std::time::Instant::now);
                let d = self.d.clone();
                delay_act(due, "Windows USER ring birth", self.act(), d);
                return Err((ACT_REQUEUED, "USER_RING_WAIT_VA_STORAGE".into()));
            }
            Some(Err(why)) => return Err(fail(why)),
            Some(Ok(lease)) => lease,
        };
        let a = &self.a;
        // Do not forward physical USERD bytes. Resolve the original object
        // declaration through the current graph and exact resource lifetime.
        let source = (|| {
            if a.class != kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A
                || a.entries < 2 || !a.entries.is_power_of_two()
                || a.entries > (1 << 20) || a.gpfifo_va & 7 != 0
                || u64::from(a.entries).checked_mul(8).and_then(|n| a.gpfifo_va.checked_add(n)).is_none()
            {
                return Err("USER_RING_SOURCE_DECLARATION".to_string());
            }
            let (handle, offset) = a.userd_source.ok_or("USER_RING_NO_USERD_DECLARATION")?;
            if handle == 0 || offset & 0x1ff != 0 {
                return Err("USER_RING_USERD_DECLARATION".into());
            }
            let userd = me.memory_access.lease(a.client, handle, offset, 0x200)
                .map_err(|e| format!("USER_RING_USERD_ACCESS {e:?}"))?;
            let Some(kf_arch::UserdMem::Sysmem { base, .. }) = a.userd else {
                return Err("USER_RING_USERD_APERTURE_NOT_IMPLEMENTED".into());
            };
            if !userd.matches_ram_location(base)
                .map_err(|e| format!("USER_RING_USERD_LOCATION {e:?}"))? {
                return Err("USER_RING_USERD_LOCATION_MISMATCH".into());
            }
            // Guest USERD initialization needs an exact pending-birth guard.
            // A memory-object lease alone cannot authorize zeroing a slot after
            // cancellation and channel-handle reuse. No write in this draft.
            Ok(userd)
        })();
        let userd = match source {
            Ok(u) => u,
            Err(why) => {
                // Never born: the queue still owns the lease on enqueue failure.
                let cleanup = me.inbox.user_va.release_after_channel_free(lease);
                let _ = me.inbox.wake.signal();
                return Err(fail(format!("{why}; storage cleanup {cleanup:?}")));
            }
        };
        let spec = match lease.storage.lock().map_err(|_| "USER_RING_STORAGE_POISONED".into())
            .and_then(|s| s.spec(0)) {
            Ok(spec) => spec,
            Err(why) => {
                let cleanup = me.inbox.user_va.release_after_channel_free(lease.clone());
                let _ = me.inbox.wake.signal();
                return Err(fail(format!("{why}; storage cleanup {cleanup:?}")));
            }
        };
        let record = Arc::new(Mutex::new(Record {
            quota: self.count.quota.as_ref().ok_or_else(|| fail("USER_RING_QUOTA_ABSENT".into()))?.clone(),
            allocation: self.pending.allocation(), key: (a.client, a.handle), idx: self.idx,
            chan: None, lease: lease.clone(), mirror: self.mirror.clone(), userd,
            source: (a.gpfifo_va, a.entries), retired: false, free_attempted: false, dead: None,
        }));
        let hold = (|| {
            let mut holds = me.user_birth_holds.lock().map_err(|_| "USER_RING_HOLDS_POISONED")?;
            if holds.len() >= 512 { return Err("USER_RING_NATIVE_ATTEMPT_BUDGET"); }
            holds.try_reserve(1).map_err(|_| "USER_RING_NATIVE_ATTEMPT_ALLOCATION")?;
            holds.push((self.pending.allocation(), record.clone()));
            Ok(())
        })();
        if let Err(why) = hold {
            let cleanup = me.inbox.user_va.release_after_channel_free(lease);
            let _ = me.inbox.wake.signal();
            return Err(fail(format!("{why}; never-born cleanup {cleanup:?}")));
        }
        if self.orphaned.load(Ordering::Acquire) {
            me.release_never_born_record(&record);
            return Err(fail("USER_RING_BIRTH_ORPHANED".into()));
        }
        let gkey = a.tsg.map(|t| (a.client, t, a.ctx_share));
        let join = gkey.and_then(|k| me.groups.lock().ok().and_then(|m| m.get(&k).map(|g| g.0)));
        // From this point a plain Err cannot prove absence of native resources:
        // legacy host birth can hide a failed unwind. Keep the attempt, storage
        // and USER count until session close if no complete Channel is returned.
        self.count.native_attempted = true;
        let native = match join {
            Some(tsg) => me.rm.birth_member(tsg, a.engine_type.unwrap_or(0), spec, false,
                kf_host::channel::PhysicalCeBelt::Deny),
            None => me.rm.birth_channel(self.mirror.space, a.engine_type.unwrap_or(0), spec),
        };
        let chan = match native {
            Ok(chan) => chan,
            Err(why) => {
                if let Ok(mut r) = record.lock() { r.dead = Some(format!("NATIVE_BIRTH_UNCERTAIN {why:?}")); }
                eprintln!("kf3: USER_RING_NATIVE_BIRTH_QUARANTINED {:#x}:{:#x} {why:?}; storage and counts retained, no token handed out", a.client, a.handle);
                return Err(fail(format!("USER_RING_NATIVE_BIRTH_UNCERTAIN {why:?}")));
            }
        };
        record.lock().map_err(|_| fail("USER_RING_RECORD_POISONED".into()))?.chan = Some(chan);
        // Every failure after native birth still has an explicit attempt owner.
        {
            let mut rings = me.user_rings.lock().map_err(|_| fail("USER_RING_RECORDS_POISONED".into()))?;
            if rings.contains_key(&chan.token) {
                return Err(fail("USER_RING_NATIVE_TOKEN_COLLISION_QUARANTINED".into()));
            }
            rings.insert(chan.token, record.clone());
        }
        if let Some(k) = gkey {
            let mut groups = me.groups.lock().map_err(|_| fail("USER_RING_GROUPS_POISONED".into()))?;
            groups.entry(k).and_modify(|g| g.1 = g.1.saturating_add(1)).or_insert((chan.tsg, 1));
        }
        let ready = if self.orphaned.load(Ordering::Acquire) {
            Err("USER_RING_BIRTH_ORPHANED".into())
        } else {
            crate::latejoin::schedule_late_joiner(me.rm, &me.guest_sched, a.client, a.tsg, chan)
                .map(|_| ())
        };
        if let Err(why) = ready {
            let cleanup = me.release_twin(a.client, a.tsg, a.ctx_share, chan);
            if cleanup.is_ok() { self.count.native_attempted = false; }
            return Err(fail(format!("{why}; native cleanup {cleanup:?}")));
        }
        let attached = me.user_births.commit(&self.pending, |identity| {
            let mut pt = me.pt.lock().map_err(|_| "USER_RING_PT_POISONED")?;
            if pt.contains_key(&(a.client, a.handle)) { return Err("USER_RING_LIVE_COLLISION".into()); }
            // Publish all metadata before making the token visible. Locks here
            // protect bounded metadata only; no native verb or source read.
            pt.insert((a.client, a.handle), PtChan {
                allocation: identity.clone(), chan, idx: self.idx, tsg: a.tsg,
                ctx_share: a.ctx_share, parent: a.parent, device: a.device,
                engine: a.engine_type.unwrap_or(0), objects: HashMap::new(),
                pending_objects: HashMap::new(), disp_sw: HashMap::new(),
                sw_ids: crate::dispsw::SwClassIds::default(), dispsw_host_refused: false,
                defapi_host: None, peek_defapi: None, ctx: CtxBind::default(), user_work: true,
                user_ring: true, live: self.count.live.clone(), twin: self.count.twin.clone(),
                notifier: None, stopped: false, disabled: false, space: self.mirror.space,
                rows: self.mirror.rows.clone(), user_regions: self.mirror.user_regions.clone(),
                ledger: self.mirror.ledger.clone(), falcon_ctx: None,
                ring_at: (a.gpfifo_va, a.entries, match a.userd {
                    Some(kf_arch::UserdMem::Sysmem { base, .. }) => Some(base), _ => None }),
            });
            if let Err(e) = self.count.quota.as_ref().ok_or("USER_RING_QUOTA_ABSENT")?.attach(chan.token) {
                pt.remove(&(a.client, a.handle));
                return Err(format!("USER_RING_TOKEN {e:?}"));
            }
            Ok(())
        });
        if let Err(why) = attached {
            let cleanup = me.release_twin(a.client, a.tsg, a.ctx_share, chan);
            if cleanup.is_ok() {
                self.count.native_attempted = false;
            }
            return Err(fail(format!("{why}; native cleanup {cleanup:?}")));
        }
        me.pt_births.fetch_add(1, Ordering::Relaxed);
        me.engine_live(a.engine_type.unwrap_or(0), true);
        // No fast-register path: every ring wakes a worker, including first use.
        Ok(format!("EXPERIMENT KF3_WIN_USER_RING_RELAY chan {:#x}:{:#x} BORN USER private-read-only-ring token {:#x} host {:#x}; OWN VAS; worker-only; source={:#x}x{}",
            a.client, a.handle, self.idx, chan.token, a.gpfifo_va, a.entries))
    }
}

impl ChanPlane {
    /// Statement-time counts have already been acquired by the existing USER
    /// admission. No new route for Linux or a channel without the birth permit.
    pub(super) fn birth_user_ring(&self, a: ChannelAlloc, mirror: Mirror, idx: u32,
        twin: Option<Arc<crate::twin::TwinState>>) -> ChanAnswer {
        let mut count = NeverBorn { live: mirror.live.clone(), twin, native_attempted: false, quota: None };
        let quota = match super::userquota::NativeQuota::reserve(self.caps.clone(), self.plane, idx) {
            Ok(q) => q,
            Err(why) => return ChanAnswer::Refused { status: NV_ERR_INSUFFICIENT_RESOURCES, why },
        };
        count.quota = Some(quota);
        let pending = match self.user_births.allocate(&self.inbox.user_va, &a, mirror.clone()) {
            Ok(p) => p,
            Err(why) => return ChanAnswer::Refused { status: NV_ERR_NOT_SUPPORTED, why },
        };
        let d = kf_gsp::Deferred::new();
        let orphaned = Arc::new(AtomicBool::new(false));
        let identity = pending.allocation();
        let key = (a.client, a.handle);
        let tx = self.acts.lock().ok().and_then(|a| a.clone());
        let canceled = orphaned.clone();
        d.on_orphaned(move || {
            canceled.store(true, Ordering::Release);
            if let Some(tx) = &tx {
                let identity = identity.clone();
                let undo: Act = Box::new(move |me| me.withdraw_user_ring(key, &identity));
                let _ = tx.send((undo, kf_gsp::Deferred::new(), "Windows USER ring withdraw"));
            }
        });
        let birth = Birth { a, mirror, idx, pending, d: d.clone(), count, orphaned };
        self.defer_cell("Windows USER ring birth", birth.act(), d)
    }

    /// Off-drainer/native-free exclusion. Retain the record after any native
    /// failure. Even a failed VA cleanup enqueue leaves Queue's strong pin.
    pub(super) fn user_ring_record(&self, token: u32) -> Option<Arc<Mutex<Record>>> {
        self.user_rings.lock().ok()?.get(&token).cloned()
    }
}

impl ChanPlane {
    pub(super) fn finish_user_ring_free(&self, token: u32,
        record: Option<&Arc<Mutex<Record>>>, native_freed: bool) {
        let Some(record) = record else { return };
        if !native_freed { return; }
        let (lease, identity, quota) = {
            let Ok(mut r) = record.lock() else { return };
            r.retired = true;
            (r.lease.clone(), r.allocation.clone(), r.quota.clone())
        };
        if let Err(why) = quota.release_confirmed() {
            eprintln!("kf3: USER_RING_QUOTA_RETAINED after confirmed native free: {why}");
        }
        // Queue metadata may contend; no publisher guard is held over it.
        let cleanup = self.inbox.user_va.release_after_channel_free(lease);
        let _ = self.inbox.wake.signal();
        if cleanup.is_ok() {
            if let Ok(mut rings) = self.user_rings.lock() { rings.remove(&token); }
            if let Ok(mut holds) = self.user_birth_holds.lock() {
                holds.retain(|(id, _)| !Arc::ptr_eq(id, &identity));
            }
        } else {
            eprintln!("kf3: USER_RING_STORAGE_CLEANUP_RETAINED host {token:#x}: {cleanup:?}");
        }
    }

    fn release_never_born_record(&self, record: &Arc<Mutex<Record>>) {
        let (lease, identity, quota) = {
            let Ok(mut r) = record.lock() else { return };
            r.retired = true;
            (r.lease.clone(), r.allocation.clone(), r.quota.clone())
        };
        let _ = quota.release_confirmed();
        if self.inbox.user_va.release_after_channel_free(lease).is_ok() {
            if let Ok(mut holds) = self.user_birth_holds.lock() {
                holds.retain(|(id, _)| !Arc::ptr_eq(id, &identity));
            }
        }
        let _ = self.inbox.wake.signal();
    }

    pub(super) fn retire_user_ring_token(&self, token: u32, identity: &Arc<()>) -> bool {
        let Some(record) = self.user_ring_record(token) else { return false };
        let quota = {
            let Ok(r) = record.lock() else { return false };
            if !Arc::ptr_eq(&r.allocation, identity) { return false; }
            r.quota.clone()
        };
        quota.retire_token()
    }

    fn withdraw_user_ring(&self, key: (u32, u32), identity: &Arc<()>)
        -> Result<String, (u32, String)> {
        let t = {
            let mut pt = self.pt.lock().map_err(|_| (NV_ERR_INVALID_STATE, "USER_RING_PT_POISONED".into()))?;
            if !pt.get(&key).is_some_and(|t| t.user_ring && Arc::ptr_eq(&t.allocation, identity)) {
                return Ok("USER_RING_WITHDRAW_ALREADY_GONE".into());
            }
            pt.remove(&key).ok_or((NV_ERR_INVALID_STATE, "USER_RING_WITHDRAW_GONE".into()))?
        };
        self.retire_user_ring_token(t.chan.token, &t.allocation);
        if let Some(twin) = &t.twin { twin.user_freeing(); }
        let native = self.release_twin(key.0, t.tsg, t.ctx_share, t.chan);
        if native.is_ok() {
            t.live.fetch_sub(1, Ordering::AcqRel);
            if let Some(twin) = &t.twin { twin.user_released(); }
            if let Some(n) = t.notifier { self.release_notifier(n); }
        }
        self.engine_live(t.engine, false);
        Ok(format!("USER_RING_WITHDRAW exact {key:x?} host {:#x} {native:?}", t.chan.token))
    }
}
