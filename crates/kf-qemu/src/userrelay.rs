//! Birth-time Windows USER ring ownership (§AI).
//!
//! No Linux allocation can enter this route without a WindowsUserRingPermit.
//! Storage is created by the VA worker; native birth/free runs on the act thread.
//! The private mutex is never acquired by the drainer or a vCPU. The publisher
//! must take it before any graph admission and retain it through the doorbell;
//! native free takes the same mutex AFTER statement-time channel removal.

use super::*;

pub(super) fn enabled() -> bool {
    // Correctness path for permitted Windows USER work, selected before native
    // publication. Explicit 0 retains the old route as a diagnostic falsifier.
    // Linux allocations still cannot acquire a WindowsUserRingPermit.
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| relay_enabled(std::env::var("KF3_WIN_USER_RING_RELAY").ok().as_deref()))
        || shadow_enabled()
}

fn relay_enabled(value: Option<&str>) -> bool {
    value != Some("0")
}

pub(super) fn shadow_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_WIN_USER_RING_SHADOW").is_ok_and(|v| v == "1"))
}

/// Private Windows USER channels use an explicit native mask identity. Zero
/// is a valid guest default, not malformed input: author bit 0 on our own
/// channel and give the scanner that SAME identity. This does not infer the
/// guest's privileged RAMFC defaults or change ordinary Linux Passthrough.
/// Native zero/id1 equivalence and the original Windows Vulkan workload are
/// measured on 595.91.07. Other driver/family compatibility is not implied.
fn authored_subdevice_id(declared: u32) -> Result<u32, String> {
    let id = if declared == 0 { 1 } else { declared };
    kf_chan::user_scan::Scan::for_subdevice(id)
        .map_err(|why| format!("USER_RING_SUBDEVICE_ID {declared:#x}: {why}"))?;
    Ok(id)
}

/// Durable demand is outside the private publication mutex: a final doorbell
/// cannot be lost merely because the act thread owns birth/Free metadata.
pub(super) struct Cell<T = Record> {
    pub idx: u32,
    pub retry: AtomicBool,
    canceled: AtomicBool,
    /// ★ 2026-10-14 (`kf_chan::user_loop`): the relay waits for private-ring progress only a look
    /// can find (mirrors `Pump::wait`, written under the record lock, read by the spin / backstop
    /// peeks without it).
    waiting: AtomicBool,
    /// Consecutive turns that found a lock / admission busy (`kf_chan::user_loop::may_retry`).
    contended: AtomicU32,
    inner: Mutex<T>,
}
impl<T> Cell<T> {
    fn new(idx: u32, value: T) -> Self {
        Self {
            idx,
            retry: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
            waiting: AtomicBool::new(false),
            contended: AtomicU32::new(0),
            inner: Mutex::new(value),
        }
    }
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, T>> {
        self.inner.lock()
    }
    pub fn try_lock(&self) -> std::sync::TryLockResult<std::sync::MutexGuard<'_, T>> {
        self.inner.try_lock()
    }
    pub fn enter(&self) -> std::sync::TryLockResult<std::sync::MutexGuard<'_, T>> {
        if !self.request() {
            return Err(std::sync::TryLockError::WouldBlock);
        }
        self.inner.try_lock().inspect_err(|e| {
            if matches!(e, std::sync::TryLockError::Poisoned(_)) {
                self.retry.store(false, Ordering::Release);
            }
        })
    }
    pub fn alive(&self, check: impl FnOnce(&T) -> bool) -> bool {
        if self.canceled.load(Ordering::Acquire) {
            return false;
        }
        match self.inner.try_lock() {
            Ok(r) => check(&r),
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(std::sync::TryLockError::Poisoned(_)) => false,
        }
    }
    pub fn request(&self) -> bool {
        if self.canceled.load(Ordering::Acquire) {
            return false;
        }
        self.retry.store(true, Ordering::Release);
        // Cancellation may race the request, but pending() and the guarded
        // index publisher also check cancellation. No canceled idx is rung.
        if self.canceled.load(Ordering::Acquire) {
            self.retry.store(false, Ordering::Release);
            return false;
        }
        true
    }
    pub fn pending(&self) -> bool {
        !self.is_canceled() && self.retry.load(Ordering::Acquire)
    }
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
    /// Consecutive contended turns so far.
    pub fn contended_run(&self) -> u32 {
        self.contended.load(Ordering::Acquire)
    }
    /// A turn found a lock / admission busy.
    pub fn contended_note(&self) {
        let _ = self
            .contended
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| Some(n.saturating_add(1)));
    }
    /// A turn ran without contention (or the holder rang).
    pub fn contended_clear(&self) {
        self.contended.store(0, Ordering::Release);
    }
    /// See [`Cell::waiting`].
    pub fn is_waiting(&self) -> bool {
        !self.is_canceled() && self.waiting.load(Ordering::Acquire)
    }
    /// Called with the record held, whenever `Pump::wait` changes.
    pub fn set_waiting(&self, waiting: bool) {
        self.waiting.store(waiting, Ordering::Release);
    }
    /// Called with inner held, before token retirement/native cleanup.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        self.retry.store(false, Ordering::Release);
        self.waiting.store(false, Ordering::Release);
    }
    /// Keep the same private exclusion as retirement through idx publication.
    pub fn wake_if(&self, active: impl FnOnce(&T) -> bool, ring: impl FnOnce(u32)) {
        if let Ok(r) = self.inner.try_lock() {
            if !self.canceled.load(Ordering::Acquire) && active(&r) {
                ring(self.idx);
            }
        }
    }
}

/// ★ DEBUG ONLY (`KF3_DEBUG_FORCE_MIGRATABLE`, `kf_rm::chanlink::FORCE_MIGRATABLE`): a guest USERD in
/// the framebuffer -- which for this VMM is the host store object -- read and written through a CPU
/// view WE armed at birth (the view `ChanPlane::userd_view` makes for a Passthrough twin), so a Linux
/// guest, whose USERD is never in guest RAM, can drive the USER-ring relay. Four bytes each way,
/// 4-aligned, inside the 0x200 bytes of the USERD; the view is given back when the last reference
/// goes (after confirmed native free).
pub(super) struct StoreUserd {
    rm: &'static kf_host::HostRm,
    store: u32,
    cookie: u64,
    region: Option<kf_linux_raw::VolatileRegion>,
    _node: kf_linux_raw::CharDevice,
    /// The USERD's offset inside the 4 KiB view.
    at: u64,
}
impl StoreUserd {
    /// Arm the view of the page holding the USERD at store offset `base`.
    pub(super) fn new(
        rm: &'static kf_host::HostRm,
        store: u32,
        base: u64,
    ) -> Result<Self, String> {
        let page = base & !0xFFF;
        if base & 0x1ff != 0 {
            return Err("USER_RING_STORE_USERD_ALIGN".into());
        }
        let (node, cookie) = rm
            .arm_cpu_view(
                kf_host::MapNode::Gpu,
                store,
                page,
                0x1000,
                kf_host::ViewAccess::ReadWrite,
            )
            .map_err(|e| format!("USER_RING_STORE_USERD_VIEW {e:?}"))?;
        let region = match kf_linux_raw::VolatileRegion::map(
            kf_linux_raw::Backing::DeviceFile { fd: node.as_fd() },
            0x1000,
            kf_linux_raw::CachePolicy::Uncached,
            kf_linux_raw::HostPageSize::query(),
        ) {
            Ok(r) => r,
            Err(e) => {
                let _ = rm.release_cpu_view(kf_host::CpuViewRelease {
                    h_memory: store,
                    p_linear_address: cookie,
                });
                return Err(format!("USER_RING_STORE_USERD_MMAP {e:?}"));
            }
        };
        Ok(Self {
            rm,
            store,
            cookie,
            region: Some(region),
            _node: node,
            at: base - page,
        })
    }
    fn word(&self, offset: u64, len: usize) -> Result<kf_linux_raw::HostOffset, kf_rm::memory_list::AccessError> {
        use kf_rm::memory_list::AccessError;
        if len != 4 || offset & 3 != 0 || offset.checked_add(4).is_none_or(|e| e > 0x200) {
            return Err(AccessError::Bounds);
        }
        Ok(kf_linux_raw::HostOffset::new(self.at + offset))
    }
    fn read(&self, offset: u64, bytes: &mut [u8]) -> Result<(), kf_rm::memory_list::AccessError> {
        use kf_rm::memory_list::AccessError;
        let at = self.word(offset, bytes.len())?;
        let v = self
            .region
            .as_ref()
            .ok_or(AccessError::Gone)?
            .load_u32(at)
            .map_err(|_| AccessError::Gone)?;
        bytes.copy_from_slice(&v.to_le_bytes());
        Ok(())
    }
    fn write(&self, offset: u64, bytes: &[u8]) -> Result<(), kf_rm::memory_list::AccessError> {
        use kf_rm::memory_list::AccessError;
        let at = self.word(offset, bytes.len())?;
        let v = u32::from_le_bytes(bytes.try_into().map_err(|_| AccessError::Bounds)?);
        self.region
            .as_ref()
            .ok_or(AccessError::Gone)?
            .store_u32(at, v)
            .map_err(|_| AccessError::Gone)
    }
    fn zero(&self) -> Result<(), kf_rm::memory_list::AccessError> {
        for w in (0..0x200).step_by(4) {
            self.write(w, &[0; 4])?;
        }
        Ok(())
    }
}
impl Drop for StoreUserd {
    fn drop(&mut self) {
        // unmap before giving the view back (the order `ChanPlane::drop_relay` uses)
        drop(self.region.take());
        let _ = self.rm.release_cpu_view(kf_host::CpuViewRelease {
            h_memory: self.store,
            p_linear_address: self.cookie,
        });
    }
}

/// Kept even after statement-time Free; only confirmed native free retires it.
#[derive(Clone)]
pub(super) enum UserdSource {
    /// ★ DEBUG ONLY: a framebuffer (host store) USERD, [`StoreUserd`].
    Store(Arc<StoreUserd>),
    /// CPU-RM resolved birth descriptor; its CPU-local handle is not a GSP name.
    Resolved(crate::mem::userdram::Userd),
    /// Later migration carries an independently registered GSP MemoryList name.
    Registered(kf_rm::memory_access::Lease),
}
fn check_userd_owner(
    current: Option<&Arc<()>>,
    expected: &Arc<()>,
) -> Result<(), kf_rm::memory_list::AccessError> {
    if current.is_some_and(|id| Arc::ptr_eq(id, expected)) {
        Ok(())
    } else {
        Err(kf_rm::memory_list::AccessError::Gone)
    }
}
impl UserdSource {
    fn owner<'a>(
        me: &'a ChanPlane,
        key: (u32, u32),
        allocation: &Arc<()>,
    ) -> Result<std::sync::MutexGuard<'a, PtMap>, kf_rm::memory_list::AccessError> {
        use kf_rm::memory_list::AccessError;
        let pt = me.pt.try_lock().map_err(|e| match e {
            std::sync::TryLockError::WouldBlock => AccessError::Busy,
            _ => AccessError::Poisoned,
        })?;
        check_userd_owner(
            pt.get(&key).filter(|t| t.user_ring).map(|t| &t.allocation),
            allocation,
        )?;
        Ok(pt)
    }
    pub fn read(
        &self,
        me: &ChanPlane,
        key: (u32, u32),
        allocation: &Arc<()>,
        offset: u64,
        bytes: &mut [u8],
    ) -> Result<(), kf_rm::memory_list::AccessError> {
        match self {
            Self::Resolved(source) => {
                let _owner = Self::owner(me, key, allocation)?;
                source.read(offset, bytes)
            }
            Self::Store(source) => {
                let _owner = Self::owner(me, key, allocation)?;
                source.read(offset, bytes)
            }
            Self::Registered(source) => {
                source.read_admitted(offset, bytes, || Self::owner(me, key, allocation))
            }
        }
    }
    pub fn write(
        &self,
        me: &ChanPlane,
        key: (u32, u32),
        allocation: &Arc<()>,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), kf_rm::memory_list::AccessError> {
        match self {
            Self::Resolved(source) => {
                let _owner = Self::owner(me, key, allocation)?;
                source.write(offset, bytes)
            }
            Self::Store(source) => {
                let _owner = Self::owner(me, key, allocation)?;
                source.write(offset, bytes)
            }
            Self::Registered(source) => {
                source.write_admitted(offset, bytes, || Self::owner(me, key, allocation))
            }
        }
    }
    /// Exact pending Book.commit owns the descriptor before Pt attachment.
    fn initialize(
        &self,
        expected: &Arc<()>,
        identity: &Arc<()>,
    ) -> Result<(), kf_rm::memory_list::AccessError> {
        check_userd_owner(Some(identity), expected)?;
        match self {
            Self::Resolved(source) => source.write(0, &[0; 0x200]),
            Self::Store(source) => source.zero(),
            Self::Registered(_) => Err(kf_rm::memory_list::AccessError::Unsupported),
        }
    }
    /// The closure must retain exact Pt and final birth RAM-generation guards
    /// through publication. A resolved descriptor needs no fabricated graph node.
    pub fn admit<T>(
        &self,
        publish: impl FnOnce() -> T,
    ) -> Result<T, kf_rm::memory_list::AccessError> {
        match self {
            Self::Resolved(_) | Self::Store(_) => Ok(publish()),
            Self::Registered(source) => source.admit(publish),
        }
    }
}

pub(super) struct Record {
    pub allocation: Arc<()>,
    pub quota: Arc<super::userquota::NativeQuota>,
    pub key: (u32, u32),
    pub idx: u32,
    pub chan: Option<kf_host::Channel>,
    pub lease: Arc<crate::userva::Lease>,
    pub mirror: Mirror,
    pub userd: UserdSource,
    pub source: (u64, u32),
    pub vas_key: VasKey,
    pub pump: Option<super::userpump::Pump>,
    notifier: Option<PtNotifier>,
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
            if let Some(q) = &self.quota {
                let _ = q.release_confirmed();
            }
            self.live.fetch_sub(1, Ordering::AcqRel);
            if let Some(t) = &self.twin {
                t.user_done();
            }
        }
    }
}

/// Validate class and bounded ring geometry before resolving own-VM USERD storage.
fn source_declaration(
    family: kf_chip::Family,
    class: u32,
    entries: u32,
    gpfifo_va: u64,
) -> Result<(), &'static str> {
    // Preserve the integration line's exact Ada admission set. The generated
    // Ada list also names c36f/c46f; admitting them needs separate native proof.
    let ada_allowed = family != kf_chip::Family::Ada
        || class == kf_abi::generated::classes::AMPERE_CHANNEL_GPFIFO_A;
    if !ada_allowed
        || family.classes().kind_of(class) != Some(kf_chip::classes::Kind::ChannelGpfifo)
        || entries < 2
        || !entries.is_power_of_two()
        || entries > (1 << 20)
        || gpfifo_va & 7 != 0
        || u64::from(entries)
            .checked_mul(8)
            .and_then(|n| gpfifo_va.checked_add(n))
            .is_none()
    {
        return Err("USER_RING_SOURCE_DECLARATION");
    }
    Ok(())
}

struct Birth {
    a: ChannelAlloc,
    mirror: Mirror,
    idx: u32,
    pending: Arc<crate::userbirth::Birth>,
    d: kf_gsp::Deferred,
    count: NeverBorn,
    orphaned: Arc<AtomicBool>,
    err_at: Option<(u32, u64, kf_arch::UserdMem)>,
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
        // CPU-local birth handles need not have a GSP MemoryList registration.
        // The CPU-RM resolved descriptor supplies bounded own-VM RAM; runtime
        // CPU access additionally retains exact Pt allocation identity.
        let source = (|| {
            source_declaration(me.family, a.class, a.entries, a.gpfifo_va)
                .map_err(str::to_string)?;
            let declared = a.subdevice_id.ok_or("USER_RING_SUBDEVICE_ID_UNMEASURED")?;
            let native_id = if shadow_enabled() && declared == 0 {
                0 // Existing opaque diagnostic retains the host default.
            } else {
                authored_subdevice_id(declared)?
            };
            let (handle, offset) = a.userd_source.ok_or("USER_RING_NO_USERD_DECLARATION")?;
            if handle == 0 || offset & 0x1ff != 0 {
                return Err("USER_RING_USERD_DECLARATION".into());
            }
            match a.userd {
                Some(kf_arch::UserdMem::Sysmem { base, size }) => {
                    crate::mem::userdram::Userd::new(me.ram, base, size)
                        .map(|source| (UserdSource::Resolved(source), native_id))
                        .map_err(|e| format!("USER_RING_RESOLVED_USERD {e:?}"))
                }
                // ★ DEBUG ONLY: only a channel the debug switch made a user ring can be here with a
                // framebuffer USERD (a Windows permit is never granted for one: the scope decision
                // of V3_WINDOWS_USER_RING_RELAY.md §10 stands for the production route).
                Some(kf_arch::UserdMem::Framebuffer { base, .. }) if kf_rm::chanlink::force_migratable_enabled() => {
                    StoreUserd::new(me.rm, me.store, base)
                        .map(|view| (UserdSource::Store(Arc::new(view)), native_id))
                }
                _ => Err("USER_RING_USERD_APERTURE_NOT_IMPLEMENTED".to_string()),
            }
        })();
        let (userd, native_id) = match source {
            Ok(u) => u,
            Err(why) => {
                // Never born: the queue still owns the lease on enqueue failure.
                let cleanup = me.inbox.user_va.release_after_channel_free(lease);
                let _ = me.inbox.wake.signal();
                return Err(fail(format!("{why}; storage cleanup {cleanup:?}")));
            }
        };
        let pump =
            match super::userpump::Pump::new(me, native_id, (a.client, a.handle))
            {
                Ok(pump) => pump,
                Err(why) => {
                    let cleanup = me.inbox.user_va.release_after_channel_free(lease);
                    let _ = me.inbox.wake.signal();
                    return Err(fail(format!("{why}; never-born cleanup {cleanup:?}")));
                }
            };
        let notifier = self
            .err_at
            .and_then(|(obj, off, at)| me.arm_notifier(a.client, a.handle, obj, off, at));
        let spec = match lease
            .storage
            .lock()
            .map_err(|_| "USER_RING_STORAGE_POISONED".into())
            .and_then(|s| s.spec(notifier.as_ref().map_or(0, |n| n.ctx)))
        {
            Ok(spec) => spec,
            Err(why) => {
                if let Some(n) = notifier {
                    me.release_notifier(n);
                }
                let cleanup = me.inbox.user_va.release_after_channel_free(lease.clone());
                let _ = me.inbox.wake.signal();
                return Err(fail(format!("{why}; storage cleanup {cleanup:?}")));
            }
        };
        let record = Arc::new(Cell::new(
            self.idx,
            Record {
                quota: self
                    .count
                    .quota
                    .as_ref()
                    .ok_or_else(|| fail("USER_RING_QUOTA_ABSENT".into()))?
                    .clone(),
                allocation: self.pending.allocation(),
                key: (a.client, a.handle),
                idx: self.idx,
                chan: None,
                lease: lease.clone(),
                mirror: self.mirror.clone(),
                userd,
                source: (a.gpfifo_va, a.entries),
                retired: false,
                free_attempted: false,
                dead: None,
                vas_key: VasKey(
                    (u64::from(a.vaspace_client) << 32)
                        | u64::from(
                            a.vaspace
                                .ok_or_else(|| fail("USER_RING_NO_VASPACE".into()))?,
                        ),
                ),
                pump: Some(pump),
                notifier,
            },
        ));
        let hold = (|| {
            let mut holds = me
                .user_birth_holds
                .lock()
                .map_err(|_| "USER_RING_HOLDS_POISONED")?;
            if holds.len() >= 512 {
                return Err("USER_RING_NATIVE_ATTEMPT_BUDGET");
            }
            holds
                .try_reserve(1)
                .map_err(|_| "USER_RING_NATIVE_ATTEMPT_ALLOCATION")?;
            holds.push((self.pending.allocation(), record.clone()));
            Ok(())
        })();
        if let Err(why) = hold {
            if let Ok(mut r) = record.lock() {
                if let Some(n) = r.notifier.take() {
                    me.release_notifier(n);
                }
            }
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
        let shadow_default = shadow_enabled() && a.subdevice_id == Some(0);
        if shadow_enabled() {
            eprintln!(
                "kf3: EXPERIMENT KF3_WIN_USER_RING_SHADOW {:#x}:{:#x} declared_id={:x?}; opaque originalGP, no CPU API/staging/witness; mask1 sampled hypothesis for declared0, not guest parity",
                a.client, a.handle, a.subdevice_id
            );
        }
        let native = match (join, shadow_default) {
            (Some(tsg), true) => me.rm.birth_member(
                tsg,
                a.engine_type.unwrap_or(0),
                spec,
                false,
                kf_host::channel::PhysicalCeBelt::Deny,
            ),
            (None, true) => {
                me.rm
                    .birth_channel(self.mirror.space, a.engine_type.unwrap_or(0), spec)
            }
            (Some(tsg), false) => me.rm.birth_user_ring_member(
                tsg,
                a.engine_type.unwrap_or(0),
                spec,
                false,
                native_id,
            ),
            (None, false) => me.rm.birth_user_ring_channel(
                self.mirror.space,
                a.engine_type.unwrap_or(0),
                spec,
                native_id,
            ),
        };
        let chan = match native {
            Ok(chan) => chan,
            Err(why) => {
                if let Ok(mut r) = record.lock() {
                    r.dead = Some(format!("NATIVE_BIRTH_UNCERTAIN {why:?}"));
                }
                eprintln!(
                    "kf3: USER_RING_NATIVE_BIRTH_QUARANTINED {:#x}:{:#x} {why:?}; storage and counts retained, no token handed out",
                    a.client, a.handle
                );
                return Err(fail(format!("USER_RING_NATIVE_BIRTH_UNCERTAIN {why:?}")));
            }
        };
        record
            .lock()
            .map_err(|_| fail("USER_RING_RECORD_POISONED".into()))?
            .chan = Some(chan);
        // Every failure after native birth still has an explicit attempt owner.
        {
            let mut rings = me
                .user_rings
                .lock()
                .map_err(|_| fail("USER_RING_RECORDS_POISONED".into()))?;
            if rings.contains_key(&chan.token) {
                return Err(fail("USER_RING_NATIVE_TOKEN_COLLISION_QUARANTINED".into()));
            }
            rings.insert(chan.token, record.clone());
        }
        if let Some(k) = gkey {
            let mut groups = me
                .groups
                .lock()
                .map_err(|_| fail("USER_RING_GROUPS_POISONED".into()))?;
            groups
                .entry(k)
                .and_modify(|g| g.1 = g.1.saturating_add(1))
                .or_insert((chan.tsg, 1));
        }
        let ready = if self.orphaned.load(Ordering::Acquire) {
            Err("USER_RING_BIRTH_ORPHANED".into())
        } else {
            crate::latejoin::schedule_late_joiner(me.rm, &me.guest_sched, a.client, a.tsg, chan)
                .map(|_| ())
        };
        if let Err(why) = ready {
            let cleanup = me.release_twin(a.client, a.tsg, a.ctx_share, chan);
            if cleanup.is_ok() {
                self.count.native_attempted = false;
            }
            return Err(fail(format!("{why}; native cleanup {cleanup:?}")));
        }
        let attached = me.user_births.commit(&self.pending, |identity| {
            // Exact pending birth excludes cancellation/reuse while zeroing the
            // newly allocated USERD. Bounded CPU access only; no native verb.
            let mut r = record
                .try_lock()
                .map_err(|_| "USER_RING_RECORD_BUSY_AT_COMMIT")?;
            r.userd
                .initialize(&r.allocation, &identity)
                .map_err(|e| format!("USER_RING_INITIAL_USERD {e:?}"))?;
            let mut pt = me.pt.lock().map_err(|_| "USER_RING_PT_POISONED")?;
            if pt.contains_key(&(a.client, a.handle)) {
                return Err("USER_RING_LIVE_COLLISION".into());
            }
            pt.insert(
                (a.client, a.handle),
                PtChan {
                    allocation: identity.clone(),
                    chan,
                    idx: self.idx,
                    tsg: a.tsg,
                    ctx_share: a.ctx_share,
                    parent: a.parent,
                    device: a.device,
                    engine: a.engine_type.unwrap_or(0),
                    objects: HashMap::new(),
                    pending_objects: HashMap::new(),
                    disp_sw: HashMap::new(),
                    sw_ids: crate::dispsw::SwClassIds::default(),
                    dispsw_host_refused: false,
                    defapi_host: None,
                    user_defapi: None,
                    user_published: false,
                    peek_defapi: None,
                    ctx: CtxBind::default(),
                    user_work: true,
                    user_ring: true,
                    live: self.count.live.clone(),
                    twin: self.count.twin.clone(),
                    notifier: r.notifier.take(),
                    stopped: false,
                    disabled: false,
                    space: self.mirror.space,
                    rows: self.mirror.rows.clone(),
                    user_regions: self.mirror.user_regions.clone(),
                    ledger: self.mirror.ledger.clone(),
                    falcon_ctx: None,
                    ring_at: (
                        a.gpfifo_va,
                        a.entries,
                        match a.userd {
                            Some(kf_arch::UserdMem::Sysmem { base, .. }) => Some(base),
                            _ => None,
                        },
                    ),
                },
            );
            if let Err(e) = self
                .count
                .quota
                .as_ref()
                .ok_or("USER_RING_QUOTA_ABSENT")?
                .attach(chan.token)
            {
                if let Some(mut t) = pt.remove(&(a.client, a.handle)) {
                    r.notifier = t.notifier.take();
                }
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
        Ok(format!(
            "Windows USER ring chan {:#x}:{:#x} BORN USER private-read-only-ring token {:#x} host {:#x}; OWN VAS; worker-only; source={:#x}x{} declared_subdevice={:x?} native_scan_subdevice={native_id:#x}",
            a.client, a.handle, self.idx, chan.token, a.gpfifo_va, a.entries, a.subdevice_id
        ))
    }
}

impl ChanPlane {
    /// Statement-time counts have already been acquired by the existing USER
    /// admission. No new route for Linux or a channel without the birth permit.
    pub(super) fn birth_user_ring(
        &self,
        a: ChannelAlloc,
        mirror: Mirror,
        idx: u32,
        twin: Option<Arc<crate::twin::TwinState>>,
        err_at: Option<(u32, u64, kf_arch::UserdMem)>,
    ) -> ChanAnswer {
        let mut count = NeverBorn {
            live: mirror.live.clone(),
            twin,
            native_attempted: false,
            quota: None,
        };
        let quota = match super::userquota::NativeQuota::reserve(self.caps.clone(), self.plane, idx)
        {
            Ok(q) => q,
            Err(why) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_INSUFFICIENT_RESOURCES,
                    why,
                };
            }
        };
        count.quota = Some(quota);
        let pending = match self
            .user_births
            .allocate(&self.inbox.user_va, &a, mirror.clone())
        {
            Ok(p) => p,
            Err(why) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_NOT_SUPPORTED,
                    why,
                };
            }
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
        let birth = Birth {
            a,
            mirror,
            idx,
            pending,
            d: d.clone(),
            count,
            orphaned,
            err_at,
        };
        self.defer_cell("Windows USER ring birth", birth.act(), d)
    }

    /// Off-drainer/native-free exclusion. Retain the record after any native
    /// failure. Even a failed VA cleanup enqueue leaves Queue's strong pin.
    pub(super) fn user_ring_record(&self, token: u32) -> Option<Arc<Cell>> {
        self.user_rings.lock().ok()?.get(&token).cloned()
    }
}

impl ChanPlane {
    pub(super) fn finish_user_ring_free(
        &self,
        token: u32,
        record: Option<&Arc<Cell>>,
        native_freed: bool,
    ) {
        let Some(record) = record else { return };
        if !native_freed {
            return;
        }
        let (lease, identity, quota, sources_ok, notifier) = {
            let Ok(mut r) = record.lock() else { return };
            r.retired = true;
            record.cancel();
            let sources_ok = r.pump.as_mut().is_none_or(|p| match p.release(&self.relay_stats) {
                Ok(()) => true,
                Err(why) => {
                    eprintln!("kf3: {why}; source views retained until session close");
                    false
                }
            });
            (
                r.lease.clone(),
                r.allocation.clone(),
                r.quota.clone(),
                sources_ok,
                r.notifier.take(),
            )
        };
        if let Some(n) = notifier {
            self.release_notifier(n);
        }
        if let Err(why) = quota.release_confirmed() {
            eprintln!("kf3: USER_RING_QUOTA_RETAINED after confirmed native free: {why}");
        }
        // Queue metadata may contend; no publisher guard is held over it.
        let cleanup = self.inbox.user_va.release_after_channel_free(lease);
        let _ = self.inbox.wake.signal();
        if cleanup.is_ok() && sources_ok {
            let mut last = false;
            if let Ok(mut rings) = self.user_rings.lock() {
                rings.remove(&token);
                last = rings.is_empty();
            }
            // ★ 2026-10-14: the event loop's counters whenever the last relay goes (bounded: the first
            // 16 times, then each power of two) -- the device may never see `stop` (a guest power-off).
            if last {
                let n = self.relay_reports.fetch_add(1, Ordering::Relaxed) + 1;
                if n <= 16 || n.is_power_of_two() {
                    eprintln!("kf3: USER_RING_LOOP SUMMARY (all relays gone, #{n}) {}", self.relay_stats.summary());
                    if self.relay_stats.life.any() {
                        eprintln!("kf3: USER_RING_LIFECYCLE SUMMARY (all relays gone, #{n}) {}", self.relay_stats.life.summary());
                    }
                    if self.relay_stats.lazy.any() {
                        eprintln!("kf3: USER_RING_LAZY_OBSERVE SUMMARY (all relays gone, #{n}) {}", self.relay_stats.lazy.summary());
                    }
                    eprintln!("kf3: {}", crate::userstore::summary());
                }
            }
            if let Ok(mut holds) = self.user_birth_holds.lock() {
                holds.retain(|(id, _)| !Arc::ptr_eq(id, &identity));
            }
        } else {
            eprintln!("kf3: USER_RING_STORAGE_CLEANUP_RETAINED host {token:#x}: {cleanup:?}");
        }
    }

    pub(super) fn retain_user_ring_notifier(
        &self,
        token: u32,
        allocation: &Arc<()>,
        n: PtNotifier,
    ) {
        if let Some(record) = self.user_ring_record(token) {
            if let Ok(mut r) = record.lock() {
                if Arc::ptr_eq(&r.allocation, allocation) && r.notifier.is_none() {
                    r.notifier = Some(n);
                    return;
                }
            }
        }
        // Unexpected owner loss: keep CPU/RM resources until session close.
        // Do not attempt native context free without confirmed channel cleanup.
        std::mem::forget(n);
        eprintln!(
            "kf3: USER_RING_NOTIFIER_OWNER_LOST host {token:#x}; retained until session close"
        );
    }

    fn release_never_born_record(&self, record: &Arc<Cell>) {
        let (lease, identity, quota, sources_ok, notifier) = {
            let Ok(mut r) = record.lock() else { return };
            r.retired = true;
            record.cancel();
            let sources_ok = r.pump.as_mut().is_none_or(|p| match p.release(&self.relay_stats) {
                Ok(()) => true,
                Err(why) => {
                    eprintln!("kf3: {why}; source views retained until session close");
                    false
                }
            });
            (
                r.lease.clone(),
                r.allocation.clone(),
                r.quota.clone(),
                sources_ok,
                r.notifier.take(),
            )
        };
        if let Some(n) = notifier {
            self.release_notifier(n);
        }
        let _ = quota.release_confirmed();
        if self.inbox.user_va.release_after_channel_free(lease).is_ok() && sources_ok {
            if let Ok(mut holds) = self.user_birth_holds.lock() {
                holds.retain(|(id, _)| !Arc::ptr_eq(id, &identity));
            }
        }
        let _ = self.inbox.wake.signal();
    }

    pub(super) fn retire_user_ring_token(&self, token: u32, identity: &Arc<()>) -> bool {
        let Some(record) = self.user_ring_record(token) else {
            return false;
        };
        let quota = {
            let Ok(r) = record.try_lock() else {
                return false;
            };
            if !Arc::ptr_eq(&r.allocation, identity) {
                return false;
            }
            record.cancel();
            r.quota.clone()
        };
        quota.retire_token()
    }

    fn withdraw_user_ring(
        &self,
        key: (u32, u32),
        identity: &Arc<()>,
    ) -> Result<String, (u32, String)> {
        let t = {
            let mut pt = self
                .pt
                .lock()
                .map_err(|_| (NV_ERR_INVALID_STATE, "USER_RING_PT_POISONED".into()))?;
            if !pt
                .get(&key)
                .is_some_and(|t| t.user_ring && Arc::ptr_eq(&t.allocation, identity))
            {
                return Ok("USER_RING_WITHDRAW_ALREADY_GONE".into());
            }
            pt.remove(&key)
                .ok_or((NV_ERR_INVALID_STATE, "USER_RING_WITHDRAW_GONE".into()))?
        };
        self.retire_user_ring_token(t.chan.token, &t.allocation);
        if let Some(twin) = &t.twin {
            twin.user_freeing();
        }
        let native = self.release_twin(key.0, t.tsg, t.ctx_share, t.chan);
        if native.is_ok() {
            t.live.fetch_sub(1, Ordering::AcqRel);
            if let Some(twin) = &t.twin {
                twin.user_released();
            }
            if let Some(n) = t.notifier {
                self.release_notifier(n);
            }
        } else if let Some(n) = t.notifier {
            self.retain_user_ring_notifier(t.chan.token, &t.allocation, n);
        }
        self.engine_live(t.engine, false);
        Ok(format!(
            "USER_RING_WITHDRAW exact {key:x?} host {:#x} {native:?}",
            t.chan.token
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_core::lifetime::Step;
    use kf_core::{HostOps, HostSlice, Owner, Plane, Translatable, VmCaps, Vmm};
    use kf_trap::token::Route;

    #[test]
    fn user_ring_source_accepts_generated_non_ada_channels() {
        for family in kf_chip::Family::ALL {
            if family == kf_chip::Family::Ada {
                continue;
            }
            for &class in family.classes().channel_gpfifo {
                assert_eq!(
                    source_declaration(family, class, 1024, 0x300200000),
                    Ok(()),
                    "{family:?} {class:#x}"
                );
            }
        }
    }

    #[test]
    fn user_ring_source_turing_accepts_both_ring_size_boundaries() {
        for entries in [2, 1 << 20] {
            assert_eq!(
                source_declaration(kf_chip::Family::Turing, 0xc46f, entries, 0),
                Ok(())
            );
        }
    }

    #[test]
    fn user_ring_source_ad104_ada_accepts_exactly_the_previous_c56f_set() {
        for class in (0..=u16::MAX as u32).chain([0x1c56f, u32::MAX]) {
            assert_eq!(
                source_declaration(kf_chip::Family::Ada, class, 1024, 0x1000).is_ok(),
                class == 0xc56f,
                "Ada class {class:#x}"
            );
        }
    }

    #[test]
    fn user_ring_source_ada_generated_predecessors_remain_refused() {
        let family = kf_chip::Family::Ada;
        for class in [0xc36f, 0xc46f] {
            assert!(family.classes().channel_gpfifo.contains(&class));
            assert_eq!(
                source_declaration(family, class, 2, 0),
                Err("USER_RING_SOURCE_DECLARATION")
            );
        }
    }

    #[test]
    fn user_ring_source_rejects_generated_non_channel_objects() {
        for family in kf_chip::Family::ALL {
            for kind in kf_chip::classes::Kind::ALL {
                if kind == kf_chip::classes::Kind::ChannelGpfifo {
                    continue;
                }
                for &class in family.classes().of_kind(kind) {
                    assert!(
                        source_declaration(family, class, 1024, 0x1000).is_err(),
                        "{family:?} {kind:?} {class:#x}"
                    );
                }
            }
        }
    }

    #[test]
    fn user_ring_source_rejects_unknown_and_foreign_channel_ids() {
        for (family, classes) in [
            (kf_chip::Family::Turing, [0, 0xc56f, 0xca6f, u32::MAX]),
            (kf_chip::Family::Ada, [0, 0xc86f, 0xca6f, u32::MAX]),
        ] {
            for class in classes {
                assert!(source_declaration(family, class, 1024, 0x1000).is_err());
            }
        }
    }

    #[test]
    fn user_ring_source_geometry_rejects_small_rings_alignment_and_overflow() {
        let family = kf_chip::Family::Ada;
        for entries in [0, 1, 3, (1 << 20) + 1, 1 << 21] {
            assert!(source_declaration(family, 0xc56f, entries, 0x1000).is_err());
        }
        for va in [1, 7, u64::MAX - 7] {
            assert!(source_declaration(family, 0xc56f, 2, va).is_err());
        }
    }

    #[test]
    fn user_ring_source_geometry_refuses_hostile_maximum_declarations() {
        let family = kf_chip::Family::Ada;
        for entries in [0, 1, 5, (1 << 20) + 2, u32::MAX] {
            assert!(source_declaration(family, 0xc56f, entries, 0).is_err());
        }
        for va in [2, 6, u64::MAX - 15] {
            assert!(source_declaration(family, 0xc56f, 1024, va).is_err());
        }
    }

    #[test]
    fn windows_relay_is_the_default_with_an_explicit_negative_control() {
        assert!(relay_enabled(None));
        assert!(relay_enabled(Some("1")));
        assert!(!relay_enabled(Some("0")));
    }

    #[test]
    fn guest_default_authors_the_same_explicit_identity_for_native_and_scan() {
        assert_eq!(authored_subdevice_id(0), Ok(1));
        assert_eq!(authored_subdevice_id(1), Ok(1));
        for bit in 0..12 {
            assert_eq!(authored_subdevice_id(1 << bit), Ok(1 << bit));
        }
        for invalid in [3, 0x1000, u32::MAX] {
            assert!(authored_subdevice_id(invalid).is_err());
        }
    }
    struct State {
        paused: bool,
        writes: usize,
    }
    struct Host {
        cell: Cell<State>,
        metadata: Mutex<()>,
    }
    #[test]
    fn resolved_userd_owner_requires_exact_channel_incarnation() {
        let allocation = Arc::new(());
        assert_eq!(check_userd_owner(Some(&allocation), &allocation), Ok(()));
        assert_eq!(
            check_userd_owner(None, &allocation),
            Err(kf_rm::memory_list::AccessError::Gone)
        );
        let reused = Arc::new(());
        assert_eq!(
            check_userd_owner(Some(&reused), &allocation),
            Err(kf_rm::memory_list::AccessError::Gone)
        );
    }
    impl HostOps for Host {
        fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
            if self.cell.alive(|_| true) {
                Translatable::Yes
            } else {
                Translatable::No
            }
        }
        fn run_translated(&self, _: u32, _: u64) -> bool {
            let Ok(mut r) = self.cell.enter() else {
                return false;
            };
            let Ok(_metadata) = self.metadata.try_lock() else {
                return false;
            };
            if r.paused {
                return false;
            }
            self.cell.retry.store(false, Ordering::Release);
            r.writes += 1;
            true
        }
        fn ring_host(&self, _: u32) {
            panic!("private USER route must use worker");
        }
        fn run_emulated(&self, _: u32, _: u64) {
            panic!("no emulation");
        }
        fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {}
        fn forge_completion(&self, _: u32) {
            panic!("no forged completion");
        }
        fn refuse_and_poison(&self, _: u32) {
            panic!("busy is not poison");
        }
        fn fault_channel(&self, _: u32) {
            panic!("record is admitted, including contention");
        }
        fn map_guest_slice(&self, _: HostSlice) {}
        fn teardown_step(&self, _: Step) {}
    }
    fn setup(vmm: &Vmm) -> (Plane<'_>, Host) {
        let plane = Plane::new(vmm, 4, 3);
        let mut caps = VmCaps::from_declared(2, 8, 8, 8);
        plane
            .allocate_channel(&mut caps, 0, Route::Translated, 11, Owner::User)
            .unwrap();
        (
            plane,
            Host {
                cell: Cell::new(
                    0,
                    State {
                        paused: false,
                        writes: 0,
                    },
                ),
                metadata: Mutex::new(()),
            },
        )
    }
    fn pass(plane: &Plane<'_>, host: &Host) {
        plane.worker_pass(host, &mut Vec::new(), 64);
    }
    #[test]
    fn plane_user_worker_admission_keeps_final_doorbell_across_record_contention() {
        let vmm = Vmm::default();
        let (plane, host) = setup(&vmm);
        let guard = host.cell.lock().unwrap();
        plane.ring_internal(0);
        pass(&plane, &host);
        assert!(host.cell.retry.load(Ordering::Acquire));
        drop(guard);
        // Production worker refresh sees durable demand without another guest ring.
        if host.cell.retry.load(Ordering::Acquire) {
            plane.ring_internal(host.cell.idx);
        }
        pass(&plane, &host);
        assert_eq!(host.cell.lock().unwrap().writes, 1);
        assert!(!host.cell.retry.load(Ordering::Acquire));
    }
    #[test]
    fn plane_final_doorbell_survives_pt_contention_and_paused_channel() {
        let vmm = Vmm::default();
        let (plane, host) = setup(&vmm);
        let guard = host.metadata.lock().unwrap();
        plane.ring_internal(0);
        pass(&plane, &host);
        assert!(host.cell.retry.load(Ordering::Acquire));
        drop(guard);
        host.cell.lock().unwrap().paused = true;
        plane.ring_internal(0);
        pass(&plane, &host);
        assert!(host.cell.retry.load(Ordering::Acquire));
        assert_eq!(host.cell.lock().unwrap().writes, 0);
        host.cell.lock().unwrap().paused = false;
        // Successful schedule-enable emits the same internal ring as production.
        plane.ring_internal(0);
        pass(&plane, &host);
        assert_eq!(host.cell.lock().unwrap().writes, 1);
    }
    #[test]
    fn retained_failed_cleanup_cannot_rearm_a_reused_guest_index() {
        let cell = Cell::new(7, ());
        // Pending final doorbell, followed by cleanup that retains all storage
        // (native Free or source-view cleanup failed). Cancellation is final.
        assert!(cell.request());
        {
            let _guard = cell.lock().unwrap();
            cell.cancel();
        }
        assert!(!cell.pending());
        assert!(!cell.alive(|_| true));
        let writes = AtomicU64::new(0);
        for _ in 0..8 {
            assert!(!cell.request());
            cell.wake_if(
                |_| true,
                |_| {
                    writes.fetch_add(1, Ordering::Relaxed);
                },
            );
        }
        assert_eq!(writes.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn cleanup_exclusion_cancels_concurrent_wake_without_index_publication() {
        let cell = Cell::new(7, ());
        let writes = AtomicU64::new(0);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let guard = cell.lock().unwrap();
            scope.spawn(|| {
                assert!(cell.request());
                cell.wake_if(
                    |_| true,
                    |_| {
                        writes.fetch_add(1, Ordering::Relaxed);
                    },
                );
                barrier.wait();
                barrier.wait();
                // Simulate a late ready/enable callback after retirement.
                assert!(!cell.request());
                cell.wake_if(
                    |_| true,
                    |_| {
                        writes.fetch_add(1, Ordering::Relaxed);
                    },
                );
            });
            barrier.wait();
            cell.cancel();
            drop(guard);
            barrier.wait();
        });
        assert!(!cell.pending());
        assert_eq!(writes.load(Ordering::Relaxed), 0);
    }
}
