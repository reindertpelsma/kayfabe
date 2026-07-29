//! ★★★ **Rung 2 of the definition of done**: replay the C's recorded bytes against the
//! Rust GSP and find where we diverge.
//!
//! `cap1_coldboot_hermetic` — 359 062 records of a **stock** 580.159.04 open guest cold-
//! booting against the C emulator on a real GA106, through PCI enumerate → VBIOS stream →
//! FWSEC/WPR2 → LibOS boot args → msgq handshake → `GSP_INIT_DONE` → `nvidia-smi -q` →
//! teardown. The C is a second implementation of the same contract and the only one a real
//! NVIDIA driver has ever accepted end to end. The Rust cannot bless itself against it.
//!
//! Everything here is **measured**, and every number below was produced by the harness
//! before it was written down. `cargo run -p kayfabe-crec --example cap1_report` prints the
//! same run in full.
//!
//! ## What this file asserts, in one paragraph
//!
//! Through the cold boot, the queue bind, `GSP_INIT_DONE` and four RPC round-trips, the
//! Rust GSP reproduces the C **exactly** in decoded projection — every one of the 498 GSP
//! register reads in that span, the published tx header, the status write pointers, the
//! command read-pointer acknowledgements — with **nine** divergences, of which **one** is
//! a ledger row (GSP-D1) and **eight** are four distinct findings. Past that point the
//! capture stops being able to answer, and the reason is itself a finding: the C's
//! guest-RAM read set is a strict subset of ours in three independent places, each of
//! which is one of the ledger's own rows. (Beyond the limit the register plane diverges in
//! exactly one place, `MAILBOX0`'s suspend sentinel, and that is a *consequence* of the
//! limit — fn-47 is one of the 173 commands the capture can no longer carry us to.)
//!
//! ## ★★ The findings, named
//!
//! | # | what | evidence | which side is wrong |
//! |---|---|---|---|
//! | **F-1** | `cap1` contains exactly **one** interrupt raise in 359 062 records, and it is the driver's own `INTR_LEAF_TRIGGER` self-test, **not** a GSP SWGEN0. The C posts 202 status elements and announces **none** of them. | the single `IrqRaise` record follows a write of `0x81` to `0xb81640`; `nvkvm_gsp_raise_swgen0` is reachable only from `nvkvm_gsp_deliver_events`, which returns immediately with no os-event registered, and no CUDA process runs in `cap1` | neither — it *works*, because the guest polls (`kgspWaitForRmInitDone`). But it means this capture constrains the interrupt plane **not at all**, which sharpens the pre-registered limit that attributed the raise to `INIT_DONE`. |
//! | **F-2** | We publish a command read-pointer acknowledgement **on the bind**; the C publishes none until the first doorbell. | `kayfabe_gsp::boot`'s B4 drain-on-publish: at 580 the ring is already non-empty at bind time (`writePtr = 2`, and the capture shows exactly that in the guest's own tx header), because `kgspQueueAsyncInitRpcs_IMPL` runs before `_kgspBootGspRm` | **ours is better and the capture proves the premise** — `writePtr = 2` at bind is in the artifact. The C recovers only because the guest rings the door again immediately. |
//! | **F-3** | We ask the shell to announce the status queue on the bind and on **every** subsequent service; the C announces nothing. | `GspFsm::post` latches `swgen0_pending` and `service_command_queue` re-raises while it is set; E10 clears it, and `cap1` contains **zero** `IRQSCLR` writes | **undetermined, and the capture cannot decide it** — the guest never wrote `IRQSCLR` because it never received an interrupt to clear. A guest that does clear would see one edge per batch; a guest that does not would see one per doorbell. |
//! | **F-4** | Two replies match on every field a guest matches on — slot, `seqNum`, `function`, `sequence`, `rpc_result`, `rpc.length` — and differ in the **body**. | `GET_GSP_STATIC_INFO` (fn 65) and one `GSP_RM_CONTROL` (fn 76). The C does **not** echo these: `C: nvkvm_gpu_emul.c:3434-3452` splices a captured GA106 `GspStaticConfigInfo` blob into the reply | **the documentation is wrong, on our side.** `kayfabe_gsp::EchoOk`'s doc says the C echoes *"every command that is not on the async list"* citing `C:2410-2416`. The capture shows that is incomplete: the C models at least fn 65 with fabricated content, and fn 76 replies are built from the control's own response size rather than echoed. `EchoOk` is a faithful *default*, not a faithful *model of the C*. |
//!
//! ## ★★★ And the structural one — why `cap1` cannot be closed
//!
//! Three of the ledger's rows say the C reads **less** guest memory than a correct
//! implementation must, and a hermetic capture can only answer reads its subject made:
//!
//! - **GSP-D8** — the C computes `sharedMemPhysAddr + offset` and never reads the region's
//!   page table. Our bind does. Without a named reconstruction the replay stops **at the
//!   bind**.
//! - **GSP-D2** — the C has no flow control and never reads the peer's status-queue read
//!   pointer. Our `post` does, on every post.
//! - **GSP-D6** — the C reads element 0 of a multi-element command and advances past the
//!   continuation elements without reading them. Our receive reads all of them. The
//!   capture therefore contains **no observation at all** of command slots 7 and 8 at the
//!   moment they were live, and the ring has since been rewritten.
//!
//! The first two are reconstructible under stated assumptions ([`ReconKind`]). **The third
//! is not**, and it is the closure limit: the first multi-element command in the capture.

use kayfabe_crec::format::CKind;
use kayfabe_crec::{
    Answer, CTrace, Fill, Note, ReconKind, Replay, ReplayResult, Verdict, bench_abi, cap1_path,
    census, load_cap1,
};
use kayfabe_gsp::{BootPhase, GspFault, Observation, Transition};
use kayfabe_trace::{TraceEvent, diff};

fn cap1() -> CTrace {
    match load_cap1() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1 at {:?} did not decode: {e:?}", cap1_path()),
        Err(e) => panic!("cap1 is missing at {:?} ({e})", cap1_path()),
    }
}

fn run(fill: Fill) -> (CTrace, ReplayResult) {
    let t = cap1();
    let r = Replay::new(&t, bench_abi()).run(fill);
    (t, r)
}

// ═══════════════════════════ the replay actually happens ═══════════════════════════

#[test]
fn the_boot_fsm_is_driven_all_the_way_through_by_the_recorded_guest() {
    // ★ Non-vacuity first. A replay that passed because it never reached the transition it
    // claims to cover is the classic vacuous protocol test, so the transitions are the
    // assertion and not a side effect.
    let (_t, r) = run(Fill::Reconstructed);
    for want in [
        Transition::E12, // doorbells before anything is bound — the healthy 580 boot order
        Transition::E1,  // GSP STARTCPU: FWSEC ran, WPR2 up
        Transition::E5,  // SEC2 Booter Load
        Transition::E6,  // the boot-args mailbox pair completed: bind + INIT_DONE
        Transition::E7,  // a doorbell serviced against a live binding
        Transition::E4,  // SEC2 Booter Unload: WPR2 down, Halted
    ] {
        assert!(
            r.transitions_seen.contains(&want),
            "{want:?} never fired; transitions were {:?}",
            r.transitions_seen
        );
    }
    assert_eq!(
        r.final_phase,
        BootPhase::Halted,
        "the capture ends with the Booter Unload, so the FSM must end torn down"
    );
    // The three planes: ours, and the two this differential does not cover.
    assert_eq!(r.txns.len(), 1955, "transactions whose register we decode");
    assert_eq!(
        (
            r.unprojected.guest_writes,
            r.unprojected.irqs,
            r.unprojected.guest_reads
        ),
        (5, 1, 61),
        "the channel/pushbuffer plane and the CPU interrupt tree — reported as a number, \
         never silently dropped"
    );
}

#[test]
fn every_gsp_register_read_within_the_oracles_reach_is_served_exactly_as_the_c_served_it() {
    // ★★ The strongest positive claim in this file, and it is a *decoded* one: register
    // reads across GFW boot, both falcons' CPUCTL/HWCFG2/DMATRFCMD, the RISC-V ACTIVE bit,
    // both WPR2 halves and the SEC2 Booter mailbox. Every value the guest saw, we produce
    // — and the GA10x model that produces them was written from `ogkm-580`'s swref headers
    // and the C's arch header, never from the served values in this capture.
    let (_t, r) = run(Fill::Reconstructed);
    let limit = r.closure_limit.expect("this capture has one");
    let only_registers = |p: &kayfabe_crec::Projected, before: bool| -> Vec<TraceEvent> {
        p.events
            .iter()
            .zip(&p.notes)
            .zip(&p.txn)
            .filter(|((_, n), t)| matches!(n, Note::Register(_)) && ((**t < limit) == before))
            .map(|((e, _), _)| e.clone())
            .collect()
    };
    let c = only_registers(&r.c, true);
    let rust = only_registers(&r.rust, true);
    assert_eq!(c.len(), 498, "non-vacuity: the register plane was measured");
    assert_eq!(diff(&c, &rust), None, "a GSP register served differently");
    assert_eq!(c.len() + only_registers(&r.c, false).len(), 910);

    // ★ Beyond the closure limit the registers diverge in exactly ONE place, and it is a
    // *consequence* of the limit rather than an independent finding: `MAILBOX0` must read
    // back the suspend sentinel once fn-47 has been serviced, and fn-47 is one of the 173
    // commands the capture can no longer carry us to. So we still echo the boot-args low
    // half where the C reports `0x80000000`. Asserting which offsets differ — rather than
    // that none do — is what keeps this from being an unexamined caveat.
    let after_c = only_registers(&r.c, false);
    let after_rust = only_registers(&r.rust, false);
    let differing: Vec<u64> = after_c
        .iter()
        .zip(&after_rust)
        .filter(|(a, b)| a != b)
        .filter_map(|(a, _)| match a {
            TraceEvent::MmioRead { off, .. } => Some(*off),
            _ => None,
        })
        .collect();
    assert_eq!(
        differing,
        vec![0x0011_0040],
        "only NV_PGSP_FALCON_MAILBOX0, and only once"
    );
    assert!(
        matches!(
            after_c[differing_index(&after_c, &after_rust)],
            TraceEvent::MmioRead {
                val: 0x8000_0000,
                ..
            }
        ),
        "the C reports the suspend sentinel, whole and not OR-ed"
    );
}

/// Index of the first differing position between two equal-length register streams.
fn differing_index(a: &[TraceEvent], b: &[TraceEvent]) -> usize {
    diff(a, b)
        .expect("the caller has already established there is one")
        .at
}

// ═══════════════════ the oracle's reach, measured rather than assumed ═══════════════

#[test]
fn without_a_named_reconstruction_the_capture_stops_at_the_bind_and_gsp_d8_is_why() {
    // ★★★ The hermeticity measurement. `cap1` is hermetic — nothing but the emulator ever
    // wrote guest memory — and it still cannot answer our bind, because the C never
    // performed the read: GSP-D8, `sharedMemPhysAddr + offset` instead of the region's own
    // page table. A hermetic capture answers the reads its subject made, and no others.
    let (_t, r) = run(Fill::Observed);
    let (txn, first) = *r.unobserved.first().expect("the strict run must run out");
    assert_eq!(
        r.closure_limit,
        Some(txn),
        "the closure limit is where the capture stopped being able to answer"
    );
    assert_eq!(
        r.txns[txn].reg,
        Some(kayfabe_arch::gsp::GspReg::GspFalconMailbox1),
        "it is the second boot-args mailbox half — i.e. E6, the bind"
    );
    assert_eq!(
        (first.gpa, first.len),
        (0x1_2720_0000, 1032),
        "`RegionMap::load` reading pageTableEntryCount=129 entries at sharedMemPhysAddr"
    );
    assert_eq!(
        r.txns[txn].refusal,
        Some(GspFault::GuestRam(kayfabe_gsp::RamRefused {
            gpa: 0x1_2720_0000,
            len: 1032
        })),
        "and it refuses by name rather than reading zeros"
    );
    // Non-vacuity for the strict mode itself: it did answer everything before that.
    assert_eq!(
        r.answers,
        vec![(Answer::Observed, 9), (Answer::Unobserved, 1)],
        "eight LibOS region entries and the init-args struct, then the wall"
    );
}

#[test]
fn exactly_two_reconstructions_carry_the_replay_past_the_bind_and_both_are_ledger_rows() {
    // Each one is a **finding**, not a configuration: it is a value the capture cannot
    // contain, supplied under an assumption the harness has to name out loud.
    let (_t, r) = run(Fill::Reconstructed);
    assert_eq!(
        r.reconstructions
            .iter()
            .map(|x| (x.gpa, x.len, x.kind))
            .collect::<Vec<_>>(),
        vec![
            // GSP-D8.
            (0x1_2720_0000, 1032, ReconKind::RegionPageTable),
            // GSP-D2 — `cmdQueueBase + rxHdrOff`, the one word the C's lack of flow
            // control means it never reads. Note it sits ONE BYTE past the end of the
            // 32-byte tx-header read the C *does* make.
            (0x1_2720_1020, 4, ReconKind::PeerStatusReadPtr),
        ]
    );
    // ★ And the bound that keeps lookahead honest: every sound use in this run is well
    // inside it. Without the bound the oracle answered a command-slot read with an
    // observation 157 677 records later and the run died on the guest's own checksum.
    assert!(
        r.max_lookahead <= kayfabe_crec::oracle::LOOKAHEAD_LIMIT,
        "max lookahead {} exceeded the bound",
        r.max_lookahead
    );
    assert_eq!(r.max_lookahead, 2373);
}

#[test]
fn the_closure_limit_is_the_first_multi_element_command_and_gsp_d6_is_why() {
    // ★★★ The structural result. The C reads element 0 of a multi-element command and
    // advances its read pointer past the continuation elements **without reading them**
    // (GSP-D6). So the capture contains no observation of those elements while they were
    // live, the ring has since been rewritten, and no assumption reconstructs a payload.
    let (t, r) = run(Fill::Reconstructed);
    assert_eq!(r.closure_limit, Some(978));
    let (txn, first) = *r.unobserved.first().expect("the run must reach the wall");
    assert_eq!(txn, 978);
    assert_eq!(
        (first.gpa, first.len),
        (0x1_2720_9000, 4096),
        "command ring slot 7 — the first continuation element of the first \
         multi-element message"
    );

    // ★ Cross-checked against the artifact itself, not against our own belief: the C's own
    // read of element 0 declares a three-element message.
    let elem = t
        .records()
        .iter()
        .find(|rec| rec.kind == CKind::GuestRead && rec.a == 0x1_2720_8000)
        .expect("the C read the first element");
    let w = |o: usize| u32::from_le_bytes(elem.payload[o..o + 4].try_into().unwrap());
    assert_eq!(w(40), 3, "elemCount, at the 580 element layout's offset 40");
    assert_eq!(
        w(56),
        8276,
        "rpc.length — 48 + 8276 = 8324 bytes = 3 elements"
    );
    assert_eq!(w(60), 76, "GSP_RM_CONTROL");
    // And the C never read the continuations, at that moment or in that generation.
    let reads_of_slot7: Vec<usize> = t
        .records()
        .iter()
        .enumerate()
        .filter(|(_, rec)| rec.kind == CKind::GuestRead && rec.a == 0x1_2720_9000)
        .map(|(i, _)| i)
        .collect();
    assert!(
        reads_of_slot7.iter().all(|i| *i > 200_000),
        "every observation of slot 7 is >150 000 records later — a different generation \
         of the same ring slot, which is exactly why lookahead must be bounded: {reads_of_slot7:?}"
    );
}

// ══════════════════════════════ the divergence census ══════════════════════════════

#[test]
fn the_global_positional_diff_is_not_green_and_a_green_one_would_be_the_bug() {
    // §6.3 as literally specified: one positional diff over the whole decoded projection.
    // It is *supposed* to fail. The C has no refusal vocabulary and echoes NV_OK for
    // essentially everything, so every MUST-DIFFER row is a position where the C emits a
    // positive event and we emit a refusal — a green diff here would mean we had
    // reproduced the C's defects.
    let (_t, r) = run(Fill::Reconstructed);
    let d = diff(&r.c.events, &r.rust.events).expect("a green end-to-end diff IS the bug");
    assert_eq!(
        d.at, 255,
        "the first position where the two implementations part"
    );
    assert_eq!(
        r.c.txn[255], 492,
        "and it is inside E6 — the bind, where GSP-D1 lives"
    );
}

#[test]
fn within_the_oracles_reach_the_census_is_one_ledger_row_and_four_findings() {
    // ★★ The deliverable. Nine divergences before the closure limit; everything else in
    // the projected reach matched. Each is itemised so a reviewer can check the
    // classification rather than trust a count, and so that a change to either
    // implementation moves a line here rather than a number.
    let (_t, r) = run(Fill::Reconstructed);
    let c = census(&r);

    assert_eq!(
        c.by_id(),
        vec![("GSP-D1", 1), ("—", 8)],
        "in reach: one ledger row, eight findings"
    );
    assert_eq!(
        c.beyond_closure(),
        544,
        "beyond the closure limit the C keeps going and we cannot; counted, not \
         interpreted"
    );

    let brief: Vec<(usize, usize, String, String)> = c
        .unexplained()
        .iter()
        .map(|i| (i.txn, i.at, tag(i.c.as_ref()), tag(i.rust.as_ref())))
        .collect();
    let s = |t: usize, a: usize, x: &str, y: &str| (t, a, x.to_string(), y.to_string());
    assert_eq!(
        brief,
        vec![
            // F-2 — B4 drain-on-publish: the 580 command ring is already non-empty at bind
            // time (the guest's own tx header in this capture says writePtr = 2).
            s(492, 3, "-", "ReadPtrAcked"),
            // F-3 — we announce the status queue; the C announces nothing, ever.
            s(492, 4, "-", "Irq"),
            s(974, 3, "-", "Irq"),
            // F-4 — the reply BODY differs where every matched field agrees. fn 65:
            // the C splices a captured GspStaticConfigInfo (`C:3434-3452`).
            s(975, 0, "ElementPosted", "ElementPosted"),
            s(975, 3, "-", "Irq"),
            s(976, 3, "-", "Irq"),
            // F-4 again, fn 76: a control reply is sized by the control, not echoed.
            s(977, 0, "ElementPosted", "ElementPosted"),
            s(977, 3, "-", "Irq"),
        ]
    );

    // F-4, stated precisely: everything a guest matches on agrees; only the body differs.
    let (Some(Note::Decoded(cc)), Some(Note::Decoded(rr))) =
        (&c.unexplained()[3].c, &c.unexplained()[3].rust)
    else {
        panic!("the fn-65 reply divergence is two decoded elements")
    };
    let fields = |o: &Observation| match o {
        Observation::ElementPosted {
            slot,
            seq_num,
            function,
            sequence,
            rpc_result,
            rpc_length,
            payload_digest,
        } => (
            *slot,
            *seq_num,
            *function,
            *sequence,
            *rpc_result,
            *rpc_length,
            *payload_digest,
        ),
        other => panic!("expected an element, got {other:?}"),
    };
    let (a, b) = (fields(cc), fields(rr));
    assert_eq!(a.0..=a.5, b.0..=b.5, "slot..rpc_length agree");
    assert_eq!((a.2, a.5), (65, 1824), "GET_GSP_STATIC_INFO, 1824 bytes");
    assert_ne!(a.6, b.6, "and only the body digest differs");
}

#[test]
fn gsp_d1_is_rediscovered_in_the_artefact_rather_than_asserted() {
    // ★★ The ledger's first row says the C writes `rpc.length = 36` for a bare 32-byte
    // header. This finds it in 359 062 recorded bytes without being told where: the
    // `GSP_INIT_DONE` the C posts on the bind. Non-vacuity for the whole classifier — a
    // ledger nothing ever matches is decoration.
    let (_t, r) = run(Fill::Reconstructed);
    let c = census(&r);
    let hits: Vec<_> = c
        .items
        .iter()
        .filter(|i| matches!(i.verdict, Verdict::Expected(d) if d.id == "GSP-D1"))
        .collect();
    assert_eq!(hits.len(), 1);
    let (Some(Note::Decoded(cc)), Some(Note::Decoded(rr))) = (&hits[0].c, &hits[0].rust) else {
        panic!("GSP-D1 is a divergence between two decoded elements")
    };
    let len = |o: &Observation| match o {
        Observation::ElementPosted {
            function,
            rpc_length,
            ..
        } => (*function, *rpc_length),
        other => panic!("expected an element, got {other:?}"),
    };
    assert_eq!(
        len(cc),
        (4097, 36),
        "the C: GSP_INIT_DONE declaring 36 bytes"
    );
    assert_eq!(len(rr), (4097, 32), "us: the envelope's real size");
    let row = match hits[0].verdict {
        Verdict::Expected(d) => d,
        Verdict::Unexplained => unreachable!(),
    };
    assert!(
        row.independent_oracle.contains("g_rpc-message-header.h"),
        "the row must carry an oracle that is not us"
    );
}

// ═════════════════ F-1: the interrupt plane, measured not assumed ═════════════════

#[test]
fn cap1_raises_exactly_one_interrupt_and_it_is_the_drivers_own_self_test() {
    // ★★★ The pre-registered limit said "the C raises SWGEN0 once for INIT_DONE". The
    // artifact says otherwise, and the difference matters: the C announces **none** of the
    // 202 status elements it posts. The guest picks up `GSP_INIT_DONE` and every RPC reply
    // by polling, so this capture constrains the interrupt plane not at all.
    let t = cap1();
    let irqs: Vec<usize> = t
        .records()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == CKind::Irq)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(irqs.len(), 1, "one raise in 359 062 records");

    // What caused it: the nearest preceding MMIO record. `NV_VF_INTR_LEAF_TRIGGER` with
    // vector 129 = `NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT`, which is what
    // `_osVerifyInterrupts` writes to check that interrupts are wired at all.
    let driver = t.records()[..irqs[0]]
        .iter()
        .rev()
        .find(|r| matches!(r.kind, CKind::MmioRead | CKind::MmioWrite))
        .expect("something drove it");
    assert_eq!(driver.kind, CKind::MmioWrite);
    assert_eq!(driver.a, 0x00b8_1640, "NV_VF_INTR_LEAF_TRIGGER");
    assert_eq!(driver.b, 129, "the SW-interrupt vector, not a GSP one");

    // And therefore: zero GSP status-queue announcements, against 202 posted elements.
    let posted = t
        .records()
        .iter()
        .filter(|r| r.kind == CKind::GuestWrite && r.payload.len() == 4096)
        .count();
    assert_eq!(posted, 202);

    // The other half of the same fact, and the reason F-3 cannot be adjudicated here: the
    // guest never cleared an edge either, because it never got one.
    let irqsclr = t
        .records()
        .iter()
        .filter(|r| r.kind == CKind::MmioWrite && r.a == 0x0011_0004)
        .count();
    assert_eq!(irqsclr, 0, "zero IRQSCLR writes in the whole capture");
}

// ═══════════════════════════ the harness cannot cheat ═══════════════════════════

#[test]
fn the_classifier_has_no_catch_all_and_the_census_proves_it() {
    // A classifier that quietly explained everything would make this file worthless. The
    // guard is structural — `classify` ends in `Verdict::Unexplained` — and this is the
    // observation that it bites: eight divergences in reach that no ledger row claims.
    let (_t, r) = run(Fill::Reconstructed);
    let c = census(&r);
    assert!(
        !c.unexplained().is_empty(),
        "a differential with nothing unexplained has stopped measuring"
    );
    // And it does not credit a ledger row that did not fire: cap1 never exercises a full
    // status ring, so GSP-D2 (QueueFull) must NOT appear even though its reconstruction
    // does.
    assert!(
        !c.by_id().iter().any(|(id, _)| *id == "GSP-D2"),
        "GSP-D2 classifies a QueueFull refusal, which this capture never produces"
    );
}

#[test]
fn a_replay_with_no_lookahead_and_no_reconstruction_is_strictly_weaker() {
    // The three fills are ordered by how much they invent, and the closure limit must move
    // monotonically with that. If it did not, a "stricter" mode would be reaching further
    // than a looser one, which would mean the modes do not mean what they say.
    let t = cap1();
    let abi = bench_abi();
    let strict = Replay::new(&t, abi).run(Fill::Observed);
    let ahead = Replay::new(&t, abi).run(Fill::Lookahead);
    let recon = Replay::new(&t, abi).run(Fill::Reconstructed);
    assert_eq!(strict.closure_limit, Some(492));
    assert_eq!(
        ahead.closure_limit,
        Some(492),
        "lookahead cannot invent a page table"
    );
    assert_eq!(recon.closure_limit, Some(978));
    assert!(strict.reconstructions.is_empty() && ahead.reconstructions.is_empty());
}

fn tag(n: Option<&Note>) -> String {
    match n {
        None => "-".to_string(),
        Some(Note::Register(_)) => "Register".to_string(),
        Some(Note::Irq) => "Irq".to_string(),
        Some(Note::Undecoded { .. }) => "Undecoded".to_string(),
        Some(Note::Decoded(o)) => o.kind().to_string(),
    }
}
