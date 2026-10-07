//! ★★★★★ **Exhaustion: every token the fast path cannot register still works — through the trap**
//! (`docs/design/V3_DOORBELL_IOEVENTFD.md` §5). Its own test binary (its own process), because one
//! arm deliberately runs this process out of descriptors.
//!
//! Three limits bind a registration, and each is driven to refusal against a real KVM guest:
//! 1. **the kernel's MMIO bus** (`ENOSPC`). ⊘ Ioeventfds do NOT count toward `NR_IOBUS_DEVS`
//!    (`virt/kvm/kvm_main.c`: *"exclude ioeventfd which is limited by maximum fd"*), so the bus is
//!    filled with 1 000 coalesced-MMIO zones — non-ioeventfd devices — until the kernel refuses; then
//!    every further doorbell registration is the kernel's `ENOSPC`.
//! 2. **the descriptor limit** (`EMFILE`) — what actually bounds ioeventfds on a modern kernel: one
//!    eventfd per token.
//! 3. **our own budget** (`doorbell-ioeventfd-max`).
//!
//! In each arm the guest then stores EVERY token: the registered ones never exit and are delivered
//! by the drainer; every refused one exits — in order, with its exact value — and the trap rings its
//! twin. Named (`kf3: DBFAST … REFUSED …`) and counted (`refused_*`).

mod common;

use common::{DevSink, Guest, Host, KvmVerb, caps, kick, plane, stores};
use kf_chan::dbfast::{DbFast, RegOutcome};
use kf_chip::hwref::DieGroup;
use kf_core::Owner;
use kf_trap::Route;
use kf_trap::tokenindex::{GuestTokenFormat, TokenIndex};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// `n` passthrough channels `(runlist 0, chid 1..=n)`: their token indices and guest values, with
/// the plane's token words installed (host twin `0x5000 + chid`).
fn channels(plane: &'static kf_core::Plane<'static>, n: u32) -> Vec<(u32, u32)> {
    let fmt = GuestTokenFormat::for_die_group(DieGroup::Ga10x).expect("format");
    let ix = TokenIndex::RunlistVector;
    let c = caps();
    (1..=n)
        .map(|chid| {
            let idx = ix.of_channel(0, chid).expect("slot");
            plane
                .allocate_channel(
                    &mut c.lock().unwrap(),
                    idx,
                    Route::Passthrough,
                    0x5000 + chid,
                    Owner::User,
                )
                .expect("allocate");
            (idx, fmt.value(0, chid).expect("value"))
        })
        .collect()
}

/// Run the guest; return the trapped values in order. Every exit goes through the device's trap arm.
fn run(g: &Guest, sink: &DevSink) -> Vec<u32> {
    let mut vcpu = g.vcpu();
    let mut exits = Vec::new();
    g.run(&mut vcpu, |v| {
        exits.push(v);
        sink.trap(v);
    });
    exits
}

#[test]
fn every_token_refused_by_the_kernel_the_fd_limit_or_the_budget_still_works_through_the_trap() {
    kf_linux_raw::require_kvm!(
        "every_token_refused_by_the_kernel_the_fd_limit_or_the_budget_still_works_through_the_trap"
    );
    const K: usize = 6; // registered before the limit
    const M: usize = 6; // refused by it

    // ---- 1. the kernel: fill the MMIO bus with non-ioeventfd devices ----------------------------
    {
        let plane = plane();
        let ch = channels(plane, (K + M) as u32);
        let values: Vec<u32> = ch.iter().map(|c| c.1).collect();
        let g = Guest::new(|db| stores(db, &values));
        let sink = DevSink::new(plane, Arc::new(Host::default()));
        let f = DbFast::new(1 << 20, kick()).expect("fast path");
        f.enable(Box::new(KvmVerb(Arc::clone(&g.vm))));
        f.site_add(g.doorbell);
        for (idx, v) in &ch[..K] {
            assert!(matches!(
                f.register(*idx, *v),
                RegOutcome::Registered { placed: 1, .. }
            ));
        }
        let mut zones = 0u64;
        loop {
            match g
                .vm
                .register_coalesced_mmio(0x1_0000_0000 + zones * 0x1000, 0x1000)
            {
                Ok(()) => zones += 1,
                Err(e) => {
                    assert_eq!(
                        e,
                        kf_linux_raw::RawError::Syscall {
                            call: "KVM_REGISTER_COALESCED_MMIO",
                            errno: Some(28)
                        },
                        "the bus fills with ENOSPC"
                    );
                    break;
                }
            }
            assert!(zones <= 4096, "the MMIO bus never filled");
        }
        eprintln!(
            "EXHAUST kernel: MMIO bus full after {zones} coalesced zones (+ {K} ioeventfds held)"
        );
        for (idx, v) in &ch[K..] {
            assert!(
                matches!(
                    f.register(*idx, *v),
                    RegOutcome::Registered {
                        placed: 0,
                        refused: 1,
                        sites: 1,
                        ..
                    }
                ),
                "the kernel refuses the placement; the token stays trapped"
            );
        }
        assert_eq!(f.counters.refused_enospc.load(Ordering::Relaxed), M as u64);
        assert_eq!(f.counters.double_register.load(Ordering::Relaxed), 0);
        let exits = run(&g, &sink);
        assert_eq!(
            exits,
            values[K..].to_vec(),
            "exactly the refused tokens exit, in order"
        );
        assert_eq!(
            f.service_ready(&sink),
            K,
            "the registered ones are delivered by the drainer"
        );
        assert_eq!(sink.fast_rings.lock().unwrap().len(), K);
        for v in &values[K..] {
            assert_eq!(
                sink.trapped.lock().unwrap().get(v),
                Some(&1),
                "{v:#x} worked via the trap"
            );
        }
    }

    // ---- 2. the descriptor limit: no eventfd can be created -------------------------------------
    {
        let plane = plane();
        let ch = channels(plane, (K + M) as u32);
        let values: Vec<u32> = ch.iter().map(|c| c.1).collect();
        let g = Guest::new(|db| stores(db, &values));
        let sink = DevSink::new(plane, Arc::new(Host::default()));
        let f = DbFast::new(1 << 20, kick()).expect("fast path");
        f.enable(Box::new(KvmVerb(Arc::clone(&g.vm))));
        f.site_add(g.doorbell);
        for (idx, v) in &ch[..K] {
            assert!(matches!(
                f.register(*idx, *v),
                RegOutcome::Registered { placed: 1, .. }
            ));
        }
        // Use up every descriptor this process may open.
        let mut hog = Vec::new();
        while let Ok(fh) = std::fs::File::open("/dev/null") {
            hog.push(fh);
            assert!(hog.len() < 1 << 22, "no descriptor limit?");
        }
        for (idx, v) in &ch[K..] {
            assert_eq!(
                f.register(*idx, *v),
                RegOutcome::Refused,
                "EMFILE: no eventfd"
            );
        }
        let held = hog.len();
        drop(hog);
        eprintln!("EXHAUST fds: refused {M} registrations after {held} extra descriptors");
        assert_eq!(f.counters.refused_fd.load(Ordering::Relaxed), M as u64);
        let exits = run(&g, &sink);
        assert_eq!(
            exits,
            values[K..].to_vec(),
            "exactly the refused tokens exit, in order"
        );
        assert_eq!(f.service_ready(&sink), K);
    }

    // ---- 3. our budget ----------------------------------------------------------------------------
    {
        let plane = plane();
        let ch = channels(plane, (K + M) as u32);
        let values: Vec<u32> = ch.iter().map(|c| c.1).collect();
        let g = Guest::new(|db| stores(db, &values));
        let sink = DevSink::new(plane, Arc::new(Host::default()));
        let f = DbFast::new(K, kick()).expect("fast path");
        f.enable(Box::new(KvmVerb(Arc::clone(&g.vm))));
        f.site_add(g.doorbell);
        for (i, (idx, v)) in ch.iter().enumerate() {
            let placed = u32::from(i < K);
            assert!(matches!(
                f.register(*idx, *v),
                RegOutcome::Registered { placed: p, refused: r, .. } if p == placed && r == 1 - placed
            ));
        }
        assert_eq!(f.counters.refused_budget.load(Ordering::Relaxed), M as u64);
        let exits = run(&g, &sink);
        assert_eq!(
            exits,
            values[K..].to_vec(),
            "exactly the over-budget tokens exit, in order"
        );
        assert_eq!(f.service_ready(&sink), K);
        eprintln!(
            "EXHAUST budget: {K} placed, {M} refused and trapped — {}",
            f.status()
        );
    }
}
