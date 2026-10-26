//! Default-off, bounded TSG birth/set/readback facts. No fact is a decision input.
use kf_host::Channel;
/// Read-only owned-object seam.
pub trait Reader {
    /// Requested timeslice bookkeeping; not necessarily the hardware quantum.
    fn timeslice(&self, channel: Channel) -> Result<u64, u32>;
    /// Zcull, PM, SMPC, compute preemption and graphics preemption modes.
    fn modes(&self, channel: Channel) -> Result<[u32; 5], u32>;
}
/// Guest request next to a readback.
#[derive(Clone, Copy, Debug)]
pub enum Request {
    /// No new setter, usually birth or bind.
    None,
    /// Guest TSG timeslice, in microseconds.
    Timeslice(u64),
    /// Flags, graphics and compute preemption intent.
    Ctxsw(u32, u32, u32),
    /// Zcull binding mode.
    Zcull(u32),
}
/// Only explicit one enables fact collection.
pub fn flag_on(value: Option<&str>) -> bool {
    value == Some("1")
}
/// Per-VM diagnostic book.
pub struct Facts;
impl Facts {
    /// Independent bounds on metadata and native readback/log events.
    pub fn new(_enabled: bool, _cap: usize, _events: usize) -> Self {
        Self
    }
    /// New group or a member binding in an already observed group.
    pub fn birth(
        &self,
        _key: (u32, u32),
        _device: u32,
        _c: Channel,
        _new: bool,
        _desired: Option<u64>,
        _host: &impl Reader,
    ) -> Option<String> {
        None
    }
    /// After a setter or bind, including native refusals.
    pub fn after(
        &self,
        _c: Channel,
        _phase: &str,
        _req: Request,
        _status: u32,
        _host: &impl Reader,
    ) -> Option<String> {
        None
    }
    /// Guest lifetime invalidation; it never changes mirror rules.
    pub fn forget(&self, _client: u32, _object: u32) {}
    /// Native group telemetry release.
    pub fn group_freed(&self, _group: u32) {}
    /// Reset, retaining diagnostic sequence identity.
    pub fn clear(&self) {}
    /// Bounded diagnostic status fragment.
    pub fn summary(&self) -> String {
        String::new()
    }
}
