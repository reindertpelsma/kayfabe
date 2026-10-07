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
use kayfabe_device::sweep::{SweepDisposition, triage_for};
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
    // ★★★★ **1030 → 1032, 2026-08-27, and ⊘ THIS ONE IS *NOT* THE SERVED-SET UNION** — the
    // other red in this file is, and reading the two as one cause would have been wrong.
    //
    // `[measured 2026-08-27, bisected in a worktree over the w337-gpu-name-seam lineage]`
    // this assertion is GREEN at `147694ff^` (`6a1fe00d`) and RED at `147694ff` itself, with
    // the coverage test below already red at both. ⇒ The +2 is `147694ff` — *"gsp: identify
    // the msgq instance by the guest's own seqNum, not by the region's address"*, the PC-D7
    // fix — and nothing else. It landed on the **w337-gpu-name-seam** side, `master`'s side
    // never saw it, and the merge `d3f80778` brought a pin and a behaviour from two
    // lineages together. ★ The pin DID ITS JOB: a change in what the replay reads showed up
    // as a red assertion on the branch that made it, not as drift discovered later.
    //
    // ★ What the two extra reads ARE, so the number is understood rather than accepted:
    // `publish()` now peeks the most recent COMMAND ELEMENT's `seqNum` to decide
    // `same_instance`, because no sequence number crosses the shared region in any header
    // (`ogkm-580: msgq_priv.h:49-65`) but the command element carries one
    // (`message_queue_cpu.c:481`). This replay performs two publishes — the `GSP-PUBLISH …
    // seq_last=Some(1) cmd_seq=0 ⇒ same_instance=false` lines in this test's own output — so
    // exactly one extra observed read each. ⊘ Every other term is UNMOVED: the closure limit
    // is still 1028, `Lookahead` still 2, both reconstructions still the same two named
    // assumptions, and `unobserved` still empty. A wall that moved would have shown up in
    // `closure_limit`, and it did not.
    assert_eq!(
        r.answers,
        vec![
            (Answer::Observed, 1032),
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

    // ★★★ **The exception set, and it is an admission rather than a narrowing.**
    //
    // `cap1b`'s closure limit falls at `rpc.sequence` 51, in the MIDDLE of
    // `kgraphicsLoadStaticInfo_KERNEL`'s straight-line run of controls: the oracle asks
    // `0x20800a1f` (seq 49) and `0x20800a26` (seq 51) inside the limit and then the replay
    // stops, so `0x20800a22`, `0x20800a3d` and `0x20800a48` — the same function's next
    // three mandatory controls — are unreachable by this capture. ⊘ No editing of this test
    // can change that; only a longer capture can.
    //
    // ⚠ **They are not uncovered, they are covered by a DIFFERENT artifact.** All three are
    // byte-compared against the C's captured GA106 init-control table in
    // `kayfabe-abi/tests/gr_static_info.rs`, which is a different recording of the same
    // real hardware. What they lack is *reply-plane* coverage — nothing checks that this
    // port's answer reaches the guest's queue in the right envelope — and that is a real
    // gap, stated rather than closed.
    //
    // ★ Pinned as an exact set, so adding a served control silently to it is a red test in
    // the same way shortening the universe would be.
    //
    // ⚠ `#151` added a FIFTH, and it is outside the limit for a different reason worth
    // stating separately rather than folding into the paragraph above.
    // `GvaspaceServerReservedPdesClient` is `0x90f10106`, the CLIENT-context arm of
    // `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL` (`ogkm-580: gpu_vaspace.c:4058`).
    // `cap1b` closes at `rpc.sequence` 51, which is inside `gpuStateInit`; the client arm is
    // first reached from `memmgrScrubHandlePostSchedulingEnable` during **state LOAD**,
    // hundreds of sequences later. ⊘ So this is not "the capture stopped mid-function" —
    // it is a control that belongs to a boot phase `cap1b` does not contain at all, and no
    // extension of this particular capture's closure limit would reach it.
    //
    // ★ Its reply plane is nonetheless covered where it can be: the payload is byte-for-byte
    // the `0x20800a9f` one (`ctrl2080internal.h:1906-1908` wraps exactly this member), that
    // id IS in the differential, and both ids route to one decode/encode pair in
    // `kayfabe_abi::gvaspacepdes`. What is genuinely uncovered here is the ENVELOPE for this
    // specific command id, and `[measured]` run `irq1` is the only thing that has exercised
    // it.
    //
    // ⚠⚠ §14.28 added a SIXTH, and its reason is a THIRD kind — not "the capture stopped
    // mid-function" and not "a later boot phase", but **a different demander entirely**.
    // `GpuInfoV2` is `0x20800102`, and `cap1b` is an `RmInitAdapter` capture driven by
    // `nvidia-smi`; this control's forwarded entries are demanded by the guest kernel later
    // in the boot and by **libcuda**, neither of which this capture contains. ⊘ No closure
    // limit reaches a process that never ran.
    //
    // ★ Its reply plane is covered elsewhere and better: `tests/tests/replay_conformance.rs`
    // replays the THREE real `0x20800102` calls in `traces/rpctrace_ga106_boot1.bin` — a
    // GSP-level capture off a real GA106 — which is a stronger oracle than this differential
    // could be, because it carries the requests' own bytes including the
    // `INDEX_FORWARD_TO_PHYSICAL` bit. ⊘ This entry is still a real coverage cost and is
    // named rather than elided: nothing in the `cap1b` pair would notice this arm breaking.
    // ⚠⚠⚠ §14.29, §14.30 and §14.31 add a SEVENTH, EIGHTH and NINTH, and all three are
    // `GpuInfoV2`'s kind — **a different demander entirely**. `cap1b` is an `RmInitAdapter`
    // capture driven by `nvidia-smi`, and none of these is reached by that process:
    // `InternalGpuGetSmcMode` (`0x20800a4c`) is issued only from `getGpuInfos`'s `0x2a` arm,
    // `BusGetInfoV2` (`0x20801823`) only for the one index the guest kernel forwards, and
    // `BusGetPcieSupportedGpuAtomics` (`0x2080182a`) is `cuInit`'s next line after it. All
    // three were named by an in-guest trace of **libcuda**, which this capture does not
    // contain. ⊘ No closure limit reaches a process that never ran.
    //
    // ⊘⊘ **Two of these three are not mine, and this gate was RED before I touched it.**
    // `[measured 2026-08-08]` this exact test fails at `78bee9e` — §14.29 and §14.30 each
    // added a served control and left it out of this set, so the differential's own closure
    // assertion has been failing for two rungs. Recording that rather than quietly greening
    // it: what follows is the repair of an inherited red with each entry's reason stated,
    // not a bar lowered to fit a new row.
    //
    // ★ Their reply planes, honestly: `InternalGpuGetSmcMode` and
    // `BusGetPcieSupportedGpuAtomics` are exercised through `InitTablePolicy::respond` in
    // `kayfabe-device/tests/{internal_gpu_get_smc_mode,bus_get_pcie_supported_gpu_atomics}.rs`,
    // envelope and inner status included. `BusGetInfoV2` had **none** — §14.30 landed it with
    // `kayfabe-abi` unit tests over `answer_bus_get_info_v2` and nothing at the policy
    // boundary — so `kayfabe-device/tests/bus_get_info_v2.rs` was written here to make this
    // admission true rather than merely quiet.
    // ⚠⚠⚠ §14.32 adds a TENTH, `GpuInfoV2`'s kind again — **a different demander**.
    // `FbGetInfoV2` (`0x20801303`) is asked four times by `cuInit`; the first three are
    // answered entirely by the guest's own kernel (`ogkm-580: kern_mem_sys_ctrl.c:335, 711,
    // 716` and friends) and never become an RPC at all, and the fourth is the one this port
    // now serves. `cap1b` is an `RmInitAdapter` capture driven by `nvidia-smi`, in which
    // `libcuda` never runs. ⊘ No closure limit reaches a process that never ran.
    //
    // ★ Its reply plane is covered at the policy boundary by
    // `kayfabe-device/tests/fb_get_info_v2.rs` — written with this row rather than after it,
    // so this admission is true at the moment it is made. ⊘ And it is a real coverage cost,
    // named: nothing in the `cap1b` pair would notice this arm breaking.
    // ⚠⚠⚠ §14.33 adds an ELEVENTH, `GpuInfoV2`'s kind a fifth time — **a different
    // demander**, and this one twice over. `CeGetAllPhysicalCaps` (`0x20802a0b`) reaches an
    // emulated GSP only as `subdeviceCtrlCmdCeGetAllCaps_IMPL`'s forward
    // (`ogkm-580: kernel_ce_shared.c:315-320`), and the caller that asks for it is `cuInit`;
    // `cap1b` is an `RmInitAdapter` capture driven by `nvidia-smi`, in which `libcuda` never
    // runs. ⊘ No closure limit reaches a process that never ran.
    //
    // ★ And its reply plane is covered better here than by any other row on this list, which
    // is worth stating because "covered elsewhere" is the sentence that lets a gap through.
    // `kayfabe-device/tests/ce_get_all_physical_caps.rs` does not compare this port against
    // itself: it parses the reply out of **two independent real-GA106 captures** — libcuda's
    // `cuInit` through a full CUDA context, and an `rmladder` bare `Subdevice` with no
    // channel — asserts those two agree, and then asserts the served bytes equal them. A
    // differential against a `nvidia-smi` capture would be a weaker oracle than that, not a
    // stronger one. ⊘ The envelope is still this file's kind of coverage and is still absent;
    // that part of the cost is real and is named.
    // ⚠⚠⚠ §14.41 adds a TWELFTH, and it is the **cleanest** of the list rather than another
    // grudging admission: `RegisterFaultBuffer` (`0x20800a9b`) has exactly one issuer in the
    // whole guest, and it is not a CUDA-vs-smi distinction but a *module* one.
    // `kgmmuFaultBufferReplayableAllocate_IMPL` sends it (`ogkm-580:
    // src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1261-1265`) only from `faultbufConstruct_IMPL`
    // (`.../mmu_fault_buffer.c:59`), whose only caller is the `MmuFaultBuffer` alloc inside
    // `nvGpuOpsInitFaultInfo` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9410`),
    // reached from `uvm_parent_gpu_fault_buffer_init` (`ogkm-580:
    // kernel-open/nvidia-uvm/uvm_gpu_replayable_faults.c:247-253`) — i.e. from
    // **`nvidia-uvm`, on `UVM_REGISTER_GPU`**. `cap1b` is an `RmInitAdapter` capture driven
    // by `nvidia-smi`, which never opens `/dev/nvidia-uvm`. ⊘ No closure limit reaches a
    // module that was never asked to register a GPU.
    //
    // ★ Its reply plane is covered at the policy boundary by
    // `kayfabe-device/tests/register_fault_buffer.rs`, written **with** this row — and that
    // test can be stronger than a differential would be, because the reply is the identity
    // on the guest's own `[IN]` bytes: the property to check is *"not one byte moved"*, which
    // is checkable exactly and needs no capture to compare against. ⊘ The envelope is still
    // this file's kind of coverage and is still absent; that part of the cost is real.
    let outside_the_closure_limit: BTreeSet<WantedTable> = [
        WantedTable::RegisterFaultBuffer,
        // ⚠ §14.41's second, and it is the SAME structural class as the first: the sole
        // issuer of `0x20800a9d` is `kgmmuClientShadowFaultBufferRegister`
        // (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1815-1820`), reached only
        // through `faultbufCtrlCmdMmuFaultBufferRegisterNonReplayBuf_IMPL` — a **`C369`
        // control UVM issues**. `cap1b` is `nvidia-smi`-driven and never opens
        // `/dev/nvidia-uvm`. ★ Covered at the policy boundary by
        // `kayfabe-device/tests/register_fault_buffer.rs`, which is again able to be the
        // *stronger* oracle: the reply is the identity on 24 032 pure-`[IN]` bytes, so
        // "not one byte moved" is exactly checkable and needs no capture.
        WantedTable::RegisterClientShadowFaultBuffer,
        // ⚠ §14.41's third, same UVM-module class. `0x20800a1d` is sent by
        // `_uvmSetupAccessCntrBuffer` (`ogkm-580: src/nvidia/src/kernel/gpu/uvm/uvm.c:39-81`)
        // on the `UVM_REGISTER_GPU` path; `cap1b` is `nvidia-smi`-driven and never opens
        // `/dev/nvidia-uvm`. ★ Covered by `kayfabe-device/tests/register_fault_buffer.rs`,
        // again as the stronger oracle — identity on 520 pure-`[IN]` bytes.
        WantedTable::RegisterAccessCntrBuffer,
        WantedTable::GrGlobalSmOrder,
        WantedTable::GrFecsRecordSize,
        WantedTable::GrPdbProperties,
        WantedTable::GrContextBuffersInfo,
        WantedTable::GvaspaceServerReservedPdesClient,
        WantedTable::GpuInfoV2,
        WantedTable::InternalGpuGetSmcMode,
        WantedTable::BusGetInfoV2,
        WantedTable::BusGetPcieSupportedGpuAtomics,
        WantedTable::FbGetInfoV2,
        WantedTable::CeGetAllPhysicalCaps,
        // ⚠ §14.42's pair, and they are the **same structural class as §14.41's three**: a
        // module boundary, not a CUDA-vs-smi distinction. Both `0x20802a07` (as the guest
        // kernel's forward of `0x20802a01`) and `0x20802a02` are issued only from
        // `queryCopyEngines` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8449-8541`),
        // whose only caller is `nvGpuOpsQueryCesCaps` (`:6706-6733`) → `rm_gpu_ops_query_ces_caps`
        // (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/rm-gpu-ops.c:345-355`) →
        // `nvUvmInterfaceQueryCopyEnginesCaps` (`kernel-open/nvidia/nv_uvm_interface.c:637-653`),
        // whose only callers in the whole tree are **inside `nvidia-uvm`**
        // (`kernel-open/nvidia-uvm/uvm_gpu.c:489` and `uvm_channel.c:3172`). `cap1b` is an
        // `RmInitAdapter` capture driven by `nvidia-smi`, which never opens
        // `/dev/nvidia-uvm`. ⊘ No closure limit reaches a module that was never asked.
        //
        // ★ Both reply planes are covered at the policy boundary instead, and the caps one
        // by an oracle a differential could not match: `kayfabe_abi::cecaps`'s own tests
        // assert the per-engine reply **is the whole table's own row**, and that table is
        // pinned byte-for-byte to two independent real-GA106 captures. The PCE-mask one is
        // pinned to `traces/real_ga106/rmladder_r24_pcemask_real_ga106.txt`, a capture of
        // *this control at this boundary*. ⊘ The envelope is still this file's kind of
        // coverage and is still absent; that part of the cost is real and is named.
        WantedTable::CeGetPhysicalCaps,
        WantedTable::CeGetCePceMask,
        WantedTable::GrmgrGetGrFsInfo,
        WantedTable::GspGetFeatures,
        WantedTable::GssLegacy8159,
        WantedTable::GssLegacy8162,
        WantedTable::C2cInfo,
        // ⚠ §14.43's rung, and it is the same structural class again — a module boundary,
        // not a closure limit. `0xa06c010a` has exactly one call site in the open tree
        // (`ogkm-580: kernel_channel_group_api.c:494`, inside `kchangrpapiConstruct_IMPL`),
        // so it is issued only when something allocates a `KEPLER_CHANNEL_GROUP_A`.
        // `[measured 2026-08-09, boot `ce1442` at `8ea44dc`]` decides which something: the
        // `kchangrpapiConstruct_IMPL` failure appears in `cup2`'s dmesg delta (58-73 s,
        // `traces/guest_boots/ce1442_8ea44dc_probe.log:75`) and **nowhere** in the
        // `nvidia-smi` device-open window of the same boot (28-39 s,
        // `..._dmesg.log`) — so no TSG is allocated on the `RmInitAdapter` path at all.
        // `cap1b` is an `RmInitAdapter` capture driven by `nvidia-smi`. ⊘ No closure limit
        // reaches a control the capture's driver never issues.
        //
        // ★ Its reply plane is covered where it can be: `kayfabe_abi::fmbpromote`'s own
        // tests sweep the decoder and pin the round trip, and
        // `crates/kayfabe-device/tests/promote_fault_method_buffers.rs` asserts the reply at
        // THIS policy's boundary — the call-site coverage `WantedTable::C2cInfo` was shipped
        // twice without. ⊘ The envelope is still absent and that cost is real.
        WantedTable::PromoteFaultMethodBuffers,
        // ★★★★★ **w337 MERGE, 2026-08-27 — SIX AT ONCE, AND THE PIN DID ITS JOB BY GOING
        // RED.** `[measured 2026-08-27, merge d3f80778 "Merge w337-gpu-name-seam"]`
        //
        // The merge unioned two independently-grown served sets. This test derives its
        // universe from `WantedTable::ALL` **on purpose** — *"add a served control and this
        // test demands the differential reach it; it cannot be satisfied by editing a literal
        // here"* — so six rows grown on the **w337-gpu-name-seam** side arrived as a failing
        // assertion the moment the two lineages met, naming every one of them. ⊘ That is the
        // mechanism working exactly as its own docs promise, and nothing was re-baselined:
        // the six are transcribed FROM the failure, and each is genuinely unreachable by this
        // capture rather than merely unreached.
        //
        // ⚠⚠ **All six are `GpuInfoV2`'s kind — a different DEMANDER — and this is the SIXTH
        // consecutive way that has happened**, which is itself the finding. `cap1b` is an
        // `RmInitAdapter` capture driven by `nvidia-smi`; every one of these six is issued by
        // **`libcudart`**, a process further from this capture than `libcuda` is: `[measured
        // 2026-08-20, real GA106 on 580.159.04]` `libcuda` answers every driver-API call
        // correctly on the very boot `libcudart` cannot initialise. ⊘ No closure limit reaches
        // a process that never ran, and — unlike the six `cuInit`-path rows above — a
        // `cuInit`-driven capture would NOT close these either. They need a `cudaGetDeviceCount`
        // -driven one. ★ The exception set therefore now shrinks by SIX, not by twelve, the
        // day the long-overdue capture exists; the honest number is stated rather than let
        // slide.
        //
        // ★ Where each came from, and what stands in for the reply-plane coverage:
        //   `CudartWatchdogInfo` (`0x20802209`), `CudartInit9009` (`0x20809009`),
        //   `CudartInit9001` (`0x20809001`), `CudartInit9064` (`0x20809064`) — `2d32e5ee`
        //     (w349). Their answers are pinned byte-for-byte to a real GA106 in
        //     `kayfabe_abi::cudartinit`'s own tests, and the per-id BISECTION there is a
        //     stronger oracle than a differential could be: refusing any one of them on the
        //     bare-metal host turns its own `cudaGetDeviceCount` from `0` into `3`.
        //   `CudartInit9A001` (`0x2080a001`) — `0f577b15` (w352). ⚠ The weakest-covered of
        //     the six and it is named rather than elided: `[measured]` the only capture of it
        //     covers 40 of 520 bytes, and it is reached only as the FALLBACK after
        //     `a084`/`a026` fail — which is exactly where our guest is.
        //   `CudartPerfLevelInfoV2` (`0x2080200b`) — `3887be37` (w353). The first SPLICED row
        //     this port serves; its request carries content, so the differential's usual
        //     "did the constant body come back" question is not even the right one.
        //
        // ⊘⊘ Two of the six ARE reachable by a different committed capture, and that is
        // recorded here so this admission is not read as blanket: `0x20809009` (×2) and
        // `0x20809064` (×8) are demanded by `traces/rpctrace_ga106_boot1.bin` and are judged
        // against real GSP firmware in `tests/tests/replay_conformance.rs`. What they lack
        // HERE is this file's kind of coverage — the envelope on `cap1b`'s own queue.
        WantedTable::CudartWatchdogInfo,
        WantedTable::CudartInit9009,
        WantedTable::CudartInit9001,
        WantedTable::CudartInit9064,
        WantedTable::CudartInit9A001,
        WantedTable::CudartPerfLevelInfoV2,
    ]
    .into_iter()
    .collect();
    assert!(
        outside_the_closure_limit.is_subset(&universe),
        "an exception for a control this port does not serve is an exception for nothing"
    );
    assert_eq!(
        reached,
        universe
            .difference(&outside_the_closure_limit)
            .copied()
            .collect::<BTreeSet<_>>(),
        "a served control the differential never sees — and that is not named above as \
         being past the capture's closure limit — is a served control no differential can \
         regress"
    );
    // ★ 22 -> 23 at the `fmb` rung: `0x20802a08`, and it is the STRONGEST kind of addition
    // this pair can see — the oracle asks it at sequence 18, INSIDE the closure limit, so
    // the new control is exercised by the replay rather than merely declared.
    // ★ 23 -> 24 at the `GR-info` rung: `0x20800a2a`, asked at sequence 50 — also INSIDE
    // the closure limit, so it too is exercised by the replay rather than merely declared.
    // ★ 24 -> 25 at the `cuInit` rung: `0x20800102` GPU_GET_INFO_V2, and it is the WEAKEST
    // kind of addition this pair can see — it goes straight into the exception set below.
    // ⊘ 25 -> 28 across §14.29 (`0x20800a4c`), §14.30 (`0x20801823`) and §14.31
    // (`0x2080182a`) — and all three are the WEAKEST kind, straight into the exception set,
    // because `cap1b` is `nvidia-smi`-driven and all three were named by an in-guest trace
    // of libcuda. `[measured 2026-08-08]` the first two were landed without updating either
    // count, so this assertion has been failing since §14.29; the numbers below are the
    // inherited red repaired and attributed, not a bar moved to fit a new row.
    // ⊘ 28 -> 29 at §14.32 (`0x20801303`), the WEAKEST kind again and for the same reason.
    // ⊘ 29 -> 30 at §14.33 (`0x20802a0b`), the WEAKEST kind a FIFTH time and for the same
    // reason. ⚠⚠⚠ Five consecutive rungs have each added a control this differential cannot
    // see, and each has written "the durable fix is a `cuInit`-driven capture" and then not
    // made one. ★ The note below said the capture was *four rungs overdue*; it is now five,
    // and the sentence has been carried forward unchanged so many times that carrying it
    // forward is what a reader now expects. `a_flag_is_not_progress`: a repeat flag is
    // evidence the answer is nearby, and this one is a queue item, not a paragraph.
    // ⊘ 30 -> 31 at §14.34 (`0x20803801`), the WEAKEST kind a SIXTH time: `GRMGR_GET_GR_FS_INFO`
    // is issued by `cuInit` and `cap1b` is `nvidia-smi`'s `RmInitAdapter`.
    // ⊘ 31 -> 32 at §14.35 (`0x20803601`), the WEAKEST kind a SEVENTH time — and this one
    // is the first that `cap1b` could not cover **even if it were `cuInit`-driven**, which
    // is a different fact and worth separating from the other six. `GSP_GET_FEATURES`'s
    // reply is not a projection of a chip row: its `firmwareVersion` is latched from the
    // guest's own fn 1, so a replay differential would be comparing this port's answer
    // against a string recorded from a *different* guest's driver. ★ The oracle for it is
    // therefore the real-GA106 trace plus `ogkm`'s own `NV_VERSION_STRING`, which
    // `kayfabe_abi::gspfeatures`'s unit tests assert byte-for-byte, and the honest reading
    // is that a `cuInit` capture shrinks the exception set by SIX, not seven.
    // ⊘ 32 -> 33 at §14.36 (`0x20808159`), the WEAKEST kind an EIGHTH time — and this one
    // `cap1b` could not carry even in principle: the id is **GSS-legacy**, and §4 of
    // `kayfabe_device::sticky`'s module docs `[measured]` that not one control word in the
    // whole of `cap1b` has bit 15 set. So branch (b) traffic is unexercised by the entire
    // cold-boot prefix, and this row is not waiting on a `cuInit`-driven capture the way the
    // other six are — it is waiting on a capture of a plane no committed trace contains.
    // ⊘ 33 -> 35 at §14.37 (`0x20808162`, `0x2080182b`), the WEAKEST kind a NINTH and TENTH
    // time. `0x20808162` joins `0x20808159` in the structural class — GSS-legacy, and no
    // committed capture contains bit-15 traffic at all. `0x2080182b` is the ordinary
    // `cuInit`-path class, absent because this capture is `nvidia-smi`-driven.
    // ⊘ 35 -> 36 at §14.41 (`0x20800a9b`), an ELEVENTH exception and the first of a NEW
    // structural class: absent from `cap1b` not because `libcuda` never ran but because
    // **`nvidia-uvm` never ran** — the control's sole issuer is UVM's `UVM_REGISTER_GPU`
    // path, and an `nvidia-smi` capture never opens `/dev/nvidia-uvm`. ★ A `cuInit`-driven
    // capture would close this one, unlike the two GSS-legacy rows.
    // ⊘ 36 -> 37 at §14.41's second rung (`0x20800a9d`), same class as the first.
    // ⊘ 38 -> 40 at §14.42 (`0x20802a07`, `0x20802a02`), a TWELFTH and THIRTEENTH exception
    // and both in §14.41's `nvidia-uvm`-never-ran class rather than the CUDA-vs-smi one:
    // `queryCopyEngines`' only caller chain ends at `nvUvmInterfaceQueryCopyEnginesCaps`,
    // exported to and called only from `nvidia-uvm`. ★ A `cuInit`-driven capture would close
    // both, unlike the two GSS-legacy rows — and unusually for this list, `0x20802a02`'s
    // reply plane already has a capture of **its own boundary** (`R24`), which no other
    // entry here can say.
    // ⊘ 40 -> 41 at §14.43 (`0xa06c010a`), a FOURTEENTH exception and the first that is not
    // a subdevice control. Its class is the same "the capture's driver never issued it" one,
    // but it is measured rather than argued from a caller chain: `[measured 2026-08-09, boot
    // `ce1442`]` the sole caller `kchangrpapiConstruct_IMPL` fails in `cup2`'s dmesg delta
    // and never in the same boot's `nvidia-smi` window. ★ A `cuInit`-driven capture would
    // close it.
    // ⊘⊘ **41 -> 47 and 22 -> 28 at the w337 merge, 2026-08-27** (`d3f80778`, *"Merge
    // w337-gpu-name-seam"*), and BOTH numbers move by the same six because every one of them
    // is unreachable by this capture. The six are `CudartWatchdogInfo`, `CudartInit9009`,
    // `CudartInit9001`, `CudartInit9064`, `CudartInit9A001` and `CudartPerfLevelInfoV2`; the
    // exception-set entry above carries the per-id provenance (`2d32e5ee` w349, `0f577b15`
    // w352, `3887be37` w353 — all on the w337-gpu-name-seam side, none on `master`'s).
    // ★ Attributed by measurement, not arithmetic: the failing assertion NAMED all six, and
    // the two lists were updated from it. ⚠ It is the WEAKEST kind of addition this pair can
    // see, six times over, and the ratio is now the honest headline — **28 of 47 served
    // controls (59.6 %) are outside this differential's reach**, up from 22 of 41 (53.7 %).
    // The note below still says the exception set is SMALL; it is no longer small, and the
    // sentence is left standing with this correction above it rather than quietly softened.
    assert_eq!(universe.len(), 47, "non-vacuity: the universe is not empty");
    assert_eq!(
        outside_the_closure_limit.len(),
        28,
        "non-vacuity in the other direction: the exception set is SMALL, and every entry \
         costs reply-plane coverage"
    );
    // ⚠⚠ **The cost of 6 -> 15, stated rather than absorbed.** NINE of fifteen exceptions
    // are now `cuInit`-path controls, and a differential that cannot see them cannot regress
    // them. What stands in for it is a policy-boundary test per control —
    // `kayfabe-device/tests/{internal_gpu_get_smc_mode,bus_get_info_v2,
    // bus_get_pcie_supported_gpu_atomics,fb_get_info_v2,ce_get_all_physical_caps,
    // grmgr_get_gr_fs_info,register_fault_buffer}.rs` —
    // which checks the envelope, the inner status and the params offset but NOT that the
    // reply reaches a real guest queue. ⊘ The only instrument that covers that is a boot
    // (`only_live_boots_are_proof`), and the durable fix is a `cuInit`-driven capture: the
    // exception set shrinks by SIX the day one exists, and by nothing at all until then.
    //
    // ★ §14.33's one is the least bad of the five and the reason is worth stating, because
    // "covered elsewhere" is exactly the sentence that lets a gap through unread:
    // `ce_get_all_physical_caps.rs` does not compare this port against itself, it compares
    // it against **two independent real-GA106 captures** and asserts those two agree first.
    // That is a stronger oracle for the *payload* than this differential is. It is a weaker
    // one for the *envelope*, which is the part this file uniquely covers and the part that
    // stays uncovered.

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
fn every_control_the_oracle_asks_is_either_served_or_triaged() {
    // ★★★ **The gate that replaces the pre-flight table nobody kept up to date.**
    //
    // `docs/design/preinit_sweep_loop.md` §4.1 listed six controls it expected the sweep to
    // reach. It was a table in a document, so it could only ever be as current as the last
    // person to edit it — and `t134a`'s defect was precisely a control nobody had written
    // anything about. This derives the universe from the **capture**: every `fn 76` command
    // the C oracle's own boot issues inside the replay's closure limit.
    //
    // Each one must be in `WantedTable::ALL` (this port answers it) or in `SWEEP_TRIAGE`
    // (this port has written down what refusing it does, and why). ⊘ Neither list can be
    // shortened into agreement, because the universe is neither of them.
    //
    // ⚠ It is *not* a demand that everything be served. Most of these are refused on
    // purpose; see `kayfabe_device::sweep::SweepDisposition` for the five outcomes and
    // `sweep_triage.rs` for the gate that says which refusals are allowed.
    let (_t, r) = served();
    let limit = r.closure_limit.expect("this capture has one");

    let asked: BTreeSet<u32> = r
        .commands
        .iter()
        .filter(|(t, _)| *t < limit)
        .filter_map(|(_, c)| control_cmd(c))
        .collect();

    // Non-vacuity, and it is the load-bearing half: a harness that decoded nothing would
    // pass this test with an empty universe.
    assert_eq!(
        asked.len(),
        28,
        "the distinct controls the oracle's boot issues before the wall"
    );

    let unaccounted: Vec<String> = asked
        .iter()
        .filter(|c| WantedTable::from_cmd(**c).is_none() && triage_for(**c).is_none())
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "the oracle's own boot asks these and this port has neither served them nor written          down what refusing them does — which is exactly t134a's defect: {unaccounted:?}"
    );

    // ★★ And the split is counted on both sides, so a control MOVING between the two is a
    // visible change rather than an invisible one. Fourteen served, fourteen refused with a
    // written argument. ⊘ The triage table has 23 rows, not 14+14: eight of the fourteen
    // served controls are ALSO triaged (that is what makes the must-serve gate possible),
    // and `0x20800a4b` is triaged without the oracle ever asking it.
    //
    // ★★★ `0x20800a6c` is the control that most recently CROSSED this line — 13/15 -> 14/14
    // in one commit, after `0x20800301` went 12/16 -> 13/15 in the one before. Both triage
    // rows are kept and corrected rather than deleted, so the argument for refusing each and
    // the argument that overturned it sit side by side.
    //
    // ⚠ `0x20800a70` did NOT cross: it stayed refused and only its DISPOSITION changed
    // (`RefusalHalts` -> `RefusalIsInvisible`), which these two numbers cannot see. That is
    // the limit of this pair as an instrument, and `sweep_triage.rs` is where the class
    // sizes are pinned.
    let served_here: Vec<String> = asked
        .iter()
        .filter(|c| WantedTable::from_cmd(**c).is_some())
        .map(|c| format!("{c:#010x}"))
        .collect();
    // ⚠ 14 -> 17: `0x20800a9f` (seq 45), `0x20800a1f` (seq 49) and `0x20800a26` (seq 51)
    // crossed from triaged to served at the state-load rung. Their three siblings in the
    // same GR run — `0x20800a22`, `0x20800a3d`, `0x20800a48` — are served too but fall
    // PAST this capture's closure limit, so they cannot appear in `asked`.
    // ⚠ 17 -> 18 at the `fmb` rung: `0x20802a08` (seq 18) crossed from triaged to served
    // — and unlike the three above it did NOT cross because a reading was overturned by
    // argument. It crossed because a **real GA106 was asked** and gave a number the C's own
    // captured row does not carry. See `kayfabe_abi::fmbsize`.
    // ⚠ 18 -> 19 at the `GR-info` rung: `0x20800a2a` (seq 50) crossed from triaged to
    // served, and it is the FIRST crossing driven by a BOOT rather than by a reading or a
    // falsified oracle row. Run `fmb1` at `93191ee` showed the refusal killing
    // `kernel_fifo.c:2789` twenty-one engines from the call site that tolerates it.
    assert_eq!(served_here.len(), 19);
    let triaged_here: Vec<&str> = asked
        .iter()
        .filter(|c| WantedTable::from_cmd(**c).is_none())
        .map(|c| triage_for(*c).expect("accounted for above").engine)
        .collect();
    // ⚠ 14 -> 11, the mirror of the three above; then 11 -> 10 at the `fmb` rung as
    // `0x20802a08` crossed. The sum is unchanged at 28, which is the point: this pair
    // partitions the SAME asked set, so a control cannot leave one without entering the
    // other, and a rung that "served" something by dropping it from the trace would show
    // up here as a shrinking sum rather than as two independently plausible numbers.
    assert_eq!(triaged_here.len(), 9);
    assert_eq!(
        served_here.len() + triaged_here.len(),
        28,
        "the partition is over the asked set and its size is not this rung's to change"
    );

    // ⊘ A control the oracle asks may not be triaged `AmputationIntended`: that disposition
    // means "the chip lacks the engine", and the oracle's board demonstrably had it.
    // `0x20800a87` (NVLink) and `0x2080017e` (VMMU) are the two exceptions the argument
    // itself names — the caller tolerates the status by hand in both — and they are listed
    // rather than exempted by a predicate, so a third would be red.
    //
    // ⚠⚠ A third WAS here — `0x20800a2a` (GR info), added at the state-load rung — and it
    // has been REMOVED by run `fmb1` at `93191ee`. Its admission argued that it "qualifies
    // under the SECOND clause of `AmputationIntended`'s own definition — the caller's own
    // tolerance of the status", because `kgraphicsLoadStaticInfo` takes it into a bare
    // `if (status == NV_OK)` with no `else` arm. ⊘ The reading of the CALL SITE was right
    // and the conclusion was wrong: the consumer is `kfifoGetMaxSubcontextFromGr_KERNEL`
    // (`ogkm-580: kernel_fifo.c:2789`), twenty-one engines away, and it does not tolerate
    // anything. `RmInitAdapter failed! (0x25:0x40:1249)`.
    //
    // ★★★ So this list's admission criterion is now known to be WEAK in a specific way: a
    // caller's tolerance is a fact about the caller. That is the third control to enter on
    // that clause and leave on a boot, which is why the two survivors below are named and
    // not derived from a predicate — the predicate is the thing that was wrong.
    let intended: Vec<String> = asked
        .iter()
        .filter(|c| {
            triage_for(**c).is_some_and(|t| t.disposition == SweepDisposition::AmputationIntended)
        })
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert_eq!(intended, vec!["0x2080017e", "0x20800a87"]);
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
            // ★★ Twice, and that is the finding rather than a duplicate: `gpuPreInit`
            // calls `gpuBuildGenericKernelFalconList` (`ogkm-580: gpu.c:2126`) and
            // `gpuBuildKernelVideoEngineList` (`:2128`) back to back, and each issues its
            // own independent control. One chip row answers two adjacent statements.
            (WantedTable::ConstructedFalconInfo, 5, 76, 0),
            (WantedTable::ConstructedFalconInfo, 6, 76, 0),
            // ★★ The largest reply this port posts, and the first that does not fit one
            // queue element: 24620 bytes over SEVEN of them. The C's own capture asks it
            // here, at `gpuStateInit`'s `kfifoConstructEngineList` rather than anywhere in
            // `gpuPreInit` — which is why sequence 8 sits between two `gpuPreInit` rows.
            (WantedTable::InternalDeviceInfo, 8, 76, 0),
            (WantedTable::DeviceInfo, 9, 76, 0),
            (WantedTable::IntrKernelTable, 10, 76, 0),
            // ★★★ The first entry in this list that the oracle reached from a place other
            // than `gpuPreInit`. Sequences 3..=10 are that function's statement chain, one
            // control each, in source order. This one is `KernelMemorySystem`'s
            // `StatePreInit`, inside `gpuStatePreInit_IMPL`'s engine sweep — so the C's own
            // capture is where the ladder stops being a ladder, eleven commands in, and
            // this row is the first evidence of it that costs no boot.
            (WantedTable::MemorySystemStaticConfig, 11, 76, 0),
            (WantedTable::PciBarInfo, 12, 76, 0),
            // ★★★ From here down is the first BATCHED rung — four controls served in one
            // change because the sweep reaches all of them in one boot
            // (`docs/design/preinit_sweep_loop.md` §4.3). Sequences 13 and 14 fail open:
            // their callers hand RM a zeroed destination and swallow the status, so the
            // guest state is the same either way and only the envelope changes.
            (WantedTable::ConfComputeStaticInfo, 13, 76, 0),
            (WantedTable::BifStaticInfo, 14, 76, 0),
            // ★★ Three runlists in a row, and each reply carries the guest's OWN `[IN]`
            // `runlistId` rather than a number of ours. Refusing these three is where the
            // boot stopped: `kfifoChidMgrConstruct` reads a zero channel count as
            // `NV_ERR_INVALID_STATE`, which `gpuStateInit_IMPL` does not map to `NV_OK`.
            (WantedTable::FifoNumChannels, 15, 76, 0),
            (WantedTable::FifoNumChannels, 16, 76, 0),
            (WantedTable::FifoNumChannels, 17, 76, 0),
            // ★★★ The `fmb` rung's control, and the line worth reading twice: the ORACLE
            // asks it here, at sequence 18, so this port serving it is exercised by a real
            // captured boot — while the four bytes it answers with came from a real GA106
            // because the oracle's own row for this control is empty.
            (WantedTable::CeFaultMethodBufferSize, 18, 76, 0),
            // ★★★ The **event-plane** entry, and the only line here whose reply is a
            // function of the request rather than of a chip row. `paylen 60` = 40 header +
            // 20 params, which is an independent confirmation of
            // `EVENT_SET_NOTIFICATION_PARAMS_SIZE` against the oracle's own wire.
            //
            // ⚠ `rc == 0` is doing more work here than on any other line. This control's
            // reply body is copied back over the guest's own params struct
            // (`ogkm-580: rpc.c:11085-11090`) and the guest then switches on
            // `pSetEventParams->action` — so an `NV_OK` carrying an EMPTY body would pass
            // this assertion and still silently re-register notifier 0. What rules that
            // out is `kayfabe_abi::eventnotify`'s own round-trip tests, not this line;
            // this line establishes only that the reply was posted and accepted.
            (WantedTable::EventSetNotification, 25, 76, 0),
            // ★★★ The one whose refusal leaves `pKernelGmmu->pStaticInfo` pointing at
            // memory `_kgmmuInitStaticInfo` already freed (`ogkm-580: kern_gmmu.c:139-166`).
            (WantedTable::GmmuStaticInfo, 26, 76, 0),
            // ★★★ The **action** entries, and the only lines here whose control asks the
            // device to DO something rather than to describe itself. Three of them, which
            // is `kbusVerifyBar2_GM107`'s own signature — it is the only function in the
            // driver that issues this control three times (`ogkm-580:
            // kern_bus_gm107.c:4110`, `:4175`, `:4224`). `paylen 44` = 40 header + 4
            // params, an independent confirmation of
            // `L2_INVALIDATE_EVICT_PARAMS_SIZE` against the oracle's own wire.
            //
            // ⚠ `rc == 0` says less here than anywhere else in this list, and the reason is
            // the OPPOSITE of `EventSetNotification`'s. There the reply body is read back by
            // the guest, so an empty body would pass and be wrong. Here the guest reads
            // nothing at all — `kmemsysSendL2InvalidateEvict_IMPL`'s params are a stack
            // local it never reads after the call — so EVERY body passes this line, and
            // what makes the four-zero body the right one is `kayfabe_abi::l2evict`'s own
            // tests against the oracle's captured `psize 4, dlen 0`.
            (WantedTable::MemsysL2InvalidateEvict, 30, 76, 0),
            (WantedTable::MemsysL2InvalidateEvict, 31, 76, 0),
            (WantedTable::MemsysL2InvalidateEvict, 32, 76, 0),
            // ★ The fourth ask of the channel count, from a runlist the first three did not
            // cover, and the second ask of the CC static info — this one from
            // `confComputeStatePostLoad_IMPL` rather than `StateInitLocked`.
            (WantedTable::FifoNumChannels, 34, 76, 0),
            (WantedTable::ConfComputeStaticInfo, 44, 76, 0),
            // ★★★ The three the state-load rung added, and the reason this list is
            // itemised rather than counted: seq 45 is the page-table PUBLICATION
            // (`0x20800a9f`) — the only entry here whose reply is a function of the request
            // rather than of the chip — and 49/51 are GR's caps and floorsweeping masks.
            // ★★ Seq 50 (`0x20800a2a`, GR info) USED to be deliberately absent here, and
            // this comment used to say so: "it is asked between them and refused, which is
            // why this list is not a contiguous run". Run `fmb1` at `93191ee` turned that
            // deliberate gap into `RmInitAdapter failed! (0x25:0x40:1249)`, so the run is
            // contiguous now — 49, 50, 51 — and its 3712 bytes are the ones a real GA106
            // answered (`kayfabe_abi::grinfo`).
            (WantedTable::GvaspaceServerReservedPdes, 45, 76, 0),
            (WantedTable::GrCaps, 49, 76, 0),
            (WantedTable::GrInfo, 50, 76, 0),
            (WantedTable::GrFloorsweepingMasks, 51, 76, 0),
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
