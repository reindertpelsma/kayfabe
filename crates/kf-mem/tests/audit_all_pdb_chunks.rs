//! SA-03/VA-2. FALSIFIER: an object is omitted, a walk exceeds the ABI bound,
//! or an invalidate clears before every chunk and asynchronous target is live.
use kf_cuda::{WalkEntry, abi::KfMapRun};
use kf_mem::apply::PermPolicy;
use kf_mem::ledger::{Desired, MapTarget, Mapped, Settle};
use kf_mem::vasmgr::{EntryDiff, VaManager, VasKey, WalkDone, Walker};
use kf_trap::{
    ClearOutcome, Invalidate, InvalidatePort, InvalidateRegs, InvalidateRequest, PdbAperture,
    PortWrite,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct RecordingWalker {
    pending: Option<Vec<WalkEntry>>,
    submitted: Vec<Vec<u32>>,
    generation: u64,
    attempts: Vec<Vec<(u32, u64)>>,
    refuse_attempt: Option<usize>,
}
impl Walker for RecordingWalker {
    fn submit(&mut self, entries: &[WalkEntry]) -> Result<(), String> {
        assert!(self.pending.is_none());
        self.attempts
            .push(entries.iter().map(|e| (e.slot, e.pdb)).collect());
        if self.refuse_attempt == Some(self.attempts.len()) {
            return Err("injected continuation submit failure".into());
        }
        assert!(!entries.is_empty() && entries.len() <= 64);
        self.submitted
            .push(entries.iter().map(|e| e.slot).collect());
        self.pending = Some(entries.to_vec());
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        let Some(entries) = self.pending.take() else {
            return Ok(None);
        };
        self.generation += 1;
        let done = WalkDone {
            generation: self.generation,
            nrun: entries.len(),
            gpu_us: 0,
            entries: entries
                .into_iter()
                .enumerate()
                .map(|(i, e)| EntryDiff {
                    pdb: e.pdb,
                    slot: e.slot,
                    first: i,
                    runs: vec![PermPolicy::default().diff_run(&KfMapRun {
                        va: 0x1000_0000,
                        gpga: u64::from(e.slot) * 0x1000,
                        len: 0x1000,
                        flags: 0,
                        op: 1,
                        pdb_index: 0,
                    })],
                    ..Default::default()
                })
                .collect(),
        };
        Ok(Some(done))
    }
    fn ack(&mut self, generation: u64, codes: Vec<u8>) -> Result<(), String> {
        assert_eq!(generation, self.generation);
        assert!(codes.iter().all(|&c| c <= kf_cuda::abi::KFWR_ACK_APPLIED));
        Ok(())
    }
    fn reset(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn slots(&self) -> u32 {
        128
    }
}
struct Target {
    key: u64,
    maps: Rc<RefCell<Vec<(u64, bool)>>>,
    port: Rc<InvalidatePort>,
    live: Rc<Cell<bool>>,
    fail_invalidate: Rc<Cell<Option<u64>>>,
}
impl MapTarget for Target {
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        self.maps
            .borrow_mut()
            .push((self.key, self.port.trigger().read() != 0));
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        if self.fail_invalidate.get() == Some(self.key) {
            Err("injected invalidate refusal".into())
        } else {
            Ok(())
        }
    }
    fn settle(&self) -> Settle {
        if self.key == 0 && !self.live.get() {
            Settle::Pending
        } else {
            Settle::Live
        }
    }
}
struct Rig {
    va: VaManager<RecordingWalker, Target>,
    port: Rc<InvalidatePort>,
    maps: Rc<RefCell<Vec<(u64, bool)>>>,
    live: Rc<Cell<bool>>,
    fail_invalidate: Rc<Cell<Option<u64>>>,
}
fn rig(n: u64) -> Rig {
    let port = Rc::new(InvalidatePort::new(
        InvalidateRegs::from_usermode_base(0xbb_0000).unwrap(),
    ));
    let maps = Rc::new(RefCell::new(Vec::new()));
    let live = Rc::new(Cell::new(true));
    let fail_invalidate = Rc::new(Cell::new(None));
    let mut va = VaManager::new(
        RecordingWalker::default(),
        1 << 32,
        Box::new(|gpa, _| Some(gpa)),
    );
    for k in 0..n {
        va.table.insert(
            VasKey(k),
            Target {
                key: k,
                maps: maps.clone(),
                port: port.clone(),
                live: live.clone(),
                fail_invalidate: fail_invalidate.clone(),
            },
        );
        va.table
            .set_root(VasKey(k), (k + 1) * 0x10_0000, PdbAperture::Vidmem)
            .unwrap();
    }
    Rig {
        va,
        port,
        maps,
        live,
        fail_invalidate,
    }
}
fn arm(r: &Rig) -> InvalidateRequest {
    let regs = r.port.regs();
    let (lo, hi) = Invalidate::encode_pdb(0, PdbAperture::Vidmem);
    r.port.write(regs.pdb, lo);
    r.port.write(regs.upper_pdb, hi);
    let PortWrite::Publish(req) = r.port.write(regs.trigger, (1 << 31) | 3) else {
        panic!("arm")
    };
    req
}
fn check_chunks(n: u64) {
    let mut r = rig(n);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    assert!(
        r.va.in_flight(),
        "the first ABI-bounded chunk must be submitted"
    );
    let first = r.va.on_walk_ready(r.port.trigger());
    assert!(first.completed.is_empty() && first.unreconciled.is_empty());
    assert_ne!(
        r.port.trigger().read(),
        0,
        "the remaining objects are not mapped yet"
    );
    assert_eq!(r.maps.borrow().len(), 64);
    assert!(r.va.in_flight(), "the next chunk must have an event source");
    let done = r.va.on_walk_ready(r.port.trigger());
    assert_eq!(done.completed, vec![(req.seq, ClearOutcome::Cleared)]);
    assert_eq!(r.port.trigger().read(), 0);
    let mut keys: Vec<_> = r
        .maps
        .borrow()
        .iter()
        .map(|&(k, busy)| {
            assert!(busy);
            k
        })
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, (0..n).collect::<Vec<_>>());
    assert!(
        r.va.walker()
            .submitted
            .iter()
            .all(|chunk| chunk.len() <= 64)
    );
    assert_eq!(r.va.stats.unreconciled, 0);
}
#[test]
fn all_pdb_seventy_spaces_clear_after_both_chunks() {
    check_chunks(70);
}
#[test]
fn all_pdb_one_hundred_twenty_eight_spaces_clear_after_both_chunks() {
    check_chunks(128);
}
#[test]
fn a_pending_target_from_the_first_chunk_holds_the_final_clear() {
    let mut r = rig(70);
    r.live.set(false);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    let _ = r.va.on_walk_ready(r.port.trigger());
    let done = r.va.on_walk_ready(r.port.trigger());
    assert!(done.completed.is_empty());
    assert_eq!(r.maps.borrow().len(), 70);
    assert_eq!(r.va.awaiting(), 1);
    assert_ne!(r.port.trigger().read(), 0);
    r.live.set(true);
    assert_eq!(
        r.va.on_targets(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
}
#[test]
fn a_new_invalidate_between_chunks_waits_for_its_own_round() {
    let mut r = rig(70);
    let old = arm(&r);
    r.va.on_invalidate(old, r.port.trigger());
    let new = arm(&r);
    r.va.on_invalidate(new, r.port.trigger());
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(old.seq, ClearOutcome::Superseded)]
    );
    assert_ne!(r.port.trigger().read(), 0);
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(new.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.maps.borrow().len(), 140);
}

#[test]
fn an_invalidate_refusal_in_the_first_chunk_is_never_forgotten() {
    hard_failure(0);
}
#[test]
fn an_invalidate_refusal_in_the_last_chunk_still_holds_the_clear() {
    hard_failure(69);
}
fn hard_failure(key: u64) {
    let mut r = rig(70);
    r.fail_invalidate.set(Some(key));
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    let out = r.va.on_walk_ready(r.port.trigger());
    assert_eq!(r.maps.borrow().len(), 70);
    assert!(out.completed.is_empty());
    assert_eq!(out.unreconciled, vec![req.seq]);
    assert_ne!(r.port.trigger().read(), 0);
}
#[test]
fn reset_discards_unsubmitted_chunks_of_the_old_life() {
    reset_chunks(false);
}
#[test]
fn reset_discards_the_original_obligation_during_the_final_chunk() {
    reset_chunks(true);
}
fn reset_chunks(after_first: bool) {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    if after_first {
        assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    }
    assert!(r.va.abandon_life().in_flight);
    assert!(r.port.reset());
    let out = r.va.on_walk_ready(r.port.trigger());
    assert!(out.completed.is_empty());
    assert!(!r.va.in_flight());
    assert_eq!(
        r.va.walker().submitted.len(),
        if after_first { 2 } else { 1 }
    );
    assert_eq!(r.maps.borrow().len(), if after_first { 70 } else { 64 });
}

#[test]
fn an_all_pdb_split_over_seventy_spaces_finishes_after_both_chunks() {
    split_chunks(70);
}
#[test]
fn an_all_pdb_split_over_one_hundred_twenty_eight_spaces_finishes_after_both_chunks() {
    split_chunks(128);
}
fn split_chunks(n: u64) {
    let mut r = rig(n);
    r.va.on_split(None, 77, r.port.trigger());
    assert!(
        r.va.take_splits().is_empty(),
        "an accepted split waits for its walk"
    );
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert!(
        r.va.take_splits().is_empty(),
        "the second chunk is still owed"
    );
    assert!(r.va.in_flight());
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_eq!(r.va.take_splits(), vec![(77, Ok(()))]);
    assert_eq!(r.maps.borrow().len(), n as usize);
}

#[test]
fn a_root_move_in_the_first_completed_object_prevents_a_stale_clear() {
    moved_after_first(0);
}
#[test]
fn a_root_move_in_the_last_object_of_the_first_chunk_prevents_a_stale_clear() {
    moved_after_first(63);
}
fn moved_after_first(key: u64) {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_eq!(r.maps.borrow().len(), 64);
    r.va.table
        .set_root(VasKey(key), 0x1000_0000, PdbAperture::Vidmem)
        .unwrap();
    let out = r.va.on_walk_ready(r.port.trigger());
    assert!(
        out.completed.is_empty(),
        "the earlier object's new root was never walked"
    );
    assert_ne!(r.port.trigger().read(), 0);
    assert!(
        out.unreconciled.is_empty(),
        "a moved root is retried, not failed"
    );
    assert!(
        r.va.in_flight(),
        "the original invalidate must have a retry event source"
    );
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_ne!(
        r.port.trigger().read(),
        0,
        "the retry still owes its second chunk"
    );
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.port.trigger().read(), 0);
    assert_eq!(r.va.stats.unreconciled, 0);
    assert!(!r.va.in_flight());
}

// Candidate-6 review F3. FALSIFIER: a later chunk names an object already retired,
// a reused slot's old root, or a root superseded before that chunk was submitted.
#[test]
fn a_retired_first_object_of_the_next_chunk_is_not_submitted() {
    retired_before_next(64);
}
#[test]
fn a_retired_last_object_of_the_next_chunk_is_not_submitted() {
    retired_before_next(69);
}
fn retired_before_next(key: u64) {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    r.va.remove(VasKey(key)).unwrap();
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert!(
        r.va.walker().attempts[1]
            .iter()
            .all(|&(slot, _)| u64::from(slot) != key)
    );
    assert_eq!(r.va.walker().attempts[1].len(), 5);
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.maps.borrow().len(), 69);
}
#[test]
fn a_reused_next_chunk_slot_never_reads_the_retired_root() {
    let mut r = rig(128);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    let mut target = r.va.remove(VasKey(64)).unwrap();
    target.key = 700;
    r.va.table.insert(VasKey(700), target);
    r.va.table
        .set_root(VasKey(700), 0x2000_0000, PdbAperture::Vidmem)
        .unwrap();
    assert_eq!(
        r.va.table.slot(VasKey(700)),
        Some(64),
        "exercise actual slot reuse"
    );
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert!(
        r.va.walker().attempts[1]
            .iter()
            .all(|&(slot, _)| slot != 64)
    );
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.maps.borrow().len(), 127);
}
#[test]
fn a_moved_first_root_of_the_next_chunk_is_refreshed_before_submit() {
    moved_before_next(64);
}
#[test]
fn a_moved_last_root_of_the_next_chunk_is_refreshed_before_submit() {
    moved_before_next(69);
}
fn moved_before_next(key: u64) {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    r.va.table
        .set_root(VasKey(key), 0x2000_0000, PdbAperture::Vidmem)
        .unwrap();
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert!(r.va.walker().attempts[1].contains(&(key as u32, 0x2000_0000)));
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
    assert!(!r.va.in_flight(), "no stale-root walk should need a retry");
    assert_eq!(r.maps.borrow().len(), 70);
}
#[test]
fn retiring_every_remaining_object_finishes_without_an_empty_submit() {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    for k in 64..70 {
        r.va.remove(VasKey(k)).unwrap();
    }
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(req.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.va.walker().attempts.len(), 1);
    assert_eq!(r.port.trigger().read(), 0);
}
// F4. FALSIFIER: pending independent work needs an unrelated future event after
// the continuation submission failed and no walk remains in flight.
#[test]
fn a_failed_continuation_immediately_pumps_a_queued_root_walk() {
    let mut r = rig(70);
    r.va.walker_mut().refuse_attempt = Some(2);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    r.va.schedule_walk(VasKey(0), r.port.trigger());
    r.va.on_walk_ready(r.port.trigger());
    assert!(
        r.va.in_flight(),
        "the queued root needs a completion event source now"
    );
    assert_eq!(r.va.pending(), 0);
    assert_eq!(r.va.walker().attempts.len(), 3);
    assert_eq!(r.va.walker().attempts[2].len(), 1);
    r.va.on_walk_ready(r.port.trigger());
    assert!(!r.va.in_flight());
    assert_eq!(
        r.va.stats.unreconciled, 1,
        "the failed original is still named"
    );
    assert_ne!(r.port.trigger().read(), 0);
}
#[test]
fn a_failed_continuation_immediately_pumps_a_new_invalidate() {
    let mut r = rig(70);
    r.va.walker_mut().refuse_attempt = Some(2);
    let old = arm(&r);
    r.va.on_invalidate(old, r.port.trigger());
    let new = arm(&r);
    r.va.on_invalidate(new, r.port.trigger());
    r.va.on_walk_ready(r.port.trigger());
    assert!(r.va.in_flight(), "the new invalidate must not be stranded");
    assert_eq!(r.va.pending(), 0);
    assert_eq!(r.va.walker().attempts.len(), 3);
    assert!(r.va.on_walk_ready(r.port.trigger()).completed.is_empty());
    assert_ne!(r.port.trigger().read(), 0);
    assert_eq!(
        r.va.on_walk_ready(r.port.trigger()).completed,
        vec![(new.seq, ClearOutcome::Cleared)]
    );
    assert_eq!(r.va.stats.unreconciled, 1);
}
#[test]
fn a_withdrawn_completed_root_is_not_cleared_by_a_vacuous_retry() {
    let mut r = rig(70);
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    r.va.on_walk_ready(r.port.trigger());
    r.va.table.clear_root(VasKey(0));
    let out = r.va.on_walk_ready(r.port.trigger());
    assert!(out.completed.is_empty());
    assert_eq!(out.unreconciled, vec![req.seq]);
    assert_ne!(r.port.trigger().read(), 0);
}
#[test]
fn a_root_move_does_not_hide_an_unrelated_hard_failure() {
    let mut r = rig(70);
    r.fail_invalidate.set(Some(69));
    let req = arm(&r);
    r.va.on_invalidate(req, r.port.trigger());
    r.va.on_walk_ready(r.port.trigger());
    r.va.table
        .set_root(VasKey(0), 0x2000_0000, PdbAperture::Vidmem)
        .unwrap();
    let out = r.va.on_walk_ready(r.port.trigger());
    assert!(out.completed.is_empty());
    assert_eq!(out.unreconciled, vec![req.seq]);
    assert_ne!(r.port.trigger().read(), 0);
}
