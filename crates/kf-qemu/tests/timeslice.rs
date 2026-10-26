//! TS1 simulation: actual replay orchestration with fake RM, plus unchanged mirror rules.
use kf_host::Channel;
use kf_qemu::timeslice::{self, INVALID_STATE, NO_MEMORY, Replay, Setter};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

const KEY: (u32, u32) = (1, 10);
fn channel(group: u32, member: u32) -> Channel {
    Channel {
        tsg: group,
        chan: member,
        token: member,
        born_user: None,
    }
}
#[derive(Default)]
struct Host<'a> {
    calls: RefCell<Vec<(u32, u64)>>,
    programmed: RefCell<BTreeMap<u32, u64>>,
    fail: Cell<Option<u32>>,
    check_unlocked: Option<&'a Replay>,
}
impl Setter for Host<'_> {
    fn set(&self, c: Channel, us: u64) -> Result<(), u32> {
        if let Some(r) = self.check_unlocked {
            assert!(!r.summary().contains("busy"));
        }
        self.calls.borrow_mut().push((c.tsg, us));
        if let Some(s) = self.fail.get() {
            return Err(s);
        }
        // Synthetic hardware rounds to16us. Product code must pass the ORIGINAL request.
        self.programmed.borrow_mut().insert(c.tsg, us / 16 * 16);
        Ok(())
    }
}
fn set(r: &Replay, h: &Host<'_>, key: (u32, u32), us: u64, groups: &[Channel]) -> Result<(), u32> {
    r.apply(&r.prepare(key, 2, us)?, groups, h)
}
fn replay() -> Replay {
    Replay::new(true, 4, 8)
}

#[test]
fn early_set_reaches_the_first_born_group_exactly_once() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    r.birth(KEY, channel(100, 2), false, &h).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 400)]);
}
#[test]
fn two_early_sets_replay_only_the_newest_request() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    set(&r, &h, KEY, 2048, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 2048)]);
}
#[test]
fn early_then_live_set_wins_and_is_replayed_to_later_shards() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    set(&r, &h, KEY, 4000, &[channel(100, 1), channel(100, 2)]).unwrap();
    r.birth(KEY, channel(200, 3), true, &h).unwrap();
    assert_eq!(
        *h.calls.borrow(),
        vec![(100, 400), (100, 4000), (200, 4000)]
    );
}
#[test]
fn live_first_set_is_retained_for_a_later_new_group() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 1000, &[channel(100, 1)]).unwrap();
    r.birth(KEY, channel(200, 2), true, &h).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 1000), (200, 1000)]);
}
#[test]
fn explicit_members_and_ctxshares_never_duplicate_a_group_replay() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 2048, &[]).unwrap();
    for member in 1..=64 {
        r.birth(KEY, channel(100, member), member == 1, &h).unwrap();
    }
    assert_eq!(*h.calls.borrow(), vec![(100, 2048)]);
}
#[test]
fn legacy_kill_switch_shards_each_get_the_requested_value() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 1000, &[]).unwrap();
    for group in 100..104 {
        r.birth(KEY, channel(group, group + 1), true, &h).unwrap();
    }
    assert_eq!(h.calls.borrow().len(), 4);
    assert!(h.calls.borrow().iter().all(|&(_, us)| us == 1000));
}
#[test]
fn never_born_intents_are_bounded_per_client() {
    let r = replay();
    let h = Host::default();
    for t in 10..14 {
        set(&r, &h, (1, t), 400, &[]).unwrap();
    }
    assert_eq!(r.prepare((1, 14), 2, 400).err(), Some(NO_MEMORY));
    assert_eq!(r.len(), 4);
}
#[test]
fn never_born_intents_are_bounded_vm_wide() {
    let r = replay();
    let h = Host::default();
    for c in 1..=2 {
        for t in 10..14 {
            set(&r, &h, (c, t), 400, &[]).unwrap();
        }
    }
    assert_eq!(r.prepare((3, 10), 2, 400).err(), Some(NO_MEMORY));
    assert_eq!(r.len(), 8);
}

#[test]
fn free_cannot_hide_unbounded_queued_tickets_from_the_global_cap() {
    let r = replay();
    let mut held = Vec::new();
    for tsg in 10..18 {
        held.push(r.prepare((1, tsg), 2, 400).unwrap());
        r.forget(1, tsg);
    }
    assert_eq!(r.len(), 0);
    assert_eq!(r.prepare((1, 99), 2, 400).err(), Some(NO_MEMORY));
    for ticket in held {
        r.cancel(&ticket);
    }
}

#[test]
fn canceling_a_freed_ticket_releases_exactly_one_queue_slot() {
    let r = replay();
    let held: Vec<_> = (10..18)
        .map(|tsg| {
            let t = r.prepare((1, tsg), 2, 400).unwrap();
            r.forget(1, tsg);
            t
        })
        .collect();
    r.cancel(&held[0]);
    assert!(r.prepare((1, 90), 2, 400).is_ok());
    assert_eq!(r.prepare((1, 91), 2, 400).err(), Some(NO_MEMORY));
}
#[test]
fn free_before_birth_erases_the_saved_value() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.forget(1, 10);
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert!(h.calls.borrow().is_empty());
    assert_eq!(r.len(), 0);
}
#[test]
fn same_numeric_tsg_after_free_cannot_inherit_old_timeslice() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.forget(1, 10);
    set(&r, &h, KEY, 2048, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 2048)]);
}
#[test]
fn queued_set_from_freed_lifetime_cannot_mutate_reused_tsg() {
    let r = replay();
    let h = Host::default();
    let old = r.prepare(KEY, 2, 400).unwrap();
    r.forget(1, 10);
    set(&r, &h, KEY, 2048, &[]).unwrap();
    assert_eq!(r.apply(&old, &[channel(100, 1)], &h), Err(INVALID_STATE));
    assert!(h.calls.borrow().is_empty());
    assert_eq!(r.desired(KEY), Some(2048));
}
#[test]
fn free_client_cancels_every_reserved_ticket_in_that_namespace() {
    let r = replay();
    let h = Host::default();
    let a = r.prepare(KEY, 2, 400).unwrap();
    let b = r.prepare((1, 11), 2, 1000).unwrap();
    r.forget(1, 1);
    for ticket in [a, b] {
        assert_eq!(r.apply(&ticket, &[], &h), Err(INVALID_STATE));
    }
    assert_eq!(r.len(), 0);
}
#[test]
fn device_free_purges_intents_even_when_no_host_member_was_born() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.forget(1, 2);
    assert_eq!(r.desired(KEY), None);
    assert_eq!(r.len(), 0);
}
#[test]
fn client_namespaces_cannot_steal_each_others_value() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    set(&r, &h, (2, 10), 1000, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    r.birth((2, 10), channel(200, 1), true, &h).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 400), (200, 1000)]);
}
#[test]
fn failed_host_replay_retains_the_value_for_a_retry_birth() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    h.fail.set(Some(0x1f));
    assert_eq!(r.birth(KEY, channel(100, 1), true, &h), Err(0x1f));
    assert_eq!(r.desired(KEY), Some(400));
    h.fail.set(None);
    r.birth(KEY, channel(200, 1), true, &h).unwrap();
    assert_eq!(r.last_applied(200), Some(400));
}
#[test]
fn host_invalid_state_is_not_misreported_as_invalid_argument() {
    let r = replay();
    let h = Host::default();
    h.fail.set(Some(INVALID_STATE));
    assert_eq!(
        set(&r, &h, KEY, 400, &[channel(100, 1)]),
        Err(INVALID_STATE)
    );
    assert_eq!(r.desired(KEY), None);
}
#[test]
fn requested_value_is_not_rounded_on_the_cpu() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 1001, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert_eq!(h.programmed.borrow().get(&100), Some(&992));
    assert_eq!(r.last_applied(100), Some(1001));
}
#[test]
fn host_rounding_refusal_is_a_birth_failure_not_a_consumed_intent() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, u64::MAX, &[]).unwrap();
    h.fail.set(Some(0x1f));
    assert_eq!(r.birth(KEY, channel(100, 1), true, &h), Err(0x1f));
    assert_eq!(r.desired(KEY), Some(u64::MAX));
    assert_eq!(r.last_applied(100), None);
}
#[test]
fn native_group_free_drops_only_telemetry_not_guest_intent() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    r.group_freed(100);
    assert_eq!(r.last_applied(100), None);
    assert_eq!(r.desired(KEY), Some(400));
}
#[test]
fn canceled_queue_reservations_do_not_spend_the_table_forever() {
    let r = replay();
    for i in 0..32 {
        let t = r.prepare((1, i + 10), 2, 400).unwrap();
        r.cancel(&t);
    }
    assert_eq!(r.len(), 0);
}
#[test]
fn reset_cancels_old_tickets_and_forgets_all_requested_values() {
    let r = replay();
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    let t = r.prepare(KEY, 2, 1000).unwrap();
    r.clear();
    assert_eq!(r.apply(&t, &[], &h), Err(INVALID_STATE));
    assert_eq!(r.len(), 0);
}
#[test]
fn status_reports_early_replayed_and_last_us_without_host_locking() {
    let r = replay();
    let h = Host {
        check_unlocked: Some(&r),
        ..Host::default()
    };
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert_eq!(r.stats().early_sets, 1);
    assert_eq!(r.stats().replayed, 1);
    assert_eq!(r.stats().dropped, 0);
    assert!(r.summary().contains("0x64:400"));
}
#[test]
fn measurement_zero_suppresses_only_replay_and_counts_it() {
    let r = Replay::new(false, 4, 8);
    let h = Host::default();
    set(&r, &h, KEY, 400, &[]).unwrap();
    r.birth(KEY, channel(100, 1), true, &h).unwrap();
    assert!(h.calls.borrow().is_empty());
    assert_eq!(r.stats().dropped, 1);
    set(&r, &h, KEY, 1000, &[channel(100, 1)]).unwrap();
    assert_eq!(*h.calls.borrow(), vec![(100, 1000)]);
}
#[test]
fn default_on_kill_switch_is_exact_zero_only() {
    for value in [None, Some("1"), Some("false"), Some("")] {
        assert!(timeslice::flag_on(value));
    }
    assert!(!timeslice::flag_on(Some("0")));
}
