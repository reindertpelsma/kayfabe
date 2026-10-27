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
    /// PM binding mode.
    Pm(u32),
}
/// Only explicit one enables fact collection.
pub fn flag_on(value: Option<&str>) -> bool {
    value == Some("1")
}
/// Process-wide default-off fact switch, sampled once.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| flag_on(std::env::var("KF3_TSG_FACTS").ok().as_deref()))
}

#[derive(Clone)]
struct Group {
    key: (u32, u32),
    device: u32,
    generation: u64,
    host_birth: u64,
    desired: Option<u64>,
    ctxsw: Option<(u32, u32, u32)>,
}
#[derive(Default)]
struct Book {
    lives: std::collections::BTreeMap<(u32, u32), (u32, u64)>,
    groups: std::collections::BTreeMap<u32, Group>,
    generation: u64,
    birth: u64,
    events: usize,
    dropped: u64,
}
/// Per-VM diagnostic book. Generations name observed guest TSG lifetimes,
/// starting at first birth/SET; Free and reset end the lifetime.
pub struct Facts {
    enabled: bool,
    cap: usize,
    max_events: usize,
    book: std::sync::Mutex<Book>,
}
impl Facts {
    /// Independent bounds on metadata and native readback/log events.
    pub fn new(enabled: bool, cap: usize, events: usize) -> Self {
        Self {
            enabled,
            cap,
            max_events: events,
            book: Default::default(),
        }
    }
    /// Observe a TSG lifetime before its first group, without a host call.
    pub fn scope(&self, key: (u32, u32), device: u32) {
        if self.enabled
            && let Ok(mut b) = self.book.lock()
        {
            Self::scope_locked(&mut b, key, device, self.cap);
        }
    }
    fn scope_locked(b: &mut Book, key: (u32, u32), device: u32, cap: usize) -> Option<u64> {
        if let Some(&(old_device, generation)) = b.lives.get(&key) {
            return (old_device == device).then_some(generation);
        }
        if b.lives.len() >= cap {
            return None;
        }
        b.generation = b.generation.checked_add(1)?;
        b.lives.insert(key, (device, b.generation));
        Some(b.generation)
    }
    /// New group or a member binding in an already observed group.
    pub fn birth(
        &self,
        key: (u32, u32),
        device: u32,
        c: Channel,
        new: bool,
        desired: Option<u64>,
        host: &impl Reader,
    ) -> Option<String> {
        if !self.enabled {
            return None;
        }
        {
            let mut b = self.book.lock().ok()?;
            if new {
                if !b.groups.contains_key(&c.tsg) && b.groups.len() >= self.cap {
                    b.dropped = b.dropped.saturating_add(1);
                    return None;
                }
                let Some(generation) = Self::scope_locked(&mut b, key, device, self.cap) else {
                    b.dropped = b.dropped.saturating_add(1);
                    return None;
                };
                b.birth = b.birth.checked_add(1)?;
                let host_birth = b.birth;
                b.groups.insert(
                    c.tsg,
                    Group {
                        key,
                        device,
                        generation,
                        host_birth,
                        desired,
                        ctxsw: None,
                    },
                );
            }
        }
        self.after(
            c,
            if new { "birth" } else { "bind" },
            Request::None,
            0,
            host,
        )
    }
    /// After a setter or bind, including native refusals. No guard across either GET.
    pub fn after(
        &self,
        c: Channel,
        phase: &str,
        req: Request,
        status: u32,
        host: &impl Reader,
    ) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let group = {
            let mut b = self.book.lock().ok()?;
            if !b.groups.contains_key(&c.tsg) {
                return None;
            }
            if b.events >= self.max_events {
                b.dropped = b.dropped.saturating_add(1);
                return None;
            }
            let g = b.groups.get_mut(&c.tsg)?;
            match req {
                Request::Timeslice(us) => g.desired = Some(us),
                Request::Ctxsw(flags, gfx, compute) => g.ctxsw = Some((flags, gfx, compute)),
                _ => {}
            }
            let group = g.clone();
            b.events += 1;
            group
        };
        let timeslice = host.timeslice(c);
        let modes = host.modes(c);
        Some(format!(
            "TSG-FACT client={:#x} tsg={:#x} generation={} host_birth={} \
            host_tsg={:#x} host_channel={:#x} phase={phase} request={req:?} \
            desired_us={:?} ctxsw_requested={:?} setter_status={status:#x} \
            timeslice_get={timeslice:?} ctxsw_get={modes:?}",
            group.key.0,
            group.key.1,
            group.generation,
            group.host_birth,
            c.tsg,
            c.chan,
            group.desired,
            group.ctxsw
        ))
    }
    /// Guest lifetime invalidation; it never changes mirror rules.
    pub fn forget(&self, client: u32, object: u32) {
        if let Ok(mut b) = self.book.lock() {
            b.lives.retain(|key, (device, _)| {
                !(key.0 == client && (client == object || key.1 == object || *device == object))
            });
            b.groups.retain(|_, g| {
                !(g.key.0 == client
                    && (client == object || g.key.1 == object || g.device == object))
            });
        }
    }
    /// Native group telemetry release; logical lifetime remains until guest Free.
    pub fn group_freed(&self, group: u32) {
        if let Ok(mut b) = self.book.lock() {
            b.groups.remove(&group);
        }
    }
    /// Reset, retaining diagnostic sequence identity.
    pub fn clear(&self) {
        if let Ok(mut b) = self.book.lock() {
            b.groups.clear();
            b.lives.clear();
        }
    }
    /// Bounded diagnostic status fragment.
    pub fn summary(&self) -> String {
        let Ok(b) = self.book.try_lock() else {
            return "tsg_facts=busy".into();
        };
        format!(
            "tsg_fact_groups={} tsg_fact_lives={} tsg_fact_events={} tsg_fact_dropped={}",
            b.groups.len(),
            b.lives.len(),
            b.events,
            b.dropped
        )
    }
}
impl Reader for kf_host::HostRm {
    fn timeslice(&self, channel: Channel) -> Result<u64, u32> {
        self.get_timeslice_status(channel)
    }
    fn modes(&self, channel: Channel) -> Result<[u32; 5], u32> {
        self.get_ctxsw_modes_status(channel)
    }
}

/// Authored SET plus read-only facts, only on the act thread.
pub struct Authored<'a> {
    /// Owned native RM connection.
    pub rm: &'a kf_host::HostRm,
    /// VM-local diagnostic book.
    pub facts: &'a Facts,
}
impl crate::timeslice::Setter for Authored<'_> {
    fn set(&self, channel: Channel, us: u64) -> Result<(), u32> {
        let result = crate::timeslice::Setter::set(self.rm, channel, us);
        if let Some(line) = self.facts.after(
            channel,
            "timeslice-set",
            Request::Timeslice(us),
            result.as_ref().err().copied().unwrap_or(0),
            self.rm,
        ) {
            eprintln!("kf3: {line}");
        }
        result
    }
}
