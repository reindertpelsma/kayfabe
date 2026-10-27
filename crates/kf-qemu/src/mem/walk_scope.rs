// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Scope is a host mapping mutation domain, not a claim that guest page tables never alias.
//! Unknown work stays global. Worker queries never wait for the VA thread's metadata locks.
use super::{Inbox, MemStatement, SplitTarget, VasKey};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

const MAX_STATEMENT_SPACES: usize = 512;

#[derive(Debug, Default)]
pub(super) struct Scope {
    keys: Vec<VasKey>,
    global: bool,
    sequence: Option<u64>,
}

/// Bounded outstanding statement identities. Overflow is a global hold until settled.
#[derive(Debug, Default)]
pub(super) struct Statements {
    keys: BTreeMap<VasKey, u64>,
    global_through: u64,
}

impl Statements {
    pub(super) fn add(&mut self, statement: &MemStatement, seq: u64) {
        let key = match statement {
            MemStatement::PageDir(s) => Some(VasKey(
                (u64::from(s.client.0) << 32) | u64::from(s.vaspace.0),
            )),
            MemStatement::Retire { client, vaspace }
            | MemStatement::UnsetPageDir { client, vaspace } => {
                Some(VasKey((u64::from(*client) << 32) | u64::from(*vaspace)))
            }
            MemStatement::BarPde(_) | MemStatement::Sysmembar => None,
        };
        if let Some(key) = key
            && (self.keys.contains_key(&key) || self.keys.len() < MAX_STATEMENT_SPACES)
        {
            self.keys.insert(key, seq);
        } else {
            self.global_through = seq;
        }
    }

    pub(super) fn settle(&mut self, seq: u64) {
        self.keys.retain(|_, last| *last > seq);
        if self.global_through <= seq {
            self.global_through = 0;
        }
    }
}

/// Why a USER relay must hold future source reads/publications. Other-space work is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Clear,
    OwnWalk,
    Split,
    Global,
    OtherWalk,
}
impl Reason {
    pub(crate) fn held(self) -> bool {
        matches!(self, Self::OwnWalk | Self::Split | Self::Global)
    }
}

impl Inbox {
    /// VA thread, before any synchronous apply: publish ALL pending/active mutation targets.
    pub(crate) fn set_walk_scope(&self, keys: Vec<VasKey>, global: bool, sequence: Option<u64>) {
        let global = global
            || keys.len() > 512
            || keys
                .iter()
                .any(|k| *k == super::K_BAR1 || *k == super::K_BAR2);
        let keys = if keys.len() > 512 { Vec::new() } else { keys };
        if let Ok(mut scope) = self.walk_scope.lock() {
            *scope = Some(Scope {
                keys,
                global,
                sequence,
            });
        }
    }

    /// The scoped source predicate. Unknown-identity diagnostic readers retain the aggregate gate.
    pub(crate) fn walk_reason(&self, key: VasKey) -> Reason {
        let Ok(scope) = self.walk_scope.try_lock() else {
            return Reason::Global;
        };
        let sequence = scope.as_ref().and_then(|s| s.sequence);
        if self
            .walk_port
            .get()
            .and_then(|p| p.armed_request())
            .is_some_and(|r| Some(r.seq) != sequence)
        {
            // A new vCPU request appeared since the VA worker resolved the scope.
            return Reason::Global;
        }
        let mut other = false;
        if let Some(s) = scope.as_ref() {
            if s.global {
                return Reason::Global;
            }
            if s.keys.contains(&key) {
                return Reason::OwnWalk;
            }
            other |= !s.keys.is_empty();
        } else if self.walk_busy.load(Ordering::Acquire) {
            return Reason::Global;
        }
        drop(scope);
        let Ok(statements) = self.statement_scope.try_lock() else {
            return Reason::Global;
        };
        if statements.global_through != 0 {
            return Reason::Global;
        }
        if statements.keys.contains_key(&key) {
            return Reason::OwnWalk;
        }
        other |= !statements.keys.is_empty();
        drop(statements);
        // Same lock order as queue -> running transfer: there is no unaccounted gap.
        let Ok(queued) = self.splits.try_lock() else {
            return Reason::Global;
        };
        let Ok(running) = self.split_tokens.try_lock() else {
            return Reason::Global;
        };
        if queued.len().saturating_add(running.len()) > 512 {
            return Reason::Global;
        }
        for target in queued
            .iter()
            .map(|(_, _, target)| target)
            .chain(running.values().map(|(_, target)| target))
        {
            match target {
                SplitTarget::Space(k) if *k == key => return Reason::Split,
                SplitTarget::Space(_) => other = true,
                SplitTarget::Pdb(_) => return Reason::Global,
            }
        }
        if other {
            Reason::OtherWalk
        } else {
            Reason::Clear
        }
    }

    pub(crate) fn walk_busy_for(&self, key: VasKey) -> bool {
        self.walk_reason(key).held()
    }
}

impl Inbox {
    /// Shared by turn entry, guarded publication, migration and the adversarial relay adapter.
    pub(crate) fn source_gate(&self, key: VasKey, token: u32) -> Reason {
        let reason = self.walk_reason(key);
        if reason.held() {
            self.wait_walk_for(key, token);
        }
        reason
    }

    /// Register first and wake the VA thread: a release racing insertion is observed next loop.
    pub(crate) fn wait_walk_for(&self, key: VasKey, token: u32) {
        if let Ok(mut waiters) = self.scoped_waiters.lock() {
            if waiters.len() < 512 || waiters.contains_key(&token) {
                waiters.insert(token, key);
            } else {
                // Bounded metadata fallback; the legacy idle wake still serves overflow.
                drop(waiters);
                self.wait_walk(token);
                return;
            }
        }
        let _ = self.wake.signal();
    }

    /// Wake ready scoped rings even while an unrelated manager target remains busy.
    pub(crate) fn take_ready_walk_waiters(&self) -> Vec<u32> {
        let all = self
            .scoped_waiters
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default();
        let mut ready = Vec::new();
        for (token, key) in all {
            if self.walk_busy_for(key) {
                if let Ok(mut w) = self.scoped_waiters.lock() {
                    w.entry(token).or_insert(key);
                }
            } else {
                ready.push(token);
            }
        }
        ready
    }
}

use std::sync::atomic::AtomicU64;
use std::time::Instant;

/// Constant-size diagnostic state carried by one admitted USER record.
#[derive(Debug, Default)]
pub(crate) struct Hold {
    cause: AtomicU64,
    began: AtomicU64,
    last: AtomicU64,
}

#[derive(Debug, Default)]
struct Counter {
    events: AtomicU64,
    ns: AtomicU64,
    max_ns: AtomicU64,
}

/// Cumulative VM heartbeat diagnostics; no values below decide scheduling.
#[derive(Debug)]
pub(crate) struct Stats {
    epoch: Instant,
    own: Counter,
    split: Counter,
    global: Counter,
    other_allowed: AtomicU64,
    pub(crate) marker_during_walk: AtomicU64,
    pub(crate) writeback_busy_walk: AtomicU64,
    pub(crate) writeback_busy_ns_max: AtomicU64,
}
impl Default for Stats {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            own: Counter::default(),
            split: Counter::default(),
            global: Counter::default(),
            other_allowed: AtomicU64::new(0),
            marker_during_walk: AtomicU64::new(0),
            writeback_busy_walk: AtomicU64::new(0),
            writeback_busy_ns_max: AtomicU64::new(0),
        }
    }
}
impl Stats {
    fn counter(&self, cause: u64) -> Option<&Counter> {
        match cause {
            1 => Some(&self.own),
            2 => Some(&self.split),
            3 => Some(&self.global),
            _ => None,
        }
    }

    /// Account time until this observation, including release; same-cause probes do not restart it.
    pub(crate) fn observe(&self, hold: &Hold, reason: Reason) {
        let now = u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.observe_at(hold, reason, now);
    }

    fn observe_at(&self, hold: &Hold, reason: Reason, now: u64) {
        let cause = match reason {
            Reason::OwnWalk => 1,
            Reason::Split => 2,
            Reason::Global => 3,
            _ => 0,
        };
        if reason == Reason::OtherWalk {
            self.other_allowed.fetch_add(1, Ordering::Relaxed);
        }
        let previous = hold.cause.swap(cause, Ordering::Relaxed);
        let last = hold.last.swap(now, Ordering::Relaxed);
        if let Some(counter) = self.counter(previous) {
            counter
                .ns
                .fetch_add(now.saturating_sub(last), Ordering::Relaxed);
            counter.max_ns.fetch_max(
                now.saturating_sub(hold.began.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
        }
        if previous != cause {
            hold.began.store(now, Ordering::Relaxed);
            if let Some(counter) = self.counter(cause) {
                counter.events.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Track observed guest-GET admission contention, not the GPU's unknown finish time.
    pub(crate) fn writeback(&self, since: &mut Option<u64>, busy: bool, walk: bool) {
        let now = u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.writeback_at(since, busy, walk, now);
    }
    fn writeback_at(&self, since: &mut Option<u64>, busy: bool, walk: bool, now: u64) {
        if busy {
            if walk && since.is_none() {
                *since = Some(now);
                self.writeback_busy_walk.fetch_add(1, Ordering::Relaxed);
            }
        } else if let Some(start) = since.take() {
            self.writeback_busy_ns_max
                .fetch_max(now.saturating_sub(start), Ordering::Relaxed);
        }
    }

    /// Hold times end at a later probe/release/retirement; not a timestamp of GPU completion.
    pub(crate) fn summary(&self) -> String {
        let row = |c: &Counter| {
            format!(
                "{}/{}/{}",
                c.events.load(Ordering::Relaxed),
                c.ns.load(Ordering::Relaxed) / 1000,
                c.max_ns.load(Ordering::Relaxed) / 1000
            )
        };
        format!(
            "hold[count/us/max_us] own={} split={} global={} other_allowed={} \
            ref_changes_walk={} completion_wb_busy_walk={} completion_wb_busy_max_us={}",
            row(&self.own),
            row(&self.split),
            row(&self.global),
            self.other_allowed.load(Ordering::Relaxed),
            self.marker_during_walk.load(Ordering::Relaxed),
            self.writeback_busy_walk.load(Ordering::Relaxed),
            self.writeback_busy_ns_max.load(Ordering::Relaxed) / 1000
        )
    }
}

impl Inbox {
    /// Same scoped decision at all three USER adapter sites; instrumentation is diagnostic only.
    pub(crate) fn user_source_ready(&self, key: VasKey, token: u32, hold: &Hold) -> bool {
        let reason = self.source_gate(key, token);
        self.walk_stats.observe(hold, reason);
        !reason.held()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_probes_accumulate_one_hold_without_resetting_its_maximum() {
        let stats = Stats::default();
        let hold = Hold::default();
        stats.observe_at(&hold, Reason::OwnWalk, 10);
        stats.observe_at(&hold, Reason::OwnWalk, 30);
        stats.observe_at(&hold, Reason::Clear, 90);
        assert_eq!(stats.own.events.load(Ordering::Relaxed), 1);
        assert_eq!(stats.own.ns.load(Ordering::Relaxed), 80);
        assert_eq!(stats.own.max_ns.load(Ordering::Relaxed), 80);
    }
    #[test]
    fn changing_cause_closes_the_previous_hold_and_other_space_is_allowed() {
        let stats = Stats::default();
        let hold = Hold::default();
        stats.observe_at(&hold, Reason::Split, 10);
        stats.observe_at(&hold, Reason::Global, 20);
        stats.observe_at(&hold, Reason::OtherWalk, 50);
        assert_eq!(stats.split.ns.load(Ordering::Relaxed), 10);
        assert_eq!(stats.global.ns.load(Ordering::Relaxed), 30);
        assert_eq!(stats.global.max_ns.load(Ordering::Relaxed), 30);
        assert_eq!(stats.other_allowed.load(Ordering::Relaxed), 1);
        assert_eq!(hold.cause.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn a_long_hold_keeps_its_original_start_across_many_probes() {
        let stats = Stats::default();
        let hold = Hold::default();
        stats.observe_at(&hold, Reason::OwnWalk, 0);
        for now in 1..=100 {
            stats.observe_at(&hold, Reason::OwnWalk, now);
        }
        stats.observe_at(&hold, Reason::Clear, 101);
        assert_eq!(stats.own.max_ns.load(Ordering::Relaxed), 101);
        assert_eq!(stats.own.ns.load(Ordering::Relaxed), 101);
    }
    #[test]
    fn repeated_writeback_contention_counts_one_episode_and_keeps_its_start() {
        let stats = Stats::default();
        let mut since = None;
        stats.writeback_at(&mut since, true, true, 10);
        stats.writeback_at(&mut since, true, true, 20);
        stats.writeback_at(&mut since, false, false, 100);
        assert_eq!(stats.writeback_busy_walk.load(Ordering::Relaxed), 1);
        assert_eq!(stats.writeback_busy_ns_max.load(Ordering::Relaxed), 90);
        assert_eq!(since, None);
    }
    #[test]
    fn writeback_correlation_excludes_nonwalk_busy_and_recovers_for_the_next_episode() {
        let stats = Stats::default();
        let mut since = None;
        stats.writeback_at(&mut since, true, false, 5);
        assert_eq!(since, None);
        stats.writeback_at(&mut since, true, true, 10);
        stats.writeback_at(&mut since, true, true, 30);
        stats.writeback_at(&mut since, false, false, 70);
        stats.writeback_at(&mut since, true, true, 80);
        stats.writeback_at(&mut since, false, false, 90);
        assert_eq!(stats.writeback_busy_walk.load(Ordering::Relaxed), 2);
        assert_eq!(stats.writeback_busy_ns_max.load(Ordering::Relaxed), 60);
    }
}

impl Inbox {
    /// The VA loop's release transaction: settle before computing readiness or draining waiters.
    /// A waiter release is a hint only; every later source admission rechecks the current state.
    pub(crate) fn finish_gate_loop(
        &self,
        keys: Vec<VasKey>,
        global: bool,
        sequence: Option<u64>,
        manager_busy: bool,
        settled: Option<u64>,
    ) -> (bool, Vec<u32>) {
        let advanced = settled.is_some_and(|n| self.settle(n));
        let busy = manager_busy || !self.all_settled();
        self.set_walk_busy(busy);
        self.set_walk_scope(keys, global, sequence);
        let mut ready = self.take_ready_walk_waiters();
        if !busy {
            ready.extend(self.take_walk_waiters());
        }
        (advanced, ready)
    }
}
