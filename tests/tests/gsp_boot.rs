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
    fold,
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
    // WPR2 already up" (`ogkm: kernel_gsp.c:4805-4809`).
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
    // MCTP validation refuses (`ogkm: message_queue_cpu.c:737-759`).
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
/// ogkm writes lo then hi (`ogkm: kernel_gsp_tu102.c:392-403`) and the C keys the whole
/// handshake on the `MAILBOX1` write (`C:4298-4302`) — but that is a write *order*, not a
/// protocol guarantee, and a trigger keyed on one half is a trigger that fires with a
/// stale partner. This drives the halves in the **reverse** order.
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
/// A re-link resets the *position* (`msgqRxLink` sets `rxReadPtr = 0`, `ogkm: msgq.c:435`)
/// and nothing anywhere assigns `rxSeqNum` — it is only `++`'d
/// (`ogkm: message_queue_cpu.c:836`). The C learned this the expensive way: zeroing the
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
}

/// ★ GSP-D4, as a **negative trace**: the passing condition is that we *differ* from the C.
///
/// The C parses whatever its stale `q_*` fields point at and answers `NV_OK` — 508 log
/// lines of `cmd fn=1959520414 seq=4055862830 -> echo NV_OK` on the measured run. Here the
/// same doorbell is one named refusal, zero elements posted, and — the part that makes it
/// a security property rather than a behaviour — **zero guest-RAM reads**.
#[test]
fn a_doorbell_on_an_unbound_queue_refuses_by_name_and_reads_zero_guest_ram() {
    let mut w = World::new(P580, MODEL_A);
    let m = w.arch.model();
    w.wr(GspReg::GspFalconCpuctl, m.startcpu()).unwrap();

    // Fill the (unbound) guest memory with plausible-looking garbage, as a stale queue
    // would be full of.
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
/// fn-47 is `_issueRpcAndWait` (`ogkm: rpc.c:9146-9170`), so the reply comes **first**;
/// only then may `MAILBOX0` report the suspend sentinel the close poll is waiting on
/// (`ogkm: kernel_gsp_tu102.c:352-357`). A fault-and-stop posture on this path hangs the
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
/// The poll runs without the API lock and accepts eight functions
/// (`ogkm: kernel_gsp.c:1419-1440`); `POST_EVENT` is not one of them. The gate here is the
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
/// `ogkm: message_queue_cpu.c:768-782` handles only `<`. There is no recovery for `>`, and
/// `rxSeqNum++` happens anyway at `:836`, so the two streams stay one apart forever.
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
    // full becomes indistinguishable from empty (`ogkm: msgq.c:490`, the `-1`).
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
    // the bound is `msgCount - 1` outstanding — `ogkm: msgq.c:490`'s `-1`, which is what
    // keeps a full ring distinguishable from an empty one. Corrected rather than relaxed.
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
/// store (`ogkm: msgq.c:416-419`), and ours is the status queue.
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
    w.fsm.device_reset();
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
    // (`ogkm: msgq.c:236-251` derived, `nv: r535/gsp.c:1164-1172` verbatim).
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
