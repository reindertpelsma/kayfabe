//! Split costs are shared-walk attribution, never additive per-ticket RM accounting.
use super::core::{Gauge, KEEP, PENDING, SLOW_NS, Span};
use std::collections::{BTreeMap, VecDeque};
/// Guest-selected split domain, recorded without treating it as a host address.
#[derive(Debug, Clone, Copy)]
pub enum SplitScope {
    /// Exact VMM VA-space identity.
    Space(u64),
    /// Guest PDB, or all PDBs.
    Pdb(Option<u64>),
}
#[derive(Debug, Clone)]
pub(super) struct Split {
    pub ticket: u64,
    pub token: u32,
    pub scope: SplitScope,
    pub worker: u64,
    pub requested: u64,
    pub queued: Option<u64>,
    pub started: Option<u64>,
    pub applied: Option<u64>,
    pub ready: Option<u64>,
    pub released: Option<u64>,
    pub taken: Option<u64>,
    pub success: Option<bool>,
    pub walk: u64,
    pub walks: u64,
    pub space: u64,
    pub targets: usize,
    pub wall_ns: u64,
    pub rm: [u64; 8],
    pub diff: u64,
    pub mapped: u64,
    pub unmapped: u64,
    pub work: Vec<Span>,
    pub context_checked: bool,
    last_start: u64,
    last_wall: u64,
    saved: bool,
}
#[derive(Debug, Default)]
pub(super) struct Splits {
    pub pending: BTreeMap<u64, Split>,
    pub slow: VecDeque<Split>,
    pub gauges: [Gauge; 5],
    pub overflow: u64,
    pub missing: u64,
    pub canceled: u64,
}
impl Splits {
    pub fn request(&mut self, ticket: u64, token: u32, scope: SplitScope, worker: u64, at: u64) {
        if self.pending.len() == PENDING {
            self.pending.pop_first();
            self.overflow = self.overflow.saturating_add(1);
        }
        self.pending.insert(
            ticket,
            Split {
                ticket,
                token,
                scope,
                worker,
                requested: at,
                queued: None,
                started: None,
                applied: None,
                ready: None,
                released: None,
                taken: None,
                success: None,
                walk: 0,
                walks: 0,
                space: 0,
                targets: 0,
                wall_ns: 0,
                rm: [0; 8],
                diff: 0,
                mapped: 0,
                unmapped: 0,
                work: Vec::new(),
                context_checked: false,
                last_start: 0,
                last_wall: 0,
                saved: false,
            },
        );
    }
    fn get(&mut self, ticket: u64) -> Option<&mut Split> {
        if !self.pending.contains_key(&ticket) {
            self.missing = self.missing.saturating_add(1);
        }
        self.pending.get_mut(&ticket)
    }
    pub fn queued(&mut self, ticket: u64, at: u64) {
        if let Some(s) = self.get(ticket) {
            s.queued.get_or_insert(at);
        }
    }
    pub fn start(&mut self, ticket: u64, walk: u64, space: u64, targets: usize, at: u64) {
        if let Some(s) = self.get(ticket) {
            s.started.get_or_insert(at);
            s.last_start = at;
            s.last_wall = 0;
            s.walk = walk;
            s.walks = s.walks.saturating_add(1);
            s.space = space;
            s.targets = targets;
        }
    }
    pub fn applied(
        &mut self,
        ticket: u64,
        space: u64,
        diff: u64,
        rm: [u64; 8],
        maps: [u64; 2],
        at: u64,
    ) {
        if let Some(s) = self.get(ticket) {
            s.applied = Some(at);
            s.space = space;
            s.diff = s.diff.saturating_add(diff);
            for (total, n) in s.rm.iter_mut().zip(rm) {
                *total = total.saturating_add(n);
            }
            s.mapped = s.mapped.saturating_add(maps[0]);
            s.unmapped = s.unmapped.saturating_add(maps[1]);
            let wall = at.saturating_sub(s.last_start);
            s.wall_ns = s.wall_ns.saturating_add(wall.saturating_sub(s.last_wall));
            s.last_wall = wall;
        }
    }
    pub fn ready(&mut self, ticket: u64, success: bool, at: u64) {
        if let Some(s) = self.get(ticket) {
            s.ready.get_or_insert(at);
            s.success = Some(success);
        }
    }
    pub fn released(&mut self, ticket: u64, success: bool, at: u64) {
        if let Some(s) = self.get(ticket) {
            s.released.get_or_insert(at);
            s.success = Some(success);
        }
    }
    pub fn taken(&mut self, ticket: u64, at: u64) {
        let Some(mut s) = self.pending.remove(&ticket) else {
            return;
        };
        s.taken = Some(at);
        for (g, interval) in self.gauges.iter_mut().zip([
            s.started.and_then(|n| n.checked_sub(s.requested)),
            s.ready.zip(s.started).and_then(|(a, b)| a.checked_sub(b)),
            s.released.zip(s.ready).and_then(|(a, b)| a.checked_sub(b)),
            s.taken.zip(s.released).and_then(|(a, b)| a.checked_sub(b)),
            at.checked_sub(s.requested),
        ]) {
            if let Some(n) = interval {
                g.count = g.count.saturating_add(1);
                g.max = g.max.max(n);
            }
        }
        if at.saturating_sub(s.requested) > SLOW_NS {
            self.remember(s);
        }
    }
    fn remember(&mut self, mut s: Split) {
        if let Some(old) = self.slow.iter_mut().find(|old| old.ticket == s.ticket) {
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
    }
    pub fn expire(&mut self, at: u64) {
        let mut save = Vec::new();
        for s in self.pending.values_mut() {
            if !s.saved && at.saturating_sub(s.requested) > SLOW_NS {
                s.saved = true;
                save.push(s.clone());
            }
        }
        for s in save {
            self.remember(s);
        }
    }
    pub fn next_due(&self) -> u64 {
        self.pending
            .values()
            .filter(|s| !s.saved)
            .map(|s| s.requested.saturating_add(SLOW_NS + 1))
            .min()
            .unwrap_or(u64::MAX)
    }
    pub fn cancel(&mut self, ticket: u64) {
        if self.pending.remove(&ticket).is_some() {
            self.canceled = self.canceled.saturating_add(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(s: &mut Splits, t: u64) {
        s.request(t, 0x80c, SplitScope::Space(0x2a3), 1, 10);
        s.queued(t, 20);
    }
    #[test]
    fn ens11_slow_split_updates_release_before_the_requesting_worker_resumes() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.expire(SLOW_NS + 100);
        s.ready(1, true, SLOW_NS + 110);
        s.released(1, true, SLOW_NS + 120);
        s.expire(SLOW_NS + 130);
        assert_eq!(s.slow[0].ready, Some(SLOW_NS + 110));
        assert_eq!(s.slow[0].released, Some(SLOW_NS + 120));
        assert!(s.slow[0].taken.is_none());
    }
    #[test]
    fn ens11_split_worker_resume_refreshes_the_final_blocking_context() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.expire(SLOW_NS + 100);
        s.slow[0].context_checked = true;
        s.taken(1, SLOW_NS + 150);
        assert!(!s.slow[0].context_checked);
    }
    #[test]
    fn ens11_split_stages_include_queue_walk_signal_and_worker_resume() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.start(1, 5, 0x2a3, 1, 30);
        s.applied(1, 0x2a3, 7, [1, 2, 3, 4, 5, 6, 7, 8], [2, 3], 50);
        s.ready(1, true, 60);
        s.released(1, true, 70);
        s.taken(1, 90);
        assert_eq!(s.gauges.map(|g| g.max), [20, 30, 10, 20, 80]);
        assert_eq!(
            s.gauges[..4].iter().map(|g| g.max).sum::<u64>(),
            s.gauges[4].max
        );
    }
    #[test]
    fn ens11_split_pending_after_release_exposes_a_late_worker_resume() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.start(1, 5, 0x2a3, 1, 30);
        s.ready(1, true, 40);
        s.released(1, true, 50);
        s.expire(SLOW_NS + 100);
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.slow.len(), 1);
        s.taken(1, SLOW_NS + 110);
        assert!(s.pending.is_empty());
        assert_eq!(s.slow[0].taken, Some(SLOW_NS + 110));
    }
    #[test]
    fn ens11_split_cost_is_shared_walk_attribution_not_one_walk_per_ticket() {
        let mut s = Splits::default();
        request(&mut s, 1);
        request(&mut s, 2);
        for ticket in [1, 2] {
            s.start(ticket, 99, 0x2a3, 1, 30);
            s.applied(ticket, 0x2a3, 4, [2; 8], [2, 1], SLOW_NS + 100);
            s.ready(ticket, true, SLOW_NS + 110);
            s.released(ticket, true, SLOW_NS + 120);
            s.taken(ticket, SLOW_NS + 130);
        }
        assert_eq!(s.slow.len(), 2);
        assert_eq!(s.slow[0].walk, s.slow[1].walk);
        assert_eq!(s.slow[0].rm, [2; 8]);
        assert_eq!(s.slow[1].diff, 4);
    }
    #[test]
    fn ens11_split_request_and_history_stay_bounded_under_hostile_churn() {
        let mut s = Splits::default();
        for t in 0..1000 {
            request(&mut s, t);
        }
        assert_eq!(s.pending.len(), 512);
        assert_eq!(s.overflow, 488);
        s.expire(SLOW_NS + 100);
        assert_eq!(s.slow.len(), 64);
    }
    #[test]
    fn ens11_split_retries_keep_request_time_and_all_actual_walks() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.start(1, 1, 7, 1, 30);
        s.applied(1, 7, 2, [1; 8], [1, 1], 40);
        s.start(1, 2, 7, 1, 50);
        s.applied(1, 7, 3, [2; 8], [2, 1], 70);
        let a = &s.pending[&1];
        assert_eq!(a.requested, 10);
        assert_eq!(a.started, Some(30));
        assert_eq!(a.walks, 2);
        assert_eq!(a.wall_ns, 30);
        assert_eq!(a.rm, [3; 8]);
    }
    #[test]
    fn ens11_canceled_split_cannot_inherit_a_reused_channel_result() {
        let mut s = Splits::default();
        request(&mut s, 1);
        s.cancel(1);
        request(&mut s, 2);
        s.ready(1, true, 30);
        assert_eq!(s.missing, 1);
        assert_eq!(s.pending[&2].ready, None);
    }
}
