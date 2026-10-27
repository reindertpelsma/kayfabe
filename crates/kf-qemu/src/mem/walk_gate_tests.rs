//! Production Inbox contracts for scoped source admission and its release/wake protocol.
//! The original four unrelated-space questions are active tests after the EXPECTED RED proof.

use super::{Inbox, SplitTarget};
use kf_mem::vasmgr::VasKey;

const A: VasKey = VasKey(0x100);
const B: VasKey = VasKey(0x200);

fn current_user_gate(inbox: &Inbox, ring_space: VasKey) -> bool {
    inbox.walk_busy_for(ring_space)
}

#[test]
fn a_space_split_holds_its_own_ring_until_successful_completion() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_user_split(7, A).ok().unwrap();
    assert!(current_user_gate(&inbox, A));
    assert_eq!(
        inbox.take_split_requests(),
        vec![(t, SplitTarget::Space(A))]
    );
    assert!(current_user_gate(&inbox, A));
    inbox.finish_split(t, Ok(()));
    assert!(!current_user_gate(&inbox, A));
}

#[test]
fn an_all_pdb_split_holds_every_potentially_affected_space() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, None);
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, B));
    inbox.take_split_requests();
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, B));
    inbox.finish_split(t, Ok(()));
    assert!(!current_user_gate(&inbox, A));
    assert!(!current_user_gate(&inbox, B));
}

#[test]
fn a_queued_walk_of_a_does_not_hold_an_unrelated_ring_in_b() {
    let inbox = Inbox::new().unwrap();
    inbox.request_user_split(7, A).ok().unwrap();
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "B has no pending mapping change"
    );
}

#[test]
fn a_running_walk_of_a_does_not_hold_an_unrelated_ring_in_b() {
    let inbox = Inbox::new().unwrap();
    inbox.request_user_split(7, A).ok().unwrap();
    inbox.take_split_requests();
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "taking a split must preserve its scope"
    );
}

#[test]
fn completing_a_releases_a_while_an_independent_walk_of_b_remains_pending() {
    let inbox = Inbox::new().unwrap();
    let a = inbox.request_user_split(7, A).ok().unwrap();
    inbox.request_user_split(9, B).ok().unwrap();
    inbox.take_split_requests();
    inbox.finish_split(a, Ok(()));
    assert!(current_user_gate(&inbox, B));
    assert!(!current_user_gate(&inbox, A), "B must not extend A's hold");
}

#[test]
fn a_scoped_retire_statement_does_not_hold_an_unrelated_ring() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    });
    assert!(!inbox.all_settled());
    assert!(current_user_gate(&inbox, A));
    assert!(
        !current_user_gate(&inbox, B),
        "unsettled is currently a VM-global gate"
    );
}

#[test]
fn two_own_space_requests_do_not_release_each_other() {
    let inbox = Inbox::new().unwrap();
    let a = inbox.request_user_split(7, A).ok().unwrap();
    let b = inbox.request_user_split(8, A).ok().unwrap();
    inbox.take_split_requests();
    inbox.finish_split(a, Ok(()));
    assert!(inbox.walk_busy_for(A));
    assert!(!inbox.walk_busy_for(B));
    inbox.finish_split(b, Ok(()));
    assert!(!inbox.walk_busy_for(A));
}

#[test]
fn taking_an_own_request_preserves_its_target_until_host_invalidate_ack() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_user_split(7, A).ok().unwrap();
    inbox.take_split_requests();
    inbox.set_walk_scope(vec![A], false, None);
    assert!(inbox.walk_busy_for(A));
    assert!(!inbox.walk_busy_for(B));
    inbox.set_walk_scope(vec![], false, None);
    assert!(
        inbox.walk_busy_for(A),
        "the split's host ACK has not arrived"
    );
    inbox.finish_split(t, Ok(()));
    assert!(!inbox.walk_busy_for(A));
}

#[test]
fn an_unresolved_named_pdb_split_is_global_even_after_dequeue() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, Some(0x3000));
    inbox.take_split_requests();
    assert!(inbox.walk_busy_for(A));
    assert!(inbox.walk_busy_for(B));
    inbox.finish_split(t, Ok(()));
    assert!(!inbox.walk_busy_for(A));
}

#[test]
fn manager_global_fallback_and_cpu_window_targets_hold_every_ring() {
    for key in [super::K_BAR1, super::K_BAR2] {
        let inbox = Inbox::new().unwrap();
        inbox.set_walk_scope(vec![key], false, None);
        assert!(inbox.walk_busy_for(A));
        assert!(inbox.walk_busy_for(B));
    }
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![], true, None);
    assert!(inbox.walk_busy_for(A));
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn a_new_armed_generation_invalidates_the_resolved_scope_snapshot() {
    let inbox = Inbox::new().unwrap();
    let regs =
        kf_trap::InvalidateRegs::from_usermode_base(kf_trap::memmap::VF_USERMODE_PAGE).unwrap();
    let port = std::sync::Arc::new(kf_trap::InvalidatePort::new(regs));
    inbox.walk_port.set(port.clone()).unwrap();
    let first = port.trigger().arm_next();
    inbox.set_walk_scope(vec![A], false, Some(first));
    assert!(!inbox.walk_busy_for(B));
    port.trigger().arm_next();
    assert!(
        inbox.walk_busy_for(B),
        "the new request has not been resolved"
    );
    assert!(inbox.walk_busy_for(A));
}

#[test]
fn an_unresolved_first_arm_closes_even_an_idle_scoped_snapshot() {
    let inbox = Inbox::new().unwrap();
    let regs =
        kf_trap::InvalidateRegs::from_usermode_base(kf_trap::memmap::VF_USERMODE_PAGE).unwrap();
    let port = std::sync::Arc::new(kf_trap::InvalidatePort::new(regs));
    inbox.walk_port.set(port.clone()).unwrap();
    inbox.set_walk_scope(vec![], false, None);
    port.trigger().arm_next();
    assert!(inbox.walk_busy_for(A));
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn scoped_statement_settlement_never_erases_a_later_statement() {
    let inbox = Inbox::new().unwrap();
    let st = kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    };
    inbox.push(st);
    inbox.take();
    inbox.push(st);
    inbox.settle(1);
    assert!(inbox.walk_busy_for(A));
    assert!(!inbox.walk_busy_for(B));
    assert!(
        !inbox.all_settled(),
        "held RPC reply ordering is still global"
    );
    inbox.settle(2);
    assert!(!inbox.walk_busy_for(A));
    assert!(inbox.all_settled());
}

#[test]
fn statement_scope_overflow_is_global_and_recovers_after_settlement() {
    let inbox = Inbox::new().unwrap();
    for vaspace in 1..=513 {
        inbox.push(kf_rm::barpde::MemStatement::Retire { client: 0, vaspace });
    }
    assert!(inbox.walk_busy_for(VasKey(1000)));
    inbox.settle(512);
    assert!(inbox.walk_busy_for(VasKey(1000)));
    inbox.settle(513);
    assert!(!inbox.walk_busy_for(VasKey(1000)));
}

#[test]
fn sysmembar_is_global_and_is_not_released_by_settling_an_older_statement() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Sysmembar);
    inbox.push(kf_rm::barpde::MemStatement::Sysmembar);
    inbox.settle(1);
    assert!(inbox.walk_busy_for(A));
    assert!(inbox.walk_busy_for(B));
    inbox.settle(2);
    assert!(!inbox.walk_busy_for(B));
}

#[test]
fn ready_waiter_a_is_rung_while_b_is_still_held_without_a_timer() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_busy(true);
    inbox.set_walk_scope(vec![A, B], false, None);
    inbox.wait_walk_for(A, 7);
    inbox.wait_walk_for(B, 9);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    inbox.set_walk_scope(vec![B], false, None);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    inbox.set_walk_scope(vec![], false, None);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![9]);
}

#[test]
fn completion_between_check_and_waiter_insertion_keeps_the_trailing_wake() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    assert!(inbox.walk_busy_for(A));
    inbox.set_walk_scope(vec![], false, None);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    inbox.wait_walk_for(A, 7);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
}

#[test]
fn reset_drops_old_waiter_identity_before_a_token_is_reused() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    inbox.wait_walk_for(A, 7);
    inbox.reset_life();
    inbox.set_walk_scope(vec![B], false, None);
    inbox.wait_walk_for(B, 7);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    inbox.set_walk_scope(vec![], false, None);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
}

#[test]
fn scoped_waiter_metadata_is_bounded_and_overflow_keeps_the_legacy_wake() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    for token in 0..600 {
        inbox.wait_walk_for(A, token);
    }
    assert_eq!(inbox.scoped_waiters.lock().unwrap().len(), 512);
    assert_eq!(inbox.take_walk_waiters().len(), 88);
    inbox.set_walk_scope(vec![], false, None);
    assert_eq!(inbox.take_ready_walk_waiters().len(), 512);
}

#[test]
fn another_key_enqueued_after_take_survives_settling_the_older_key() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    });
    inbox.take();
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: B.0 as u32,
    });
    inbox.settle(1);
    assert!(!inbox.walk_busy_for(A));
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn bar1_target_scope_retains_a_global_hold_for_cpu_source_views() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![super::K_BAR1], false, None);
    assert!(inbox.walk_busy_for(A));
}

#[test]
fn bar2_target_scope_retains_a_global_hold_for_cpu_source_views() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![super::K_BAR2], false, None);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn manager_snapshot_overflow_is_closed_even_for_an_unlisted_ring() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope((0..513).map(VasKey).collect(), false, None);
    assert!(inbox.walk_busy_for(VasKey(1000)));
}

#[test]
fn turn_publication_and_migration_all_call_the_native_scoped_helper() {
    let src = include_str!("../userpump.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert_eq!(
        src.matches("!me.inbox.user_source_ready(r.vas_key, r.idx, &r.walk_hold)")
            .count(),
        3
    );
    let predicate = include_str!("walk_scope.rs");
    assert!(predicate.contains("self.source_gate(key, token)"));
    assert!(predicate.contains("self.wait_walk_for(key, token);"));
}

#[test]
fn publication_rechecks_scope_before_any_guarded_native_write() {
    let src = include_str!("../userpump.rs");
    let body = src
        .split("fn publish_guarded(")
        .nth(1)
        .unwrap()
        .split("let userd = r.userd.clone();")
        .next()
        .unwrap();
    assert!(body.contains("!me.inbox.user_source_ready(r.vas_key, r.idx, &r.walk_hold)"));
}

#[test]
fn migration_rechecks_scope_before_switching_any_source_or_generation() {
    let src = include_str!("../userpump.rs");
    let body = src
        .split("USER_RING_MIGRATION_RING")
        .nth(1)
        .unwrap()
        .split("r.userd = userrelay::UserdSource::Registered")
        .next()
        .unwrap();
    assert!(body.contains("!me.inbox.user_source_ready(r.vas_key, r.idx, &r.walk_hold)"));
}

#[test]
fn manager_scope_is_published_before_synchronous_walk_apply_and_after_ack() {
    let src = include_str!("../device.rs");
    let before = src
        .split("let r = m.on_walk_ready(trigger);")
        .next()
        .unwrap();
    assert!(
        before.rfind("set_walk_scope(").unwrap()
            < before.rfind("take_ready_walk_waiters()").unwrap()
    );
    assert_eq!(src.matches("m.walk_gate_scope(armed)").count(), 2);
    assert!(src.contains("finish_gate_loop("));
}

#[test]
fn scoped_ready_drain_follows_statement_settlement_in_the_real_va_loop() {
    let src = include_str!("../device.rs");
    assert!(src.contains(".finish_gate_loop("));
    let body = include_str!("walk_scope.rs")
        .split("pub(crate) fn finish_gate_loop(")
        .nth(1)
        .unwrap();
    assert!(
        body.find("self.settle(n)").unwrap() < body.find("self.take_ready_walk_waiters()").unwrap()
    );
}

#[test]
fn settled_statement_waiter_is_ready_while_an_unrelated_manager_walk_continues() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![B], false, None);
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    });
    inbox.wait_walk_for(A, 7);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    inbox.settle(1);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn the_source_gate_registers_a_queued_split_waiter_before_giving_up() {
    let inbox = Inbox::new().unwrap();
    let ticket = inbox.request_user_split(7, A).ok().unwrap();
    assert!(inbox.source_gate(A, 7).held());
    inbox.take_split_requests();
    inbox.finish_split(ticket, Ok(()));
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
}

#[test]
fn the_source_gate_registers_an_active_walk_waiter_before_giving_up() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    assert!(inbox.source_gate(A, 7).held());
    inbox.set_walk_scope(vec![], false, None);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
}

#[test]
fn the_before_apply_snapshot_is_not_replaced_by_only_an_end_of_loop_snapshot() {
    let src = include_str!("../device.rs");
    let apply = src.find("let r = m.on_walk_ready(trigger);").unwrap();
    let publish = src.find(".set_walk_scope(").unwrap();
    assert!(publish < apply);
}

#[test]
fn oversized_snapshot_with_repeated_targets_still_holds_an_unrelated_ring() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A; 513], false, None);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn the_va_release_transaction_resumes_a_statement_waiter_before_another_space_finishes() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Retire {
        client: 0,
        vaspace: A.0 as u32,
    });
    inbox.wait_walk_for(A, 7);
    let (advanced, ready) = inbox.finish_gate_loop(vec![B], false, None, true, Some(1));
    assert!(advanced);
    assert_eq!(ready, vec![7]);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn the_va_release_transaction_wakes_legacy_overflow_when_its_last_statement_settles() {
    let inbox = Inbox::new().unwrap();
    inbox.push(kf_rm::barpde::MemStatement::Sysmembar);
    inbox.wait_walk(7);
    let (advanced, ready) = inbox.finish_gate_loop(vec![], false, None, false, Some(1));
    assert!(advanced);
    assert_eq!(ready, vec![7]);
    assert!(!inbox.walk_busy());
}

fn page_dir(key: VasKey) -> kf_rm::barpde::MemStatement {
    kf_rm::barpde::MemStatement::PageDir(kf_rm::rmrpc::PageDirStatement {
        client: kf_arch::ids::HClient(0),
        vaspace: kf_arch::ids::HObject(key.0 as u32),
        pdb: kf_arch::ids::Pdb(0x1000_0000),
        pdb_aperture: Some(kf_arch::Aperture::Vidmem),
    })
}

#[test]
fn per_space_applied_a_resumes_while_b_walks_and_its_rpc_reply_barrier_stays_closed() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.take();
    inbox.wait_walk_for(A, 7);
    inbox.note_statements_applied(1);
    let (advanced, ready) = inbox.finish_gate_loop(vec![B], false, None, true, None);
    assert!(!advanced);
    assert!(!inbox.all_settled(), "RPC reply settlement remains VM-wide");
    assert_eq!(ready, vec![7], "A finished; only B is still being walked");
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn per_space_applied_a_releases_while_b_keeps_its_own_pending_statement() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.push(page_dir(B));
    inbox.take();
    inbox.wait_walk_for(A, 7);
    inbox.wait_walk_for(B, 9);
    inbox.note_statements_applied(2);
    let (advanced, ready) = inbox.finish_gate_loop(vec![B], false, None, true, None);
    assert!(!advanced);
    assert_eq!(inbox.counts(), (2, 0));
    assert_eq!(ready, vec![7]);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn per_space_applied_milestone_never_clears_a_newer_untaken_space() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.take();
    inbox.push(page_dir(B));
    inbox.note_statements_applied(1);
    inbox.finish_gate_loop(vec![], false, None, true, None);
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn per_space_applied_milestone_never_clears_a_newer_untaken_request_for_the_same_space() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.take();
    inbox.push(page_dir(A));
    inbox.note_statements_applied(1);
    inbox.finish_gate_loop(vec![B], false, None, true, None);
    assert!(inbox.walk_busy_for(A));
}

#[test]
fn applied_space_is_released_at_scope_publication_before_foreign_synchronous_apply() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.take();
    inbox.note_statements_applied(1);
    inbox.wait_walk_for(A, 7);
    inbox.set_walk_scope(vec![B], false, None);
    assert_eq!(inbox.take_ready_walk_waiters(), vec![7]);
    assert!(!inbox.all_settled());
}

#[test]
fn new_doorbell_for_applied_space_does_not_wait_for_the_foreign_apply_to_return() {
    let inbox = Inbox::new().unwrap();
    inbox.push(page_dir(A));
    inbox.take();
    inbox.note_statements_applied(1);
    inbox.set_walk_scope(vec![B], false, None);
    assert!(!inbox.source_gate(A, 7).held());
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn registration_after_release_returns_ready_without_waiting_for_a_foreign_walk() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    assert!(inbox.walk_busy_for(A));
    inbox.set_walk_scope(vec![B], false, None);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    assert!(inbox.register_walk_waiter(A, 7));
    assert!(inbox.walk_busy_for(B));
}

#[test]
fn registration_after_idle_release_preserves_the_immediate_continuation() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    assert!(inbox.walk_busy_for(A));
    inbox.set_walk_scope(vec![], false, None);
    assert!(inbox.take_ready_walk_waiters().is_empty());
    assert!(inbox.register_walk_waiter(A, 7));
}

#[test]
fn source_gate_continues_when_release_drains_between_check_and_registration() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    let reason = inbox.source_gate_between(A, 7, || {
        inbox.set_walk_scope(vec![B], false, None);
        assert!(inbox.take_ready_walk_waiters().is_empty());
    });
    assert!(!reason.held());
    assert!(inbox.walk_busy_for(B));
}
#[test]
fn source_gate_continues_when_idle_release_precedes_its_registration() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    let reason = inbox.source_gate_between(A, 7, || {
        inbox.set_walk_scope(vec![], false, None);
        inbox.take_ready_walk_waiters();
    });
    assert!(!reason.held());
}
#[test]
fn translated_wait_reowes_the_exact_guest_token_if_ready_during_a_foreign_walk() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![B], false, None);
    let mut owed = Vec::new();
    inbox.register_walk_waiter_and_rearm(A, 7, |token| owed.push(token));
    assert_eq!(owed, vec![7]);
}
#[test]
fn translated_wait_reowes_after_idle_release_but_not_while_its_own_walk_is_active() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_scope(vec![A], false, None);
    let mut owed = Vec::new();
    inbox.register_walk_waiter_and_rearm(A, 7, |token| owed.push(token));
    assert!(owed.is_empty());
    inbox.set_walk_scope(vec![], false, None);
    inbox.register_walk_waiter_and_rearm(A, 7, |token| owed.push(token));
    assert_eq!(owed, vec![7]);
}
#[test]
fn caller_announces_processed_statements_after_apply_and_wakes_before_foreign_apply() {
    let src = include_str!("../device.rs");
    let apply = src.find("let line = crate::mem::apply_statement(").unwrap();
    let note = src
        .find("self.mem.inbox.note_statements_applied(taken);")
        .unwrap();
    let gpu = src.find("let r = m.on_walk_ready(trigger);").unwrap();
    assert!(apply < note && note < gpu);
    assert!(src[..gpu].rfind("take_ready_walk_waiters()").unwrap() > note);
    assert!(
        include_str!("../chan.rs")
            .contains("register_walk_waiter_and_rearm(g.key, g.guest_idx, |token|")
    );
}

#[test]
fn ens11_resolved_named_pdb_split_releases_unrelated_space() {
    let inbox = Inbox::new().unwrap();
    let ticket = inbox.request_split(7, Some(0x1000));
    inbox.take_split_requests();
    inbox.set_walk_scope(vec![A], false, None);
    inbox.mark_split_scopes_resolved(&[ticket]);
    assert!(current_user_gate(&inbox, A));
    assert!(!current_user_gate(&inbox, B));
}
#[test]
fn ens11_resolved_named_pdb_split_aliases_hold_only_the_resolved_union() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, Some(0x1000));
    let u = inbox.request_split(8, Some(0x1000));
    inbox.take_split_requests();
    let c = VasKey(0x300);
    inbox.set_walk_scope(vec![A, c], false, None);
    inbox.mark_split_scopes_resolved(&[t, u]);
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, c));
    assert!(!current_user_gate(&inbox, B));
}
#[test]
fn ens11_late_unresolved_pdb_split_restores_the_conservative_hold() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, Some(0x1000));
    inbox.take_split_requests();
    inbox.set_walk_scope(vec![A], false, None);
    inbox.mark_split_scopes_resolved(&[t]);
    inbox.request_split(8, Some(0x2000));
    assert!(current_user_gate(&inbox, B));
}
#[test]
fn ens11_unknown_or_all_pdb_work_cannot_be_marked_as_resolved() {
    for pdb in [None, Some(0x1000)] {
        let inbox = Inbox::new().unwrap();
        let t = inbox.request_split(7, pdb);
        inbox.take_split_requests();
        inbox.set_walk_scope(vec![], true, None);
        inbox.mark_split_scopes_resolved(&[t]);
        assert!(current_user_gate(&inbox, A));
        assert!(current_user_gate(&inbox, B));
    }
}

#[test]
fn ens11_idle_layout_admission_cannot_clear_a_queued_split_interrupt() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let inbox = Inbox::new().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    inbox.bind_layout_interrupt(stop.clone());
    let t = inbox.request_split(7, Some(0x1000));
    assert!(stop.load(Ordering::Acquire));
    assert!(!inbox.allow_deferred_layout());
    assert!(stop.load(Ordering::Acquire));
    inbox.take_split_requests();
    assert!(!inbox.allow_deferred_layout());
    inbox.finish_split(t, Ok(()));
    assert!(inbox.allow_deferred_layout());
    assert!(!stop.load(Ordering::Acquire));
}
#[test]
fn ens11_a_user_split_arriving_after_idle_admission_interrupts_the_next_layout_verb() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let inbox = Inbox::new().unwrap();
    let stop = Arc::new(AtomicBool::new(true));
    inbox.bind_layout_interrupt(stop.clone());
    assert!(inbox.allow_deferred_layout());
    assert!(!stop.load(Ordering::Acquire));
    inbox.request_user_split(8, A).ok().unwrap();
    assert!(stop.load(Ordering::Acquire));
    assert!(!inbox.allow_deferred_layout());
}

#[test]
fn ens11_all_pdb_stays_global_even_with_an_empty_resolved_snapshot() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, None);
    inbox.take_split_requests();
    inbox.set_walk_scope(vec![], false, None);
    inbox.mark_split_scopes_resolved(&[t]);
    assert!(current_user_gate(&inbox, A));
    assert!(current_user_gate(&inbox, B));
}
#[test]
fn ens11_all_pdb_cannot_borrow_the_previous_named_target_union() {
    let inbox = Inbox::new().unwrap();
    let t = inbox.request_split(7, None);
    inbox.take_split_requests();
    inbox.set_walk_scope(vec![A], false, None);
    inbox.mark_split_scopes_resolved(&[t]);
    assert!(current_user_gate(&inbox, B));
}
