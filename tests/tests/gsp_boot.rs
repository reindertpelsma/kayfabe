//! ★★ The faked-GSP conformance suite (`mode2_gsp_port_plan.md` S0–S5).
//!
//! **Integration over unit** (`testing_doctrine.md` §3.1): the centre of this file is a
//! real guest driver booting against the real FSM through real guest memory — an
//! independent re-implementation of ogkm's own `msgq`/RPC receive path
//! (`kayfabe_tests::gspworld::Guest`) on the other side of the wire. The isolated cases at
//! the bottom localise; the composed boots discover.
//!
//! **Mean, not happy path.** The boots run over a *fragmented* region, a 7-slot ring
//! (small on purpose, so wrap and fullness are reached rather than described), a
//! multi-element command, a full ring, a hostile read pointer, a corrupted element, three
//! driver lifetimes in one process, and a re-acquire with live sequence numbers.
//!
//! **Non-vacuity is asserted, not assumed.** Every boot asserts the exact
//! `Transition`s that fired — the classic vacuous protocol test is a replay that passes
//! because the script never reached the transition it claims to cover — and every refusal
//! test carries the arm where the same call **succeeds**.

use kayfabe_abi::{DriverVersion, versions};
use kayfabe_arch::GspReg;
use kayfabe_gsp::{
    BootPhase, EchoOk, GspFault, GspFsm, MsgCount, MsgqAbi, Observation, OutgoingRpc, Projection,
    QueueState, RecordingRam, RegionMap, RxLinkCode, Transition, TransportHdr, TxHeader,
    available_elements, checksum32, free_elements, rx_link_check,
};
use kayfabe_tests::gspworld::{
    FakeRam, GspArch, GspWorld, Guest, GuestRefusal, MODEL_A, MODEL_B, NoGspArch, P580, P610, PAGE,
    REAL_QUEUE_SIZE, RingId, STAGING_BYTES, fold,
};

/// The composed world lives in the shared harness so the mean test can drive the same one
/// (`testing_doctrine.md` §3.1.3: wired into `l1_mean.rs`, not only into a fresh file).
type World = GspWorld;

const INIT_DONE: u32 = 0x1001;
const FN_SET_GUEST_SYSTEM_INFO: u32 = 1;
const FN_UNLOADING: u32 = 47;
const FN_GSP_SET_SYSTEM_INFO: u32 = 72;
const FN_SET_REGISTRY: u32 = 73;
const FN_RM_CONTROL: u32 = 76;
const POST_EVENT: u32 = 0x1003;

// ───────────────────────────────── the composed boots ─────────────────────────────────

/// ★ The whole boot, end to end, with the guest's own `msgqRxLink` as the judge.
///
/// This is the S1+S2+S3+S4 acceptance test: if the published header were wrong in any of
/// the nine ways `msgqRxLink` checks, if the checksum covered the wrong bytes, if the
/// sequence number were stamped in the wrong field, or if the read pointer went to the
/// unswapped location, the guest here would refuse exactly as the driver would.
#[test]
fn a_stock_guest_boots_links_the_status_queue_and_drains_init_done() {
    let mut w = World::new(P580, MODEL_A);

    // Before anything: WPR2 must read DOWN, or `_kgspBootGspRm` bails with "unexpected
    // WPR2 already up" (`ogkm-580: kernel_gsp.c:3873-3877`, `ogkm-610: :4805-4809` —
    // the same `kgspIsWpr2Up_HAL` early-fail, byte-identical, only relocated).
    assert_eq!(w.rd(GspReg::Wpr2AddrHi), 0, "cold device, WPR2 down");

    let transitions = w.boot();
    assert_eq!(
        transitions,
        vec![Transition::E1, Transition::E6, Transition::E5],
        "FWSEC STARTCPU, the boot-args publish, then the Booter Load — and the publish \
         fires on the SECOND mailbox half, not on a particular one",
    );
    assert_ne!(w.rd(GspReg::Wpr2AddrHi), 0, "WPR2 up after FWSEC");
    assert_eq!(w.fsm.phase(), BootPhase::Booted);

    // The guest's own acceptance predicate, and then its own receive path.
    let msgs = w.link_and_drain();
    assert_eq!(msgs.len(), 1, "exactly GSP_INIT_DONE is waiting");
    assert_eq!(msgs[0].function, INIT_DONE);
    assert_eq!(
        msgs[0].sequence, 0,
        "kgspWaitForRmInitDone polls (0x1001, 0)"
    );
    assert_eq!(msgs[0].seq_num, 0, "the first status message is seqNum 0");
    assert!(msgs[0].payload.is_empty());

    // B6's init RPCs: two commands, one doorbell, and NEITHER may be answered.
    w.guest
        .send(&mut w.ram, FN_GSP_SET_SYSTEM_INFO, 10, &[1, 2, 3, 4])
        .unwrap();
    w.guest
        .send(&mut w.ram, FN_SET_REGISTRY, 11, &[5; 64])
        .unwrap();
    let report = w.doorbell().unwrap();
    assert!(report.transitions.contains(&Transition::E7));
    assert_eq!(report.commands.len(), 2, "both init RPCs were decoded");
    assert_eq!(report.commands[0].code, FN_GSP_SET_SYSTEM_INFO);
    assert_eq!(report.commands[1].code, FN_SET_REGISTRY);
    assert_eq!(
        w.guest.recv(&mut w.ram).unwrap(),
        vec![],
        "72 and 73 are _issueRpcAsync — an echo would surface as an unexpected event",
    );

    // The guest drained INIT_DONE, so the FSM has observed it and is Running.
    assert!(
        report.transitions.contains(&Transition::Running),
        "Running is entered on the OBSERVED drain, not on the post",
    );
    assert_eq!(w.fsm.phase(), BootPhase::Running);

    // A synchronous RPC now gets its reply, matched on (function, sequence).
    w.guest
        .send(&mut w.ram, FN_SET_GUEST_SYSTEM_INFO, 12, &[0xAB; 16])
        .unwrap();
    w.doorbell().unwrap();
    let replies = w.guest.recv(&mut w.ram).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].function, FN_SET_GUEST_SYSTEM_INFO);
    assert_eq!(
        replies[0].sequence, 12,
        "the reply echoes the transaction id"
    );
    assert_eq!(replies[0].seq_num, 1, "the status stream's second message");
    assert_eq!(replies[0].rpc_result, 0);
}

/// ★ Constraint 1 — **the architecture seam, pushed on.**
///
/// The same FSM, the same guest, the same bytes on the wire — driven through a register
/// model with a different BAR, base, stride, STARTCPU bit, Unload sentinel, WPR2
/// encoding, interrupt bit and suspend sentinel. If a single offset or bit position had
/// leaked into `kayfabe-gsp`, one of these two runs would fail.
#[test]
fn the_same_boot_runs_unchanged_under_a_second_register_model() {
    let mut a = World::new(P580, MODEL_A);
    let mut b = World::new(P580, MODEL_B);
    assert_eq!(a.boot(), b.boot(), "the same transitions fire");

    let ma = a.link_and_drain();
    let mb = b.link_and_drain();
    assert_eq!(ma, mb, "byte-identical protocol under two register models");

    // The models disagree about everything they are allowed to disagree about.
    assert_ne!(
        MODEL_A.at(GspReg::Wpr2AddrHi),
        MODEL_B.at(GspReg::Wpr2AddrHi),
        "non-vacuity: the two models really are different maps",
    );
    assert_ne!(a.rd(GspReg::Wpr2AddrHi), b.rd(GspReg::Wpr2AddrHi));
    assert_ne!(MODEL_A.startcpu(), MODEL_B.startcpu());
    assert_ne!(MODEL_A.unload_arg(), MODEL_B.unload_arg());

    // …and the FSM's abstract answer is identical.
    assert_eq!(a.fsm.phase(), b.fsm.phase());
    assert_eq!(a.fsm.observe().wpr2_up, b.fsm.observe().wpr2_up);
}

/// ★ Constraint 2 — **the version seam, pushed on.**
///
/// The same logical exchange under the two element layouts that genuinely exist: 48 bytes
/// with `elemCount@40` (r535/r570/580) and 16 bytes with MCTP/NVDM transport headers
/// (610). Then the bite: each side's guest **rejects** the other's encoding, which is what
/// makes "these are different protocols" a measurement rather than a claim.
#[test]
fn the_same_boot_runs_under_both_element_layouts_and_neither_accepts_the_others_bytes() {
    for profile in [P580, P610] {
        let mut w = World::new(profile, MODEL_A);
        assert_eq!(
            w.boot(),
            vec![Transition::E1, Transition::E6, Transition::E5],
            "{}: the boot is the same shape",
            profile.name
        );
        let msgs = w.link_and_drain();
        assert_eq!(msgs.len(), 1, "{}: INIT_DONE arrived", profile.name);
        assert_eq!(msgs[0].function, INIT_DONE, "{}", profile.name);
    }

    // A 610 guest reading a 580-encoded element: the transport words are absent, so its
    // MCTP validation refuses (`ogkm-610: message_queue_cpu.c:737-759`). ★ That block is
    // 610-ONLY: 580 has no MCTP/NVDM header check anywhere in `message_queue_cpu.c`, which
    // is why the refusal is named against the 610 driver and not against a shared rule.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    let mut guest_610 = Guest::new(
        P610,
        w.guest.pages.clone(),
        w.guest.boot_args_gpa,
        MODEL_A.rmargs_id,
    );
    guest_610
        .rx_link(&mut w.ram)
        .expect("the ring geometry is version-independent");
    // Which check fires first is itself informative: the 610 driver reads its `rpc.length`
    // at `hdr(16) + 8 = 24`, which in a 580 element is inside the (zero) AAD buffer, so the
    // length bound refuses before the MCTP validation is reached. Either refusal is the
    // same finding — a 610 driver cannot read a 580 element — and asserting the set rather
    // than one member is asserting what is actually known.
    assert!(
        matches!(
            guest_610.recv(&mut w.ram),
            Err(GuestRefusal::BadLength(_) | GuestRefusal::MctpViolation)
        ),
        "a 610 driver refuses the 580 element it was handed — the layouts are not \
         interchangeable, which is the whole reason the version key exists",
    );

    // And the reverse: a 580 guest reading a 610-encoded element reads its checksum out of
    // the wrong word.
    let mut v = World::new(P610, MODEL_A);
    v.boot();
    let mut guest_580 = Guest::new(
        P580,
        v.guest.pages.clone(),
        v.guest.boot_args_gpa,
        MODEL_A.rmargs_id,
    );
    guest_580.rx_link(&mut v.ram).expect("geometry links");
    assert!(
        matches!(
            guest_580.recv(&mut v.ram),
            Err(GuestRefusal::BadChecksum | GuestRefusal::BadLength(_))
        ),
        "a 580 driver cannot read a 610 element",
    );
}

/// ★★ Constraint 4 — **the GPU restarts, in-process, repeatedly.**
///
/// This is the C artifact's measured failure. After fn-47 the teardown STARTCPU arrives
/// with `was_suspended == true` and `C:4255-4283` calls it a re-acquire: it re-raises
/// WPR2 and re-latches `bootargs_dumped`/`q_ready`, so the **next driver life points at
/// the previous life's queue GPA** and its `msgqRxLink` spins on `-7` (71 064 retries,
/// `docs/reference/mode2_bench_lifecycle.md` §3).
///
/// Three full lifetimes, each with **freshly allocated guest memory at different
/// addresses**, in one process. The cycle count is asserted so this cannot pass by not
/// looping.
#[test]
fn three_driver_lifetimes_in_one_process_leave_no_latch_and_no_stale_binding() {
    let mut w = World::new(P580, MODEL_A);
    let mut completed = 0usize;
    let mut lives = Vec::new();

    for life in 0..3 {
        if life > 0 {
            // A new driver life allocates new queues. The old GPAs are still mapped and
            // still hold the previous life's data — which is exactly what makes a stale
            // binding *work* just well enough to be a disaster.
            w.allocate_guest_memory();
        }
        lives.push(w.guest.pages[0]);

        let t = w.boot();
        assert_eq!(
            t,
            vec![Transition::E1, Transition::E6, Transition::E5],
            "life {life}: a fresh boot, with no latch suppressing the re-publish",
        );
        let msgs = w.link_and_drain();
        assert_eq!(
            msgs.iter().map(|m| m.function).collect::<Vec<_>>(),
            vec![INIT_DONE],
            "life {life}: the guest LINKED (no -7) and got INIT_DONE",
        );
        assert_eq!(
            msgs[0].seq_num, 0,
            "life {life}: a new MESSAGE_QUEUE_INFO starts at rxSeqNum 0, and so do we",
        );

        // Run something, so the life is not just a boot.
        w.guest
            .send(&mut w.ram, FN_RM_CONTROL, 100 + life, &[7; 32])
            .unwrap();
        w.doorbell().unwrap();
        assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1);
        assert_eq!(w.fsm.phase(), BootPhase::Running, "life {life}: live");

        // Teardown: fn-47, then the trailing STARTCPU.
        w.guest
            .send(&mut w.ram, FN_UNLOADING, 200 + life, &[])
            .unwrap();
        let r = w.doorbell().unwrap();
        assert!(
            r.transitions.contains(&Transition::E9),
            "life {life}: fn-47 serviced"
        );
        assert_eq!(
            w.guest.recv(&mut w.ram).unwrap().len(),
            1,
            "life {life}: fn-47 is synchronous — an unanswered one blocks rmmod",
        );
        assert_eq!(w.fsm.phase(), BootPhase::Suspending);

        let m = w.arch.model();
        let r = w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
        assert_eq!(
            r.transitions,
            vec![Transition::E2],
            "life {life}: the trailing teardown STARTCPU is E2, NOT a re-acquire",
        );
        assert_eq!(w.fsm.phase(), BootPhase::Halted);
        assert_eq!(
            *w.fsm.queue(),
            QueueState::Unbound,
            "life {life}: the binding died with the life — no GPA survives to be stale",
        );
        assert_eq!(w.rd(GspReg::Wpr2AddrHi), 0, "life {life}: WPR2 down");
        completed += 1;
    }

    assert_eq!(completed, 3, "three lifetimes actually ran");
    assert_eq!(
        lives
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "non-vacuity: each life really did use different guest memory",
    );

    // …and a fourth boot still works after an explicit device reset, which is the other
    // ordering: the VMM reset rather than the guest's own teardown.
    assert_eq!(w.fsm.device_reset(), Transition::E11);
    assert_eq!(w.fsm.phase(), BootPhase::Cold);
    assert_eq!(*w.fsm.queue(), QueueState::Unbound);
    w.allocate_guest_memory();
    w.boot();
    assert_eq!(w.link_and_drain().len(), 1, "post-reset boot still links");
}

/// ★ The boot-args pair completes on **whichever half lands second** ([inferred] I4).
///
/// ogkm writes lo then hi (`kgspProgramLibosBootArgsAddr_TU102`, byte-identical at both
/// tags: `ogkm-580: kernel_gsp_tu102.c:363-374`, `ogkm-610: :392-403`) and the C keys the
/// whole handshake on the `MAILBOX1` write (`C:4298-4302`) — but that is a write *order*,
/// not a protocol guarantee, and a trigger keyed on one half is a trigger that fires with
/// a stale partner. This drives the halves in the **reverse** order.
#[test]
fn the_boot_args_pair_completes_on_whichever_half_lands_second() {
    let mut w = World::new(P580, MODEL_A);
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    let gpa = w.guest.boot_args_gpa;

    // High half first.
    let r = w.wr(GspReg::GspFalconMailbox1, gpa >> 32).unwrap();
    assert!(
        r.transitions.is_empty(),
        "one half is not a pair — nothing may be published from half an address",
    );
    let r = w.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF).unwrap();
    assert_eq!(
        r.transitions,
        vec![Transition::E6],
        "the pair completed on the LOW half this time, and the publish happened",
    );
    assert_eq!(
        w.link_and_drain().len(),
        1,
        "…and the guest links and drains"
    );
}

/// ★ The **other** restart ordering: an idle release with no teardown STARTCPU.
///
/// `MESSAGE_QUEUE_INFO` is built in `kgspConstructEngine` and destroyed only in
/// `kgspDestruct` (module unload), so an idle release keeps the guest's `rxSeqNum` alive.
/// A re-link resets the *position* (`msgqRxLink` sets `rxReadPtr = 0`,
/// `ogkm-580: src/common/shared/msgq/msgq.c:436`, `ogkm-610: src/nvidia/src/libraries/msgq/msgq.c:435`
/// — the library moved trees and every line shifted by one, the code is identical)
/// and nothing anywhere in either tag assigns `rxSeqNum` — it is only `++`'d
/// (`ogkm-580: message_queue_cpu.c:782`, `ogkm-610: :836`; at 580 the `++` sits in the
/// `msgqRxMarkConsumed`-succeeded branch, at 610 it is unconditional at `exit:` — either
/// way there is no assignment). The C learned this the expensive way: zeroing the
/// sequence made the re-posted `INIT_DONE` arrive at 0 ≪ N, the guest filed it as an old
/// package and ignored it, and the second context hung (`C:3459-3483`).
#[test]
fn an_idle_release_re_acquire_rebinds_and_preserves_the_sequence_numbers() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    for i in 0..3 {
        w.guest
            .send(&mut w.ram, FN_RM_CONTROL, 300 + i, &[9; 8])
            .unwrap();
        w.doorbell().unwrap();
        assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1);
    }
    assert_eq!(w.guest.rx_seq, 4, "one INIT_DONE plus three replies");

    // The idle release: fn-47 is serviced, but no teardown STARTCPU follows — the guest
    // keeps its MESSAGE_QUEUE_INFO and re-boots into the SAME queues.
    w.guest.send(&mut w.ram, FN_UNLOADING, 400, &[]).unwrap();
    w.doorbell().unwrap();
    assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1, "the fn-47 ack");
    let seq_before = w.guest.rx_seq;
    assert_eq!(seq_before, 5, "…which advanced the stream once more");
    assert_eq!(w.fsm.phase(), BootPhase::Suspending);
    assert_eq!(
        w.rd(GspReg::GspFalconMailbox0),
        MODEL_A.suspend_sentinel(),
        "and only then does MAILBOX0 report suspended — the close poll's answer",
    );

    // The re-acquire: STARTCPU (E1 out of Suspending is E2, so the guest's own reload
    // sequence is a fresh boot) and a mailbox re-write.
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap(); // E2 -> Halted
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap(); // E1 -> FwsecRan
    let gpa = w.guest.boot_args_gpa;
    w.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF).unwrap();
    let r = w.wr(GspReg::GspFalconMailbox1, gpa >> 32).unwrap();
    assert!(r.transitions.contains(&Transition::E6), "the queue rebinds");

    // The guest re-links (its own MESSAGE_QUEUE_INFO survived, so rxSeqNum is still 4)…
    w.guest.linked = false;
    w.guest.rx_link(&mut w.ram).expect("rebind links");
    assert_eq!(w.guest.rx_seq, seq_before, "the guest kept its sequence");
    let msgs = w.guest.recv(&mut w.ram).expect("no sequence gap");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].function, INIT_DONE);
    assert_eq!(
        msgs[0].seq_num, seq_before,
        "★ the re-posted INIT_DONE carries the PRESERVED sequence — at 0 the guest would \
         have filed it as an old package and hung",
    );

    // ★★ …and the COMMAND stream resumes where it stopped, which is the mirror image and
    // was broken until B4's drain exposed it.
    //
    // The status queue's position resets on a rebind and its sequence does not, because
    // `msgqRxLink` assigns `rxReadPtr = 0` and nothing ever assigns `rxSeqNum`. The
    // command queue has **no re-link on the producing side**: nothing resets the guest's
    // tx `writePtr` short of `msgqTxCreate`, which runs only from `_gspMsgQueueInit` at
    // module load (`ogkm-580: message_queue_cpu.c:155-161`). So the producer is still at 4
    // here, and a consumer that restarted at 0 would re-read four already-answered
    // commands and then refuse `SeqNumGap` forever — no recovery branch exists for
    // `seqNum >` (`ogkm-580: :699-714`).
    assert_eq!(
        w.guest.tx_write_ptr, 4,
        "non-vacuity: the guest's producer really did NOT rewind",
    );
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 500, &[2; 8])
        .unwrap();
    let r = w
        .doorbell()
        .expect("the command ring resumed where it stopped");
    assert_eq!(
        r.commands.len(),
        1,
        "exactly the ONE new command — not the four this life had already answered",
    );
    assert_eq!(r.commands[0].sequence, 500);
}

/// ★ GSP-D4, as a **negative trace**: the passing condition is that we *differ* from the C.
///
/// The C parses whatever its stale `q_*` fields point at and answers `NV_OK` — 508 log
/// lines of `cmd fn=1959520414 seq=4055862830 -> echo NV_OK` on the measured run. Here the
/// same doorbell is one named refusal, zero elements posted, and — the part that makes it
/// a security property rather than a behaviour — **zero guest-RAM reads**.
///
/// ★★ **Narrowed by B4, and the narrowing is legitimate because the test's SUBJECT
/// changed.** It used to assert `QueueNotBound` for *any* unbound doorbell. That set now
/// contains a second, entirely healthy member: at 580 the guest queues its two init RPCs
/// and rings `QUEUE_HEAD(0)` **before bootstrap** (`ogkm-580: kernel_gsp.c:3753-3777` from
/// `:4141`, before `_kgspBootGspRm` at `:4184`), so a pre-bind doorbell happens on every
/// boot and classifying it as the stale-binding attack signature would be a false positive
/// in the ledger. This test keeps the *measured* case — a doorbell after a teardown, which
/// is the one `docs/reference/mode2_bench_lifecycle.md` §4's 508 log lines came from — and
/// the pre-bind case gains its own test
/// ([`the_580_boot_order_rings_the_doorbell_twice_before_the_binding_exists`]), so total
/// coverage strictly increases. This is **not** "narrow a test to make it pass".
#[test]
fn a_doorbell_on_an_unbound_queue_refuses_by_name_and_reads_zero_guest_ram() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();

    // The measured shape: the guest tears its driver down, and then rings anyway.
    w.guest.send(&mut w.ram, FN_UNLOADING, 7, &[]).unwrap();
    w.doorbell().unwrap();
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    assert_eq!(
        w.fsm.phase(),
        BootPhase::Halted,
        "a binding HAS existed, and died"
    );
    assert_eq!(*w.fsm.queue(), QueueState::Unbound);

    w.ram.reads.clear();
    let before = w.ram.reads.len();
    let err = w.doorbell().unwrap_err();
    assert_eq!(
        err,
        GspFault::QueueNotBound,
        "the exact refusal, not is_err()"
    );
    assert_eq!(
        w.ram.reads.len(),
        before,
        "★ zero guest-RAM reads: the refusal is placed BEFORE the parse, so there is no \
         window in which arbitrary memory is interpreted",
    );

    // The non-vacuity arm (`testing_doctrine.md` §2.2): the same doorbell, once bound,
    // must SUCCEED and must read.
    w.allocate_guest_memory();
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 1, &[3; 16])
        .unwrap();
    w.ram.reads.clear();
    let ok = w.doorbell().expect("a bound doorbell is served");
    assert_eq!(ok.commands.len(), 1, "…and it really did reach the parse");
    assert!(!w.ram.reads.is_empty(), "…and it really did read guest RAM");
}

/// ★ §7-G8 — the liveness obligations, in order.
///
/// fn-47 is `_issueRpcAndWait` (`rpcUnloadingGuestDriver_v1F_07`,
/// `ogkm-580: rpc.c:9168-9192`, `ogkm-610: :9146-9170` — same body, 580 reaches the
/// payload through the `rpc_message` macro where 610 uses `rpcGetVgpuMessageData`), so
/// the reply comes **first**; only then may `MAILBOX0` report the suspend sentinel the
/// close poll is waiting on (`kgspWaitForProcessorSuspend_TU102`,
/// `ogkm-580: kernel_gsp_tu102.c:1241-1249`, `ogkm-610: :351-359`; and see the
/// sentinel-shape seam pinned below — 580 tests `mailbox == 0x80000000`, 610 tests a
/// mask). A fault-and-stop posture on this path hangs the
/// guest's `rmmod`, which is why no refusal in this crate can stop the register surface.
#[test]
fn fn47_is_answered_before_the_suspend_sentinel_appears() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    assert_eq!(
        w.rd(GspReg::GspFalconMailbox0),
        w.guest.boot_args_gpa & 0xFFFF_FFFF,
        "before fn-47 MAILBOX0 still echoes the boot-args pointer",
    );

    w.guest.send(&mut w.ram, FN_UNLOADING, 77, &[]).unwrap();
    let r = w.doorbell().unwrap();
    assert!(r.transitions.contains(&Transition::E9));

    let acks = w.guest.recv(&mut w.ram).unwrap();
    assert_eq!(acks.len(), 1, "the ack was posted");
    assert_eq!(acks[0].function, FN_UNLOADING);
    assert_eq!(acks[0].sequence, 77, "matched on (function, sequence)");
    assert_eq!(
        w.rd(GspReg::GspFalconMailbox0),
        MODEL_A.suspend_sentinel(),
        "and only after the reply does the sentinel appear",
    );
    // The device is still answering — a refusal is per-message, never per-device.
    assert_eq!(w.rd(GspReg::GfwBootProgress), 0xff);
}

/// ★ §7-G7 — an unsolicited event before the guest is out of its bootup poll is a guest
/// `NV_ASSERT(0)`.
///
/// The poll runs without the API lock and accepts a short allowlist — ★ SEAM: **six**
/// functions at 580 (`ogkm-580: kernel_gsp.c:1464-1482`) and **eight** at 610
/// (`ogkm-610: :1419-1440`; 610 adds `GSP_LOAD_EXEC_GENERIC_BOOTLOADER` and
/// `GSP_LOAD_EXEC_HS_BINARY` and drops `GSP_RUN_CPU_SEQUENCER`). `POST_EVENT` is in
/// **neither** list, so the `NV_ASSERT(0)` this test pins holds at both tags — but the
/// count does not, and this test runs `P580`. The gate here is the
/// *observed* drain of `GSP_INIT_DONE`, so this test also pins that the observation, not
/// the posting, is what opens the window.
#[test]
fn an_unsolicited_event_is_refused_until_the_guest_has_drained_init_done() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();

    let event = OutgoingRpc {
        function: POST_EVENT,
        sequence: 0,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: vec![0; 32],
    };
    assert_eq!(
        w.fsm.post_event(&mut w.ram, &event),
        Err(GspFault::NotRunning),
        "INIT_DONE is posted but not yet drained — the guest is inside its boot poll",
    );

    // The guest links, drains, and issues its first post-init RPC.
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_SET_GUEST_SYSTEM_INFO, 5, &[])
        .unwrap();
    w.doorbell().unwrap();
    w.guest.recv(&mut w.ram).unwrap();
    assert_eq!(w.fsm.phase(), BootPhase::Running);

    // The non-vacuity arm: the same event now succeeds, and the guest accepts it.
    w.fsm
        .post_event(&mut w.ram, &event)
        .expect("Running accepts events");
    let got = w.guest.recv(&mut w.ram).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].function, POST_EVENT);
    assert_eq!(got[0].payload.len(), 32);
}

/// ★ GSP-D6 — a command whose payload spans elements is read **whole**.
///
/// The C advances past continuation elements without reading them (`C:3341-3350`), so a
/// multi-element command is silently truncated to its first 4048 payload bytes.
#[test]
fn a_multi_element_command_is_read_whole_not_truncated() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();

    // Three elements' worth: 48 + 32 + payload > 2 * 4096.
    let payload: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
    let elements = w
        .guest
        .send(&mut w.ram, FN_RM_CONTROL, 42, &payload)
        .expect("the ring has room");
    assert_eq!(
        elements, 3,
        "non-vacuity: this really is a multi-element command"
    );

    let r = w.doorbell().unwrap();
    assert_eq!(r.commands.len(), 1, "three elements, ONE logical command");
    assert_eq!(r.commands[0].elements, 3);
    assert_eq!(
        r.commands[0].payload, payload,
        "every byte survived — the C would have kept only the first element's worth",
    );
    assert!(
        r.commands[0].payload.len() > 4096,
        "non-vacuity: the payload really did exceed one element",
    );
}

/// ★ GSP-D2 — over-posting is refused instead of corrupting the stream.
///
/// The failure it prevents is not "a message is lost": the guest reads an element whose
/// `seqNum` is **greater** than its `rxSeqNum`, and the recovery branch at
/// `ogkm-580: message_queue_cpu.c:699-714` (`ogkm-610: :768-782`) handles only `<`. There
/// is no recovery for `>` at either tag, and `rxSeqNum++` happens anyway once the retries
/// are exhausted (`ogkm-580: :782`, `ogkm-610: :836`), so the two streams stay one apart
/// forever.
#[test]
fn over_posting_is_refused_as_queue_full_and_the_guest_never_sees_a_sequence_gap() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    let count = w.guest.msg_count;
    assert_eq!(
        count, 7,
        "a small ring, so fullness is reached rather than described"
    );

    // Post until the ring refuses. At most msgCount - 1 elements may be outstanding, or
    // full becomes indistinguishable from empty (`ogkm-580: msgq.c:491`,
    // `ogkm-610: :490` — same `msgqTxGetFreeSpace` line, the `-1`).
    let mut posted = 0u32;
    let mut refusal = None;
    for i in 0..20 {
        let rpc = OutgoingRpc {
            function: POST_EVENT,
            sequence: i,
            rpc_result: 0,
            rpc_result_private: 0,
            payload: vec![0; 16],
        };
        match w.fsm.post(&mut w.ram, &rpc) {
            Ok(()) => posted += 1,
            Err(e) => {
                refusal = Some(e);
                break;
            }
        }
    }
    assert_eq!(
        refusal,
        Some(GspFault::QueueFull { needed: 1, free: 0 }),
        "the exact refusal, with the numbers that produced it",
    );
    // ★ This expectation was wrong when first written (it said `count - 2`, on the
    // assumption that INIT_DONE still occupied a slot) and the failing test was the
    // finding: `link_and_drain` above consumes INIT_DONE, so the whole ring is free and
    // the bound is `msgCount - 1` outstanding — the `-1` in `msgqTxGetFreeSpace`
    // (`ogkm-580: msgq.c:491`, `ogkm-610: :490`), which is what keeps a full ring
    // distinguishable from an empty one. Corrected rather than relaxed.
    assert_eq!(
        posted,
        count - 1,
        "msgCount - 1 is the outstanding bound (msgq.c:490's -1)",
    );

    // The guest drains everything with no gap — which is the property the refusal bought.
    let msgs = w.guest.recv(&mut w.ram).expect("no sequence gap");
    assert_eq!(msgs.len() as u32, posted);
    for (i, m) in msgs.iter().enumerate() {
        assert_eq!(m.seq_num as usize, i + 1, "strictly monotonic, no hole");
    }

    // And once drained, posting resumes — QueueFull is back-pressure, not a wedge.
    let rpc = OutgoingRpc {
        function: POST_EVENT,
        sequence: 99,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: vec![0; 16],
    };
    w.fsm.post(&mut w.ram, &rpc).expect("space was freed");
}

/// ★ The command-queue read pointer is published into the **swapped** location, and the
/// guest keeps making progress past a full ring's worth of commands.
///
/// The C found this the expensive way: writing the ack to `cmd_base + rxHdrOff` — the
/// unswapped location — left the guest computing zero free space and reporting *"buffer
/// is full"* once ~63 command elements had accumulated (`C:3352-3358`). With
/// `MSGQ_FLAGS_SWAP_RX` agreed, each side writes the read pointer into **its own** backing
/// store (`msgqRxLink`'s `rxSwapped` arm, identical at both tags:
/// `ogkm-580: msgq.c:417-420`, `ogkm-610: :416-419`), and ours is the status queue.
#[test]
fn the_guest_makes_progress_past_a_full_rings_worth_of_commands() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    let count = w.guest.msg_count;

    // Four times the ring's capacity, so the ack must be visible or the guest wedges.
    let rounds = 4 * count;
    for i in 0..rounds {
        assert!(
            w.guest.free_space(&mut w.ram) > 0,
            "the guest ran out of free space at command {i} — the read-pointer ack is not              where its msgqTxGetFreeSpace looks",
        );
        w.guest.send(&mut w.ram, FN_RM_CONTROL, i, &[1; 8]).unwrap();
        w.doorbell().unwrap();
        assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1, "reply {i}");
    }
    assert!(rounds > count, "non-vacuity: the ring really did wrap");
}

/// ★ GSP-D8 — the region is addressed through **its own page table**, not linearly.
///
/// The bite is arithmetic: for a fragmented region, `sharedMemPhysAddr + offset` names a
/// different page than the table does for every page but the first. This test asserts
/// both that the two disagree (so the fragmentation is real) and that every access the
/// FSM made landed on a page the table names.
#[test]
fn a_fragmented_region_is_addressed_through_its_page_table_not_linearly() {
    let mut w = World::new(P580, MODEL_A);
    let pages = w.guest.pages.clone();
    let linear_base = pages[0];
    assert_ne!(
        pages[1],
        linear_base + PAGE,
        "non-vacuity: the region really is fragmented",
    );

    w.ram.reads.clear();
    w.ram.writes.clear();
    w.boot();
    let touched: Vec<u64> = w
        .ram
        .reads
        .iter()
        .chain(w.ram.writes.iter())
        .map(|(gpa, _)| *gpa & !(PAGE - 1))
        .collect();
    assert!(!touched.is_empty(), "non-vacuity: the boot touched memory");

    let legal: std::collections::BTreeSet<u64> = pages
        .iter()
        .copied()
        .chain([w.guest.boot_args_gpa, w.guest.rmargs_gpa])
        .collect();
    for gpa in &touched {
        assert!(
            legal.contains(gpa),
            "access at {gpa:#x} is outside every page the guest's table names",
        );
    }

    // The status queue's header would have been written at the WRONG page under linear
    // addressing — which is the whole of GSP-D8.
    let stat_off = w.guest.stat_off;
    assert_ne!(
        w.guest.gpa_of(stat_off),
        linear_base + stat_off,
        "linear addressing would have published the tx header into another page",
    );

    // …and the guest, resolving through the same table, finds it.
    assert!(w.guest.rx_link(&mut w.ram).is_ok());
}

/// ★ S5 — the decoded projection, and the negative trace's assertion in one line.
#[test]
fn the_projection_records_what_the_guest_would_observe_and_names_the_refusal() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    let QueueState::Bound(binding) = w.fsm.queue().clone() else {
        panic!("bound after boot");
    };
    let projection =
        Projection::new(binding.geometry(), P580.layout()).expect("a bound geometry projects");

    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 21, &[4; 8])
        .unwrap();

    let (log, reads) = {
        let mut rec = RecordingRam::new(&mut w.ram).with_projection(projection.clone());
        let (bar, off) = MODEL_A.at(GspReg::GspQueueHead(0));
        w.fsm
            .mmio_write(&mut rec, &w.arch, &mut EchoOk, bar, off, 1)
            .expect("served");
        (rec.log, rec.reads)
    };
    assert!(
        reads > 0,
        "non-vacuity: the projection saw a real service pass"
    );
    assert_eq!(log.count("ElementPosted"), 1, "one reply element");
    assert_eq!(
        log.count("ReadPtrAcked"),
        1,
        "consumption published exactly once"
    );
    assert_eq!(log.count("WritePtrAdvanced"), 1);
    let Observation::ElementPosted {
        function,
        sequence,
        rpc_length,
        ..
    } = log
        .items()
        .iter()
        .find(|o| o.kind() == "ElementPosted")
        .expect("posted")
        .clone()
    else {
        unreachable!()
    };
    assert_eq!(function, FN_RM_CONTROL);
    assert_eq!(sequence, 21);
    assert_eq!(
        rpc_length, 40,
        "★ GSP-D1: 32-byte envelope + 8-byte body. The C would have written 36 for a bare \
         header; the envelope's real size is 32",
    );

    // The negative trace: the same doorbell after a teardown is ONE named refusal and
    // ZERO posted elements.
    //
    // ★ B4: it must be a *teardown*, not a `device_reset`. After a reset the FSM is
    // `Cold` and has never bound in this device life, which is the healthy pre-bootstrap
    // case ([`Transition::E12`]) and not the stale-binding one this trace is about.
    w.guest.send(&mut w.ram, FN_UNLOADING, 55, &[]).unwrap();
    w.doorbell().unwrap();
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    assert_eq!(w.fsm.phase(), BootPhase::Halted);
    let (log, reads) = {
        let mut rec = RecordingRam::new(&mut w.ram).with_projection(projection);
        let (bar, off) = MODEL_A.at(GspReg::GspQueueHead(0));
        let err = w
            .fsm
            .mmio_write(&mut rec, &w.arch, &mut EchoOk, bar, off, 1)
            .expect_err("unbound");
        rec.refused(&err);
        (rec.log, rec.reads)
    };
    assert_eq!(reads, 0, "zero guest reads");
    assert_eq!(log.count("ElementPosted"), 0);
    assert_eq!(
        log.refusals().iter().map(|f| f.0).collect::<Vec<_>>(),
        vec!["GspFault::QueueNotBound"],
        "exactly one refusal, named",
    );
    assert!(
        log.unseen_kinds().contains(&"ElementPosted"),
        "the non-vacuity instrument agrees nothing was posted",
    );
}

// ──────────────────────── the 580 element-count invariants (B1–B4) ────────────────────────

fn event(payload_len: usize) -> OutgoingRpc {
    OutgoingRpc {
        function: POST_EVENT,
        sequence: 0,
        rpc_result: 0,
        rpc_result_private: 0,
        payload: vec![0xE1; payload_len],
    }
}

/// ★★ **B1 — the oracle itself.** At 580 the guest consumes by the `elemCount` **field**;
/// at 610 there is no such field and it derives the count from `rpc.length`.
///
/// This is the axis the two protocols actually differ on, and until the instrument models
/// it no test of any 580 invariant could fail before its fix — it would pass against a
/// mock that derived the count for both profiles, which is the worst kind of green.
///
/// 580: `nElements = pMQI->pCmdQueueElement->elemCount`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:652-658`), and *that* is
/// what `msgqRxMarkConsumed` advances the ring by (`:774`). Its `msgLen` sanity check at
/// `:760-770` runs after the element is already consumed and gates nothing.
/// 610: `ogkm-610: message_queue_cpu.c:698-705`, consumed at `:838`.
#[test]
fn a_580_guest_consumes_by_elem_count_not_by_declared_length() {
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.guest
        .rx_link(&mut w.ram)
        .expect("the published header links");
    // A second element behind INIT_DONE, so a run of two is genuinely available — the
    // driver's copy loop stops at the first element the ring does not have
    // (`ogkm-580: msgq.c:673-693`), so a count with nothing behind it proves nothing.
    w.fsm
        .post(&mut w.ram, &event(16))
        .expect("the ring has room");
    assert_eq!(w.guest.rx_read_ptr, 0);

    // One word, in the guest's memory only: element 0 now CLAIMS two elements while its
    // own `rpc.length` still implies one.
    w.poke_element(RingId::Status, 0, 48, 32, 40, 2);

    let msgs = w
        .guest
        .recv(&mut w.ram)
        .expect("a valid, if dishonest, element");
    assert_eq!(
        msgs.len(),
        1,
        "★ the second element was swallowed as this message's continuation, because the \
         FIELD said so — a length-derived guest would have reported two messages",
    );
    assert_eq!(msgs[0].function, INIT_DONE);
    assert_eq!(
        w.guest.rx_read_ptr, 2,
        "the ring advanced by elemCount, not by ceil(msgLen / msgSize)",
    );
    assert_eq!(
        w.guest.rx_seq, 1,
        "…and exactly ONE message's worth of sequence was consumed",
    );

    // ★ Non-vacuity: the same word, on 610, changes nothing about consumption — offset 40
    // there is `rpc.sequence` (payload@16 + 24) and carries no count at all
    // (`ogkm-610: message_queue_priv.h:52-67`).
    let mut v = World::new(P610, MODEL_A);
    v.boot();
    v.guest.rx_link(&mut v.ram).expect("links");
    v.fsm.post(&mut v.ram, &event(16)).expect("room");
    v.poke_element(RingId::Status, 0, 16, 8, 40, 2);
    let msgs = v.guest.recv(&mut v.ram).expect("a valid element");
    assert_eq!(
        msgs.len(),
        2,
        "610 derives the count from rpc.length, so a 2 at +40 consumes nothing extra",
    );
    assert_eq!(
        msgs[0].sequence, 2,
        "…and non-vacuity for the poke itself: at 610 that offset really is rpc.sequence, \
         so the write DID land somewhere the guest reads",
    );
    assert_eq!(v.guest.rx_read_ptr, 2, "two messages, two elements");
}

/// ★★★ **B2 / GSP-S1 — we may not emit an element count the guest's staging buffer
/// cannot hold.** A memory-safety bound aimed at the *guest's* kernel.
///
/// `_gspMsgQueueInit` allocates `4096 + GSP_MSG_QUEUE_ELEMENT_SIZE_MAX + msgqGetMetaSize()`
/// from `portMemAllocNonPaged` and carves the **live `msgq` metadata immediately after**
/// the staging area (`ogkm-580: message_queue_cpu.c:132-134, 143-145`); the loop that
/// fills it copies one element per iteration with no bound but ring availability
/// (`:628, 648-650`). So an element declaring more than
/// `queueElementSizeMax / element_size` elements memcpys past a kernel allocation.
#[test]
fn an_over_wide_element_count_is_refused_before_it_can_overrun_the_guest_staging_buffer() {
    const HDR_580: usize = 48;
    const ENVELOPE: usize = 32;
    let max_bytes = STAGING_BYTES;

    // The bound is derived from the geometry, never written down.
    assert_eq!(
        kayfabe_gsp::max_elements(PAGE as u32, max_bytes),
        16,
        "65536 / 4096 — the driver's own carve, computed rather than transcribed",
    );

    let encode = |profile: &kayfabe_tests::gspworld::Profile, element_size: u32, payload: usize| {
        kayfabe_gsp::encode_message(
            &profile.layout(),
            0x0300_0000,
            element_size,
            max_bytes,
            0,
            &OutgoingRpc {
                function: FN_RM_CONTROL,
                sequence: 1,
                rpc_result: 0,
                rpc_result_private: 0,
                payload: vec![0xA5; payload],
            },
        )
    };

    // Exactly at the bound still encodes, and stamps the count it really occupies.
    let at_bound = max_bytes as usize - HDR_580 - ENVELOPE;
    let run = encode(&P580, PAGE as u32, at_bound).expect("16 elements is legal");
    assert_eq!(run.len(), 16 * PAGE as usize);
    assert_eq!(
        u32::from_le_bytes(run[40..44].try_into().unwrap()),
        16,
        "elemCount is what the run actually occupies",
    );

    // One byte more, on the bench's geometry, is caught by the LENGTH bound first —
    // because there `element_size` divides `element_size_max` and the two coincide. That
    // coincidence is precisely why the count bound cannot be left implicit.
    assert_eq!(
        encode(&P580, PAGE as u32, at_bound + 1),
        Err(GspFault::MsgLenOutOfRange {
            declared: 65_489,
            min: 32,
            max: 65_488,
        }),
    );

    // ★★ The geometry where they genuinely differ, and where the derivation stops
    // implying the bound. `element_size` is the guest's own published `msgSize`, and
    // `msgqRxLink` accepts any value at or above `MSGQ_MSG_SIZE_MIN`
    // (`ogkm-580: src/common/shared/msgq/msgq.c:340-343`) — so a non-dividing element size
    // is input this crate can be handed, not a hypothetical. The staging buffer then holds
    // floor(65536 / 5000) = 13 elements while a message of the maximum LEGAL length still
    // occupies ceil(65536 / 5000) = 14.
    assert_eq!(kayfabe_gsp::max_elements(5000, max_bytes), 13);
    assert_eq!(
        encode(&P580, 5000, at_bound),
        Err(GspFault::ElementCountOutOfRange { count: 14, max: 13 }),
        "the exact variant and both fields — the length was in range, the COUNT was not",
    );

    // ★ And it is not gated on the element layout: the staging buffer exists on every
    // version, so a bound expressed as `if the layout has an elemCount field` would be a
    // branch on version identity. 610 has no such field and is refused identically.
    assert_eq!(
        encode(&P610, 5000, at_bound),
        Err(GspFault::ElementCountOutOfRange { count: 14, max: 13 }),
        "the bound belongs to the geometry, not to the field that happens to carry it",
    );
}

/// ★★ The **bite check** for the clamp above (`testing_doctrine.md` §1c): this is what
/// reaches a guest when nothing refuses.
///
/// Built by hand rather than through `encode_message`, because the count under test is one
/// `encode_message` now refuses to write. Driven on the **real 63-slot ring**, because the
/// copy loop stops at the first unavailable element — only a ring that can actually hold
/// 17+ elements makes the overrun reachable, which is exactly the bench's geometry.
#[test]
#[should_panic(expected = "staging buffer")]
fn without_the_clamp_an_over_wide_element_count_overruns_the_guest_staging_buffer() {
    let mut w = World::new_sized(P580, MODEL_A, REAL_QUEUE_SIZE);
    w.boot();
    w.guest.rx_link(&mut w.ram).expect("links");
    assert_eq!(
        w.guest.msg_count, 63,
        "non-vacuity: the ring the driver really builds, not the 7-slot one",
    );

    // INIT_DONE plus 61 more: 62 elements outstanding, the most a 63-slot ring can hold
    // (`ogkm-580: msgq.c:490`'s -1).
    for _ in 0..61 {
        w.fsm
            .post(&mut w.ram, &event(16))
            .expect("the ring has room");
    }
    w.poke_element(RingId::Status, 0, 48, 32, 40, 62);
    let _ = w.guest.recv(&mut w.ram);
}

/// ★★ **B3 — the ring advances by the producer's number, and a disagreement is refused
/// with the cursor left where it was.**
///
/// The guest writes `pCQE->elemCount = GSP_MSG_QUEUE_BYTES_TO_ELEMENTS(msgLen)` and then
/// advances its own `writePtr` by **that field**
/// (`ogkm-580: message_queue_cpu.c:482, 578`). A consumer that advances by a derivation
/// therefore desynchronises the ring against a producer that disagrees, permanently and
/// silently: the resulting mismatch is `seqNum >` , for which the driver's recovery branch
/// does not exist (`ogkm-580: :699-714` handles only `<`).
#[test]
fn a_command_whose_elem_count_disagrees_with_its_length_is_refused_and_does_not_move_the_ring() {
    let read_ptr = |w: &World| match w.fsm.queue() {
        QueueState::Bound(b) => b.command_cursor().read_ptr,
        QueueState::Unbound => panic!("bound"),
    };

    // Arm 1 — the field claims two elements, the length implies one.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 9, &[1; 16])
        .expect("one element");
    let before = read_ptr(&w);
    w.poke_element(RingId::Command, 0, 48, 32, 40, 2);
    assert_eq!(
        w.doorbell(),
        Err(GspFault::ElementCountMismatch {
            declared: 2,
            derived: 1
        }),
    );
    assert_eq!(
        read_ptr(&w),
        before,
        "a refused message does not advance the cursor — the refusal stays visible",
    );

    // Arm 2 — the same field, over the STAGING bound. A hard refusal that no amount of
    // ring filling can turn into a valid message, so it outranks "producer not finished".
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 9, &[1; 16])
        .expect("one element");
    w.poke_element(RingId::Command, 0, 48, 32, 40, 62);
    assert_eq!(
        w.doorbell(),
        Err(GspFault::ElementCountOutOfRange { count: 62, max: 16 }),
        "GSP-S1 on the receive side, where the count is guest-written rather than derived",
    );

    // Arm 3 — the mirrored positive: a genuine two-element command, whose field and length
    // agree, decodes and advances the cursor by exactly two.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    let payload = vec![0x3C; 4100];
    let n = w
        .guest
        .send(&mut w.ram, FN_RM_CONTROL, 10, &payload)
        .expect("room");
    assert_eq!(n, 2, "non-vacuity: this really is a two-element command");
    let before = read_ptr(&w);
    let r = w.doorbell().expect("an honest element is served");
    assert_eq!(r.commands.len(), 1);
    assert_eq!(r.commands[0].elements, 2);
    assert_eq!(r.commands[0].payload, payload);
    assert_eq!(read_ptr(&w), before + 2, "the cursor moved by exactly two");

    // ★ Non-vacuity across the version seam: at 610 there is no field to disagree with, so
    // the same poke can never produce `ElementCountMismatch`. It lands on `rpc.sequence`
    // instead, which the guest reads and we report.
    let mut v = World::new(P610, MODEL_A);
    v.boot();
    v.link_and_drain();
    v.guest
        .send(&mut v.ram, FN_RM_CONTROL, 9, &[1; 16])
        .expect("one element");
    v.poke_element(RingId::Command, 0, 16, 8, 40, 2);
    let r = v.doorbell().expect("610 has no elemCount to disagree with");
    assert_eq!(r.commands.len(), 1);
    assert_eq!(r.commands[0].sequence, 2, "the poke landed on rpc.sequence");
}

/// ★★ **B4 — 580 queues its init RPCs BEFORE bootstrap, so the doorbell rings twice while
/// nothing is bound, on every healthy boot.**
///
/// `kgspQueueAsyncInitRpcs_IMPL` sends `GSP_SET_SYSTEM_INFO` then `SET_REGISTRY`
/// (`ogkm-580: kernel_gsp.c:3753-3777`) from `kgspInitRm_IMPL` at `:4141` — **before**
/// `_kgspBootGspRm` at `:4184`, therefore before FWSEC, Booter Load, RISC-V start and the
/// status-queue link — and `rpcSendMessage` rings `kgspSetCmdQueueHead_HAL` unconditionally
/// after every submit (`:425`), with no "is the GSP up" gate in `_kgspRpcSanityCheck`
/// (`:281-321`). At 610 the same two RPCs are sent from *inside* `kgspBootstrap_TU102`
/// (`ogkm-610: kernel_gsp_tu102.c:576-585`, `kernel_gsp.c:4686-4709`) and the door never
/// rings early.
///
/// Two things follow, and both are asserted here: the pre-bind doorbell is **not** the
/// stale-binding signature, and the bind inherits a **non-empty ring** it must drain
/// itself rather than by waiting for the guest to ring again.
#[test]
fn the_580_boot_order_rings_the_doorbell_twice_before_the_binding_exists() {
    let mut w = World::new(P580, MODEL_A);
    let m = w.arch.model();

    for (i, (function, payload)) in [
        (FN_GSP_SET_SYSTEM_INFO, vec![1u8, 2, 3, 4]),
        (FN_SET_REGISTRY, vec![5u8; 64]),
    ]
    .into_iter()
    .enumerate()
    {
        w.guest
            .send(&mut w.ram, function, 10 + i as u32, &payload)
            .expect("the guest queues before the GSP exists");
        w.ram.reads.clear();
        let r = w
            .doorbell()
            .expect("a pre-bind doorbell is the healthy 580 order, not a refusal");
        assert_eq!(
            r.transitions,
            vec![Transition::E12],
            "doorbell {i}: classified as pre-bootstrap, NOT as the stale binding \
             QueueNotBound describes",
        );
        assert!(r.commands.is_empty(), "doorbell {i}: nothing was parsed");
        assert!(
            w.ram.reads.is_empty(),
            "doorbell {i}: ★ zero guest-RAM reads — the classification changed, the \
             security property did not",
        );
    }
    assert_eq!(w.fsm.phase(), BootPhase::Cold);
    assert_eq!(*w.fsm.queue(), QueueState::Unbound);

    // Only now does the bootstrap run.
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    let gpa = w.guest.boot_args_gpa;
    w.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF).unwrap();
    let e6 = w.wr(GspReg::GspFalconMailbox1, gpa >> 32).unwrap();

    assert_eq!(e6.transitions, vec![Transition::E6]);
    assert_eq!(
        e6.commands.len(),
        2,
        "★ the backlog was drained BY THE BIND — before B4 this was empty until the guest \
         happened to ring the door again, and the port recovered by luck",
    );
    assert_eq!(e6.commands[0].code, FN_GSP_SET_SYSTEM_INFO);
    assert_eq!(e6.commands[1].code, FN_SET_REGISTRY);

    // Booter Load, and then the guest's own acceptance predicate.
    w.wr(GspReg::Sec2FalconMailbox0, 0).unwrap();
    w.wr(GspReg::Sec2FalconCpuctl, m.startcpu()).unwrap();
    let msgs = w.link_and_drain();
    assert_eq!(
        msgs.iter().map(|m| m.function).collect::<Vec<_>>(),
        vec![INIT_DONE],
        "72 and 73 are _issueRpcAsync, so the drain answered NEITHER — an echo would \
         surface in the driver as an unexpected event",
    );
}

/// ★ **B9 — the suspend sentinel REPLACES the mailbox shadow; it is never OR-ed onto it.**
///
/// 580 tests exact equality — `return (mailbox == 0x80000000)`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_tu102.c:1226-1238`,
/// the constant inlined, no `INTERRUPT_PROCESSOR_SUSPENDED_VALUE` symbol in that tree) —
/// while 610 tests a **mask** (`ogkm-610: kernel_gsp_tu102.c:333, 348`). So a shadow still
/// holding a boot-args half with bit 31 set reads as suspended at 610 and hangs the
/// teardown poll *forever* at 580, which polls it from two places: after fn-47
/// (`ogkm-580: kernel_gsp.c:4310`) and as a bootstrap liveness fallback (`:551` of
/// `kernel_gsp_tu102.c`).
///
/// ★ This item changes no production behaviour — `GspFsm::observe` exposes `suspended` as
/// a bool and the encoding lives behind `GspModel`, which is already right. **The pin is
/// the deliverable**, and the arm that fails today is the bite against a second model that
/// deliberately OR-s.
#[test]
fn the_suspend_sentinel_replaces_the_mailbox_shadow_rather_than_setting_a_bit() {
    // A legal boot-args low half with bit 31 ALREADY set.
    const SHADOW: u64 = 0x8000_1234;

    let drive = |model: kayfabe_tests::gspworld::FakeGspModel| -> (u64, u64) {
        let mut w = World::new(P580, model);
        w.boot();
        w.link_and_drain();
        // A lone MAILBOX0 write: the pair was consumed by the publish, so this updates the
        // register shadow without re-triggering E6.
        let r = w.wr(GspReg::GspFalconMailbox0, SHADOW).unwrap();
        assert!(r.transitions.is_empty(), "half a pair publishes nothing");
        let awake = w.rd(GspReg::GspFalconMailbox0);
        w.guest.send(&mut w.ram, FN_UNLOADING, 4, &[]).unwrap();
        w.doorbell().unwrap();
        assert_eq!(w.fsm.phase(), BootPhase::Suspending);
        (awake, w.rd(GspReg::GspFalconMailbox0))
    };

    let sentinel = MODEL_A.suspend_sentinel();
    let (awake, suspended) = drive(MODEL_A);
    assert_eq!(
        awake, SHADOW,
        "not suspended: the register reads back verbatim"
    );
    assert_ne!(
        awake, sentinel,
        "…so the guest's `mailbox == sentinel` poll is FALSE, which is the point",
    );
    assert_eq!(
        suspended, sentinel,
        "★ suspended: EXACTLY the sentinel, with no bit of the shadow surviving",
    );

    // ★ The bite: a model that OR-s instead of replacing. At 610 its answer still reads as
    // suspended; at 580 the equality never holds and `rmmod` hangs on the close poll.
    let (awake, suspended) = drive(MODEL_A.with_or_ed_suspend_sentinel());
    assert_eq!(awake, SHADOW);
    assert_eq!(
        suspended,
        SHADOW | sentinel,
        "non-vacuity: the wrong model really did OR",
    );
    assert_ne!(
        suspended, sentinel,
        "★ and the pin catches it — a 580-shaped `== sentinel` poll never terminates",
    );
    assert_ne!(
        suspended & sentinel,
        0,
        "…while a 610-shaped `& sentinel` poll would be satisfied, which is why the \
         difference is invisible to anyone testing only the mask",
    );
}

/// ★★ **B6 — the 610 transport words come out of `kayfabe-abi`, and only the two bit
/// fields the driver reads may be asserted on.**
///
/// The words used to be `(0x0000_0001, 0x0000_10de)` placeholders. What is *newly*
/// testable is not that they are different numbers but that the mock stopped being
/// **stricter than the driver**: 610 validates `REF_VAL(MCTP_HEADER_VERSION, mctpHeader)`
/// and `REF_VAL(MCTP_MSG_HEADER_VENDOR_ID, nvdmHeader)` and nothing else
/// (`ogkm-610: message_queue_cpu.c:735-762`).
#[test]
fn the_610_transport_words_are_the_drivers_own_and_only_two_fields_are_validated() {
    let t = P610.mctp().expect("610 carries transport words");
    assert_eq!(
        t.header_word, 0xC000_0001,
        "the assembled MCTP transport header"
    );
    assert_eq!(t.nvdm_word, 0x2510_DE7E, "the assembled NVDM header");
    assert_eq!(
        P580.mctp(),
        None,
        "580 has no MCTP at all — ogkm-580 has no mctp_format.h"
    );

    // The words really do go on the wire, and a conforming 610 guest accepts them.
    let mut w = World::new(P610, MODEL_A);
    w.boot();
    let el = w.status_element_gpa(0);
    let mut words = [0u8; 8];
    kayfabe_gsp::GuestRam::read(&mut w.ram, el, &mut words).unwrap();
    assert_eq!(
        u32::from_le_bytes(words[0..4].try_into().unwrap()),
        0xC000_0001,
    );
    assert_eq!(
        u32::from_le_bytes(words[4..8].try_into().unwrap()),
        0x2510_DE7E,
    );
    assert_eq!(w.link_and_drain().len(), 1, "…and the guest accepts them");

    // A wrong **version nibble** is the driver's own `NV_ERR_INVALID_DATA`.
    let mut w = World::new(P610, MODEL_A);
    w.boot();
    w.guest.rx_link(&mut w.ram).expect("links");
    w.poke_element(RingId::Status, 0, 16, 8, 0, 0xC000_0002);
    assert_eq!(
        w.guest.recv(&mut w.ram),
        Err(GuestRefusal::MctpViolation),
        "MCTP_HEADER_VERSION != 1",
    );

    // …and so is a wrong **vendor id**.
    let mut w = World::new(P610, MODEL_A);
    w.boot();
    w.guest.rx_link(&mut w.ram).expect("links");
    w.poke_element(RingId::Status, 0, 16, 8, 4, 0x2500_007E);
    assert_eq!(
        w.guest.recv(&mut w.ram),
        Err(GuestRefusal::MctpViolation),
        "MCTP_MSG_HEADER_VENDOR_ID != 0x10de",
    );
}

/// ★★ The other half of B6, and the one that keeps the instrument honest: **the guest does
/// NOT check SOM, EOM, the packet sequence, or the NVDM type byte.**
///
/// Those fields are written by `mctpCreateTransportHeader`/`mctpCreateNvdmHeader` and never
/// read back — the receiver's whole validation is two `REF_VAL`s
/// (`ogkm-610: message_queue_cpu.c:735-762`). An oracle that enforced the whole words
/// would be stricter than the driver, and any later test asserting "the guest rejects a bad
/// NVDM type" would be asserting a behaviour that does not exist. This is the arm that
/// stops that drift, and it is the same rule §4.4 already applies to the RPC `signature`.
#[test]
fn the_610_guest_does_not_check_som_eom_or_the_nvdm_type() {
    let t = P610.mctp().expect("610 carries transport words");

    // Clear SOM and EOM (31:31, 30:30) and set a nonzero packet SEQ (29:28); keep the
    // version nibble. Change the NVDM type byte (31:24); keep the vendor id.
    let mangled_mctp = (t.header_word & !0xF000_0000) | 0x1000_0000;
    let mangled_nvdm = (t.nvdm_word & 0x00FF_FFFF) | 0xAB00_0000;
    assert_ne!(
        mangled_mctp, t.header_word,
        "non-vacuity: the word really changed"
    );
    assert_ne!(mangled_nvdm, t.nvdm_word);
    assert_eq!(
        mangled_mctp & t.header_validated_mask,
        t.header_word & t.header_validated_mask,
        "…but the validated field is untouched",
    );
    assert_eq!(
        mangled_nvdm & t.nvdm_validated_mask,
        t.nvdm_word & t.nvdm_validated_mask,
    );

    let mut w = World::new(P610, MODEL_A);
    w.boot();
    w.guest.rx_link(&mut w.ram).expect("links");
    w.poke_element(RingId::Status, 0, 16, 8, 0, mangled_mctp);
    w.poke_element(RingId::Status, 0, 16, 8, 4, mangled_nvdm);
    let msgs = w
        .guest
        .recv(&mut w.ram)
        .expect("the driver reads neither SOM/EOM/SEQ nor the NVDM type");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].function, INIT_DONE);
}

// ─────────────────────────────── the isolated algebra ───────────────────────────────

/// ★ Every `msgqRxLink` rejection code, reproduced — and cross-checked against the
/// independent guest implementation, which returns the driver's own numbers.
#[test]
fn every_msgq_rx_link_rejection_code_is_reproduced_exactly() {
    let abi = MsgqAbi {
        version: 0,
        msg_size_min: 16,
        swap_rx_flag: 1,
        region_page_size: 4096,
    };
    let good = TxHeader {
        version: 0,
        size: 0x8000,
        msg_size: 4096,
        msg_count: 7,
        write_ptr: 0,
        flags: 1,
        rx_hdr_off: 32,
        entry_off: 4096,
    };
    assert_eq!(rx_link_check(&good, 32, 0x8000, 4096, &abi), Ok(()));

    let cases: Vec<(RxLinkCode, TxHeader, u32, u32)> = vec![
        (RxLinkCode::MsgSizeBelowMin, good, 0x8000, 8),
        (RxLinkCode::MsgSizeAboveQueue, good, 1024, 4096),
        (
            RxLinkCode::EntryOffTooLarge,
            TxHeader {
                entry_off: 0x8000,
                ..good
            },
            0x8000,
            4096,
        ),
        (
            RxLinkCode::SizeMismatch,
            TxHeader { size: 0, ..good },
            0x8000,
            4096,
        ),
        (
            RxLinkCode::MsgSizeMismatch,
            TxHeader {
                msg_size: 2048,
                ..good
            },
            0x8000,
            4096,
        ),
        (
            RxLinkCode::VersionMismatch,
            TxHeader { version: 1, ..good },
            0x8000,
            4096,
        ),
        (
            RxLinkCode::DerivedFieldsMismatch,
            TxHeader {
                msg_count: 6,
                ..good
            },
            0x8000,
            4096,
        ),
        (
            RxLinkCode::DerivedFieldsMismatch,
            TxHeader {
                rx_hdr_off: 8,
                ..good
            },
            0x8000,
            4096,
        ),
    ];
    for (want, hdr, size, msg_size) in cases {
        assert_eq!(
            rx_link_check(&hdr, 32, size, msg_size, &abi),
            Err(want),
            "code {} not reproduced",
            want.code()
        );
    }

    // ★ The one that matters: `-7` has exactly ONE cause, and it is the signature of a
    // status queue that never received a tx header — a freshly allocated, zeroed queue.
    assert_eq!(RxLinkCode::SizeMismatch.code(), -7);
    let zeroed = TxHeader::default();
    assert_eq!(
        rx_link_check(&zeroed, 32, 0x8000, 4096, &abi),
        Err(RxLinkCode::SizeMismatch),
        "a zeroed status queue fails -7, exactly as the bench observed 71 064 times",
    );
    assert_eq!(RxLinkCode::NullBackingStore.code(), -5, "there is no -4");
}

/// ★ The checksum: folds to zero, and a one-bit flip **anywhere** in the covered range is
/// detected — cross-checked against an independent fold.
#[test]
fn the_checksum_folds_to_zero_and_a_one_bit_flip_anywhere_is_detected() {
    let mut el = vec![0u8; 4096];
    for (i, b) in el.iter_mut().enumerate().take(200) {
        *b = (i % 253) as u8;
    }
    let len = 48 + 40usize;
    // Zero the checksum field, fold, store — the sender's own order.
    el[32..36].copy_from_slice(&0u32.to_le_bytes());
    let sum = checksum32(&el, len);
    el[32..36].copy_from_slice(&sum.to_le_bytes());
    assert_eq!(checksum32(&el, len), 0, "the whole element folds to zero");
    assert_eq!(fold(&el, len), 0, "…and the independent fold agrees");
    assert_ne!(sum, 0, "non-vacuity: the checksum is not trivially zero");

    for bit in 0..(len * 8) {
        let mut bad = el.clone();
        bad[bit / 8] ^= 1 << (bit % 8);
        assert_ne!(
            checksum32(&bad, len),
            0,
            "a flip at bit {bit} went undetected"
        );
    }

    // The fold reads to the next 8-byte boundary past the declared length, which its own
    // comment licenses — so a length that is not a multiple of 8 still covers the tail.
    assert_eq!(checksum32(&[0xffu8; 8], 1), 0xffff_ffff ^ 0xffff_ffff);
    assert_eq!(checksum32(&[0xffu8; 8], 8), 0);
}

/// ★ The ring algebra, against `msgq` line for line — including the aliasing that the
/// producer's `-1` exists to prevent.
#[test]
fn the_ring_algebra_matches_msgq_line_for_line() {
    let n = MsgCount::new(7).unwrap();

    // free = readPtr + msgCount - writePtr - 1, one conditional subtraction.
    assert_eq!(
        free_elements(0, 0, n),
        Ok(6),
        "empty ring: msgCount - 1 free"
    );
    assert_eq!(
        free_elements(0, 6, n),
        Ok(0),
        "msgCount - 1 outstanding: full"
    );
    assert_eq!(free_elements(3, 3, n), Ok(6));
    assert_eq!(free_elements(3, 2, n), Ok(0));

    // available = writePtr + msgCount - readPtr, NO -1.
    assert_eq!(available_elements(0, 0, n), Ok(0), "empty");
    assert_eq!(available_elements(6, 0, n), Ok(6));
    assert_eq!(available_elements(0, 6, n), Ok(1), "wrapped");

    // free + outstanding + 1 == msgCount, for every legal pair.
    for wp in 0..7 {
        for rp in 0..7 {
            let free = free_elements(rp, wp, n).unwrap();
            let outstanding = available_elements(wp, rp, n).unwrap();
            assert_eq!(
                free + outstanding + 1,
                n.get(),
                "conservation fails at wp={wp} rp={rp}",
            );
        }
    }

    // Hostile pointers are named, not clamped.
    assert_eq!(
        free_elements(7, 0, n),
        Err(GspFault::PeerReadPtrOutOfRange { value: 7, count: 7 }),
    );
    assert_eq!(
        available_elements(99, 0, n),
        Err(GspFault::PeerWritePtrOutOfRange {
            value: 99,
            count: 7
        }),
    );
    // And zero is not a divisor one can construct.
    assert_eq!(MsgCount::new(0), Err(GspFault::MsgCountZero));
    assert!(MsgCount::new(1).is_ok());
}

/// ★ Constraint 2's other half — **a version below the floor is refused by name.**
///
/// The C keys its ABI profile on the major version alone and returns the 570 profile for
/// anything unrecognised (`nvkvm_abi.h:105-121`), so an unknown driver silently gets the
/// wrong struct sizes. Here it is a refusal that names the version.
#[test]
fn a_driver_version_below_the_floor_is_refused_by_name() {
    let old = DriverVersion {
        major: 535,
        minor: 0,
        patch: 0,
    };
    assert_eq!(
        versions::table_for(old),
        Err(kayfabe_abi::wire::AbiError::NoTableForVersion {
            major: 535,
            minor: 0,
            patch: 0
        }),
        "no nearest-neighbour fallback: MISS = FAULT",
    );
    // The non-vacuity arm: the bench's own version resolves.
    assert!(versions::table_for(versions::BENCH_DRIVER).is_ok());
}

/// ★ An architecture with no GSP model faults by name rather than serving a default.
#[test]
fn an_arch_without_a_gsp_model_faults_by_name() {
    let arch = NoGspArch::default();
    let mut fsm = GspFsm::new(P580.abi());
    let mut ram = FakeRam::default();
    assert_eq!(
        fsm.mmio_read(&arch, 0, 0x110000),
        Some(Err(GspFault::NoGspModel)),
    );
    assert_eq!(
        fsm.mmio_write(&mut ram, &arch, &mut EchoOk, 0, 0x110000, 2),
        Err(GspFault::NoGspModel),
    );
    // Non-vacuity: with a model, the same read is served.
    let arch = GspArch::new(MODEL_A);
    let (bar, off) = MODEL_A.at(GspReg::GfwBootProgress);
    assert_eq!(fsm.mmio_read(&arch, bar, off), Some(Ok(0xff)));
    // …and an offset no model claims is `None`, never a defaulted zero.
    assert!(fsm.mmio_read(&arch, 0, 0x9999_9999).is_none());
}

/// ★ Constraint 5 — the wire encoding is **little-endian on any host**, and no type's
/// in-memory layout is relied on.
///
/// Asserted against golden bytes rather than against `cfg(target_endian)`, so the test
/// means the same thing on the aarch64 leg of CI as on x86-64.
#[test]
fn the_wire_encoding_is_little_endian_on_any_host() {
    let hdr = TxHeader {
        version: 0,
        size: 0x0004_0000,
        msg_size: 0x1000,
        msg_count: 63,
        write_ptr: 0x0102_0304,
        flags: 1,
        rx_hdr_off: 32,
        entry_off: 0x1000,
    };
    let bytes = hdr.encode();
    assert_eq!(
        bytes.len(),
        32,
        "eight u32s, and sizeof(msgqTxHeader) == 32"
    );
    assert_eq!(
        &bytes[4..8],
        &[0x00, 0x00, 0x04, 0x00],
        "size, little-endian"
    );
    assert_eq!(
        &bytes[16..20],
        &[0x04, 0x03, 0x02, 0x01],
        "writePtr, little-endian",
    );
    assert_eq!(TxHeader::decode(&bytes), Ok(hdr), "round-trip");
    // A truncated header is a named refusal, not a partial decode.
    assert_eq!(
        TxHeader::decode(&bytes[..31]),
        Err(GspFault::Truncated { need: 32, have: 31 }),
    );
    // The reference geometry three independent trees agree on
    // (`msgqTxCreate`'s geometry, identical at both tags: `ogkm-580: msgq.c:237-252`,
    // `ogkm-610: :236-251` — derived; `nv: r535/gsp.c:1164-1172` verbatim).
    assert_eq!((0x0004_0000u32 - 0x1000) / 0x1000, 63);
}

/// ★ Hostile geometry — every scalar the guest publishes, refused by name.
#[test]
fn hostile_geometry_is_refused_by_name() {
    let abi = P580.abi();
    let mut ram = FakeRam::default();
    ram.alloc_range(0x1_0000, 4);

    // A page table whose entries are not page-aligned.
    assert_eq!(
        RegionMap::from_pages(4096, vec![0x1_0001]),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::UnalignedEntry {
                index: 0,
                value: 0x1_0001
            }
        )),
    );
    // A page size that is not a power of two.
    assert!(matches!(
        RegionMap::from_pages(3000, vec![0]),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::BadPageSize { .. }
        )),
    ));
    // An empty region.
    assert!(matches!(
        RegionMap::from_pages(4096, vec![]),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::NoEntries
        )),
    ));
    // The non-vacuity arm.
    let region = RegionMap::from_pages(4096, vec![0x1_0000, 0x3_0000]).expect("valid");
    assert_eq!(region.len(), 8192);
    // An access past the end names the range it refused.
    assert_eq!(
        region.runs(8000, 1000),
        Err(GspFault::RegionOutOfRange {
            offset: 8000,
            len: 1000,
            region_len: 8192
        }),
    );

    // A queue whose flags do not agree to SWAP_RX would deadlock silently; refused.
    let mut w = World::new(P580, MODEL_A);
    w.guest.flags = 0;
    w.guest.write_cmd_header(&mut w.ram);
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    let gpa = w.guest.boot_args_gpa;
    w.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF).unwrap();
    assert_eq!(
        w.wr(GspReg::GspFalconMailbox1, gpa >> 32),
        Err(GspFault::SwapRxNotAgreed { flags: 0 }),
    );

    // A queue that declares zero elements is the C's SIGFPE.
    let mut w = World::new(P580, MODEL_A);
    w.guest.msg_count = 0;
    w.guest.write_cmd_header(&mut w.ram);
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();
    let gpa = w.guest.boot_args_gpa;
    w.wr(GspReg::GspFalconMailbox0, gpa & 0xFFFF_FFFF).unwrap();
    assert_eq!(
        w.wr(GspReg::GspFalconMailbox1, gpa >> 32),
        Err(GspFault::MsgCountZero),
    );
    let _ = abi;
}

/// ★ Hostile *messages* — a corrupt element, a sequence gap, and an impossible length,
/// each refused with the number that caused it.
#[test]
fn hostile_command_elements_are_refused_by_name() {
    // A corrupted element: the guest's checksum no longer folds to zero.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 1, &[1; 16])
        .unwrap();
    let elem = w
        .guest
        .gpa_of(w.guest.cmd_off + u64::from(w.guest.entry_off));
    let mut byte = [0u8; 1];
    kayfabe_gsp::GuestRam::read(&mut w.ram, elem + 60, &mut byte).unwrap();
    byte[0] ^= 0x40;
    kayfabe_gsp::GuestRam::write(&mut w.ram, elem + 60, &byte).unwrap();
    assert!(
        matches!(w.doorbell(), Err(GspFault::ChecksumMismatch { .. })),
        "a corrupt element is refused, not parsed",
    );

    // A sequence gap: the guest's element carries a seqNum we are not expecting.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest.tx_seq = 5;
    w.guest
        .send(&mut w.ram, FN_RM_CONTROL, 1, &[1; 16])
        .unwrap();
    assert_eq!(
        w.doorbell(),
        Err(GspFault::SeqNumGap {
            expected: 0,
            got: 5
        }),
    );

    // An impossible declared length: `rpc.length == 0` passes the driver's own sanity
    // check and then produces garbage upstream, so the lower bound here is the envelope.
    let mut w = World::new(P580, MODEL_A);
    w.boot();
    w.link_and_drain();
    w.guest.send(&mut w.ram, FN_RM_CONTROL, 1, &[]).unwrap();
    let elem = w
        .guest
        .gpa_of(w.guest.cmd_off + u64::from(w.guest.entry_off));
    kayfabe_gsp::GuestRam::write(&mut w.ram, elem + 56, &0u32.to_le_bytes()).unwrap();
    assert!(
        matches!(
            w.doorbell(),
            Err(GspFault::MsgLenOutOfRange {
                declared: 0,
                min: 32,
                ..
            })
        ),
        "rpc.length = 0 is refused at the lower bound, not silently consumed",
    );
}

/// ★ A duplicated function id would make classification answer with whichever arm matched
/// first — a mis-transcribed table that looks like a working one.
#[test]
fn a_duplicated_function_id_is_refused() {
    let mut codes = kayfabe_tests::gspworld::FUNCTIONS;
    assert!(codes.validated().is_ok(), "the real table is distinct");
    codes.set_registry = codes.gsp_set_system_info;
    assert_eq!(
        codes.validated(),
        Err(GspFault::DuplicateFunctionCode { code: 72 }),
    );
}

/// ★ The element layout is validated at construction, so no decode path can be handed a
/// self-inconsistent one.
#[test]
fn an_impossible_element_layout_cannot_be_constructed() {
    use kayfabe_gsp::{ElementLayout, LayoutError};
    // A field outside the header.
    assert_eq!(
        ElementLayout::new(16, 32, 12, None, TransportHdr::None),
        Err(LayoutError::FieldOutsideHeader {
            offset: 32,
            hdr_size: 16
        }),
    );
    // Two fields at the same offset.
    assert_eq!(
        ElementLayout::new(48, 32, 32, None, TransportHdr::None),
        Err(LayoutError::FieldsOverlap { a: 32, b: 32 }),
    );
    // A header too small to hold anything.
    assert_eq!(
        ElementLayout::new(4, 0, 0, None, TransportHdr::None),
        Err(LayoutError::HeaderTooSmall { hdr_size: 4 }),
    );
    // ★ The element count really is written at the layout's own offset. Asserted on OUR
    // encoding rather than on the guest's acceptance, deliberately: the C's comment claims
    // *"the guest reads the element-count from the elemCount field (@40)"*
    // (`C:1561-1592`), but no vendored tree shows a 580 receive path — 610 derives the
    // count from `rpc.length` and has no such field at all — and citing a C comment is
    // citing a belief (two have already turned out false). So we emit it where the layout
    // says, and no test here asserts that a driver requires it. Settling that is open item
    // O4: vendor a 580 tree.
    let run = kayfabe_gsp::encode_message(
        &P580.layout(),
        0x0300_0000,
        4096,
        4096 * 16,
        0,
        &OutgoingRpc {
            function: FN_RM_CONTROL,
            sequence: 1,
            rpc_result: 0,
            rpc_result_private: 0,
            payload: vec![0; 9000],
        },
    )
    .expect("three elements");
    assert_eq!(run.len(), 3 * 4096);
    assert_eq!(
        u32::from_le_bytes(run[40..44].try_into().unwrap()),
        3,
        "elemCount at the 580 layout's offset, and nowhere else",
    );
    assert_eq!(
        u32::from_le_bytes(run[24..28].try_into().unwrap()),
        0,
        "…in particular NOT at 24, which on 610 is rpc.sequence",
    );

    // The non-vacuity arm: both real layouts construct.
    assert!(P580.layout().elem_count_off().is_some());
    assert!(P610.layout().elem_count_off().is_none());
    assert_eq!(P580.layout().hdr_size(), 48);
    assert_eq!(P610.layout().hdr_size(), 16);
}

// ──────────────────── the region, pinned to concrete guest addresses ────────────────────
//
// ★★ Everything below asserts **where a byte lands**, not that a walk happened. The gap
// these close is that the region was exercised for *shape* — fragmented vs linear, refused
// vs accepted — over a single witness geometry, so three address/offset operators and a
// bound could all be wrong for *some* inputs and invisible for the one input tested. The
// remedy is the sweep: several bases (including 0), several page sizes, several run
// lengths, the empty range, the exactly-one-page range, and the byte at each end.

/// ★★ `runs` decomposes a range into the **exact** `(gpa, len)` pairs the page table
/// names — swept over three page sizes and three bases, with both boundaries.
///
/// Every expectation here is a literal, computed by hand from the table. A reference
/// implementation would be the same arithmetic twice and would agree with a wrong answer.
#[test]
fn a_region_decomposes_a_range_into_the_exact_gpas_its_table_names() {
    // ── A. 4 KiB pages, fragmented and out of order.
    let a = RegionMap::from_pages(4096, vec![0x9000, 0x2000, 0x1_5000]).expect("valid");
    assert_eq!(a.len(), 12288);
    assert_eq!(
        a.runs(0, 0),
        Ok(vec![]),
        "an empty range resolves to no runs"
    );
    assert_eq!(a.runs(0, 1), Ok(vec![(0x9000, 1)]), "the very first byte");
    assert_eq!(
        a.runs(0, 4096),
        Ok(vec![(0x9000, 4096)]),
        "exactly one page is one run, and it does not spill",
    );
    assert_eq!(
        a.runs(4095, 2),
        Ok(vec![(0x9FFF, 1), (0x2000, 1)]),
        "a two-byte read straddling a page boundary is two runs of one byte",
    );
    // ★ The killer for a `-`→`+` in the page-remainder: at a non-zero offset *within* a
    // page, with more than a page still to go, `page_size + within` and
    // `page_size - within` are both plausible-looking and only one lands on the boundary.
    assert_eq!(
        a.runs(100, 12188),
        Ok(vec![(0x9064, 3996), (0x2000, 4096), (0x1_5000, 4096),]),
        "the first run stops at the page boundary, 3996 bytes in, not 4196",
    );
    assert_eq!(
        a.runs(4096, 8192),
        Ok(vec![(0x2000, 4096), (0x1_5000, 4096)]),
    );
    assert_eq!(
        a.runs(12287, 1),
        Ok(vec![(0x1_5FFF, 1)]),
        "the very last byte"
    );
    assert_eq!(
        a.runs(12288, 1),
        Err(GspFault::RegionOutOfRange {
            offset: 12288,
            len: 1,
            region_len: 12288
        }),
        "one byte past the end names the range it refused",
    );
    assert_eq!(
        a.runs(0, 12289),
        Err(GspFault::RegionOutOfRange {
            offset: 0,
            len: 12289,
            region_len: 12288
        }),
    );
    assert_eq!(
        a.runs(u64::MAX, 2),
        Err(GspFault::RegionOutOfRange {
            offset: u64::MAX,
            len: 2,
            region_len: 12288
        }),
        "an offset+len that overflows is refused, not wrapped",
    );

    // ── B. 16-byte pages: the same arithmetic at a granularity where an off-by-a-page is
    // an off-by-16, and a run spans three pages.
    let b = RegionMap::from_pages(16, vec![0x1_0000, 0x1_0030, 0x1_0010, 0x1_0080]).expect("valid");
    assert_eq!(b.len(), 64);
    assert_eq!(
        b.runs(4, 40),
        Ok(vec![(0x1_0004, 12), (0x1_0030, 16), (0x1_0010, 12)]),
        "12 + 16 + 12 = 40, and the middle page is whole",
    );
    assert_eq!(
        b.runs(0, 64),
        Ok(vec![
            (0x1_0000, 16),
            (0x1_0030, 16),
            (0x1_0010, 16),
            (0x1_0080, 16),
        ]),
        "a contiguous request over a fragmented region is still one run per page",
    );
    assert_eq!(b.runs(63, 1), Ok(vec![(0x1_008F, 1)]));
    assert_eq!(
        b.runs(0, 65),
        Err(GspFault::RegionOutOfRange {
            offset: 0,
            len: 65,
            region_len: 64
        }),
    );

    // ── C. 256-byte pages, and page 0 lives at guest-physical **zero**. A base of zero is
    // legal and must resolve like any other; a suite that moved every fixture off zero to
    // catch address-blindness must keep one that has not.
    let c = RegionMap::from_pages(256, vec![0, 0x1_0000]).expect("valid");
    assert_eq!(c.len(), 512);
    assert_eq!(c.runs(0, 256), Ok(vec![(0, 256)]));
    assert_eq!(c.runs(1, 300), Ok(vec![(1, 255), (0x1_0000, 45)]));
    assert_eq!(c.runs(511, 1), Ok(vec![(0x1_00FF, 1)]));
}

/// ★★ `read`/`write` place **each byte** on the page the table names, and consume the
/// source buffer in order.
///
/// The property a stalled source cursor breaks is not "a write happened" but "the bytes
/// on page 2 are the bytes that follow the ones on page 1". Asserted against raw guest
/// memory read back at literal addresses, plus the exact `(gpa, len)` the RAM port saw.
#[test]
fn a_region_write_places_each_byte_on_the_page_its_table_names() {
    let mut ram = FakeRam::default();
    for gpa in [0x3_0000u64, 0x1_0000, 0x2_0000] {
        ram.alloc(gpa);
    }
    let region = RegionMap::from_pages(4096, vec![0x3_0000, 0x1_0000, 0x2_0000]).expect("valid");

    // A pattern with no period that divides a page, so a mis-sliced copy cannot happen to
    // land on the right bytes.
    let data: Vec<u8> = (0..12188u32).map(|i| (i % 251) as u8).collect();
    ram.writes.clear();
    ram.reads.clear();
    region.write(&mut ram, 100, &data).expect("in range");

    assert_eq!(
        ram.writes,
        vec![(0x3_0064, 3996), (0x1_0000, 4096), (0x2_0000, 4096)],
        "three writes, at the three addresses the table names, of the three lengths the \
         page boundaries impose",
    );

    let mut back = vec![0u8; 3996];
    kayfabe_gsp::GuestRam::read(&mut ram, 0x3_0064, &mut back).unwrap();
    assert_eq!(back, data[..3996], "page 0 holds the first 3996 bytes");
    let mut back = vec![0u8; 4096];
    kayfabe_gsp::GuestRam::read(&mut ram, 0x1_0000, &mut back).unwrap();
    assert_eq!(
        back,
        data[3996..8092],
        "page 1 holds the bytes that FOLLOW page 0's, not the buffer's start again",
    );
    let mut back = vec![0u8; 4096];
    kayfabe_gsp::GuestRam::read(&mut ram, 0x2_0000, &mut back).unwrap();
    assert_eq!(back, data[8092..12188], "page 2 holds the tail");

    // …and the read path reassembles them in the same order.
    let mut round = vec![0u8; 12188];
    ram.reads.clear();
    region.read(&mut ram, 100, &mut round).expect("in range");
    assert_eq!(
        ram.reads,
        vec![(0x3_0064, 3996), (0x1_0000, 4096), (0x2_0000, 4096)],
    );
    assert_eq!(round, data, "round-trip is byte-exact");

    // A `u32` straddling a page boundary is split at the boundary and reassembled.
    region
        .write_u32(&mut ram, 4094, 0xAABB_CCDD)
        .expect("in range");
    let mut lo = [0u8; 2];
    kayfabe_gsp::GuestRam::read(&mut ram, 0x3_0000 + 4094, &mut lo).unwrap();
    let mut hi = [0u8; 2];
    kayfabe_gsp::GuestRam::read(&mut ram, 0x1_0000, &mut hi).unwrap();
    assert_eq!(lo, [0xDD, 0xCC], "the low half stays on page 0");
    assert_eq!(hi, [0xBB, 0xAA], "the high half starts page 1");
    assert_eq!(region.read_u32(&mut ram, 4094), Ok(0xAABB_CCDD));

    // The whole-region bound, on the write side too.
    assert_eq!(
        region.write(&mut ram, 12288, &[0u8; 1]),
        Err(GspFault::RegionOutOfRange {
            offset: 12288,
            len: 1,
            region_len: 12288
        }),
    );
}

/// ★★ `load` walks the guest's **self-describing** table: the page-size and count
/// predicates are swept, a one-entry table is a legal region, a table that spans several
/// of its own pages is followed through entries it has already read, and a misaligned
/// entry is named by its exact index.
#[test]
fn loading_a_region_walks_its_own_table_and_names_the_entry_that_is_wrong() {
    let mut ram = FakeRam::default();
    ram.alloc_range(0x2_0000, 2);

    // ── The page-size predicate, swept. Zero is one cause; *not a power of two* is the
    // other, and it is a separate one — a table stride of 24 or 4095 is as unusable as a
    // stride of 0, and neither may be silently accepted.
    let one = |ps: u64| RegionMap::load(&mut FakeRam::default(), 0x2_0000, 1, ps, 64);
    for ps in [0u64, 3, 24, 96, 4095, 12288] {
        assert_eq!(
            one(ps),
            Err(GspFault::RegionMalformed(
                kayfabe_gsp::RegionError::BadPageSize { page_size: ps }
            )),
            "page size {ps} is not a legal table stride",
        );
    }

    // ── The count predicates.
    assert_eq!(
        RegionMap::load(&mut ram, 0x2_0000, 0, 4096, 64),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::NoEntries
        )),
    );
    assert_eq!(
        RegionMap::load(&mut ram, 0x2_0000, 65, 4096, 64),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::TooManyEntries {
                declared: 65,
                max: 64
            }
        )),
    );

    // ── ★ The exactly-one-page region. Its table has one entry, which is the page the
    // table itself starts on, and the walk must **stop** there rather than reach for a
    // second table page that the region does not have.
    let mut ram = FakeRam::default();
    ram.alloc(0x7_F000);
    kayfabe_gsp::GuestRam::write(&mut ram, 0x7_F000, &0x7_F000u64.to_le_bytes()).unwrap();
    let solo = RegionMap::load(&mut ram, 0x7_F000, 1, 4096, 64).expect("a one-page region");
    assert_eq!(solo.len(), 4096);
    assert_eq!(solo.runs(0, 4096), Ok(vec![(0x7_F000, 4096)]));
    assert_eq!(
        solo.runs(0, 4097),
        Err(GspFault::RegionOutOfRange {
            offset: 0,
            len: 4097,
            region_len: 4096
        }),
    );

    // ── A table that spans three of its own pages, fragmented, at a 16-byte stride so the
    // walk takes two entries per table page and has to follow the table twice.
    let p: [u64; 5] = [0x5_0000, 0x5_0300, 0x5_0100, 0x5_0080, 0x5_00C0];
    assert_ne!(
        p[1],
        p[0] + 16,
        "non-vacuity: the table is really fragmented"
    );
    let mut ram = FakeRam::default();
    ram.alloc(0x5_0000);
    let put = |ram: &mut FakeRam, at: u64, vals: &[u64]| {
        let mut bytes = Vec::new();
        for v in vals {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        kayfabe_gsp::GuestRam::write(ram, at, &bytes).unwrap();
    };
    put(&mut ram, p[0], &[p[0], p[1]]);
    put(&mut ram, p[1], &[p[2], p[3]]);
    put(&mut ram, p[2], &[p[4]]);
    let walked = RegionMap::load(&mut ram, p[0], 5, 16, 64).expect("a five-page region");
    assert_eq!(walked.len(), 80);
    assert_eq!(
        walked.runs(0, 80),
        Ok(vec![
            (p[0], 16),
            (p[1], 16),
            (p[2], 16),
            (p[3], 16),
            (p[4], 16),
        ]),
        "every page of the region is where its own table said, in table order",
    );

    // ── ★ The reported index of a misaligned entry is that entry's **global** index —
    // swept over the first entry of the first table page, a later entry of the same page,
    // and an entry on the *second* table page, because those three are what tell an
    // index that counts from the batch apart from one that counts twice or multiplies.
    for (bad_index, entries) in [
        (0usize, vec![vec![p[0] + 1, p[1]], vec![p[2], p[3]]]),
        (1, vec![vec![p[0], p[1] + 8], vec![p[2], p[3]]]),
        (2, vec![vec![p[0], p[1]], vec![p[2] + 4, p[3]]]),
        (3, vec![vec![p[0], p[1]], vec![p[2], p[3] + 12]]),
    ] {
        let mut ram = FakeRam::default();
        ram.alloc(0x5_0000);
        put(&mut ram, p[0], &entries[0]);
        put(&mut ram, p[1], &entries[1]);
        let value = entries[bad_index / 2][bad_index % 2];
        assert_eq!(
            RegionMap::load(&mut ram, p[0], 4, 16, 64),
            Err(GspFault::RegionMalformed(
                kayfabe_gsp::RegionError::UnalignedEntry {
                    index: bad_index,
                    value
                }
            )),
            "entry {bad_index} is the one that is wrong",
        );
    }

    // The same index discipline on the direct constructor.
    assert_eq!(
        RegionMap::from_pages(16, vec![p[0], p[1], p[2] + 4, p[3]]),
        Err(GspFault::RegionMalformed(
            kayfabe_gsp::RegionError::UnalignedEntry {
                index: 2,
                value: p[2] + 4
            }
        )),
    );

    // ── A table the guest declared but did not back is a RAM refusal, not a zero-filled
    // region.
    let mut empty = FakeRam::default();
    assert_eq!(
        RegionMap::load(&mut empty, 0x9_0000, 1, 4096, 64),
        Err(GspFault::GuestRam(kayfabe_gsp::RamRefused {
            gpa: 0x9_0000,
            len: 8
        })),
    );
}

/// ★ `peek_len` names the exact byte count a short first element denied it — on both
/// element layouts, whose header sizes differ, so the number cannot be a constant.
#[test]
fn peek_len_on_a_short_element_names_the_exact_byte_it_needed() {
    for p in [P580, P610] {
        let layout = p.layout();
        let hdr = layout.hdr_size();
        // `rpc.length` is the third word of the envelope: hdr + 2*4, and the read needs
        // four bytes from there.
        let need = hdr + 12;
        for have in [0usize, 4, hdr, hdr + 8, need - 1] {
            assert_eq!(
                kayfabe_gsp::peek_len(&layout, &vec![0u8; have], 4096, 65536),
                Err(GspFault::Truncated { need, have }),
                "{}: {have} bytes is short of {need}",
                p.name,
            );
        }

        // The boundary from the other side: exactly `need` bytes is enough, and the value
        // read is the one at that offset and no other.
        let mut first = vec![0u8; need];
        first[hdr + 8..hdr + 12].copy_from_slice(&96u32.to_le_bytes());
        let len = kayfabe_gsp::peek_len(&layout, &first, 4096, 65536).expect("exactly enough");
        assert_eq!(len.rpc_length(), 96, "the declared length, read verbatim");
        assert_eq!(
            len.msg_len(),
            hdr as u32 + 96,
            "the checksum's coverage is the header plus the declared length",
        );
        assert_eq!(len.elements(), 1);

        // …and a length outside the transport's bounds is refused with both bounds.
        first[hdr + 8..hdr + 12].copy_from_slice(&31u32.to_le_bytes());
        assert_eq!(
            kayfabe_gsp::peek_len(&layout, &first, 4096, 65536),
            Err(GspFault::MsgLenOutOfRange {
                declared: 31,
                min: 32,
                max: 65536 - hdr as u32
            }),
        );
        first[hdr + 8..hdr + 12].copy_from_slice(&(65536 - hdr as u32 + 1).to_le_bytes());
        assert_eq!(
            kayfabe_gsp::peek_len(&layout, &first, 4096, 65536),
            Err(GspFault::MsgLenOutOfRange {
                declared: 65536 - hdr as u32 + 1,
                min: 32,
                max: 65536 - hdr as u32
            }),
        );
    }
}

/// ★ A run sized **exactly** to its declared message decodes; one byte less is refused.
///
/// The transport reads whole elements, so the exact-fit case never arises on the bench —
/// which is precisely why the bound between "enough" and "one short" was never pinned.
#[test]
fn a_run_sized_exactly_to_its_message_decodes_and_one_byte_less_does_not() {
    for p in [P580, P610] {
        let layout = p.layout();
        let hdr = layout.hdr_size();
        let payload: Vec<u8> = (0..40u32).map(|i| (i * 7 % 251) as u8).collect();
        let run = kayfabe_gsp::encode_message(
            &layout,
            0x0300_0000,
            4096,
            65536,
            9,
            &OutgoingRpc {
                function: FN_RM_CONTROL,
                sequence: 3,
                rpc_result: 0,
                rpc_result_private: 0,
                payload: payload.clone(),
            },
        )
        .expect("one element");
        assert_eq!(
            run.len(),
            4096,
            "{}: the wire form is a whole element",
            p.name
        );

        let len = kayfabe_gsp::peek_len(&layout, &run, 4096, 65536).expect("declared");
        let msg_len = hdr + 32 + payload.len();
        assert_eq!(len.msg_len() as usize, msg_len);

        let decoded = kayfabe_gsp::decode_message(&layout, &run[..msg_len], len, 9, p.table())
            .expect("a run of exactly msg_len bytes is complete");
        assert_eq!(decoded.seq_num, 9);
        assert_eq!(decoded.elements, 1);
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.envelope.function, FN_RM_CONTROL);
        assert_eq!(decoded.envelope.sequence, 3);
        assert_eq!(decoded.envelope.length, 32 + payload.len() as u32);
        assert_eq!(decoded.envelope.payload_len, payload.len());
        assert_eq!(
            decoded,
            kayfabe_gsp::decode_message(&layout, &run, len, 9, p.table()).expect("padded"),
            "the padded element and the exact-fit run decode identically",
        );

        assert_eq!(
            kayfabe_gsp::decode_message(&layout, &run[..msg_len - 1], len, 9, p.table()),
            Err(GspFault::Truncated {
                need: msg_len,
                have: msg_len - 1
            }),
            "{}: one byte short is a named refusal",
            p.name,
        );
    }
}

/// ★ The two `msgqRxLink` size boundaries, from both sides.
///
/// `-3` and `-6` are adjacent predicates over the same three numbers, and the equality
/// case of each is the one geometry the driver actually produces: a message exactly as
/// large as its queue, and a queue that holds exactly one message flush to its end.
#[test]
fn the_rx_link_size_predicates_are_exact_at_their_boundaries() {
    let abi = MsgqAbi {
        version: 0,
        msg_size_min: 16,
        swap_rx_flag: 1,
        region_page_size: 4096,
    };
    let good = TxHeader {
        version: 0,
        size: 0x8000,
        msg_size: 4096,
        msg_count: 7,
        write_ptr: 0,
        flags: 1,
        rx_hdr_off: 32,
        entry_off: 4096,
    };

    // `-3` is `msgSize > size`, strictly. At equality the check passes and the *next*
    // predicate is the one that speaks.
    assert_eq!(
        rx_link_check(&good, 32, 4096, 4096, &abi),
        Err(RxLinkCode::EntryOffTooLarge),
        "msgSize == size is not yet -3; entryOff is what makes it impossible",
    );
    assert_eq!(
        rx_link_check(&good, 32, 4095, 4096, &abi),
        Err(RxLinkCode::MsgSizeAboveQueue),
        "one byte smaller and it is -3",
    );

    // `-6` is `size < entryOff + msgSize`, strictly. A queue whose elements end exactly at
    // its last byte is legal — refusing it would reject the tightest legal ring.
    let tight = TxHeader {
        size: 8192,
        msg_count: 1,
        ..good
    };
    assert_eq!(
        rx_link_check(&tight, 32, 8192, 4096, &abi),
        Ok(()),
        "entryOff + msgSize == size exactly: one element, flush to the end",
    );
    let over = TxHeader {
        size: 8191,
        msg_count: 1,
        ..good
    };
    assert_eq!(
        rx_link_check(&over, 32, 8191, 4096, &abi),
        Err(RxLinkCode::EntryOffTooLarge),
        "one byte less and the element does not fit",
    );
}

/// ★★ A binding publishes the status queue at **position zero**, whatever the guest's own
/// command queue was at — and every offset it derives is a literal this test names.
///
/// The published header is the guest's header with `writePtr` reset. Carrying the guest's
/// value across instead would start our producer mid-ring at a position the peer's
/// `readPtr` does not agree with, which is silent: every `msgqRxLink` check still passes.
#[test]
fn a_binding_publishes_the_status_queue_at_position_zero() {
    let abi = MsgqAbi {
        version: 0,
        msg_size_min: 16,
        swap_rx_flag: 1,
        region_page_size: 4096,
    };
    // Sixteen pages in descending order: linear addressing would resolve every offset
    // above the first page to the wrong place.
    let pages: Vec<u64> = (0..16u64).map(|i| 0x10_0000 + (15 - i) * 4096).collect();
    let mut ram = FakeRam::default();
    for &gpa in &pages {
        ram.alloc(gpa);
    }
    let region = RegionMap::from_pages(4096, pages.clone()).expect("valid");

    let guest = TxHeader {
        version: 0,
        size: 0x8000,
        msg_size: 4096,
        msg_count: 7,
        write_ptr: 5,
        flags: 1,
        rx_hdr_off: 32,
        entry_off: 4096,
    };
    region
        .write(&mut ram, 0, &guest.encode())
        .expect("in range");
    // Non-vacuity: the value we are asserting gets reset is really in guest memory.
    let mut readback = [0u8; 32];
    kayfabe_gsp::GuestRam::read(&mut ram, pages[0], &mut readback).unwrap();
    assert_eq!(TxHeader::decode(&readback), Ok(guest));
    assert_ne!(guest.write_ptr, 0);

    let geom = kayfabe_gsp::MsgqGeometry::bind(&mut ram, region, 0, 0x8000, &abi)
        .expect("a legal geometry");
    assert_eq!(
        geom.published_header(),
        TxHeader {
            write_ptr: 0,
            ..guest
        },
        "writePtr is reset and every other field is the guest's own",
    );
    assert_eq!(geom.msg_count().get(), 7);
    assert_eq!(geom.element_size(), 4096);

    // The four pointer offsets, as literals — and the C's own bug is the last of them:
    // acknowledging our consumption into the *command* queue's rx header instead of the
    // status queue's left the guest seeing zero free space.
    assert_eq!(geom.cmd_write_ptr_off(), 16);
    assert_eq!(geom.stat_write_ptr_off(), 0x8010);
    assert_eq!(geom.peer_stat_read_ptr_off(), 32);
    assert_eq!(geom.cmd_read_ptr_ack_off(), 0x8020);
    assert_ne!(
        geom.cmd_read_ptr_ack_off(),
        geom.peer_stat_read_ptr_off(),
        "the two read pointers are in different backing stores",
    );
    let slot3 = geom.msg_count().slot(3);
    assert_eq!(geom.cmd_element_off(slot3), 4096 + 3 * 4096);
    assert_eq!(geom.stat_element_off(slot3), 0x8000 + 4096 + 3 * 4096);

    // …and each of those offsets resolves through the table, not linearly.
    assert_eq!(
        geom.region().runs(geom.stat_write_ptr_off(), 4),
        Ok(vec![(pages[8] + 16, 4)]),
        "the status queue's writePtr is on page 8 of the region, wherever that is",
    );
    assert_eq!(
        geom.region().runs(geom.cmd_element_off(slot3), 4096),
        Ok(vec![(pages[4], 4096)]),
    );
}
