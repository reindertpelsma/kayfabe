//! Bounded TSG timeslice intent; host calls are outside the bookkeeping lock.

use kf_host::Channel;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Guest client/TSG key.
pub type Key = (u32, u32);
/// Host resource exhaustion for a bounded intent table.
pub const NO_MEMORY: u32 = 0x51;
/// An allocation was freed or superseded before its act ran.
pub const INVALID_STATE: u32 = 0x40;
/// Maximum remembered TSGs per guest client.
pub const PER_CLIENT: usize = 256;
/// Maximum remembered TSGs in the VM.
pub const TOTAL: usize = 4096;

/// Authored host verb seam. The host owns rounding and validation.
pub trait Setter {
    /// SET on this owned host channel's TSG; return the actual RM status on failure.
    fn set(&self, channel: Channel, us: u64) -> Result<(), u32>;
}

// The existing authored SET verb, fixed eight-byte scalar payload. Preserve RM's
// status before RmError's categorization can collapse distinct native errors.
impl Setter for kf_host::HostRm {
    fn set(&self, channel: Channel, us: u64) -> Result<(), u32> {
        let mut payload = us.to_le_bytes();
        match self.raw_control_opaque(
            channel.tsg,
            kf_rm::chanlink::TSG_SET_TIMESLICE,
            &mut payload,
        ) {
            Ok(0) => Ok(()),
            Ok(status) => Err(status),
            Err(_) => Err(INVALID_STATE),
        }
    }
}

/// One queued SET, tied to an allocation lifetime rather than only numeric handles.
#[derive(Clone, Debug)]
pub struct Ticket {
    key: Key,
    device: u32,
    us: u64,
    life: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
}

/// Counters are saturating, and never a decision input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// SETs accepted before any live group exists.
    pub early_sets: u64,
    /// Host calls replaying an early/saved intent onto a newly born group.
    pub replayed: u64,
    /// Replays deliberately suppressed by the A/B kill switch.
    pub dropped: u64,
}

/// Default-on behavior; explicit zero is the A/B kill switch.
pub fn flag_on(value: Option<&str>) -> bool {
    value != Some("0")
}

/// Sample the A/B kill switch once. Needed behavior is enabled by default.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| flag_on(std::env::var("KF3_TIMESLICE_REPLAY").ok().as_deref()))
}

struct Entry {
    device: u32,
    life: Arc<AtomicBool>,
    waiting: usize,
    us: Option<u64>,
}

struct Applied {
    key: Key,
    device: u32,
    us: u64,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    groups: BTreeMap<u32, Applied>,
    queued: usize,
    stats: Stats,
    telemetry_omitted: u64,
}

const APPLIED_CAP: usize = 65536;

/// One VM's bounded intent table and diagnostic applied-group records.
pub struct Replay {
    enabled: bool,
    per_client: usize,
    total: usize,
    state: Mutex<State>,
}

impl Replay {
    /// Bounds are explicit so a tiny fake can exhaust them without hardware.
    pub fn new(enabled: bool, per_client: usize, total: usize) -> Self {
        Self {
            enabled,
            per_client,
            total,
            state: Mutex::default(),
        }
    }
    /// Reserve a bounded lifetime before queueing an act.
    pub fn prepare(&self, key: Key, device: u32, us: u64) -> Result<Ticket, u32> {
        let mut s = self.state.lock().map_err(|_| INVALID_STATE)?;
        if s.queued >= self.total {
            return Err(NO_MEMORY);
        }
        if !s.entries.contains_key(&key) {
            if s.entries.len() >= self.total
                || s.entries.keys().filter(|k| k.0 == key.0).count() >= self.per_client
            {
                return Err(NO_MEMORY);
            }
            s.entries.insert(
                key,
                Entry {
                    device,
                    life: Arc::new(AtomicBool::new(true)),
                    waiting: 0,
                    us: None,
                },
            );
        }
        let e = s.entries.get_mut(&key).ok_or(INVALID_STATE)?;
        if e.device != device {
            return Err(INVALID_STATE);
        }
        e.waiting = e.waiting.checked_add(1).ok_or(NO_MEMORY)?;
        let life = e.life.clone();
        s.queued = s.queued.checked_add(1).ok_or(NO_MEMORY)?;
        Ok(Ticket {
            key,
            device,
            us,
            life,
            active: Arc::new(AtomicBool::new(true)),
        })
    }
    /// Apply live groups once each, or remember an early SET. No lock across host calls.
    pub fn apply(
        &self,
        ticket: &Ticket,
        groups: &[Channel],
        host: &impl Setter,
    ) -> Result<(), u32> {
        if !ticket.active.swap(false, Ordering::AcqRel) {
            return Err(INVALID_STATE);
        }
        let mut result = Ok(());
        let mut done = BTreeSet::new();
        for &c in groups {
            if !ticket.life.load(Ordering::Acquire) {
                result = Err(INVALID_STATE);
                break;
            }
            if done.insert(c.tsg) {
                if let Err(status) = host.set(c, ticket.us) {
                    result = Err(status);
                    break;
                }
                if !self.applied(
                    ticket.key,
                    ticket.device,
                    &ticket.life,
                    c.tsg,
                    ticket.us,
                    false,
                ) {
                    result = Err(INVALID_STATE);
                    break;
                }
            }
        }
        if !ticket.life.load(Ordering::Acquire) {
            result = Err(INVALID_STATE);
        }
        if !self.finish(ticket, result.is_ok(), groups.is_empty()) {
            return Err(INVALID_STATE);
        }
        result
    }
    /// A mirror-accepted birth; only a NEW host group gets the saved value.
    pub fn birth(
        &self,
        key: Key,
        channel: Channel,
        new_group: bool,
        host: &impl Setter,
    ) -> Result<(), u32> {
        if !new_group {
            return Ok(());
        }
        let plan = {
            let s = self.state.lock().map_err(|_| INVALID_STATE)?;
            s.entries
                .get(&key)
                .and_then(|e| e.us.map(|us| (e.device, us, e.life.clone())))
        };
        let Some((device, us, life)) = plan else {
            return Ok(());
        };
        if !self.enabled {
            let mut s = self.state.lock().map_err(|_| INVALID_STATE)?;
            s.stats.dropped = s.stats.dropped.saturating_add(1);
            return Ok(());
        }
        if !life.load(Ordering::Acquire) {
            return Err(INVALID_STATE);
        }
        host.set(channel, us)?;
        if !self.applied(key, device, &life, channel.tsg, us, true) {
            return Err(INVALID_STATE);
        }
        Ok(())
    }
    /// Release a reservation whose act could not be queued.
    pub fn cancel(&self, ticket: &Ticket) {
        if ticket.active.swap(false, Ordering::AcqRel) {
            self.finish(ticket, false, false);
        }
    }
    /// Guest object/client/device free, before numeric handles can be reused.
    pub fn forget(&self, client: u32, object: u32) {
        let Ok(mut s) = self.state.lock() else { return };
        s.entries.retain(|key, e| {
            let gone =
                key.0 == client && (object == client || key.1 == object || e.device == object);
            if gone {
                e.life.store(false, Ordering::Release);
            }
            !gone
        });
        s.groups.retain(|_, e| {
            !(e.key.0 == client && (object == client || e.key.1 == object || e.device == object))
        });
    }
    /// A native group was actually released; guest intent outlives its last member.
    pub fn group_freed(&self, group: u32) {
        if let Ok(mut s) = self.state.lock() {
            s.groups.remove(&group);
        }
    }
    /// Device reset clears all pending/non-host state.
    pub fn clear(&self) {
        if let Ok(mut s) = self.state.lock() {
            for e in s.entries.values() {
                e.life.store(false, Ordering::Release);
            }
            s.entries.clear();
            s.groups.clear();
            // Outstanding old-life acts still own queue reservations until run/canceled.
        }
    }
    /// Clients with non-host intent, including TSGs that never had a channel.
    pub fn clients(&self) -> Vec<u32> {
        self.state
            .lock()
            .map(|s| {
                s.entries
                    .keys()
                    .map(|k| k.0)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Number of remembered/reserved TSGs.
    pub fn len(&self) -> usize {
        self.state.lock().map_or(0, |s| s.entries.len())
    }
    /// No remembered or reserved TSGs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Most recent successfully accepted request for one guest TSG.
    pub fn desired(&self, key: Key) -> Option<u64> {
        self.state.lock().ok()?.entries.get(&key)?.us
    }
    /// Last successfully authored request on this host group, not its rounded hardware value.
    pub fn last_applied(&self, group: u32) -> Option<u64> {
        self.state.lock().ok()?.groups.get(&group).map(|e| e.us)
    }
    /// Atomic snapshot of the counters.
    pub fn stats(&self) -> Stats {
        self.state.lock().map_or(Stats::default(), |s| s.stats)
    }
    /// Bounded status fragment; using a try-lock also makes lock scope testable.
    pub fn summary(&self) -> String {
        let Ok(s) = self.state.try_lock() else {
            return "timeslice=busy".into();
        };
        let groups: Vec<_> = s
            .groups
            .iter()
            .take(16)
            .map(|(group, e)| format!("{group:#x}:{}", e.us))
            .collect();
        format!(
            "timeslice_early_sets={} timeslice_replayed={} timeslice_dropped={} \
            timeslice_last_us=[{}] timeslice_remembered={} timeslice_queued={} \
            timeslice_more_groups={} timeslice_telemetry_omitted={}",
            s.stats.early_sets,
            s.stats.replayed,
            s.stats.dropped,
            groups.join(","),
            s.entries.len(),
            s.queued,
            s.groups.len().saturating_sub(16),
            s.telemetry_omitted
        )
    }

    fn finish(&self, ticket: &Ticket, success: bool, early: bool) -> bool {
        let Ok(mut s) = self.state.lock() else {
            return false;
        };
        s.queued = s.queued.saturating_sub(1);
        let Some(e) = s.entries.get_mut(&ticket.key) else {
            return false;
        };
        if !Arc::ptr_eq(&e.life, &ticket.life) {
            return false;
        }
        e.waiting = e.waiting.saturating_sub(1);
        if success {
            e.us = Some(ticket.us);
        }
        let empty = e.waiting == 0 && e.us.is_none();
        if empty {
            e.life.store(false, Ordering::Release);
            s.entries.remove(&ticket.key);
        }
        if success && early {
            s.stats.early_sets = s.stats.early_sets.saturating_add(1);
        }
        true
    }

    fn applied(
        &self,
        key: Key,
        device: u32,
        life: &Arc<AtomicBool>,
        group: u32,
        us: u64,
        replayed: bool,
    ) -> bool {
        let Ok(mut s) = self.state.lock() else {
            return false;
        };
        if !s
            .entries
            .get(&key)
            .is_some_and(|e| Arc::ptr_eq(&e.life, life))
        {
            return false;
        }
        if s.groups.contains_key(&group) || s.groups.len() < APPLIED_CAP {
            s.groups.insert(group, Applied { key, device, us });
        } else {
            s.telemetry_omitted = s.telemetry_omitted.saturating_add(1);
        }
        if replayed {
            s.stats.replayed = s.stats.replayed.saturating_add(1);
        }
        true
    }
}
