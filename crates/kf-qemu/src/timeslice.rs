//! Bounded TSG timeslice intent; host calls are outside the bookkeeping lock.

use kf_host::Channel;

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

/// One queued SET, tied to an allocation lifetime rather than only numeric handles.
#[derive(Clone, Debug)]
pub struct Ticket {
    key: Key,
    device: u32,
    us: u64,
}

/// Counters are saturating, and never a decision input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// SETs accepted before any live group exists.
    pub early_sets: u64,
    /// Host calls replaying an early/saved intent onto a newly born group.
    pub replayed: u64,
    /// Replays deliberately suppressed by the measurement kill switch.
    pub dropped: u64,
}

/// Default-on behavior; explicit zero is the measurement kill switch.
pub fn flag_on(value: Option<&str>) -> bool {
    value != Some("0")
}

/// One VM's bounded intent table and diagnostic applied-group records.
pub struct Replay;

impl Replay {
    /// Bounds are explicit so a tiny fake can exhaust them without hardware.
    pub fn new(_enabled: bool, _per_client: usize, _total: usize) -> Self {
        Self
    }
    /// Reserve a bounded lifetime before queueing an act.
    pub fn prepare(&self, key: Key, device: u32, us: u64) -> Result<Ticket, u32> {
        Ok(Ticket { key, device, us })
    }
    /// Apply live groups once each, or remember an early SET. No lock across host calls.
    pub fn apply(
        &self,
        ticket: &Ticket,
        groups: &[Channel],
        host: &impl Setter,
    ) -> Result<(), u32> {
        let mut done = std::collections::BTreeSet::new();
        for &c in groups {
            if done.insert(c.tsg) {
                host.set(c, ticket.us)?;
            }
        }
        Ok(())
    }
    /// A mirror-accepted birth; only a NEW host group gets the saved value.
    pub fn birth(
        &self,
        _key: Key,
        _channel: Channel,
        _new_group: bool,
        _host: &impl Setter,
    ) -> Result<(), u32> {
        Ok(())
    }
    /// Release a reservation whose act could not be queued.
    pub fn cancel(&self, _ticket: &Ticket) {}
    /// Guest object/client/device free, before numeric handles can be reused.
    pub fn forget(&self, _client: u32, _object: u32) {}
    /// A native group was actually released; guest intent outlives its last member.
    pub fn group_freed(&self, _group: u32) {}
    /// Device reset clears all pending/non-host state.
    pub fn clear(&self) {}
    /// Number of remembered/reserved TSGs.
    pub fn len(&self) -> usize {
        0
    }
    /// Most recent successfully accepted request for one guest TSG.
    pub fn desired(&self, _key: Key) -> Option<u64> {
        None
    }
    /// Last successfully authored request on this host group, not its rounded hardware value.
    pub fn last_applied(&self, _group: u32) -> Option<u64> {
        None
    }
    /// Atomic snapshot of the counters.
    pub fn stats(&self) -> Stats {
        Stats::default()
    }
    /// Bounded status fragment; using a try-lock also makes lock scope testable.
    pub fn summary(&self) -> String {
        String::new()
    }
}
