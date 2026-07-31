//! ★★★ **The reply plane, differenced** — `cap1b`, the capture that can be closed *past*
//! the first multi-element command, replayed against the policy chain a **guest** is
//! answered by.
//!
//! ## Why this file exists, stated as the gap it closes
//!
//! `cap1_differential.rs` is the transport proof and it is a good one, but it has two
//! properties that together made a whole plane invisible:
//!
//! 1. it replays `cap1`, whose closure limit is **txn 978** — the first multi-element
//!    command, which is GSP-D6's blind spot in the *recorder*; and
//! 2. it drives [`kayfabe_gsp::EchoOk`], the C's own *acknowledge-everything* baseline, so
//!    **no served control's body is ever produced**, and therefore none is ever compared.
//!
//! Of the controls this port serves, only `GSP_INIT_DONE`, fn 1, fn 65, fn 228 and
//! `INTERNAL_GPU_GET_CHIP_INFO` even *arrive* before txn 978; device-info, the interrupt
//! kernel table, the PCI-BAR table and the user-register access map all arrive after it.
//! So four of the five [`kayfabe_device::inittables::InitTablePolicy`] replies had **no
//! reply-plane differential coverage at all**, and a defect in one of them could not have
//! turned a test red. That is how `StaticInfoPolicy`'s missing size check survived.
//!
//! This file fixes both halves: `cap1b` for the reach, `kayfabe_crec::served_policy` for
//! the plane.
//!
//! ## ★★ What `cap1b` is, and what it is not
//!
//! The **same experiment** as `cap1`, re-captured at the C's `819282d`, where
//! `nvkvm_m3_service_cmdq` reads the continuation slots through the recorder chokepoint and
//! throws the bytes away. GSP-D6 is unchanged as a *defect* — the C still acts on element 0
//! alone and still emits byte-identical replies, proven against a same-binary control on
//! every channel (`C: traces/mode2_c_reference/README.md`) — it is merely **witnessed**:
//! 32/32 continuation elements observed, against 0/32 before.
//!
//! ⊘ It is not a superset of `cap1`. It is driven by a script rather than by hand, so its
//! `nvidia-smi -q` is not SIGPIPE-truncated and it carries more RPC work after
//! `GSP_INIT_DONE`. The bring-up prefix is the same, which is what a boot differential needs.
//!
//! ## ★★★ The measured result: the wall MOVED, and to the predicted place
//!
//! | | `cap1` | `cap1b` |
//! |---|---|---|
//! | closure limit | txn **978** | txn **1028** |
//! | why | GSP-D6 — an unobservable continuation element | GSP-D2 — our own `QueueFull`, real flow control against a guest that had stopped draining |
//! | served controls exercised | 1 of 5 | **5 of 5** |
//!
//! The second wall is a *different kind of thing* from the first. GSP-D6 was oracle
//! blindness: no assumption reconstructs a payload the recorder never saw. GSP-D2 is our
//! implementation declining to over-post where the C posts unconditionally — the C ran on
//! and the guest survived, and we stop. ⊘ **Do not relax flow control to green this
//! diff**; the refusal is the correct behaviour and the ledger row says so.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_crec::format::CKind;
use kayfabe_crec::{
    Answer, CTrace, Fill, Note, ReconKind, Replay, ReplayResult, Verdict, bench_abi, cap1b_path,
    census, load_cap1, load_cap1b, served_policy,
};
use kayfabe_device::inittables::WantedTable;
use kayfabe_gsp::{BootPhase, GspFault, Observation, Transition};

fn cap1b() -> CTrace {
    match load_cap1b() {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => panic!("cap1b at {:?} did not decode: {e:?}", cap1b_path()),
        Err(e) => panic!("cap1b is missing at {:?} ({e})", cap1b_path()),
    }
}

/// The run this whole file is about: `cap1b`, the served chain, reconstructions on.
fn served() -> (CTrace, ReplayResult) {
    let t = cap1b();
    let r = Replay::new(&t, bench_abi())
        .with_policy(served_policy)
        .run(Fill::Reconstructed);
    (t, r)
}

/// The `NV2080_CTRL_CMD_*` id a decoded `GSP_RM_CONTROL` names, or `None` for anything else.
///
/// ★ Read at the **same offset the policy reads it at** — `rpc_gsp_rm_control_v03_00`'s
/// `cmd` is the third word of the control header, after `hClient` and `hObject`. A test
/// that hard-coded a different offset would agree with itself and with nothing else.
fn control_cmd(cmd: &kayfabe_gsp::RpcCommand) -> Option<u32> {
    if cmd.code != 76 || cmd.payload.len() < 12 {
        return None;
    }
    Some(u32::from_le_bytes(cmd.payload[8..12].try_into().ok()?))
}

// ══════════════════ the capture is the one the file claims it is ══════════════════

#[test]
fn the_capture_is_the_witnessed_re_take_and_says_so_in_its_own_header() {
    // ★ The provenance is INSIDE the artifact — an oracle whose provenance lives in a
    // README stops being an oracle the moment the README and the file part company. The
    // C source revision is the field that matters most: this bench once served a binary
    // built from a revision nobody had recorded, for weeks.
    let t = cap1b();
    assert_eq!(t.records().len(), 360_725);
    assert!(t.header().hermetic(), "m2fwd=off — nothing else wrote RAM");
    assert!(t.closed_cleanly());
    assert_eq!(t.header().n_errors, 0);
    let p = t.header().provenance.as_str();
    assert!(
        p.contains("emulator-src-commit: 819282d"),
        "the GSP-D6 witness revision, named by the file: {p}"
    );
    assert!(
        p.contains("GSP-D6 continuation elements witnessed"),
        "and the capture says what it was taken to show"
    );
    assert!(p.contains("580.159.04") && p.contains("GA106"));
}

#[test]
fn the_continuation_elements_cap1_could_not_witness_are_witnessed_here() {
    // ★★★ The whole reason a second capture exists, measured against the first rather
    // than asserted. `cap1`'s wall was command ring slot 7 at `0x1_2720_9000`: the C
    // advanced its read pointer past it without reading it, so no observation of that slot
    // exists while it was live, and no named assumption reconstructs one.
    //
    // ⚠ ⊘ **Not** a comparison of the two captures' read counts: `cap1b` runs a full
    // `nvidia-smi -q` where `cap1`'s was SIGPIPE-truncated, so it does more RPC work and a
    // raw delta means nothing. This counts by *shape*, inside each capture: a message head
    // carries the RPC envelope's `header_version` at the element header's end, a
    // continuation element does not, and the head's own `elemCount` says how many
    // continuations are owed. The measurement is `owed` vs `witnessed`.
    let abi = bench_abi();
    let hdr = abi.element.hdr_size();
    let count_off = abi
        .element
        .elem_count_off()
        .expect("580 carries elemCount; a layout without one cannot be measured this way");
    let audit = |t: &CTrace| -> (usize, usize, usize) {
        let recs = t.records();
        let elems: Vec<usize> = recs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == CKind::GuestRead && r.payload.len() == 4096)
            .map(|(i, _)| i)
            .collect();
        let word = |i: usize, o: usize| {
            u32::from_le_bytes(recs[i].payload[o..o + 4].try_into().expect("4 bytes"))
        };
        // `rpc_message_header_v.header_version` — the first word after the element header
        // (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_headers.h`), and the field
        // `kayfabe_gsp`'s own decode checks. Present on a head, absent on a continuation.
        let is_head = |i: usize| word(i, hdr) == 0x0300_0000;
        let (mut multi, mut owed, mut witnessed) = (0, 0, 0);
        for (pos, h) in elems.iter().enumerate() {
            if !is_head(*h) {
                continue;
            }
            let n = word(*h, count_off) as usize;
            if n > 1 {
                multi += 1;
                owed += n - 1;
                witnessed += (1..n)
                    .filter(|j| elems.get(pos + j).is_some_and(|nx| !is_head(*nx)))
                    .count();
            }
        }
        (multi, owed, witnessed)
    };
    let a = load_cap1()
        .expect("cap1 is committed")
        .expect("cap1 decodes");
    assert_eq!(
        audit(&a),
        (5, 24, 0),
        "cap1: five multi-element commands, twenty-four continuations owed, NONE witnessed \
         — GSP-D6 in the artifact"
    );
    assert_eq!(
        audit(&cap1b()),
        (9, 32, 32),
        "cap1b: every continuation the producer declared is in the file"
    );
}

// ════════════════════════ the wall moved, and to where ════════════════════════

#[test]
fn cap1b_closes_the_replay_past_cap1s_wall_and_the_new_wall_is_a_different_finding() {
    // ★★★ **The headline.** `cap1` stops at txn 978 because the capture cannot answer a
    // read; `cap1b` carries the replay 50 transactions further and stops because *we*
    // refuse to post — real flow control against a guest whose status ring is full.
    //
    // ⊘ The second wall is not a defect to be relaxed away. The C posts unconditionally
    // and the ring overwrites unconsumed elements; the guest's own recovery branch handles
    // only `seqNum < rxSeqNum` (`ogkm-580: message_queue_cpu.c:699-713`), so over-posting
    // desynchronises the stream permanently. Greening this diff by removing the refusal
    // would be reproducing GSP-D2 rather than measuring it.
    let (_t, r) = served();
    let cap1 = Replay::new(
        &load_cap1()
            .expect("cap1 is committed")
            .expect("cap1 decodes"),
        bench_abi(),
    )
    .with_policy(served_policy)
    .run(Fill::Reconstructed);

    assert_eq!(cap1.closure_limit, Some(978), "the first capture's wall");
    assert_eq!(r.closure_limit, Some(1028), "and this one's");
    assert_eq!(
        r.txns[1028].refusal,
        Some(GspFault::QueueFull { needed: 9, free: 1 }),
        "GSP-D2: we decline the post the C would have made"
    );
    // Non-vacuity for "past": the transactions between the two walls are real work, not
    // padding — every one of them carries a command.
    let between = r
        .commands
        .iter()
        .filter(|(t, _)| (978..1028).contains(t))
        .count();
    assert_eq!(between, 50, "fifty commands cap1 could never reach");

    // ★★★ **PC-D1, in the artifact.** The answered stream must have no HOLE. Before the
    // fix the pass at txn 1028 consumed `rpc.sequence` 52, failed to post its reply, and
    // went on to answer 53 at txn 1029 — a command silently swallowed, and a guest blocked
    // on `_issueRpcAndWait` for the whole RPC timeout. Now 52 is left owed, so the stream
    // simply stops there.
    let answered: Vec<(u32, u32)> = r
        .commands
        .iter()
        .map(|(_, c)| (c.code, c.sequence))
        .collect();
    // ⚠ The first two commands are the pre-bind async pair — fn 72 and fn 73, both at
    // `rpc.sequence` 0, because `_issueRpcAsync` does not advance the counter a reply is
    // matched on. The awaited stream starts after them, and it is the awaited stream that
    // may not have a hole.
    assert_eq!(&answered[..2], &[(72, 0), (73, 0)]);
    let awaited: Vec<u32> = answered[2..].iter().map(|(_, s)| *s).collect();
    assert_eq!(
        awaited,
        (0..=51).collect::<Vec<u32>>(),
        "a hole here is a command consumed and never replied to. Before the fix the pass at \
         txn 1028 consumed rpc.sequence 52, failed to post its reply, and went on to answer \
         53 at txn 1029 — swallowed, with the guest blocked on _issueRpcAndWait for the \
         whole RPC timeout. Now 52 is left OWED and the stream simply stops."
    );

    // And the read that stopped `cap1` is *answered* here rather than reconstructed: the
    // reconstruction list is unchanged, so nothing was invented to buy the extra reach.
    assert_eq!(
        r.reconstructions.iter().map(|x| x.kind).collect::<Vec<_>>(),
        vec![ReconKind::RegionPageTable, ReconKind::PeerStatusReadPtr],
        "the same two named assumptions as cap1, and no third"
    );
    assert!(r.unobserved.is_empty(), "no read went unanswered");
    assert_eq!(
        r.answers,
        vec![
            (Answer::Observed, 1030),
            (Answer::Lookahead, 2),
            (Answer::Reconstructed(ReconKind::RegionPageTable), 1),
            (Answer::Reconstructed(ReconKind::PeerStatusReadPtr), 1),
        ]
    );
    assert!(r.max_lookahead <= kayfabe_crec::oracle::LOOKAHEAD_LIMIT);
}

#[test]
fn the_multi_element_command_cap1_died_on_is_served_here() {
    // ★★ The same message, in the same place in the boot, at both captures: `fn 76`,
    // `rpc.length = 8276`, three elements. In `cap1` it is the closure limit. Here it is
    // decoded, classified and answered — which is the difference between a wall and a rung.
    let (_t, r) = served();
    let (txn, cmd) = r
        .commands
        .iter()
        .find(|(_, c)| c.elements > 1)
        .expect("cap1b reaches a multi-element command");
    assert_eq!(*txn, 980);
    assert_eq!((cmd.code, cmd.elements), (76, 3));
    assert_eq!(
        control_cmd(cmd),
        Some(kayfabe_abi::regaccessmap::NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP),
        "and it is a control this port SERVES, not one it refuses"
    );
}

// ═════════════════ the deliverable: every served control is differenced ═════════════════

#[test]
fn every_control_this_port_serves_is_exercised_by_the_replay() {
    // ★★★ **The coverage assertion, and the universe is DERIVED.** A hand-written list of
    // "the controls we serve" is the defect shape this repository has been bitten by most
    // often: shortening the list weakens the gate with zero red tests. So the universe
    // comes from `WantedTable::from_cmd` itself — the policy's own classifier — plus fn 65,
    // which is a different policy. Add a served control and this test demands the
    // differential reach it; it cannot be satisfied by editing a literal here.
    let (_t, r) = served();
    let limit = r.closure_limit.expect("this capture has one");

    let reached: BTreeSet<WantedTable> = r
        .commands
        .iter()
        .filter(|(t, _)| *t < limit)
        .filter_map(|(_, c)| control_cmd(c).and_then(WantedTable::from_cmd))
        .collect();
    let universe = WantedTable::ALL.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        reached, universe,
        "a served control the differential never sees is a served control no differential \
         can regress"
    );
    assert_eq!(universe.len(), 5, "non-vacuity: the universe is not empty");

    // fn 65 is `StaticInfoPolicy`, and fn 228 is `InertPolicy`. Both are answered here too,
    // so all three answering links of the chain are exercised in one run.
    let functions: BTreeSet<u32> = r
        .commands
        .iter()
        .filter(|(t, _)| *t < limit)
        .map(|(_, c)| c.code)
        .collect();
    for want in [1, 65, 76, 103, 228] {
        assert!(functions.contains(&want), "fn {want} never arrived");
    }
}

#[test]
fn the_served_replies_are_the_ones_posted_and_each_carries_the_result_it_earned() {
    // ★★ What a reply-plane test has to establish before any comparison means anything:
    // that OUR side posted a reply for each served control, at the right sequence, with
    // the envelope result the policy decided. Read off our own projection, joined to the
    // command stream by `rpc.sequence`.
    let (_t, r) = served();
    let mut posted: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for (i, n) in r.rust.notes.iter().enumerate() {
        if let Note::Decoded(Observation::ElementPosted {
            function,
            sequence,
            rpc_result,
            ..
        }) = n
            && r.rust.txn[i] < r.closure_limit.expect("there is one")
        {
            posted.insert(*sequence, (*function, *rpc_result));
        }
    }

    let answered: Vec<(WantedTable, u32, u32, u32)> = r
        .commands
        .iter()
        .filter(|(t, _)| *t < r.closure_limit.expect("there is one"))
        .filter_map(|(_, c)| {
            let w = control_cmd(c).and_then(WantedTable::from_cmd)?;
            let (f, rc) = *posted.get(&c.sequence)?;
            Some((w, c.sequence, f, rc))
        })
        .collect();
    // ⊘ Itemised, not counted: a reviewer can check which control got which result, and a
    // change moves a line here rather than a number. Every one succeeded — asserting
    // `rc == 0 || rc == NOT_SUPPORTED` would pass on a port that served nothing.
    assert_eq!(
        answered,
        vec![
            (WantedTable::ChipInfo, 3, 76, 0),
            (WantedTable::UserRegisterAccessMap, 4, 76, 0),
            (WantedTable::DeviceInfo, 9, 76, 0),
            (WantedTable::IntrKernelTable, 10, 76, 0),
            (WantedTable::PciBarInfo, 12, 76, 0),
        ]
    );
}

#[test]
fn our_interrupt_kernel_table_is_byte_identical_to_a_real_ga106s_own_reply() {
    // ★★★ **The positive result, and the only one in this file that is an agreement rather
    // than a difference.** The C does not echo this control: it splices a blob captured
    // from a real GA106 (`C: src/qemu/mode2_initctrl_ga106.h`, row
    // `{0x20800a5c, 0x0, 2112, 2112, ctl_20800a5c}`). We *generate* ours from
    // `kayfabe_device::ga10x::GA106_INTR_TABLE` and the subtree map. The two agree on every
    // byte of a 2152-byte reply.
    //
    // ⊘ This is not reading a golden back at the constant under test: the two sides share
    // no source. One is silicon's answer recorded through a driver; the other is an
    // encoder written from `ogkm-580` headers.
    let (_t, r) = served();
    let c = census(&r);
    let intr_txn = r
        .commands
        .iter()
        .find(|(_, c)| {
            control_cmd(c)
                == Some(kayfabe_abi::inittables::NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE)
        })
        .map(|(t, _)| *t)
        .expect("the guest asks for the interrupt kernel table");
    assert_eq!(intr_txn, 986);
    let element_diverged = c.items.iter().any(|i| {
        i.txn == intr_txn
            && matches!(i.c, Some(Note::Decoded(Observation::ElementPosted { .. })))
            && matches!(
                i.rust,
                Some(Note::Decoded(Observation::ElementPosted { .. }))
            )
    });
    assert!(
        !element_diverged,
        "our interrupt kernel table no longer matches the captured GA106 reply"
    );

    // Non-vacuity, and it is load-bearing: the *other* served controls DO differ, so a
    // harness that could not tell the two apart would pass this test for the wrong reason.
    let chip_txn = r
        .commands
        .iter()
        .find(|(_, c)| {
            control_cmd(c)
                == Some(kayfabe_abi::chipinfo::NV2080_CTRL_CMD_INTERNAL_GPU_GET_CHIP_INFO)
        })
        .map(|(t, _)| *t)
        .expect("the guest asks for the chip identity");
    assert!(
        c.items.iter().any(|i| i.txn == chip_txn
            && matches!(i.c, Some(Note::Decoded(Observation::ElementPosted { .. })))),
        "chip info must still differ — we answer as OUR device, not as the bench's"
    );
}

// ══════════════════════ the run itself, so the numbers are not free ══════════════════════

#[test]
fn the_boot_fsm_is_driven_all_the_way_through_and_the_census_is_itemised() {
    let (_t, r) = served();
    for want in [
        Transition::E12,
        Transition::E1,
        Transition::E5,
        Transition::E6,
        Transition::E7,
        Transition::E4,
    ] {
        assert!(
            r.transitions_seen.contains(&want),
            "{want:?} never fired; transitions were {:?}",
            r.transitions_seen
        );
    }
    assert_eq!(r.final_phase, BootPhase::Halted);
    assert_eq!(r.txns.len(), 2053);
    assert_eq!(
        r.rust.census(),
        vec![
            ("ElementPosted", 61),
            ("Irq", 56),
            ("ReadPtrAcked", 273),
            ("Register", 912),
            ("TxHeaderPublished", 1),
            ("WritePtrAdvanced", 53),
        ]
    );
    // ★★ `ReadPtrAcked` is the number PC-D2 moved, and it moved *toward the C*: 54 -> 273
    // against the C's 272. Before the fix the consumption acknowledgement was written after
    // the drain's `?`s, so a pass that faulted published nothing and the guest kept reading
    // a stale `readPtr`. It is now published however the pass ended.
    assert_eq!(
        r.c.census()
            .iter()
            .find(|(k, _)| *k == "ReadPtrAcked")
            .map(|(_, n)| *n),
        Some(272),
        "the C acknowledges on every pass, and now so do we"
    );
    // ★ The classifier still has no catch-all here either, and GSP-D2 now DOES fire —
    // which is the one ledger row `cap1` could never exercise.
    let c = census(&r);
    assert!(!c.unexplained().is_empty());
    assert!(
        c.items
            .iter()
            .any(|i| matches!(i.verdict, Verdict::Expected(d) if d.id == "GSP-D2")),
        "cap1b reaches a QueueFull, so the row that classifies one must fire"
    );
}
