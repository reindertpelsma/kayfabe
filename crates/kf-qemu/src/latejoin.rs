//! ★ 2026-10-09 — **a channel that joins a guest TSG the guest has already scheduled.**
//! `STATUS: LIVE (hardwired), 2026-10-10` — `docs/design/V3_LATE_TSG_JOINER.md`. [measured, Windows 11 / RTX 4070,
//! `traces/windows_tdr_hunt_20261010` runs 287, 288 vs 290] without it the compute+copy joiners of a D3D12 device's TSG
//! (ctxShare 1, 2) kept host `GP_GET=0` under rung work, the context flush hung and Windows reset the GPU (1 and 4 TDRs in
//! the hold); with it 15 joiners were `Authored`, none stayed unfetched, 0 TDR, 0 new host Xid. The design note's
//! section 6 falsifier was not met. (Was default off behind `KF3_SCHEDULE_LATE_JOINERS=1` until then.)
//!
//! The guest schedules a TSG ONCE (`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`, `0xa06c0101`), and a channel
//! it allocates into that TSG afterwards gets no schedule of its own: `[measured]` Windows' D3D12
//! device creation allocates the TSG, its first channel, schedules, and then allocates three more
//! channels under the same TSG (their own context shares), on the real GPU (VFIO) and under kf3
//! alike. kf3 schedules a twin's host group only at the guest's own schedule statement, for the
//! twins that EXIST then; a twin born later is in a host group nobody schedules, and the work it
//! is rung with is never fetched (run 111: `GPGet=0`, `GPPut>0`, TDR, `0x116`).
//!
//! This module is the rule and nothing else — pure state and one decision, with the host verb
//! behind a seam ([`GroupScheduler`]) so the tests need no GPU:
//!
//! * **State is what the guest told us** ([`GuestTsgSched`]): the last `bEnable` of a schedule
//!   statement that named the TSG, recorded at statement time on the drainer (statement order is
//!   the act thread's order), forgotten when the TSG (or its client) is freed. A TSG the guest
//!   never scheduled, or later disabled, is not scheduled.
//! * **The verb is the ordinary authored one**: the twin's own host group, `bEnable = 1`
//!   (`HostRm::schedule_enable`) — the verb every first member of a group already gets, on a
//!   group that holds only channels the guest has enabled. Nothing is forwarded from guest bytes.
//! * **Idempotent with the guest's own later schedule**: [`schedule_group_once`] is the one
//!   place a host group is scheduled, once per group per statement; re-asking an enabled group is
//!   what RM does for every re-sent schedule.

use kf_host::{Channel, HostRm};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// What the guest has said about the scheduling of its TSGs: `(hClient, hTsg)` → the last
/// `bEnable` of a schedule statement that named it.
#[derive(Debug, Default)]
pub struct GuestTsgSched {
    by_tsg: HashMap<(u32, u32), bool>,
}

impl GuestTsgSched {
    /// A schedule statement on `object` (a TSG or a channel — the key is never ambiguous: one
    /// handle names one object per client).
    pub fn record(&mut self, client: u32, object: u32, enable: bool) {
        self.by_tsg.insert((client, object), enable);
    }

    /// `object` was freed (`object == client` frees the whole client).
    pub fn forget(&mut self, client: u32, object: u32) {
        if client == object {
            self.by_tsg.retain(|k, _| k.0 != client);
        } else {
            self.by_tsg.remove(&(client, object));
        }
    }

    /// The guest has scheduled `tsg` and not disabled it since.
    #[must_use]
    pub fn scheduled(&self, client: u32, tsg: u32) -> bool {
        self.by_tsg.get(&(client, tsg)).copied().unwrap_or(false)
    }

    /// Rows held (a leak check for the tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_tsg.len()
    }

    /// No rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_tsg.is_empty()
    }
}

/// The host's group-scheduling verb — the seam. Production is [`HostRm::schedule_enable`].
pub trait GroupScheduler {
    /// `GPFIFO_SCHEDULE` on `chan`'s host group with `bEnable = enable`.
    ///
    /// # Errors
    /// The host's refusal, by name.
    fn schedule_group(&self, chan: Channel, enable: bool) -> Result<(), String>;
}

impl GroupScheduler for HostRm {
    fn schedule_group(&self, chan: Channel, enable: bool) -> Result<(), String> {
        self.schedule_enable(chan, enable)
            .map_err(|e| format!("host {:#x}: {e:?}", chan.token))
    }
}

/// What [`schedule_late_joiner`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateJoin {
    /// The channel is not in a TSG.
    NoTsg,
    /// The guest has not scheduled its TSG (or disabled it since): nothing to follow.
    GuestNotScheduled,
    /// The twin's host group was scheduled, once, from the guest's own scheduled state.
    Authored,
}

/// ★ A twin was just born: if (and only if) the guest has scheduled the TSG it was allocated in,
/// author the schedule of the twin's own host group. Called on the act thread, in the birth act,
/// after the host channel exists — one verb, no waiting. ⊘ The state lock is released BEFORE the
/// host verb: the drainer takes it at every schedule statement and must never wait behind an ioctl.
/// A poisoned lock reads as "not scheduled" (nothing is authored on a doubt).
///
/// # Errors
/// The host's refusal, by name (the birth is then refused: a twin the guest enabled that the host
/// would not enable is not a twin that runs).
pub fn schedule_late_joiner<S: GroupScheduler>(
    host: &S,
    guest: &Mutex<GuestTsgSched>,
    client: u32,
    tsg: Option<u32>,
    chan: Channel,
) -> Result<LateJoin, String> {
    let Some(tsg) = tsg else {
        return Ok(LateJoin::NoTsg);
    };
    let scheduled = guest.lock().is_ok_and(|g| g.scheduled(client, tsg));
    if !scheduled {
        return Ok(LateJoin::GuestNotScheduled);
    }
    host.schedule_group(chan, true)?;
    Ok(LateJoin::Authored)
}

/// The one place a twin's host group is scheduled for a guest statement: twins of one guest TSG
/// that share a host group schedule it once (`done` is the statement's set of groups).
///
/// # Errors
/// The host's refusal, by name.
pub fn schedule_group_once<S: GroupScheduler>(
    host: &S,
    done: &mut HashSet<u32>,
    chan: Channel,
    enable: bool,
) -> Result<(), String> {
    if done.insert(chan.tsg) {
        host.schedule_group(chan, enable)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CLIENT: u32 = 0xc1d0_0046;
    const TSG: u32 = 0xff0e_0000;

    /// A fake host: records every authored group schedule, and what RM would hold afterwards.
    #[derive(Default)]
    struct FakeHost {
        calls: RefCell<Vec<(u32, bool)>>,
        refuse: bool,
    }

    impl FakeHost {
        fn enabled_groups(&self) -> Vec<u32> {
            let mut on: Vec<u32> = Vec::new();
            for (g, en) in self.calls.borrow().iter() {
                on.retain(|x| x != g);
                if *en {
                    on.push(*g);
                }
            }
            on.sort_unstable();
            on
        }
    }

    impl GroupScheduler for FakeHost {
        fn schedule_group(&self, chan: Channel, enable: bool) -> Result<(), String> {
            if self.refuse {
                return Err("host refused".into());
            }
            self.calls.borrow_mut().push((chan.tsg, enable));
            Ok(())
        }
    }

    fn twin(group: u32, token: u32) -> Channel {
        Channel {
            tsg: group,
            chan: 0x100 + token,
            token,
            born_user: None,
        }
    }

    /// Run 111's shape: the first channel is born, the guest schedules the TSG (one twin then),
    /// and three channels join afterwards.
    #[test]
    fn a_twin_born_into_a_scheduled_tsg_gets_exactly_one_authored_schedule() {
        let host = FakeHost::default();
        let guest = Mutex::new(GuestTsgSched::default());
        let first = twin(0x1000, 0x1d);
        // The first member is born before the guest's schedule: nothing to follow.
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), first),
            Ok(LateJoin::GuestNotScheduled)
        );
        // The guest's schedule statement: its one twin's group.
        guest.lock().expect("state").record(CLIENT, TSG, true);
        let mut done = HashSet::new();
        schedule_group_once(&host, &mut done, first, true).expect("guest schedule");
        assert_eq!(host.calls.borrow().len(), 1);
        // The late joiner (its own context share: its own host group).
        let late = twin(0x2000, 0x1e);
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), late),
            Ok(LateJoin::Authored)
        );
        assert_eq!(
            *host.calls.borrow(),
            vec![(0x1000, true), (0x2000, true)],
            "exactly one authored schedule, on the late joiner's own group"
        );
        assert_eq!(host.enabled_groups(), vec![0x1000, 0x2000]);
    }

    /// A TSG the guest has not scheduled — a channel that is not in a TSG at all — and a switch
    /// that is off author nothing.
    #[test]
    fn nothing_is_authored_unless_the_guest_scheduled_the_tsg() {
        let host = FakeHost::default();
        let guest = Mutex::new(GuestTsgSched::default());
        let c = twin(0x3000, 0x20);
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), c),
            Ok(LateJoin::GuestNotScheduled)
        );
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, None, c),
            Ok(LateJoin::NoTsg)
        );
        let scheduled = Mutex::new(GuestTsgSched::default());
        scheduled.lock().expect("state").record(CLIENT, TSG, true);
        // Another client's TSG with the same handle is not this client's.
        assert_eq!(
            schedule_late_joiner(&host, &scheduled, CLIENT + 1, Some(TSG), c),
            Ok(LateJoin::GuestNotScheduled)
        );
        assert!(host.calls.borrow().is_empty());
    }

    /// The guest scheduled, then disabled the TSG: the channel born after is the guest's disabled
    /// one. Re-enabling brings the rule back; a freed TSG (handle reused) forgets both.
    #[test]
    fn a_tsg_the_guest_later_disabled_or_freed_schedules_no_joiner() {
        let host = FakeHost::default();
        let guest = Mutex::new(GuestTsgSched::default());
        let c = twin(0x2000, 0x1e);
        guest.lock().expect("state").record(CLIENT, TSG, true);
        guest.lock().expect("state").record(CLIENT, TSG, false);
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), c),
            Ok(LateJoin::GuestNotScheduled)
        );
        guest.lock().expect("state").record(CLIENT, TSG, true);
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), c),
            Ok(LateJoin::Authored)
        );
        // The TSG is freed; the handle comes back as a new, unscheduled group.
        guest.lock().expect("state").forget(CLIENT, TSG);
        assert!(guest.lock().expect("state").is_empty());
        assert_eq!(
            schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), c),
            Ok(LateJoin::GuestNotScheduled)
        );
        // Freeing the client takes every row it held.
        guest.lock().expect("state").record(CLIENT, TSG, true);
        guest
            .lock()
            .expect("state")
            .record(CLIENT, 0xff0e_0001, true);
        guest.lock().expect("state").record(CLIENT + 1, TSG, true);
        guest.lock().expect("state").forget(CLIENT, CLIENT);
        assert_eq!(guest.lock().expect("state").len(), 1);
        assert!(guest.lock().expect("state").scheduled(CLIENT + 1, TSG));
        assert_eq!(
            host.calls.borrow().len(),
            1,
            "only the re-enabled birth authored"
        );
    }

    /// The guest's own later schedule (a TSG schedule that now names the joiner too, or a
    /// re-send) over groups the joiner already scheduled: one verb per group per statement, no
    /// error, and the set of enabled groups is unchanged.
    #[test]
    fn the_guests_own_later_schedule_is_idempotent_with_the_authored_one() {
        let host = FakeHost::default();
        let guest = Mutex::new(GuestTsgSched::default());
        guest.lock().expect("state").record(CLIENT, TSG, true);
        let first = twin(0x1000, 0x1d);
        let late_a = twin(0x2000, 0x1e);
        // 0x1f shares 0x1e's context share: the SAME host group.
        let late_b = twin(0x2000, 0x1f);
        let late_c = twin(0x3000, 0x20);
        for late in [late_a, late_b, late_c] {
            assert_eq!(
                schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), late),
                Ok(LateJoin::Authored)
            );
        }
        let before = host.enabled_groups();
        assert_eq!(before, vec![0x2000, 0x3000]);
        // Now the guest sends its own schedule over every twin of the TSG.
        let mut done = HashSet::new();
        for c in [first, late_a, late_b, late_c] {
            schedule_group_once(&host, &mut done, c, true).expect("guest schedule");
        }
        assert_eq!(done.len(), 3, "three distinct host groups, each asked once");
        assert_eq!(host.enabled_groups(), vec![0x1000, 0x2000, 0x3000]);
        // And a repeat of the same statement changes nothing.
        let mut again = HashSet::new();
        for c in [first, late_a, late_b, late_c] {
            schedule_group_once(&host, &mut again, c, true).expect("re-send");
        }
        assert_eq!(host.enabled_groups(), vec![0x1000, 0x2000, 0x3000]);
    }

    /// The host's refusal is the birth's refusal, by name — never a swallowed stall.
    #[test]
    fn a_host_refusal_is_returned_by_name() {
        let host = FakeHost {
            refuse: true,
            ..FakeHost::default()
        };
        let guest = Mutex::new(GuestTsgSched::default());
        guest.lock().expect("state").record(CLIENT, TSG, true);
        let err = schedule_late_joiner(&host, &guest, CLIENT, Some(TSG), twin(0x2000, 0x1e))
            .expect_err("refused");
        assert!(err.contains("refused"), "{err}");
    }
}
