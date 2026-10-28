//! Actual ENS12 Inbox + VaManager handoff with a test-only walker/apply target.
//! No CUDA, RM device, guest RAM or GPU command executes in this harness.
use kf_chan::host::Gate;
use kf_cuda::WalkEntry;
use kf_mem::apply::DiffRun;
use kf_mem::ledger::{AP_VIDMEM, Desired, MapTarget, Mapped, Settle};
use kf_mem::vasmgr::{EntryDiff, VaManager, VasKey, WalkDone, Walker};
use kf_qemu::mem::{Inbox, SplitTarget};
use kf_trap::{Invalidate, InvalidatePort, InvalidateRegs, PdbAperture, PortWrite};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const KEY: VasKey = VasKey(0x100);
const PDB: u64 = 0x1000;

#[derive(Default)]
struct State {
    ready: Cell<bool>,
    live: Cell<bool>,
    events: RefCell<Vec<&'static str>>,
}
struct TestWalker {
    state: Rc<State>,
    queued: Option<Vec<WalkEntry>>,
}
impl Walker for TestWalker {
    fn submit(&mut self, entries: &[WalkEntry]) -> Result<(), String> {
        assert!(self.queued.is_none());
        self.queued = Some(entries.to_vec());
        self.state.events.borrow_mut().push("submit");
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<WalkDone>, String> {
        if !self.state.ready.get() {
            return Ok(None);
        }
        let Some(entries) = self.queued.take() else {
            return Ok(None);
        };
        self.state.events.borrow_mut().push("collected");
        Ok(Some(WalkDone {
            generation: 1,
            nrun: entries.len(),
            entries: entries
                .iter()
                .enumerate()
                .map(|(first, e)| EntryDiff {
                    pdb: e.pdb,
                    slot: e.slot,
                    first,
                    runs: vec![DiffRun {
                        unmap: false,
                        va: 0x2000_0000,
                        len: 0x1000,
                        at: 0x2000,
                        ap: AP_VIDMEM,
                        held: false,
                        kind: 0,
                        perm: kf_host::MapPerm::READ_WRITE,
                        privileged: false,
                        leaf: 0x1000,
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
    }
    fn ack(&mut self, _: u64, _: Vec<u8>) -> Result<(), String> {
        self.state.events.borrow_mut().push("ack");
        Ok(())
    }
    fn reset(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn slots(&self) -> u32 {
        8
    }
}
struct TestTarget(Rc<State>);
impl MapTarget for TestTarget {
    fn map(&self, _: &Desired, _: bool) -> Result<Mapped, String> {
        self.0.events.borrow_mut().push("map");
        Ok(Mapped::Placed)
    }
    fn unmap(&self, _: u64, _: bool) -> Result<(), String> {
        panic!("fixture emits only maps")
    }
    fn invalidate(&self) -> Result<(), String> {
        self.0.events.borrow_mut().push("host_invalidate");
        Ok(())
    }
    fn settle(&self) -> Settle {
        if self.0.live.get() {
            Settle::Live
        } else {
            Settle::Pending
        }
    }
}
fn rig() -> (
    Inbox,
    VaManager<TestWalker, TestTarget>,
    InvalidatePort,
    Rc<State>,
) {
    let state = Rc::new(State::default());
    let mut manager = VaManager::new(
        TestWalker {
            state: state.clone(),
            queued: None,
        },
        0x100_0000,
        Box::new(|gpa, _| Some(gpa)),
    );
    manager.table.insert(KEY, TestTarget(state.clone()));
    manager
        .table
        .set_root(KEY, PDB, PdbAperture::Vidmem)
        .unwrap();
    let port = InvalidatePort::new(InvalidateRegs::from_usermode_base(0x30000).unwrap());
    (Inbox::new().unwrap(), manager, port, state)
}
fn admit(inbox: &Inbox, manager: &mut VaManager<TestWalker, TestTarget>, port: &InvalidatePort) {
    for (ticket, target) in inbox.take_split_requests() {
        match target {
            SplitTarget::Pdb(pdb) => manager.queue_split(pdb, None, ticket, port.trigger()),
            SplitTarget::ResolvedPdb(pdb) => {
                manager.queue_split(Some(pdb), None, ticket, port.trigger());
            }
            SplitTarget::Space(key) => {
                manager.queue_split(None, Some(key), ticket, port.trigger());
            }
        }
    }
    manager.start_queued_splits(port.trigger());
}

fn split_after_apply(gated: bool) {
    let (inbox, mut manager, port, state) = rig();
    let gate = Gate::scratch().unwrap();
    let ticket = if gated {
        inbox.request_gated_split(7, KEY, gate.clone(), 11)
    } else {
        inbox.request_split(7, Some(PDB))
    };
    admit(&inbox, &mut manager, &port);
    for _ in 0..3 {
        manager.on_walk_ready(port.trigger());
        assert!(manager.take_splits().is_empty());
        assert!(inbox.split_result(ticket).is_none());
        assert_eq!(gate.value(), Some(0));
    }
    state.ready.set(true);
    state.live.set(true);
    manager.on_walk_ready(port.trigger());
    assert_eq!(
        &*state.events.borrow(),
        &["submit", "collected", "map", "host_invalidate", "ack"]
    );
    let done = manager.take_splits();
    assert_eq!(done, vec![(ticket, Ok(()))]);
    assert!(
        inbox.split_result(ticket).is_none(),
        "manager result is not yet delivered"
    );
    assert_eq!(gate.value(), Some(0));
    for (ticket, result) in done {
        assert_eq!(inbox.finish_split(ticket, result), Some(7));
    }
    assert_eq!(gate.value(), Some(if gated { 11 } else { 0 }));
    assert_eq!(inbox.split_result(ticket), Some(Ok(())));
    assert!(inbox.split_result(ticket).is_none());
    manager.on_walk_ready(port.trigger());
    assert!(manager.take_splits().is_empty());
}
#[test]
fn split_apply_order_uses_actual_manager_and_inbox() {
    split_after_apply(false);
}
#[test]
fn gated_apply_order_releases_scratch_only_after_apply() {
    split_after_apply(true);
}

#[test]
fn register_clear_waits_for_apply_and_target_settlement() {
    let (inbox, mut manager, port, state) = rig();
    let regs = port.regs();
    let (lo, hi) = Invalidate::encode_pdb(PDB, PdbAperture::Vidmem);
    port.write(regs.pdb, lo);
    port.write(regs.upper_pdb, hi);
    let PortWrite::Publish(request) = port.write(regs.trigger, 0x8000_0001) else {
        panic!("actual register decoder must publish");
    };
    inbox.set_walk_busy(true);
    inbox.wait_walk(7);
    manager.on_invalidate(request, port.trigger());
    manager.on_walk_ready(port.trigger());
    assert_ne!(port.trigger().read(), 0);
    assert_eq!(port.trigger().completed(), 0);
    state.ready.set(true);
    manager.on_walk_ready(port.trigger());
    assert!(state.events.borrow().contains(&"host_invalidate"));
    assert_ne!(
        port.trigger().read(),
        0,
        "target is still Pending after apply"
    );
    assert_eq!(port.trigger().completed(), 0);
    state.live.set(true);
    manager.on_targets(port.trigger());
    assert_eq!(port.trigger().read(), 0);
    assert_eq!(port.trigger().completed(), 1);
    inbox.set_walk_busy(false);
    assert_eq!(inbox.take_walk_waiters(), vec![7]);
    assert!(inbox.take_walk_waiters().is_empty());
    manager.on_targets(port.trigger());
    assert_eq!(port.trigger().completed(), 1);
}

#[test]
fn coalesced_target_keeps_two_channel_ticket_completions() {
    let (inbox, mut manager, port, state) = rig();
    let a = inbox.request_split(7, Some(PDB));
    let b = inbox.request_split(9, Some(PDB));
    admit(&inbox, &mut manager, &port);
    assert_eq!(state.events.borrow().as_slice(), &["submit"]);
    assert!(manager.take_splits().is_empty());
    state.ready.set(true);
    state.live.set(true);
    manager.on_walk_ready(port.trigger());
    let done = manager.take_splits();
    assert_eq!(done.len(), 2);
    let mut tokens: Vec<_> = done
        .into_iter()
        .map(|(t, r)| inbox.finish_split(t, r).expect("live channel"))
        .collect();
    tokens.sort();
    assert_eq!(tokens, vec![7, 9]);
    assert_eq!(inbox.split_result(a), Some(Ok(())));
    assert_eq!(inbox.split_result(b), Some(Ok(())));
    assert!(inbox.split_result(a).is_none() && inbox.split_result(b).is_none());
    assert!(manager.take_splits().is_empty());
}
