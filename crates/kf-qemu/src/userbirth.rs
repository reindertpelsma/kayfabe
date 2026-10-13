//! Exact pending birth identity for the restricted Windows USER relay.
//!
//! The admission caller first counts the USER twin in statement order. The VA
//! request owns its own mirror pin. This registry never releases that USER
//! count, a native channel, or a handed-out lease: those belong to the birth
//! act until native-free/never-born cleanup is confirmed.
//!
//! The opt-in USER birth caller uses commit for exact attachment and bounded
//! CPU USERD initialization. Free cancels pending scopes before removing live
//! channels and retains that ordering. Never hold
//! this registry across a host verb. The production registry signals the VA
//! worker after allocation and cancellation/orphan drop; it never performs
//! storage cleanup itself.

use crate::mem::Mirror;
use crate::userva::{Lease, Queue, Reply};
use kf_chan::user_storage::CpuView;
use kf_mem::vasmgr::VasKey;
use kf_rm::chanlink::ChannelAlloc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

/// Trusted VM pending-birth bookkeeping cap; not a die/driver fact.
const PENDING: usize = 512;
type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
struct Scope {
    key: (u32, u32),
    parent: u32,
    device: u32,
    tsg: Option<u32>,
}
impl Scope {
    fn freed_by(self, client: u32, object: u32) -> bool {
        self.key.0 == client
            && (object == client
                || object == self.key.1
                || object == self.parent
                || object == self.device
                || self.tsg == Some(object))
    }
}

/// One exact guest channel allocation, retained through queued/parked acts.
pub(crate) struct Birth<V = CpuView> {
    scope: Scope,
    allocation: Arc<()>,
    reply: Arc<Reply<V>>,
    // Accessed only under Book::pending; no native birth may precede handout.
    handed_out: std::sync::atomic::AtomicBool,
    wake: Option<Wake>,
}
impl<V> Birth<V> {
    /// Exact identity for metadata prepared before commit/token publication.
    pub(crate) fn allocation(&self) -> Arc<()> { self.allocation.clone() }
}

impl<V> Drop for Birth<V> {
    fn drop(&mut self) {
        // Metadata only. Taken refuses cancellation and remains caller-owned.
        if self.reply.cancel_before_birth().is_ok()
            && let Some(wake) = &self.wake
        {
            wake();
        }
    }
}

/// Weak rows do not own failed/abandoned acts or their pending resources.
pub(crate) struct Book<V = CpuView> {
    pending: Mutex<HashMap<(u32, u32), Weak<Birth<V>>>>,
    wake: Option<Wake>,
}
impl<V> Default for Book<V> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            wake: None,
        }
    }
}

impl<V> Book<V> {
    /// The installed callback must only signal the nonblocking VA eventfd.
    pub(crate) fn with_wake(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            wake: Some(Arc::new(wake)),
        }
    }

    /// Caller has already admitted the USER twin and checked no live channel
    /// owns this handle. No storage request without the birth-time permit.
    pub(crate) fn allocate(
        &self,
        queue: &Queue<V>,
        a: &ChannelAlloc,
        mirror: Mirror,
    ) -> Result<Arc<Birth<V>>, String> {
        let permit = a
            .user_ring_permit
            .ok_or("USER_RING_BIRTH_NO_WINDOWS_PERMIT")?;
        let vas = a.vaspace.ok_or("USER_RING_BIRTH_NO_VASPACE")?;
        if a.client == 0 || a.handle == 0 || a.client == a.handle {
            return Err("USER_RING_BIRTH_INVALID_HANDLE".into());
        }
        let scope = Scope {
            key: (a.client, a.handle),
            parent: a.parent,
            device: a.device,
            tsg: a.tsg,
        };
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_BIRTH_BOOK_POISONED")?;
        pending.retain(|_, weak| weak.strong_count() != 0);
        if pending.contains_key(&scope.key) {
            return Err("USER_RING_BIRTH_PENDING_HANDLE".into());
        }
        if pending.len() >= PENDING {
            return Err("USER_RING_BIRTH_PENDING_BUDGET".into());
        }
        pending
            .try_reserve(1)
            .map_err(|_| "USER_RING_BIRTH_PENDING_ALLOCATION")?;
        // Queue admission is bounded metadata only, with no host verb.
        let reply = queue.allocate(
            VasKey((u64::from(a.vaspace_client) << 32) | u64::from(vas)),
            mirror,
            permit,
        )?;
        let birth = Arc::new(Birth {
            scope,
            allocation: Arc::new(()),
            reply,
            handed_out: std::sync::atomic::AtomicBool::new(false),
            wake: self.wake.clone(),
        });
        pending.insert(scope.key, Arc::downgrade(&birth));
        drop(pending);
        if let Some(wake) = &self.wake {
            wake();
        }
        Ok(birth)
    }

    pub(crate) fn is_current(&self, birth: &Arc<Birth<V>>) -> bool {
        self.pending
            .lock()
            .is_ok_and(|pending| current(&pending, birth))
    }

    /// Validate the exact allocation at handout; still requires commit after
    /// native birth, since a guest Free can win during that host operation.
    pub(crate) fn poll(
        &self,
        birth: &Arc<Birth<V>>,
    ) -> Result<Option<Result<Arc<Lease<V>>, String>>, String> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_BIRTH_BOOK_POISONED")?;
        if !current(&pending, birth) {
            return Err("USER_RING_BIRTH_GONE".into());
        }
        let result = birth.reply.take()?;
        if matches!(result, Some(Ok(_))) {
            birth
                .handed_out
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(result)
    }

    /// Exact attachment transaction, before token handout/schedule. `attach`
    /// must only mutate bounded channel metadata, reject a live collision,
    /// and retain this identity in the kept channel. No native verb here.
    pub(crate) fn commit(
        &self,
        birth: &Arc<Birth<V>>,
        attach: impl FnOnce(&Arc<()>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_BIRTH_BOOK_POISONED")?;
        if !current(&pending, birth) {
            return Err("USER_RING_BIRTH_GONE".into());
        }
        if !birth.handed_out.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("USER_RING_BIRTH_NOT_HANDED_OUT".into());
        }
        attach(&birth.allocation)?;
        pending.remove(&birth.scope.key);
        Ok(())
    }

    /// ★ Device reset: the guest clients with a USER-ring birth still pending.
    pub(crate) fn clients(&self) -> Vec<u32> {
        self.pending
            .lock()
            .map(|m| m.keys().map(|k| k.0).collect())
            .unwrap_or_default()
    }

    /// Statement-time subtree cancellation, before the caller removes live
    /// channels. Taken leases remain owned by their acts; failed commit then
    /// requires confirmed native-free/never-born cleanup by that act.
    pub(crate) fn cancel(&self, client: u32, object: u32) -> Result<usize, String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "USER_RING_BIRTH_BOOK_POISONED")?;
        let mut canceled = 0;
        pending.retain(|_, weak| {
            let Some(birth) = weak.upgrade() else {
                return false;
            };
            if !birth.scope.freed_by(client, object) {
                return true;
            }
            let _ = birth.reply.cancel_before_birth();
            canceled += 1;
            false
        });
        drop(pending);
        if canceled != 0
            && let Some(wake) = &self.wake
        {
            wake();
        }
        Ok(canceled)
    }
}

fn current<V>(pending: &HashMap<(u32, u32), Weak<Birth<V>>>, birth: &Arc<Birth<V>>) -> bool {
    pending
        .get(&birth.scope.key)
        .and_then(Weak::upgrade)
        .is_some_and(|active| Arc::ptr_eq(&active, birth))
}
