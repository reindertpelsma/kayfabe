use std::collections::VecDeque;

pub(super) const PENDING: usize = 512;
pub(super) const KEEP: usize = 64;
pub(super) const SLOW_NS: u64 = 50_000_000;

/// Native publication unit; a descriptor/turn is not a Windows DMA packet identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Windows USER private command publication.
    User,
    /// Translated channel completion-tail publication.
    Translated,
    /// Other host-owned private ring, with no guest notification implied.
    Private,
}
/// Observed reason a worker could not publish; never a scheduling decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// This target's walk.
    OwnWalk,
    /// Other-target work or conservative global scope.
    OtherWalk,
    /// A split request.
    Split,
    /// A scan visit exhausted its work allowance.
    ScanBudget,
    /// PT metadata contention.
    PtLock,
    /// Private storage contention.
    Storage,
    /// Act-thread ownership.
    Act,
    /// Another/unknown worker operation.
    Worker,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Wait {
    pub cause: Cause,
    pub start: u64,
    pub end: u64,
    pub owner: u64,
}
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Span {
    pub thread: u64,
    pub role: u64,
    pub code: u64,
    pub detail: u64,
    pub start: u64,
    pub end: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NotificationSource {
    Normal,
    Storm,
}
#[derive(Debug, Clone)]
pub(super) struct Sample {
    pub stream: u64,
    pub token: u32,
    pub kind: Kind,
    pub observed: u64,
    pub published: u64,
    pub marker: Option<u64>,
    pub wrap: bool,
    pub advanced: Option<u64>,
    pub gpu: Option<u64>,
    pub vector: Option<u32>,
    pub raised: Option<u64>,
    pub source: Option<NotificationSource>,
    pub waits: Vec<Wait>,
    pub worker: u64,
    pub cookie: u64,
    pub saved: bool,
    pub work: Vec<Span>,
    pub context_checked: bool,
}
impl Sample {
    pub fn stages(&self) -> Option<[u64; 3]> {
        let observed = self.advanced?;
        let basis = self.gpu.unwrap_or(observed);
        if basis > observed {
            return None;
        }
        let stages = [
            self.published.checked_sub(self.observed)?,
            basis.checked_sub(self.published)?,
            self.raised?.checked_sub(basis)?,
        ];
        let total = self.raised?.checked_sub(self.observed)?;
        (stages[0].checked_add(stages[1])?.checked_add(stages[2])? == total).then_some(stages)
    }
    pub fn observation_lag(&self) -> Option<u64> {
        self.advanced?.checked_sub(self.gpu?)
    }
    fn slow(&self, now: u64) -> bool {
        now.saturating_sub(self.observed) > SLOW_NS
            || self
                .stages()
                .is_some_and(|s| s.into_iter().any(|n| n > SLOW_NS))
    }
}
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Gauge {
    pub count: u64,
    pub max: u64,
}
impl Gauge {
    fn add(&mut self, n: u64) {
        self.count = self.count.saturating_add(1);
        self.max = self.max.max(n);
    }
}
#[derive(Debug, Default)]
pub(super) struct State {
    pub splits: super::splits::Splits,
    pub pending: VecDeque<Sample>,
    pub slow: VecDeque<Sample>,
    pub gauges: [Gauge; 4],
    pub total: Gauge,
    pub overflow: u64,
    pub canceled: u64,
    pub early: u64,
    pub failed: u64,
    pub invalid: u64,
    pub private_done: u64,
    pub first_dump: bool,
    pub marker_unknown: u64,
    next: u64,
}
impl State {
    pub fn publish(&mut self, mut s: Sample) {
        self.next = self.next.saturating_add(1);
        s.cookie = self.next;
        if let Some(n) = s.published.checked_sub(s.observed) {
            self.gauges[0].add(n);
        } else {
            self.invalid = self.invalid.saturating_add(1);
        }
        if self.pending.len() == PENDING {
            self.pending.pop_front();
            self.overflow = self.overflow.saturating_add(1);
        }
        self.pending.push_back(s);
    }
    pub fn cover(&mut self, stream: u64, marker: u64, wrap: bool) {
        for s in &mut self.pending {
            if s.stream == stream && s.marker.is_none() {
                s.marker = Some(marker);
                s.wrap = wrap;
            }
        }
    }
    pub fn marker(&mut self, stream: u64, done: u64, at: u64, gpu: Option<u64>) {
        for s in &mut self.pending {
            let reached = s.marker.is_some_and(|m| {
                if s.wrap {
                    ((done as u32).wrapping_sub(m as u32) as i32) >= 0
                } else {
                    done >= m
                }
            });
            if s.stream != stream || s.advanced.is_some() || !reached {
                continue;
            }
            s.advanced = Some(at);
            s.gpu = gpu;
            let basis = gpu.unwrap_or(at);
            if let Some(n) = basis.checked_sub(s.published).filter(|_| basis <= at) {
                self.gauges[1].add(n);
            } else {
                self.invalid = self.invalid.saturating_add(1);
            }
            if let Some(n) = s.observation_lag() {
                self.gauges[3].add(n);
            }
            if gpu.is_none() {
                self.marker_unknown = self.marker_unknown.saturating_add(1);
            }
        }
        self.finish(at);
    }
    pub fn notify_source(
        &mut self,
        vector: u32,
        at: u64,
        success: bool,
        _source: NotificationSource,
    ) {
        self.notify(vector, at, success);
    }
    pub fn notify(&mut self, vector: u32, at: u64, success: bool) {
        if !success {
            self.failed = self.failed.saturating_add(1);
            return;
        }
        for s in &mut self.pending {
            if s.vector == Some(vector) && s.published <= at && s.raised.is_none() {
                // This is a notification opportunity on a GPU-wide vector, not proof of
                // which packet caused it. Preserve the first one, including early delivery.
                s.raised = Some(at);
            }
        }
        self.finish(at);
    }
    fn remember(&mut self, mut s: Sample) {
        if let Some(old) = self.slow.iter_mut().find(|old| old.cookie == s.cookie) {
            if s.work.is_empty() {
                s.work = std::mem::take(&mut old.work);
            }
            s.context_checked |= old.context_checked;
            *old = s;
        } else {
            if self.slow.len() == KEEP {
                self.slow.pop_front();
            }
            self.slow.push_back(s);
        }
        self.first_dump = true;
    }
    fn finish(&mut self, now: u64) {
        let mut i = 0;
        while i < self.pending.len() {
            let s = &self.pending[i];
            let private = s.kind == Kind::Private && s.vector.is_none();
            if s.advanced.is_none() || (!private && s.raised.is_none()) {
                i += 1;
                continue;
            }
            let s = self.pending.remove(i).expect("bounded index");
            if private {
                self.private_done = self.private_done.saturating_add(1);
            } else if let Some(stages) = s.stages() {
                self.gauges[2].add(stages[2]);
                self.total.add(stages.into_iter().sum());
            } else {
                self.early = self.early.saturating_add(1);
            }
            if s.slow(now) {
                self.remember(s);
            }
        }
    }
    pub fn cancel(&mut self, stream: u64) {
        let before = self.pending.len();
        self.pending.retain(|s| s.stream != stream);
        self.canceled = self
            .canceled
            .saturating_add((before - self.pending.len()) as u64);
    }
    pub fn expire(&mut self, now: u64) {
        self.splits.expire(now);
        self.first_dump |= !self.splits.slow.is_empty();
        let mut save = Vec::new();
        for s in &mut self.pending {
            if !s.saved && s.slow(now) {
                s.saved = true;
                save.push(s.clone());
            }
        }
        for s in save {
            self.remember(s);
        }
    }
}
