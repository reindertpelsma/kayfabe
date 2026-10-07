//! ★★★★★ §16.76 — **the os-event wakeup plane**, driven against a guest that links its own
//! queue and reads its own ring.
//!
//! # What these tests are FOR, and what no other test in this repository can do
//!
//! The delivery plane's two halves — *post a batch* and *gate on the previous one* — are
//! covered by the oracle **not at all**, and that is measured rather than assumed: `cap1`
//! contains **zero** `IRQSCLR` writes across 359 062 records and no CUDA process runs in it,
//! so neither the raise nor the opener is reachable in the capture
//! (`crates/kayfabe-crec/tests/cap1_differential.rs` F-1, F-3). A green `cap1` diff is not
//! evidence here. These tests and a live boot are the whole of the coverage.
//!
//! # ★★★★★ THE ONE THAT MATTERS: the gate is NOT `swgen0_pending`
//!
//! This rung was briefed as *"we hold the same flag with the opposite polarity — ours
//! re-raises where the C gates"*, with the instruction to port the C's gate onto it. **That
//! is refuted**, and [`the_gate_is_not_the_irqstat_shadow`] is the refutation as an
//! executable property:
//!
//! - the C's `gsp_swgen0_pending` is written in **one** place (`C:1830`,
//!   `nvkvm_gsp_raise_swgen0`), reachable only from `nvkvm_gsp_deliver_events` — it means
//!   *"an EVENT BATCH is outstanding"*;
//! - our `swgen0_pending` is set by `GspFsm::post`, i.e. by **every RPC reply** — it means
//!   *"the status queue has something in it"*.
//!
//! Same name, different scope. Gating on ours would have refused the first batch (a reply
//! always precedes it) and reopened only on an `IRQSCLR` that, with no interrupt raised,
//! never comes. A permanent wedge, shipped as a fix, with every mechanical check green.

use kayfabe_abi::generated::rpc::{NV_VGPU_MSG_EVENT_POST_EVENT, RpcPostEventV1700};
use kayfabe_abi::postevent::PostEvent;
use kayfabe_gsp::{EventDelivery, GspFault};
use kayfabe_tests::gspworld::{GspWorld, MODEL_A, P580};

/// `NV_VGPU_MSG_FUNCTION_SET_GUEST_SYSTEM_INFO` — the guest's first post-init RPC, used
/// here only to put a **reply** in the status queue before an event batch, which is the
/// precondition the polarity refutation turns on.
const FN_SET_GUEST_SYSTEM_INFO: u32 = 1;

fn three_events() -> Vec<PostEvent> {
    // The handles boot `w209_ffc80f8_ctl` actually carried: libcuda's own client, and the
    // event objects it registered under it.
    [0x5c00_0079u32, 0x5c00_007a, 0x5c00_007b]
        .into_iter()
        .map(|event| PostEvent {
            client: 0xc1d0_000c,
            event,
            notify_index: 35,
        })
        .collect()
}

/// Boot to `Running` with the guest having drained `GSP_INIT_DONE`, then leave **one
/// ordinary RPC reply** in flight so `swgen0_pending` is set.
fn running_world() -> GspWorld {
    let mut w = GspWorld::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_SET_GUEST_SYSTEM_INFO, 5, &[])
        .expect("the guest issues its first post-init RPC");
    w.doorbell().expect("we service it");
    w.guest.recv(&mut w.ram).expect("and it drains the reply");
    w
}

/// ★★★★★ **The refutation, executable.** A batch is delivered even though an ordinary RPC
/// reply has just set `swgen0_pending` — because the gate is a *different flag*.
///
/// ⊘ If someone later "simplifies" the two flags into one, this test goes red and the
/// message says why. That is the entire reason it exists: the two-flags design looks like
/// redundancy from every angle except this one.
#[test]
fn the_gate_is_not_the_irqstat_shadow() {
    let mut w = running_world();
    assert!(
        w.fsm.observe().swgen0_pending,
        "★ the precondition: an ordinary RPC reply has set the IRQSTAT shadow. If this ever \
         becomes false, this test has gone blind rather than green"
    );
    let events = three_events();
    let outcome = w.fsm.deliver_events(&mut w.ram, &events);
    assert_eq!(
        outcome,
        EventDelivery::Delivered {
            posted: 3,
            short: None
        },
        "★★★★★ the batch MUST be delivered here. A gate keyed on `swgen0_pending` would \
         return `Gated` — and would keep returning it forever, because the only opener is \
         an IRQSCLR the guest writes in response to an interrupt that was never raised"
    );
}

/// ★★★ **The gate itself**: post-when-drained, post-all, raise-once — and the `IRQSCLR` is
/// the only thing that reopens it (`C:1849-1873`, `C:4441`).
#[test]
fn the_gate_bounds_the_ring_to_one_batch_and_only_irqsclr_opens_it() {
    let mut w = running_world();
    let events = three_events();

    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &events),
        EventDelivery::Delivered {
            posted: 3,
            short: None
        },
        "batch one: all three posted"
    );
    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &events),
        EventDelivery::Gated,
        "★ batch two is REFUSED — this is what bounds the shared seqNum ring, and the C's \
         own comment calls it CRITICAL"
    );

    // ⊘ DRAINING IS NOT THE OPENER, and this is the assertion that pins it. The guest
    // reading the ring empties it, but the C's flag is cleared by the ISR's IRQSCLR write
    // and by nothing else — so a port that opened on "the read pointer moved" would be a
    // different protocol wearing the same name.
    let drained = w.guest.recv(&mut w.ram).expect("the guest reads its queue");
    assert_eq!(drained.len(), 3, "all three events arrived");
    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &events),
        EventDelivery::Gated,
        "⊘ still gated: emptying the ring is not the opener"
    );

    // ★ THE OPENER. `cap1` contains zero of these, so this line is the only place in the
    // repository where it is exercised at all before a live boot.
    let clear = w.arch.model().irq_clear();
    let report = w
        .wr(kayfabe_arch::gsp::GspReg::GspFalconIrqsclr, clear)
        .expect("the guest's ISR clears the edge");
    assert!(
        report.transitions.contains(&kayfabe_gsp::Transition::E10),
        "the write is classified as E10"
    );
    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &events),
        EventDelivery::Delivered {
            posted: 3,
            short: None
        },
        "★★★ and the gate reopened — delivery is live again"
    );
}

/// ★★ The four *"nothing was delivered"* answers are four different findings, and the type
/// keeps them apart. A single `Option<usize>` would make all of them one silence.
#[test]
fn nothing_delivered_is_four_different_answers() {
    // (a) nobody registered — before anything else, because it must not depend on the boot.
    let mut cold = GspWorld::new(P580, MODEL_A);
    assert_eq!(
        cold.fsm.deliver_events(&mut cold.ram, &[]),
        EventDelivery::NoneRegistered,
    );
    // (b) the guest has not drained `GSP_INIT_DONE`, so `POST_EVENT` — absent from both
    // driver tags' bootup allowlists — must not be posted.
    let mut booting = GspWorld::new(P580, MODEL_A);
    booting.boot();
    assert_eq!(
        booting
            .fsm
            .deliver_events(&mut booting.ram, &three_events()),
        EventDelivery::NotRunning,
    );
    // (c) gated, and (d) delivered, are the previous test's.
}

/// ★★★ The **body** the guest receives resolves to the pair it registered — which is the
/// whole of the wakeup: `_kgspRpcPostEvent` matches `(hClient, hEvent)` with
/// `CliGetEventInfo` and only then calls `osNotifyEvent`.
#[test]
fn the_guest_receives_a_post_event_naming_the_pair_it_registered() {
    let mut w = running_world();
    let events = three_events();
    assert!(matches!(
        w.fsm.deliver_events(&mut w.ram, &events),
        EventDelivery::Delivered { posted: 3, .. }
    ));
    let got = w.guest.recv(&mut w.ram).expect("the guest reads its queue");
    assert_eq!(got.len(), 3);
    for (msg, want) in got.iter().zip(events.iter()) {
        assert_eq!(
            msg.function, NV_VGPU_MSG_EVENT_POST_EVENT,
            "★★ the guest's own receive path classifies it as POST_EVENT"
        );
        let body = RpcPostEventV1700::decode(&msg.payload).expect("a whole event body");
        assert_eq!(body.h_client, want.client, "half the MATCH key");
        assert_eq!(body.h_event, want.event, "the other half");
        assert_eq!(
            body.notify_index, want.notify_index,
            "echoed, not rewritten"
        );
        assert_eq!(
            body.b_notify_list, 0,
            "⊘ NON-LIST: the branch that resolves one (hClient, hEvent) and calls \
             osNotifyEvent — not the one that walks notifier chains copying eventData"
        );
        assert_eq!(
            body.event_data_size, 0,
            "⊘ EMPTY: nothing was read off any silicon, and a fabricated payload would put \
             invented bytes into a guest notifier"
        );
        assert_eq!(body.status, 0, "NV_OK — the EVENT's status, not the RPC's");
    }
}

/// ★★★ A batch cut short by a full ring still **announces what landed**, and says it was
/// short. Stranding posted elements behind a gate only an `IRQSCLR` opens — and no
/// `IRQSCLR` arrives without an interrupt — would be a wedge; one spurious re-check is the
/// cheaper failure. ⊘ The C does not distinguish these at all (`nvkvm_m3_post_status`
/// returns `void`), so this is the port being more careful than its oracle in the one
/// direction the oracle cannot advise on.
#[test]
fn a_short_batch_still_announces_what_landed() {
    let mut w = running_world();
    // The small ring is 7 slots; ask for more events than can fit so the ring refuses
    // part-way rather than at the first message.
    let many: Vec<PostEvent> = (0..32u32)
        .map(|i| PostEvent {
            client: 0xc1d0_000c,
            event: 0x5c00_0000 + i,
            notify_index: 35,
        })
        .collect();
    let outcome = w.fsm.deliver_events(&mut w.ram, &many);
    let EventDelivery::Delivered { posted, short } = outcome else {
        panic!(
            "★ some of the batch must land, or this test is asserting the wrong wall: {outcome:?}"
        );
    };
    assert!(posted > 0 && posted < many.len(), "cut short at {posted}");
    assert!(
        matches!(short, Some(GspFault::QueueFull { .. })),
        "and it says WHY it was short, by name: {short:?}"
    );
    // ★ The gate closed anyway — the elements that landed are real messages the guest must
    // be told to drain.
    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &many),
        EventDelivery::Gated,
        "a short batch is still an outstanding batch"
    );
    let got = w.guest.recv(&mut w.ram).expect("the guest reads its queue");
    assert_eq!(got.len(), posted, "exactly what landed is what arrives");
}

/// ⊘ A device life that ended must not leave the next one's gate closed: the queue the
/// batch was posted into is gone.
#[test]
fn a_reset_reopens_the_gate() {
    let mut w = running_world();
    assert!(matches!(
        w.fsm.deliver_events(&mut w.ram, &three_events()),
        EventDelivery::Delivered { .. }
    ));
    w.fsm.device_reset();
    // ⊘ Not `Gated` — and not `Delivered` either, because a cold device has no queue. The
    // point is that the answer is now about the BOOT rather than about a stale batch.
    assert_eq!(
        w.fsm.deliver_events(&mut w.ram, &three_events()),
        EventDelivery::NotRunning,
        "the gate did not survive the reset"
    );
}
