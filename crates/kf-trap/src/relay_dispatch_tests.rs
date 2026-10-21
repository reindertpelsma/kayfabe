use super::*;

#[test]
fn inline_budget_wrap_and_hostile_cursor_bounds() {
    assert_eq!(inline_delta(30, 2, 32, 4), Some(4));
    assert_eq!(inline_delta(30, 3, 32, 32), None);
    assert_eq!(inline_delta(30, 2, 32, 3), None);
    assert_eq!(inline_delta(0, 32, 32, 32), None);
    assert_eq!(inline_delta(32, 0, 32, 32), None);
    assert_eq!(inline_delta(0, 0, 0, 32), None);
    assert_eq!(inline_delta(0, 1, 31, 32), None);
    assert_eq!(inline_delta(31, 31, 32, 0), Some(0));
}

#[test]
fn full_token_and_generation_exhaustion_are_not_masked_away() {
    let row = DispatchWord::new();
    row.install(Mode::Direct, u32::MAX).unwrap();
    let Begin::Inline(t) = row.begin_inline() else {
        panic!("claim")
    };
    assert_eq!(t.host_token(), u32::MAX);
    assert_eq!(t.generation(), 1);
    t.finish();
    let exhausted = DispatchWord::new();
    exhausted
        .word
        .store(MAX_GENERATION << GENERATION_SHIFT, Ordering::Relaxed);
    assert_eq!(
        exhausted.install(Mode::CopyRing, 1),
        Err(InstallError::GenerationExhausted)
    );
}

#[test]
fn a_fresh_switch_and_a_published_switch_are_distinct() {
    let row = DispatchWord::new();
    let g = row.install(Mode::CopyRing, 77).unwrap();
    assert_eq!(row.request_worker(g), Change::Done);
    assert_eq!(row.finish_fresh_handoff(g), Change::Done);
    assert!(matches!(row.begin_inline(), Begin::WakeWorker));
    row.begin_worker(g).unwrap().finish();

    let row = DispatchWord::new();
    let g = row.install(Mode::CopyRing, 77).unwrap();
    let Begin::Inline(t) = row.begin_inline() else {
        panic!("claim")
    };
    t.mark_published();
    t.finish();
    assert_eq!(row.request_worker(g), Change::Done);
    assert_eq!(row.finish_fresh_handoff(g), Change::NativeHandoffPending);
    assert!(row.begin_worker(g).is_none());
    assert!(row.begin_direct_drain(g).is_none());
}

#[test]
fn linux_direct_dispatch_cannot_be_converted_or_claimed_by_worker() {
    let row = DispatchWord::new();
    let g = row.install(Mode::Direct, 77).unwrap();
    assert_eq!(row.request_worker(g), Change::Stale);
    assert!(row.begin_worker(g).is_none());
    row.begin_direct_drain(g).unwrap().finish();
    let Begin::Inline(t) = row.begin_inline() else {
        panic!("direct claim")
    };
    assert_eq!(t.mode(), Mode::Direct);
    t.finish();
}

#[test]
fn cleanup_failure_and_abandoned_claim_do_not_license_reuse() {
    let row = DispatchWord::new();
    let g = row.install(Mode::CopyRing, 77).unwrap();
    let Begin::Inline(t) = row.begin_inline() else {
        panic!("claim")
    };
    assert_eq!(row.request_retire(g), Change::Done);
    assert_eq!(row.finish_retire(g), Change::Busy);
    assert_eq!(row.install(Mode::CopyRing, 88), Err(InstallError::NotClean));
    drop(t); // No forged release from Drop/error/panic.
    assert_eq!(row.finish_retire(g), Change::Busy);

    let clean = DispatchWord::new();
    let g = clean.install(Mode::CopyRing, 77).unwrap();
    clean.request_retire(g);
    assert_eq!(clean.finish_retire(g), Change::Done);
    assert_eq!(
        clean.install(Mode::CopyRing, 88),
        Err(InstallError::NotClean)
    );
    assert_eq!(clean.confirm_native_cleanup(g), Change::Done);
    assert_eq!(clean.install(Mode::CopyRing, 88), Ok(g + 1));
    assert_eq!(clean.request_worker(g), Change::Stale);
    assert_eq!(clean.request_retire(g), Change::Stale);
}

#[test]
fn drained_worker_does_not_requeue_itself() {
    let row = DispatchWord::new();
    let g = row.install(Mode::Worker, 77).unwrap();
    assert!(!row.begin_worker(g).unwrap().finish());
    assert!(matches!(row.begin_inline(), Begin::WakeWorker));
    assert!(!row.begin_worker(g).unwrap().finish());
    let owner = row.begin_worker(g).unwrap();
    assert!(matches!(row.begin_inline(), Begin::WakeWorker));
    assert!(owner.finish());
    assert!(!row.begin_worker(g).unwrap().finish());
    let owner = row.begin_worker(g).unwrap();
    assert_eq!(row.request_retire(g), Change::Done);
    assert!(owner.finish());
}

// No preemption/permutation/time cap. Each program is deliberately finite;
// Loom explores its supported C11 executions, not an unbounded product proof.
fn check(program: impl Fn() + Sync + Send + 'static) {
    let mut b = loom::model::Builder::new();
    b.max_threads = 4;
    b.max_branches = 1_000;
    b.max_permutations = None;
    b.max_duration = None;
    b.preemption_bound = None;
    b.check(program);
}

#[test]
fn loom_cutover_cannot_admit_worker_over_an_inline_publication() {
    check(|| {
        use loom::sync::{Arc, atomic::AtomicU32};
        let row = Arc::new(modeled::DispatchWord::new());
        let put = Arc::new(AtomicU32::new(0));
        let g = row.install(Mode::CopyRing, 0x8000_ffff).unwrap();
        let r = row.clone();
        let p = put.clone();
        let inline = loom::thread::spawn(move || {
            if let modeled::Begin::Inline(t) = r.begin_inline() {
                assert_eq!(t.host_token(), 0x8000_ffff);
                assert_eq!(t.generation(), g);
                if t.may_publish_raw() {
                    t.mark_published(); // BEFORE real PUT in the callback.
                    p.store(1, Ordering::Relaxed);
                }
                t.finish();
            }
        });
        let r = row.clone();
        let p = put.clone();
        let switch = loom::thread::spawn(move || {
            if r.request_worker(g) == Change::Done {
                if r.finish_fresh_handoff(g) == Change::Done {
                    assert_eq!(p.load(Ordering::Relaxed), 0);
                    if let Some(t) = r.begin_worker(g) {
                        assert_eq!(t.mode(), Mode::Worker);
                        assert_eq!(p.load(Ordering::Relaxed), 0);
                        t.finish();
                    }
                }
            }
        });
        inline.join().unwrap();
        switch.join().unwrap();
        assert_eq!(row.request_worker(g), Change::Done);
        if put.load(Ordering::Relaxed) == 1 {
            assert_eq!(row.finish_fresh_handoff(g), Change::NativeHandoffPending);
            assert!(row.begin_worker(g).is_none());
        } else {
            assert_eq!(row.finish_fresh_handoff(g), Change::Done);
            row.begin_worker(g).unwrap().finish();
        }
    });
}

#[test]
fn loom_two_publishers_never_own_one_row_together() {
    check(|| {
        use loom::sync::{Arc, atomic::AtomicU32};
        let row = Arc::new(modeled::DispatchWord::new());
        let owners = Arc::new(AtomicU32::new(0));
        let g = row.install(Mode::CopyRing, 77).unwrap();
        let mut jobs = Vec::new();
        for _ in 0..2 {
            let r = row.clone();
            let o = owners.clone();
            jobs.push(loom::thread::spawn(move || {
                if let modeled::Begin::Inline(t) = r.begin_inline() {
                    assert_eq!(o.fetch_add(1, Ordering::Relaxed), 0);
                    t.mark_published();
                    assert_eq!(o.fetch_sub(1, Ordering::Relaxed), 1);
                    t.finish();
                }
            }));
        }
        for j in jobs {
            j.join().unwrap();
        }
        assert_eq!(owners.load(Ordering::Relaxed), 0);
        // Any contention fallback remains serviceable through the same guard.
        row.begin_worker(g).unwrap().finish();
        assert!(!row.pending());
    });
}

#[test]
fn loom_free_and_reuse_cannot_change_an_active_ticket_metadata() {
    check(|| {
        use loom::sync::{Arc, atomic::AtomicU32};
        let row = Arc::new(modeled::DispatchWord::new());
        let metadata = Arc::new(AtomicU32::new(1));
        let g = row.install(Mode::CopyRing, 77).unwrap();
        let r = row.clone();
        let m = metadata.clone();
        let inline = loom::thread::spawn(move || {
            if let modeled::Begin::Inline(t) = r.begin_inline() {
                assert_eq!(m.load(Ordering::Relaxed), t.generation());
                if t.may_publish_raw() {
                    t.mark_published();
                }
                assert_eq!(m.load(Ordering::Relaxed), t.generation());
                t.finish();
            }
        });
        let r = row.clone();
        let m = metadata.clone();
        let free = loom::thread::spawn(move || {
            if r.request_retire(g) == Change::Done && r.finish_retire(g) == Change::Done {
                assert_eq!(r.install(Mode::CopyRing, 88), Err(InstallError::NotClean));
                // Models successful native cleanup, not an elapsed timeout.
                match r.confirm_native_cleanup(g) {
                    Change::Done => {}
                    // A delayed doorbell can publish PENDING on a dead row.
                    // Retain the retirement/resources and retry off-vCPU.
                    Change::Retry => return,
                    other => panic!("unexpected cleanup {other:?}"),
                }
                m.store(g + 1, Ordering::Relaxed);
                // A stale unprivileged ring can change only PENDING; no new
                // allocation until install succeeds. One CAS may need retry.
                match r.install(Mode::CopyRing, 88) {
                    Ok(new) => assert_eq!(new, g + 1),
                    Err(InstallError::Retry) => {}
                    other => panic!("unexpected allocation {other:?}"),
                }
            }
        });
        inline.join().unwrap();
        free.join().unwrap();
    });
}

#[test]
fn loom_pending_doorbell_is_preserved_or_consumed_by_a_later_claim() {
    check(|| {
        use loom::sync::Arc;
        let row = Arc::new(modeled::DispatchWord::new());
        let g = row.install(Mode::CopyRing, 77).unwrap();
        let modeled::Begin::Inline(owner) = row.begin_inline() else {
            panic!("claim")
        };
        let r = row.clone();
        let ring = loom::thread::spawn(move || match r.begin_inline() {
            modeled::Begin::Inline(t) => {
                t.mark_published();
                t.finish();
            }
            modeled::Begin::WakeWorker => {}
            modeled::Begin::Ignore => panic!("live ring lost"),
        });
        let wake = owner.finish();
        ring.join().unwrap();
        // Either the owner saw pending and returned a wake, pending persists for
        // worker service, or the later inline claim consumed the ring itself.
        let published = row.word.load(Ordering::Acquire) & PUBLISHED != 0;
        assert!(wake || row.pending() || published);
        row.begin_worker(g).unwrap().finish();
        assert!(!row.pending());
    });
}

#[test]
fn loom_direct_last_ring_has_a_drain_without_inspection_conversion() {
    check(|| {
        use loom::sync::Arc;
        let row = Arc::new(modeled::DispatchWord::new());
        let g = row.install(Mode::Direct, u32::MAX).unwrap();
        let modeled::Begin::Inline(owner) = row.begin_inline() else {
            panic!("claim")
        };
        let r = row.clone();
        let ring = loom::thread::spawn(move || match r.begin_inline() {
            modeled::Begin::Inline(t) => {
                assert_eq!(t.mode(), Mode::Direct);
                t.mark_published();
                t.finish();
            }
            modeled::Begin::WakeWorker => {}
            modeled::Begin::Ignore => panic!("live direct ring lost"),
        });
        let wake = owner.finish();
        ring.join().unwrap();
        let published = row.word.load(Ordering::Acquire) & PUBLISHED != 0;
        assert!(wake || row.pending() || published);
        assert_eq!(row.request_worker(g), Change::Stale);
        assert!(row.begin_worker(g).is_none());
        let drain = row
            .begin_direct_drain(g)
            .expect("final direct ring is serviceable");
        assert_eq!(drain.mode(), Mode::Direct);
        assert_eq!(drain.host_token(), u32::MAX);
        drain.mark_published();
        drain.finish();
        assert!(!row.pending());
    });
}

#[test]
fn loom_delayed_callback_claims_replacement_metadata_only_after_admission() {
    check(|| {
        use loom::sync::{Arc, atomic::AtomicU32, mpsc};
        let row = Arc::new(modeled::DispatchWord::new());
        let metadata = Arc::new(AtomicU32::new(1));
        let g = row.install(Mode::CopyRing, 77).unwrap();
        let (observed_tx, observed_rx) = mpsc::channel();
        let (installed_tx, installed_rx) = mpsc::channel();
        let r = row.clone();
        let m = metadata.clone();
        let delayed = loom::thread::spawn(move || {
            let result = r.begin_inline_after_observe(|| {
                observed_tx.send(()).unwrap();
                installed_rx.recv().unwrap();
            });
            let modeled::Begin::Inline(t) = result else {
                panic!("replacement claim")
            };
            assert_eq!(t.generation(), g + 1);
            assert_eq!(t.host_token(), 88);
            assert_eq!(m.load(Ordering::Relaxed), t.generation());
            t.finish();
        });
        observed_rx.recv().unwrap();
        assert_eq!(row.request_retire(g), Change::Done);
        assert_eq!(row.finish_retire(g), Change::Done);
        assert_eq!(row.confirm_native_cleanup(g), Change::Done);
        metadata.store(g + 1, Ordering::Relaxed);
        assert_eq!(row.install(Mode::CopyRing, 88), Ok(g + 1));
        installed_tx.send(()).unwrap();
        delayed.join().unwrap();
    });
}

#[test]
fn loom_later_put_is_observed_or_leaves_durable_work() {
    check(|| {
        use loom::sync::{Arc, atomic::AtomicU32};
        let row = Arc::new(modeled::DispatchWord::new());
        let put = Arc::new(AtomicU32::new(1));
        let handled = Arc::new(AtomicU32::new(0));
        let g = row.install(Mode::CopyRing, 77).unwrap();
        let modeled::Begin::Inline(owner) = row.begin_inline() else {
            panic!("claim")
        };
        let r = row.clone();
        let p = put.clone();
        let h = handled.clone();
        let ring = loom::thread::spawn(move || {
            p.store(2, Ordering::Release);
            match r.begin_inline() {
                modeled::Begin::Inline(t) => {
                    h.store(p.load(Ordering::Acquire), Ordering::Relaxed);
                    t.finish();
                }
                modeled::Begin::WakeWorker => {}
                modeled::Begin::Ignore => panic!("live ring lost"),
            }
        });
        handled.store(put.load(Ordering::Acquire), Ordering::Relaxed);
        let wake = owner.finish();
        ring.join().unwrap();
        assert!(handled.load(Ordering::Relaxed) == 2 || row.pending() || wake);
        if row.pending() || wake {
            let t = row.begin_worker(g).unwrap();
            handled.store(put.load(Ordering::Acquire), Ordering::Relaxed);
            assert!(!t.finish());
        }
        assert_eq!(handled.load(Ordering::Relaxed), 2);
        assert!(!row.pending());
    });
}

// ---- PHASE 2 protocol, model level (brief appendix C): the bounded inline copy of GP entries

fn ring_of(values: &[u64], entries: u32) -> Vec<u64> {
    let mut r = vec![0u64; entries as usize];
    for (i, v) in values.iter().enumerate() {
        r[i] = *v;
    }
    r
}

#[test]
fn inline_plan_copies_exactly_the_delta_for_every_delta_up_to_the_budget_and_loads_each_slot_once()
{
    let k = MAX_INLINE_GP;
    for delta in 0..=k {
        let ring: Vec<u64> = (0..32u64).map(|i| 0x100 + i).collect();
        let mut loads = Vec::new();
        let plan = inline_plan(5, 5 + delta, 32, 32, |s| {
            loads.push(s);
            ring[s as usize]
        })
        .unwrap();
        assert_eq!(
            (plan.copy, plan.stop),
            (delta, InlineStop::None),
            "delta {delta}"
        );
        assert_eq!(
            loads,
            (5..5 + delta).collect::<Vec<_>>(),
            "each slot once, in order"
        );
        for i in 0..delta as usize {
            assert_eq!(
                plan.entries[i],
                ring[5 + i],
                "the stored value is the loaded value"
            );
        }
    }
}

#[test]
fn inline_plan_hands_everything_above_the_budget_to_the_worker_and_loads_nothing() {
    let k = MAX_INLINE_GP;
    for delta in [k + 1, k + 2, 16, 31] {
        let mut loaded = 0;
        let plan = inline_plan(0, delta, 32, 32, |_| {
            loaded += 1;
            0
        })
        .unwrap();
        assert_eq!(
            (plan.copy, plan.stop),
            (0, InlineStop::Budget),
            "delta {delta}"
        );
        assert_eq!(
            loaded, 0,
            "work per call is bounded by K, not by the guest's delta"
        );
    }
    // the whole ring minus one slot
    assert_eq!(
        inline_plan(7, 6, 32, 32, |_| 0).unwrap().stop,
        InlineStop::Budget
    );
}

#[test]
fn inline_plan_wraps_and_counts_credits() {
    let ring = ring_of(&[], 32);
    let plan = inline_plan(30, 2, 32, 4, |s| 0x200 + u64::from(s)).unwrap();
    assert_eq!((plan.copy, plan.stop), (4, InlineStop::None));
    assert_eq!(
        &plan.entries[..4],
        &[0x200 + 30, 0x200 + 31, 0x200, 0x200 + 1]
    );
    let mut loaded = 0;
    let plan = inline_plan(30, 2, 32, 3, |_| {
        loaded += 1;
        0
    })
    .unwrap();
    assert_eq!((plan.copy, plan.stop, loaded), (0, InlineStop::Credits, 0));
    let _ = ring;
}

#[test]
fn inline_plan_refuses_a_hostile_cursor_by_name_and_loads_nothing() {
    let mut loaded = 0;
    let mut f = |_| {
        loaded += 1;
        0u64
    };
    assert_eq!(
        inline_plan(0, 32, 32, 32, &mut f),
        Err(InlineRefusal::PutBound)
    );
    assert_eq!(
        inline_plan(0, u32::MAX, 32, 32, &mut f),
        Err(InlineRefusal::PutBound)
    );
    assert_eq!(
        inline_plan(32, 0, 32, 32, &mut f),
        Err(InlineRefusal::GetBound)
    );
    assert_eq!(
        inline_plan(0, 1, 31, 32, &mut f),
        Err(InlineRefusal::Geometry)
    );
    assert_eq!(
        inline_plan(0, 0, 0, 32, &mut f),
        Err(InlineRefusal::Geometry)
    );
    assert_eq!(loaded, 0);
    // GP_PUT "moving backwards" is indistinguishable from a huge delta by one load: the worker takes it
    assert_eq!(
        inline_plan(10, 9, 32, 32, |_| 0).unwrap().stop,
        InlineStop::Budget
    );
}

#[test]
fn inline_plan_stops_before_a_sync_wait_entry_wherever_it_is_in_the_delta() {
    let k = MAX_INLINE_GP;
    for delta in 1..=k {
        for at in 0..delta {
            let sync = 1u64 << 63;
            let mut ring: Vec<u64> = (0..32u64).map(|i| 0x300 + i).collect();
            ring[(3 + at) as usize] |= sync | (1 << 41); // SYNC_WAIT, also with a length (LENGTH > 0 form)
            let mut loads = Vec::new();
            let plan = inline_plan(3, 3 + delta, 32, 32, |s| {
                loads.push(s);
                ring[s as usize]
            })
            .unwrap();
            assert_eq!(
                plan.stop,
                InlineStop::SyncWait { at },
                "delta {delta} at {at}"
            );
            assert_eq!(plan.copy, at, "the copy stops BEFORE the SYNC_WAIT entry");
            for i in 0..at as usize {
                assert_eq!(plan.entries[i], ring[3 + i]);
            }
            assert_eq!(
                loads.len(),
                (at + 1) as usize,
                "the SYNC_WAIT entry is peeked, never forwarded"
            );
        }
    }
}

#[test]
fn the_inline_copy_takes_no_lock_makes_no_os_call_and_reads_no_pushbuffer() {
    // A static pin: the production source of the dispatch word and the inline planner names none of these.
    let text = include_str!("relay_dispatch.rs");
    let prod = text.split("#[cfg(test)]").next().unwrap();
    for banned in [
        "Mutex",
        ".lock(",
        "RwLock",
        "mmap",
        "ioctl",
        "syscall",
        "Vec<",
        "Box<",
        "thread::sleep",
        "read_pb",
    ] {
        assert!(
            !prod.contains(banned),
            "the inline path must not use {banned}"
        );
    }
    let imp = include_str!("relay_dispatch_impl.rs");
    for banned in [
        "Mutex", ".lock(", "mmap", "ioctl", "syscall", "Vec<", "Box<",
    ] {
        assert!(
            !imp.contains(banned),
            "the dispatch word must not use {banned}"
        );
    }
}

#[test]
fn loom_a_sync_wait_stop_hands_the_rest_to_the_worker_without_losing_or_repeating_an_entry() {
    check(|| {
        use loom::sync::{Arc, Mutex, atomic::AtomicU32};
        let row = Arc::new(modeled::DispatchWord::new());
        let g = row.install(Mode::CopyRing, 77).unwrap();
        // guest ring: entries 0, 1, 2 (SYNC_WAIT), 3; GP_PUT = 4; the private ring is the `out` log
        let ring: Arc<Vec<u64>> = Arc::new(vec![10, 11, 12 | (1 << 63), 13]);
        let cursor = Arc::new(AtomicU32::new(0));
        let out = Arc::new(Mutex::new(Vec::<u64>::new()));
        let wake = Arc::new(AtomicU32::new(0));
        let (r, ring2, cursor2, out2, wake2) = (
            row.clone(),
            ring.clone(),
            cursor.clone(),
            out.clone(),
            wake.clone(),
        );
        let vcpu = loom::thread::spawn(move || match r.begin_inline() {
            modeled::Begin::Inline(t) => {
                let get = cursor2.load(Ordering::Acquire);
                let plan = inline_plan(get, 4, 32, 32, |s| ring2[s as usize]).unwrap();
                if plan.copy > 0 {
                    t.mark_published();
                    let mut o = out2.lock().unwrap();
                    for i in 0..plan.copy as usize {
                        o.push(plan.entries[i]);
                    }
                    cursor2.store(get + plan.copy, Ordering::Release);
                }
                if plan.stop != InlineStop::None {
                    wake2.store(1, Ordering::Release); // PENDING + a non-blocking wake
                }
                if t.finish() {
                    wake2.store(1, Ordering::Release);
                }
            }
            modeled::Begin::WakeWorker => wake2.store(1, Ordering::Release),
            modeled::Begin::Ignore => panic!("live ring lost"),
        });
        // the worker may try while the vCPU is still inside its claim; whatever it misses is durable
        let mut drained = false;
        if let Some(t) = row.begin_worker(g) {
            let get = cursor.load(Ordering::Acquire);
            let mut o = out.lock().unwrap();
            for i in get..4 {
                o.push(ring[i as usize] & !(1 << 63)); // the SYNC_WAIT bit is honoured by the hardware, the value is the entry
            }
            cursor.store(4, Ordering::Release);
            drained = true;
            t.finish();
        }
        vcpu.join().unwrap();
        if !drained && (wake.load(Ordering::Acquire) == 1 || row.pending()) {
            let t = row
                .begin_worker(g)
                .expect("the worker drains after the claimant left");
            let get = cursor.load(Ordering::Acquire);
            let mut o = out.lock().unwrap();
            for i in get..4 {
                o.push(ring[i as usize] & !(1 << 63));
            }
            cursor.store(4, Ordering::Release);
            drop(o);
            t.finish();
        }
        assert_eq!(
            *out.lock().unwrap(),
            vec![10, 11, 12, 13],
            "every entry exactly once, in order"
        );
        assert_eq!(cursor.load(Ordering::Acquire), 4);
    });
}
