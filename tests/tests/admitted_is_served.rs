//! ★★★★★ **The `admitted` / `served` gap — measured, scoped, and turned into a ratchet.**
//!
//! # ⊘⊘ FIRST, THE REFUTATION: the wall was NOT invisible. It was UNARGUED.
//!
//! The brief that commissioned this file said of `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`
//! (`0xa06c0101`), the control `cuCtxCreate` died on:
//!
//! > **Why nothing saw it:** it is allowlisted but absent from `OBJECT_CONTROLS`. Clearing
//! > the first gate means **no `FaultTag` is ever built**, so no refusal-census row, no
//! > counter, silent fall to the unserviced ledger.
//!
//! Every clause of the mechanism is true. The conclusion — *"nothing saw it"* — is
//! **false**, and the counter-evidence was already committed to this repository:
//!
//! ```text
//! run_s44_b17381c_rmtrace_qemu.log:149
//!   nvkvm:   unserviced fn 76 cmd 0xa06c0101
//! ```
//!
//! `[measured 2026-08-10, over traces/guest_boots/*_qemu.log]` **six** committed boot logs
//! carry that exact line, by command id, in full: `s39_fd92017_kernelarm`,
//! `s40_4733730_acceptcensus`, `s41b_62e757f_twophase`, `s42_21f967b_gpuscope`,
//! `s43_b17381c_cumjoin`, `s44_b17381c_rmtrace`. The instrument recorded it the first time
//! the port reached that point and every time after.
//!
//! ★★★★ **The defect is not visibility, it is RANK.** `s44`'s ledger prints *42 distinct*
//! unserviced ids in one undifferentiated block. One of them ended `cuCtxCreate`; forty-one
//! were survivable. Nothing in the list says which — the ledger records membership and
//! deliberately nothing else. So the datum sat on disk, correct and complete, for six
//! rungs, and what was missing was **an argument attached to each entry**.
//!
//! ⇒ A gate that made the id *more visible* would have closed nothing. This file instead
//! makes each id **carry a written position**, so a new one cannot appear without somebody
//! stating what they believe about it. `[measured 2026-08-10]` the list it forces is **41
//! other ids**, all of them already on disk in this repository — that number is the real
//! finding, and it is not a flattering one: the instrument was never the problem.
//!
//! # ⊘ SECOND: `admitted ⊆ served` is REFUTED as a literal invariant — measured
//!
//! `[measured 2026-08-10, rev 1f38160]` the bench boundary's capability table admits
//! **163** controls by name; the production chain (`kayfabe_device::served_policy`, object
//! seat filled) has an arm for **21** of them. **142 are admitted and served by nothing.**
//!
//! ★ That is not 142 bugs, and demanding they all be served would be demanding the wrong
//! thing. The two sets are about **different planes**:
//!
//! | set | plane | who decides |
//! |---|---|---|
//! | `capability::CONTROLS_*` | the guest **userspace ioctl** boundary, ported from gVisor `nvproxy` | *may the guest name this at all* |
//! | the served chain | the **GSP RPC** boundary | *what do we answer when the guest's KERNEL forwards one* |
//!
//! Most of the 142 never reach our GSP: the guest's own kernel RM answers them locally out
//! of state it already has, and they cross no boundary we own. Serving them would be
//! building answers for traffic that does not exist. ⇒ The invariant with force is not over
//! the allowlist; it is over **what a boot measured the guest actually sending us**, which
//! is what the assertions below quantify over. The 142 is retained here as a *number with a
//! scope*, because a reader who meets `admitted ⊆ served` in a brief deserves to meet the
//! measurement that bounds it.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

fn abi() -> DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// The production chain with the object seat filled — the shipped composition's answer
/// surface, not a subset of it.
fn chain() -> Box<dyn CommandPolicy> {
    kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        abi(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks {
            publications: None,
            objects: Some(Box::new(kayfabe_rmrpc::ObjectPolicy::new(
                &abi(),
                kayfabe_abi::GuestOs::Linux,
                kayfabe_core::gpu::Gpu::new(
                    Box::new(kayfabe_chips::Ga10xArch::new()),
                    Box::new(kayfabe_isolate::StillbornIsolates::new(
                        "admitted_is_served",
                    )),
                    kayfabe_core::gpa::GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
                )
                .expect("the port's object model realizes"),
                kayfabe_device::ga10x::GA106_ENGINES,
            ))),
        },
    )
}

/// A `GSP_RM_CONTROL` carrying `params_size` bytes of zeros.
fn control(cmd: u32, params_size: usize) -> RpcCommand {
    let mut payload = vec![0u8; 40 + params_size];
    payload[0..4].copy_from_slice(&0xc1e0_0006u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0x0000_000au32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params_size as u32).to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The params sizes each id is probed at. ⊘ Not a guess at the right one — the union is
/// what separates *"nothing claims this id"* from *"nothing claims it at THIS size"*. A
/// link that claims by id and then refuses a wrong `paramsSize` is **served** for this
/// file's purposes: it has an opinion, which is the whole property under test.
const PROBE_SIZES: &[usize] = &[
    0, 3, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 128, 256, 560, 1024,
];

/// Is any link in the chain willing to decide `cmd`?
///
/// ⊘ A fresh chain per probe, deliberately: `kayfabe_device::sticky::StickyAnswerGuard`
/// sits in the production shape and remembers answers, so a reused chain would let one
/// probe's result colour the next one's — a sweep whose earlier questions change its later
/// answers is not a sweep.
fn is_served(cmd: u32) -> bool {
    PROBE_SIZES
        .iter()
        .any(|&n| chain().respond(&control(cmd, n)).is_some())
}

// =====================================================================================
// THE UNIVERSE THAT HAS FORCE: the ids a committed boot log RECORDED
// `[measured 2026-08-10, boots s01…s44 — traces/guest_boots/*_qemu.log]`
// =====================================================================================

/// ★ **w294 — the suffixes that WITNESS a boot**, as opposed to a standalone probe artefact
/// that happens to live in the same directory.
///
/// `[measured 2026-08-14]` the directory's suffix census is `qemu`×129, `dmesg`×128,
/// `probe`×127, `serial`×9, `isolate`×8, `hostdmesg`×6, and then **twenty-odd singletons**
/// (`mtree`, `isolatefd`, `stack`, `census`, `w209ctl`, …) that are one-off captures, not
/// boots. Only the first group means *a guest was booted under this tag*.
///
/// ⊘ `qemu` is deliberately **absent** from this list: a gate whose universe is "tags that
/// have a QEMU log" cannot ever find one missing.
const BOOT_WITNESS_SUFFIXES: &[&str] = &["probe", "dmesg", "serial", "hostdmesg", "harness"];

/// The committed boot logs this gate reads. ⊘ **The whole directory, every suffix** —
/// enumerating one file is how `s43`'s alloc failures were missed (they were in the dmesg
/// log while the probe log, a `dmesg | tail -40`, had scrolled past them:
/// `execution_plane_increments.md` §16.55.4).
const BOOT_LOGS: &str = "traces/guest_boots";

/// The newest boot this file is calibrated against. Named, not inferred: a gate whose
/// universe depends on lexical filename order changes meaning when a tag is added.
///
/// ⚠ **w294 — this constant is a CALIBRATION POINT, not a claim about recency**, and it was
/// read as the latter. It has said `s47_81582e3_ctxsw` since §16.60 while five later boots
/// entered the tree, because the only test that reads it
/// ([`the_s45_wall_is_served_and_the_newest_boot_no_longer_records_it`]) asks whether *that*
/// boot still records `0x20801210` — a question whose answer never expires. Nothing checked
/// that a **newer** boot had arrived, so "newest" quietly stopped being true.
/// ⇒ [`every_committed_boot_tag_has_its_qemu_log`] is the gate that quantifies over the
/// whole directory instead, which is what actually needed doing.
const NEWEST_BOOT: &str = "s47_81582e3_ctxsw";

/// Every `unserviced fn 76 cmd 0x…` id in every committed boot log, mapped to the set of
/// boot tags that recorded it.
///
/// ⊘ Parsed rather than transcribed. A transcribed list is a second copy of the evidence
/// that drifts from it silently, which is the shape this whole file exists to end.
fn ledger_ids() -> BTreeMap<u32, BTreeSet<String>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root is the tests crate's parent")
        .join(BOOT_LOGS);
    let mut out: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut files = 0usize;
    for e in std::fs::read_dir(&dir).expect("the committed boot logs are in the tree") {
        let p = e.expect("a readable dir entry").path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(tag) = name
            .strip_prefix("run_")
            .and_then(|s| s.strip_suffix("_qemu.log"))
        else {
            continue;
        };
        files += 1;
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        for line in text.lines() {
            let Some(rest) = line.split("unserviced fn 76 cmd 0x").nth(1) else {
                continue;
            };
            let hex: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
            if let Ok(cmd) = u32::from_str_radix(&hex, 16) {
                out.entry(cmd).or_default().insert(tag.to_string());
            }
        }
    }
    assert!(
        files >= 40,
        "only {files} boot logs found under {BOOT_LOGS} — the sweep lost its evidence set",
    );
    out
}

/// ★★★ **The graduated set: ids that once reached the unserviced ledger and are now
/// SERVED.**
///
/// ⊘ Kept rather than deleted, and machine-checked in both directions below: an id that
/// leaves the ledger and comes back is a regression, and a list that forgets cannot say so.
/// This is also the only place the gate can show its own direction of travel — every row
/// here was once a row in [`LEDGER`].
static GRADUATED: &[u32] = &[
    // ★ `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`, answered by
    // `kayfabe_device::setpagedir::SetPageDirPolicy` since §16.30.
    0x0080_1813,
    // ★★★★ `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` — the wall `cuCtxCreate` stopped at, answered
    // by `kayfabe_rmrpc::ObjectPolicy` since §16.56. It sat in [`LEDGER`]'s position for
    // SIX committed boots (`s39`…`s44`) with nobody required to say anything about it.
    0xa06c_0101,
    // ★★★★ `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` — the wall at record **331** of
    // 456 `[measured 2026-08-10, boots s45_748a207_tsgsched and s46_1a9e93c_abi35]`,
    // answered by `kayfabe_rmrpc::ObjectPolicy` since §16.59. ⊘ It entered [`LEDGER`] one rung ago, on this gate's very first outing, and
    // leaves it the next: that is the shortest a row has ever sat there, and it is what the
    // gate was built for.
    //
    // ⚠ "Served" here means **classified, then answered** — `NV_OK` for a wait-for-idle
    // request, `NV_ERR_NOT_SUPPORTED` (this control's own documented status,
    // `ogkm-580: ctrl2080gr.h:791-795`) for a request that asks this port to preempt a
    // context. Both are decisions and both leave the ledger.
    0x2080_1210,
    // ★★★★★ `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` — answered by
    // `kayfabe_rmrpc::ObjectPolicy` since §16.75.
    //
    // ⊘ **Its row in [`LEDGER`] was WRONG about the id, not merely incomplete**, and the
    // correction is the rung. That row read *"forgiven every time: the guest asks, we
    // refuse, and it keeps going for another 50 records"* — true of the *stream* and false
    // of the *guest*: `subdeviceCtrlCmdMcServiceInterrupts_IMPL`
    // (`ogkm-580: src/nvidia/src/kernel/gpu/intr/intr.c:219-225`) **returns on the spot**
    // for any non-`NV_OK`, so `intrServiceStallList_HAL` at `:278` — the guest servicing its
    // own stall interrupts for the engines it named — never ran once in thirteen attempts.
    // "The process continued" is not "the request was forgiven": every other id on this list
    // is forgiven by a caller that maps `0x56` to `NV_OK`, and this one was *obeyed*.
    //
    // ⚠ "Served" here means `NV_OK` with the request's `engines` word **echoed**. A zero
    // body would hand the guest `engines = 0` and make its step 2 service the empty set —
    // `ogkm-580: rpc.c:11085-11090` copies the reply's params over the caller's struct
    // whenever `paramsSize != 0`, and it is 4 here.
    0x2080_1702,
    // ★★★★★ **w292 — THE FOUR INPUT-ONLY CONTROLS, and `0x83de0309` is the one that ended
    // `cuCtxCreate`.** Answered by `kayfabe_rmrpc::ObjectPolicy::respond_input_only` since
    // w292, by owner ruling (2026-08-14).
    //
    // ⊘ "Served" here means **`NV_OK` with the guest's OWN BYTES echoed**, and that is the
    // whole reply real hardware gives: `[measured, host_reference_ga106/ctx_r1]` a real
    // GA106 leaves all four parameter blocks byte-identical across the call
    // (`ppost == ppre`). They carry no `[OUT]` field, so there is no body to get wrong —
    // the `#203` zero-fill defect is impossible here rather than merely avoided.
    //
    // ⚠ Each row's authority is recorded beside it in
    // `kayfabe_abi::submit::INPUT_ONLY_CONTROLS`, including the one where the C oracle is
    // **silent rather than negative** (`0xa06c0105` appears zero times in `cap3`).
    0x2081_0108,
    // ⊘⊘⊘ **`0x83de_0309` LEFT THIS LIST ON 2026-08-14 (w295), AND "GRADUATED" WAS THE
    // WRONG WORD FOR IT ALL ALONG.**
    //
    // Graduating means *"it used to fall to the unserviced ledger and now the chain answers
    // it"*. `[measured]` the first half is right — 16 committed boots have it in their
    // ledger. The second half was true for one day and should not have been: the chain
    // answered it while `capability::DENIED_CLASSES` **refused the guest's `RM_ALLOC` of
    // its class**, so what was being answered was a control on an object we do not hold.
    //
    // ⊘⊘ **AND THE FIRST DRAFT OF THIS COMMENT WAS WRONG, REFUTED BY THE BOOT THAT
    // VERIFIED THE CHANGE.** It said the id *"has NOT gone back to the ledger"* and is now
    // *"refused by name as `ControlNotPermitted::Refused`"*. `[measured, boot w295cup2, rev
    // 940c0648]` it goes **straight back to the ledger** — `unserviced fn 76 cmd
    // 0x83de0309`, distinct unserviced ids 40 → 41 — and `grep -c ControlNotPermitted` over
    // that boot's QEMU log is **0**.
    //
    // ★ Why: the capability table is consulted by `translate_control` on the **bridge**
    // plane; the **reply** plane is the seat chain, and a control no seat claims falls to
    // the `UnservicedLedger` without the capability answer ever becoming the wire's answer.
    // ⇒ This file's own lesson — *"ADMITTED and SERVED are different gates"* — in the
    // **refusal** direction. Retracting the `OBJECT_CONTROLS` row is what moved the id; the
    // class gate is what makes the TABLE consistent. Two acts, one visible in a boot.
    // Its `LEDGER` row records the position it actually has.
    0xa06c_0103,
    0xa06c_0105,
    // ★★★★★ **w294 — THE CUDA PERF LIMIT PAIR, AND THEY ENTERED AND LEFT THIS LIST IN THE
    // SAME COMMIT, FOR A REASON THAT INDICTS THE GATE.**
    //
    // `[measured 2026-08-14]` both ids sit in `run_w290pdrain_qemu.log`'s unserviced ledger.
    // That boot ran on 2026-08-13 and **its QEMU log was never committed** — only its
    // `probe`/`dmesg`/`hostdmesg`/`harness` logs were — so this gate's universe
    // (`traces/guest_boots/*_qemu.log`) could not see them, `files >= 40` still passed on
    // the forty older logs, and the gate reported healthy while blind to the newest boot.
    // `every_committed_boot_tag_has_its_qemu_log` is the assertion that ends that, and it
    // fires on this tree as it stood before this commit — a known-positive, not a hope.
    //
    // ⚠ "Served" here means `NV_OK` with the guest's own bytes, via
    // `ObjectPolicy::respond_input_only`. What it asserts: `0x00802009` declares *"clocks
    // are limited on CUDA's behalf"* on a device that models **no clock domain at all**, so
    // there is no observable it can make false — the `kayfabe_device::inert` eligibility
    // rule, applied to a control. `0x00802004` is its teardown half, and refusing THAT is
    // the actively unsafe side: `deviceKPerfCudaLimitCliDisable`
    // (`ogkm-580: kern_cuda_limit.c:62-75`) checks our status **before** `nCudaLimitRefCnt
    // = 0`, so a refusal leaves the guest's own refcount permanently non-zero.
    //
    // ⊘ The id a reader will look for — `0x00801909` — is deliberately absent; it cannot
    // reach us. See `kayfabe_abi::submit::PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES` and
    // `the_cuda_limit_pair_is_served_and_the_ioctl_id_is_not`.
    0x0080_2004,
    0x0080_2009,
];

static LEDGER: &[u32] = &[
    0x0080_0294,
    0x0080_1814,
    0x2080_012c,
    // ★★ `NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS`. `0x56` is the **correct** answer, not a
    // gap: a GeForce GA106 has no ECC and real hardware returns exactly it — the C returns
    // it deliberately (`C: nvkvm_gpu_emul.c:3111`). Recorded here anyway, because the point
    // of the list is that every id carries a belief.
    0x2080_012f,
    0x2080_013f,
    0x2080_014b,
    0x2080_0157,
    0x2080_017e,
    0x2080_0a1e,
    0x2080_0a2c,
    0x2080_0a2e,
    0x2080_0a30,
    0x2080_0a34,
    0x2080_0a38,
    0x2080_0a3f,
    0x2080_0a4b,
    0x2080_0a70,
    0x2080_0a80,
    0x2080_0a87,
    0x2080_0a9a,
    0x2080_0a9c,
    0x2080_0a9e,
    0x2080_0ab8,
    0x2080_0afe,
    0x2080_0aff,
    0x2080_0b03,
    0x2080_0b05,
    // ★★★★ §16.57 — **THE NEW FIRST WALL**, and the five rows below are this gate firing
    // on its FIRST outing, on the very next boot. `[measured 2026-08-10, boot
    // s45_748a207_tsgsched]` `every_unserviced_id_a_boot_recorded_is_classified` went red
    // the moment `s45`'s log entered the tree and named exactly these five — none of which
    // any earlier boot had reached, because `cup2` had never got this far.
    //
    // ⊘ `0x20801210` LEFT this list at §16.59, and `0x20801702` at §16.75 — both are in
    // [`GRADUATED`] now. Each sat here for exactly one rung, which is the shortest any id has
    // sat in this position and is what the gate was built to produce.
    //
    // `NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK` on a `GT200_DEBUGGER`
    // (`ogkm-580: ctrl/ctrl83de/ctrl83dedebug.h:225`, `class/cl83de.h:33`) — libcuda arming
    // SM exception reporting on `0x5c000072`. Forgiven: the object is freed 18 records
    // later and `cup2` continues past it.
    // `NVA06C_CTRL_CMD_SET_TIMESLICE` and `NVA06C_CTRL_CMD_PREEMPT`, both on the TSG. ★
    // Both arrive **after** teardown has begun (records 344 and 352, inside the `FREE`
    // burst), so neither is a wall: they are RM tearing the group down. ⊘ Recorded anyway —
    // an id whose position in the stream is the whole of its meaning is exactly the kind
    // this list must not let pass silently.
    0x2080_1357,
    0x2080_2068,
    0x2080_2a0f,
    0x2080_2a12,
    0x2080_8513,
    0x2080_852e,
    0x2080_9009,
    0x2080_a612,
    0x2080_a618,
    // ★ `NV2081_BINAPI` — the §14.26 "phantom". Admitted by the `BinApiRule` rather than by
    // a table row, which is the admission class this file's module doc says it cannot sweep.
    0x2081_0110,
    0x208f_1105,
    // ★★★ `NV40_I2C`'s control, and it is the PRECEDENT for the row below it: a control on
    // a class this port denies, which has sat here correctly the whole time.
    0x402c_0101,
    // ★★★★★ **`0x83de0309` — BACK ON THIS LIST ON 2026-08-14 (w295), AND ITS POSITION IS
    // THE OPPOSITE OF THE ONE IT LEFT WITH.**
    //
    // `[measured, 16 committed boots]` `p1b_29e7c25_planectl` … `w216_f5f55ad_mcbudget` all
    // have it in their unserviced ledger. w292 graduated it — the chain began answering
    // `NV_OK` with the guest's own 4 bytes echoed, on owner ruling, and the ruling's
    // premise was measured and correct: RM's default exception mask when the control is
    // never called is `_ALL`, and libcuda asks for `0x3a`, which excludes `_FATAL`.
    //
    // ⊘⊘ **What the ruling could not have weighed is that we refuse the OBJECT.**
    // `GT200_DEBUGGER` (`0x83de`) is in `capability::DENIED_CLASSES` and
    // `[measured 2026-08-14, run_w294cup2_qemu.log]` the guest's `RM_ALLOC` of it comes
    // back `AllocClassNotPermitted::Refused id=0x000083de`. ⇒ "Serving" it set nothing,
    // because there was nothing on our side to set. *"Refusing is more permissive than
    // serving"* holds only where serving actually writes; here both answers left the guest
    // on RM's `_ALL` default, and only one of them told the truth about it.
    //
    // ★ **The position, stated:** this id is refused, by name, with `GT200_DEBUGGER`'s own
    // reason, for exactly as long as the class is denied — and it is admitted again the
    // instant the class is, with no edit here. Which of those two the port should be is an
    // OWNER ruling on security surface; the evidence, and the boundary admitting the class
    // would widen, are set out in `docs/design/class_control_consistency.md`.
    //
    // ⊘⊘ **A PREDICTION MADE HERE WAS WRONG WITHIN THE HOUR, AND IS LEFT AS THE
    // CORRECTION IT EARNED.** The first draft said *"it will stop appearing in future
    // boots' unserviced ledgers — a named refusal answers the command and never reaches
    // that list."* `[measured, boot w295cup2]` it appears there **on the very next boot**:
    // `unserviced fn 76 cmd 0x83de0309`, and the boot's distinct-unserviced count went
    // 40 → 41 with that id as the +1. The capability answer never reaches the reply plane.
    // ⇒ ★ A table edit is not a wire fact, and the difference is one boot wide.
    0x83de_0309,
    0xa06f_0112,
];

// =====================================================================================
// The gate
// =====================================================================================

/// ★★★★★ **Every control a committed boot RECORDED as unserviced is listed in [`LEDGER`]
/// — and every listed id is still unanswered.** `[measured 2026-08-10, boots s01…s44]`
///
/// This is the gate that would have fired at `s39`, six rungs before `s44` named the wall:
/// `0xa06c0101` entered the ledger there, and adding its row would have meant writing down
/// a belief about a control on `cuCtxCreate`'s own critical path.
#[test]
fn every_unserviced_id_a_boot_recorded_is_classified() {
    let seen = ledger_ids();
    let listed: BTreeSet<u32> = LEDGER.iter().copied().collect();
    assert_eq!(
        listed.len(),
        LEDGER.len(),
        "`LEDGER` has a duplicate id — a set that repeats itself is not a list of decisions",
    );

    // Direction 1 — a NEW unserviced id must be listed.
    let unclassified: Vec<String> = seen
        .iter()
        .filter(|(cmd, _)| !listed.contains(cmd) && !is_served(**cmd))
        .map(|(cmd, boots)| {
            let mut b: Vec<&str> = boots.iter().map(String::as_str).collect();
            b.sort_unstable();
            format!(
                "{cmd:#010x}  (recorded by {} boot(s): {})",
                b.len(),
                b.join(", ")
            )
        })
        .collect();
    assert!(
        unclassified.is_empty(),
        "★★★ {} control id(s) reached the unserviced ledger in a committed boot and this \
         port has no recorded position on them. ⊘ Do not just add rows: the whole reason \
         this gate exists is that `0xa06c0101` sat in exactly this position for SIX boots \
         while `cuCtxCreate` died on it. Serve it, or list it and say in the comment what \
         you believe:\n  {}",
        unclassified.len(),
        unclassified.join("\n  "),
    );

    // Direction 2 — a listed id that is now SERVED must MOVE to `GRADUATED`, or the list
    // rots into a permanent excuse.
    let stale: Vec<String> = listed
        .iter()
        .filter(|cmd| is_served(**cmd))
        .map(|cmd| format!("{cmd:#010x}"))
        .collect();
    assert!(
        stale.is_empty(),
        "★ these ids are listed as unserviced but the chain now answers them — move them to \
         `GRADUATED`, so `LEDGER` keeps meaning \"what we do not answer\":\n  {}",
        stale.join("\n  "),
    );

    // Direction 3 — no phantom rows. A position on an id no boot ever recorded is a
    // position about nothing, and it dilutes the list exactly as boilerplate would.
    let phantom: Vec<String> = listed
        .iter()
        .filter(|cmd| !seen.contains_key(cmd))
        .map(|cmd| format!("{cmd:#010x}"))
        .collect();
    assert!(
        phantom.is_empty(),
        "★ these ids are listed but no committed boot log ever recorded them:\n  {}",
        phantom.join("\n  "),
    );
}

/// ⊘ **`GRADUATED` is CHECKED in both directions, not asserted by its author.** A
/// classification a machine can verify and does not is a comment.
///
/// ★ The second half is the one that earns the list: an id that once reached the ledger and
/// is now answered must **stay** answered. A silent regression here is a control this port
/// used to decide and stopped deciding, which is the §14.21 shape exactly (a claim landed,
/// killed the adapter, and was reverted) — except that a revert nobody records reads as
/// "this was never served".
#[test]
fn every_graduated_id_was_once_in_the_ledger_and_is_still_answered() {
    let seen = ledger_ids();
    for &cmd in GRADUATED {
        assert!(
            seen.contains_key(&cmd),
            "{cmd:#010x} is listed as graduated, but no committed boot log ever recorded it \
             as unserviced — it cannot have graduated from a position it never held",
        );
        assert!(
            is_served(cmd),
            "★★★ {cmd:#010x} REGRESSED: it is recorded as graduated out of the unserviced \
             ledger, and the chain no longer answers it",
        );
    }
    assert!(
        !GRADUATED.is_empty(),
        "the graduated list is empty — this gate can no longer show its own direction",
    );
}

/// ★★★★★ **§16.59/§16.60 — `0x20801210` is served, and the boot that proves it also proves
/// serving it was not enough.** `[measured 2026-08-10, boot s47_81582e3_ctxsw]`
///
/// The transition, in the same shape as [`the_s44_wall_is_recorded_by_the_boots_and_answered_by_the_port`]:
/// the earlier boot logs record the id as unserviced, the newest one does not, and the chain
/// answers it. Record 331 went `status=0x56` → `status=0x00000000`.
///
/// ⊘⊘ **And record 332 still begins the `FREE` burst.** That is the finding this test exists
/// to keep attached to the id: `0x20801210` was named "the wall" because it was the last
/// non-zero record before teardown, it is now zero, and teardown starts in the same place.
/// A future reader who finds this control served must not infer that it ever blocked
/// anything — see §16.60.
#[test]
fn the_s45_wall_is_served_and_the_newest_boot_no_longer_records_it() {
    let cmd = kayfabe_abi::submit::NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE;
    let seen = ledger_ids();
    let boots = seen
        .get(&cmd)
        .expect("0x20801210 is recorded as unserviced by the s45/s46 boot logs");
    assert!(
        boots
            .iter()
            .any(|t| t.starts_with("s45_") || t.starts_with("s46_")),
        "★ the evidence that this id WAS a wall has left the tree: {boots:?}",
    );
    assert!(
        !boots.iter().any(|t| t == NEWEST_BOOT),
        "★★★ {NEWEST_BOOT} still records 0x20801210 as unserviced — the control is claimed \
         in this tree, so either the boot ran an older binary or the seat is not reached",
    );
    assert!(
        is_served(cmd),
        "★★★ 0x20801210 is unserved — §16.59 claimed it",
    );
    assert!(
        GRADUATED.contains(&cmd) && !LEDGER.contains(&cmd),
        "0x20801210 is served: it belongs in GRADUATED and not in LEDGER",
    );
}

/// ★★★★ **The wall itself: `0xa06c0101` is served, and it is the one id that LEFT the
/// ledger this rung.**
///
/// ⊘ Asserted here and not only in `gpfifo_schedule.rs` because this is the file that can
/// state it as a *transition*: the boot logs still record the id (six of them do), and the
/// chain now answers it — which is precisely the shape "a wall was removed" has.
#[test]
fn the_s44_wall_is_recorded_by_the_boots_and_answered_by_the_port() {
    let cmd = kayfabe_abi::submit::NVA06C_CTRL_CMD_GPFIFO_SCHEDULE;
    let seen = ledger_ids();
    let boots = seen
        .get(&cmd)
        .expect("0xa06c0101 is recorded by the committed boot logs");
    assert!(
        boots.len() >= 6,
        "★ only {} boot(s) record 0xa06c0101 — the evidence for the refutation in this \
         file's module doc has moved, and the doc must move with it",
        boots.len(),
    );
    // ★★★★ §16.57 — and it is GONE from the newest boot, which is the transition itself.
    // `[measured 2026-08-10, boot s45_748a207_tsgsched]` record 196 — the same client, the
    // same TSG handle, the same three bytes — returns `status=0x00000000`, and the guest
    // goes on to schedule two more groups and issue 207 more RM ioctls.
    assert!(
        !boots.iter().any(|t| t == NEWEST_BOOT),
        "★★★ {NEWEST_BOOT} still records 0xa06c0101 as unserviced — the control is claimed \
         in this tree, so either the boot ran an older binary or the seat is not reached",
    );
    assert!(
        is_served(cmd),
        "★★★ 0xa06c0101 is unserved — this is the exact control `cuCtxCreate` stopped at, \
         record 196 of s44's 249",
    );
    assert!(
        !LEDGER.contains(&cmd),
        "0xa06c0101 must not be listed as unserviced — it is served",
    );
    assert!(
        GRADUATED.contains(&cmd),
        "0xa06c0101 must be recorded in `GRADUATED` — it is the id this rung moved",
    );
}

// =====================================================================================
// The scoped `admitted` count — reported, and NOT demanded
// `[measured 2026-08-10, rev 1f38160 + this increment]`
// =====================================================================================

/// ⊘ **The `admitted ⊆ served` number, pinned as MEMBERSHIP so it cannot drift silently —
/// and explicitly NOT a demand that the gap be closed.**
///
/// See this file's module doc for why the literal invariant is refuted: the allowlist gates
/// the guest's *userspace ioctl* surface, the chain answers the *GSP RPC* surface, and most
/// of the difference is traffic the guest's own kernel answers without ever reaching us.
///
/// What is asserted is the one thing that must not change quietly: **which** admitted ids
/// the chain answers. A row leaving this set is a control this port stopped deciding, and
/// that has been a real regression here before (§14.21 claimed `0x2080012b`, measured it
/// killing the adapter, and reverted).
#[test]
fn the_admitted_controls_the_chain_answers_are_exactly_these() {
    let served: Vec<String> = abi()
        .capabilities()
        .all_controls()
        .map(|e| e.cmd)
        .collect::<BTreeSet<u32>>()
        .into_iter()
        .filter(|&cmd| is_served(cmd))
        .map(|cmd| format!("{cmd:#010x}"))
        .collect();
    // `[measured 2026-08-10, rev 1f38160 + §16.56]`. ⊘ Transcribed FROM the failing
    // assertion, not predicted: the first draft of this list was written from the docs and
    // was wrong in eleven places, which is the same "a plausible-looking constant is not a
    // sourced one" the schedule doc records about three status codes.
    let expected = [
        "0x00801813", // NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY — SetPageDirPolicy, §16.30
        // ★★★★★ **w294 — the CUDA perf limit pair.** ADDED, and the ratchet made the
        // addition argued rather than silent: this assertion went red the moment the rows
        // landed, which is what it is for. ⊘ The id a reader will look for — `0x00801909` —
        // is deliberately NOT here and never can be: it is not `ROUTE_TO_PHYSICAL`, so the
        // guest's own kernel answers it. See `submit::PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES`.
        "0x00802004", // NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_DISABLE — respond_input_only
        "0x00802009", // NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_SET_CONTROL — respond_input_only
        "0x20800102", // InitTablePolicy
        "0x2080012b", // NV2080_CTRL_CMD_GPU_PROMOTE_CTX — ObjectPolicy, §14.25
        "0x20800301", // NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION — InitTablePolicy
        "0x20800a9f", // a publication control — InitTablePolicy
        // ★★★★ §16.59 — `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`, `ObjectPolicy`.
        // The wall `s45`/`s46` measured at record 331.
        "0x20801210",
        "0x20801303", // InitTablePolicy
        // ★★★★★ §16.75 — `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`, `ObjectPolicy`. The 1 Hz
        // train `w209` measured, and the one id whose `0x56` CANCELLED guest work
        // (`ogkm-580: intr.c:219-225` returns before `intrServiceStallList_HAL` at `:278`).
        "0x20801702",
        "0x20801803", // InitTablePolicy
        "0x20801823", // InitTablePolicy
        "0x2080182a", // InitTablePolicy
        "0x2080182b", // InitTablePolicy
        "0x20802a02", // InitTablePolicy
        "0x20803601", // InitTablePolicy
        "0x20803801", // InitTablePolicy
        // ★★★★★ **w292 — THE INPUT-ONLY GROUP, `ObjectPolicy::respond_input_only`.**
        // The two remaining members are `0xa06c0103`/`0xa06c0105`, below. ⊘ `0x2081_0108`
        // does NOT appear in this list because it is admitted by a RULE rather than by a
        // named row, and this list quantifies over `all_controls()`.
        //
        // ⊘⊘⊘ **`0x83de0309` LEFT THIS LIST ON 2026-08-14 (w295), and this list's own
        // warning — *"LOSING one is a control this port stopped deciding"* — is EXACTLY
        // BACKWARDS for it.** The port did not stop deciding it; it started deciding it in
        // the only plane that could be right. This list quantifies over controls the
        // capability table ADMITS, and `0x83de0309` is no longer admitted: its class
        // `GT200_DEBUGGER` (`0x83de`) is denied, and `CapabilityTable::control` now refuses
        // every control scoped to a denied class. So it is decided — as
        // `ControlNotPermitted::Refused`, one layer above the chain, with the class's own
        // reason attached — and it leaves this list because this list is about the chain.
        // ⇒ ★ This is the one shape a "losing a row is a regression" ratchet cannot judge
        // on its own, which is why the reason is written here rather than the row deleted.
        // ★★★★★ **w288 TIER 2 — `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`, `ObjectPolicy`.**
        // ADDED, and adding is the direction this list welcomes: it is the ONLY control that
        // carries a fault's ADDRESS, so *"the guest observed THE SAME FAULT, by identity"* is
        // unanswerable without it. ⊘ It is served by RELAY to the corresponding host channel
        // and never by synthesis — see `ObjectPolicy::respond_get_mmu_fault_info`.
        "0x906f0106",
        "0x90f10106", // the gvaspace PDE publication — InitTablePolicy
        "0xa06c0101",
        // ★★★★★ w292 — `SET_TIMESLICE` and `PREEMPT`, the other two input-only rows.
        "0xa06c0103",
        "0xa06c0105", // ★★★★ NVA06C_CTRL_CMD_GPFIFO_SCHEDULE — ObjectPolicy, §16.56
        "0xa06f0103", // NVA06F_CTRL_CMD_GPFIFO_SCHEDULE — ObjectPolicy, #177
        "0xa06f0104", // NVA06F_CTRL_CMD_BIND — ObjectPolicy, E9/§13.6
    ];
    assert_eq!(
        served, expected,
        "the set of ADMITTED controls the chain answers changed. Adding one is progress and \
         belongs in this list; LOSING one is a control this port stopped deciding",
    );
}

/// ⊘⊘ **THE INVERSION — and it is the half a gate over `capability.rs` alone gets exactly
/// backwards.** `[measured 2026-08-10]`
///
/// The brief's `admitted ⊆ served` is refuted numerically (see this file's module doc: 142
/// of 163). It is also **inverted** for part of the surface: there are controls the chain
/// **serves** and the allowlist does **not admit**. `capability.rs` is ported from gVisor
/// `nvproxy` and gates the guest's *userspace ioctl* boundary; `kayfabe_device::inittables`
/// answers the *GSP RPC* boundary. The two sets are not nested in either direction.
///
/// ⇒ A gate built over `capability.rs` alone would report that this port refuses things it
/// answers. **Any gate here must name which surface it quantifies over** — and this one
/// does, in both directions, by pinning the difference as membership.
#[test]
fn the_chain_serves_controls_the_allowlist_does_not_admit() {
    let admitted: BTreeSet<u32> = abi().capabilities().all_controls().map(|e| e.cmd).collect();
    let served_unadmitted: Vec<String> = kayfabe_device::inittables::WantedTable::ALL
        .iter()
        .map(|w| w.cmd_id())
        .filter(|cmd| !admitted.contains(cmd) && is_served(*cmd))
        .map(|cmd| format!("{cmd:#010x}"))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    assert!(
        !served_unadmitted.is_empty(),
        "★ the inversion vanished — either the two surfaces converged (a real event worth \
         recording) or this test stopped reaching one of them",
    );
    // ⊘ Pinned as MEMBERSHIP. A count would let one id leave as another arrived.
    assert_eq!(
        served_unadmitted, SERVED_BUT_NOT_ADMITTED,
        "the set of controls this port ANSWERS but its userspace-ioctl allowlist does not \
         ADMIT has changed. Neither direction is automatically wrong — but each one is a \
         statement about which boundary a command crosses, and it must be a deliberate one",
    );
}

/// ★★★★ **The measured inversion, pinned: 29 controls this port ANSWERS that its
/// userspace-ioctl allowlist does not ADMIT.** `[measured 2026-08-10, rev 2fa5d84]`
///
/// ⊘ Two of these were named to me as the counter-examples and both check out:
/// `0x20802a08` (`CE_GET_FAULT_METHOD_BUFFER_SIZE`) and `0xa06c010a`. The rest are the
/// `NV2080_CTRL_CMD_INTERNAL_*` family — kernel RM's own GSP traffic, which by definition
/// never crosses a userspace ioctl boundary and so has no business on an `nvproxy`-derived
/// allowlist. ⇒ The two surfaces are **not nested in either direction**, and that is a fact
/// about the planes rather than a defect in either table.
static SERVED_BUT_NOT_ADMITTED: &[&str] = &[
    "0x208001b0",
    "0x20800a1c",
    "0x20800a1d",
    "0x20800a1f",
    "0x20800a22",
    "0x20800a26",
    "0x20800a2a",
    "0x20800a32",
    "0x20800a36",
    "0x20800a3d",
    "0x20800a40",
    "0x20800a41",
    "0x20800a48",
    "0x20800a4c",
    "0x20800a59",
    "0x20800a5c",
    "0x20800a61",
    "0x20800a6c",
    "0x20800a9b",
    "0x20800a9d",
    "0x20800aac",
    "0x20800af3",
    "0x20801112",
    "0x20802a07",
    "0x20802a08",
    "0x20802a0b",
    "0x20808159",
    "0x20808162",
    "0xa06c010a",
];

/// ⊘ **Non-vacuity of the probe.** A sweep whose instrument can only ever say one thing
/// passes every assertion above while checking nothing.
#[test]
fn the_probe_can_both_answer_and_decline() {
    assert!(
        abi().capabilities().all_controls().count() > 100,
        "the admitted universe collapsed — the sweep is asserting about nothing",
    );
    assert!(
        is_served(kayfabe_abi::submit::NVA06F_CTRL_CMD_GPFIFO_SCHEDULE),
        "the probe cannot detect a control the object seat certainly claims",
    );
    assert!(
        !is_served(0xdead_0000),
        "the probe answers a command nobody claims — it cannot detect a gap",
    );
}

/// ★★★★★ **w292 — THE FOUR INPUT-ONLY CONTROLS ARE CLAIMED, DECIDED, AND ADMITTED.**
///
/// The gap this closes is the one `traces/nvdiff_w292` measured ending `cuCtxCreate`:
/// `0x83de0309` was **admitted by the capability table and served by nothing**, so it fell
/// to the unserviced ledger as `0x56`. Three assertions, because three different things
/// had to be true at once and any one of them alone would have been a fix that did not
/// fire:
///
/// 1. every row of `INPUT_ONLY_CONTROLS` is **claimed** by `OBJECT_CONTROLS` — else the
///    dispatch never runs and the ledger answers;
/// 2. every row is **permitted** by the capability table — else the bridge refuses it one
///    layer up and the answer is still `0x56`, which is exactly the trap of fixing one
///    gate and reporting the wall as closed;
/// 3. every row carries a **non-empty authority** — a served id with nobody's name on it
///    is the shape this file exists to prevent.
#[test]
fn the_w292_input_only_controls_are_claimed_and_decided() {
    use kayfabe_abi::submit::INPUT_ONLY_CONTROLS;
    assert!(!INPUT_ONLY_CONTROLS.is_empty(), "the table must not be empty");
    for row in INPUT_ONLY_CONTROLS {
        assert!(
            kayfabe_rmrpc::OBJECT_CONTROLS.contains(&row.cmd),
            "0x{:08x} {} is in INPUT_ONLY_CONTROLS but NOT claimed by OBJECT_CONTROLS — it \
             would fall to the unserviced ledger as 0x56, which is the exact defect w292 \
             measured",
            row.cmd,
            row.name
        );
        // ⊘ **w294 — `> 0` WAS AN OVER-TIGHT PROXY FOR "somebody measured this".** A real
        // control can have `paramsSize == 0`: `NV0080_CTRL_CMD_INTERNAL_PERF_CUDA_LIMIT_DISABLE`
        // is `/*paramSize=*/ 0 /* Singleton parameter list */`
        // (`ogkm-580: g_device_nvoc.c:1011`) and its only caller passes `NULL, 0`
        // (`kern_cuda_limit.c:64-69`). So zero is allowed — **but only by name**, so that
        // "measured as zero" can never be confused with "nobody filled the field in".
        const MEASURED_ZERO_PARAMS: &[u32] = &[0x0080_2004];
        assert!(
            row.params_size > 0 || MEASURED_ZERO_PARAMS.contains(&row.cmd),
            "0x{:08x} {} has params_size 0 and is not on MEASURED_ZERO_PARAMS — a zero here \
             is indistinguishable from an unfilled field unless the id is named",
            row.cmd,
            row.name
        );
        assert!(
            !row.authority.trim().is_empty(),
            "0x{:08x} {} is served with NO STATED AUTHORITY",
            row.cmd,
            row.name
        );
        assert_eq!(
            kayfabe_abi::submit::input_only_control(row.cmd).map(|r| r.cmd),
            Some(row.cmd),
            "0x{:08x} is not findable through its own lookup",
            row.cmd
        );
    }
}

/// ★★★★★ **w294 — THE SEAM, AND IT IS NOT AN ID. A BOOT WHOSE `_qemu.log` WAS NEVER
/// COMMITTED IS A BOOT THIS ENTIRE FILE CANNOT SEE.**
///
/// # The defect, measured on this tree
///
/// `[measured 2026-08-14]` `traces/guest_boots/` carried `run_w290pdrain_{probe,dmesg,
/// hostdmesg,harness}.log` and **no `run_w290pdrain_qemu.log`**. That boot — the newest in
/// the tree, and the one `traces/nvdiff_w292` is written about — put **two ids in its
/// unserviced ledger that appear in no other boot**: `0x00802009` and `0x00802004`. Neither
/// was in [`LEDGER`], neither was in [`GRADUATED`], and
/// [`every_unserviced_id_a_boot_recorded_is_classified`] **passed**, because the file that
/// recorded them was not in the universe it quantifies over.
///
/// ⊘⊘ **And nothing looked wrong.** [`ledger_ids`]'s own guard is `files >= 40`, which the
/// forty older logs satisfy on their own; the tag's other four logs were present, freshly
/// timestamped and correctly named. **Every signal said the evidence was there.** This is
/// the tree's serial-log trap in a new place: *an artefact that is absent reads as an
/// artefact that had nothing to say.*
///
/// # Why this gate and not "assert the newest boot is listed"
///
/// A gate naming one boot is a gate that ages (see [`NEWEST_BOOT`]). This one is a
/// **closure property over the directory**: any tag that produced *any* committed log must
/// have produced its `_qemu.log`, because that file is the only instrument in the tree that
/// records the GSP-RPC boundary — `unserviced fn 76 cmd 0x…`. An ioctl capture cannot
/// substitute for it: `[measured]` `0x00802009` appears in **zero** ioctl captures and
/// **can** appear in none, being `RMCTRL_FLAGS_INTERNAL`.
///
/// ★ **Known-positive, and it fired on first run rather than being hoped for:** against the
/// tree as it stood before this commit it named `w290pdrain`. `[measured]` it also named two
/// tags this gate should NOT have claimed — see [`BOOT_WITNESS_SUFFIXES`].
#[test]
fn every_committed_boot_tag_has_its_qemu_log() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root is the tests crate's parent")
        .join(BOOT_LOGS);
    // Every `run_<tag>_<suffix>.log` in the directory, grouped by tag.
    let mut tags: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in std::fs::read_dir(&dir).expect("the committed boot logs are in the tree") {
        let p = e.expect("a readable dir entry").path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix("run_").and_then(|s| s.strip_suffix(".log")) else {
            continue;
        };
        // `run_<tag>_<suffix>.log` — the suffix is everything after the last `_`.
        let Some((tag, suffix)) = rest.rsplit_once('_') else {
            continue;
        };
        tags.entry(tag.to_string())
            .or_default()
            .insert(suffix.to_string());
    }
    // ⊘ **A TAG IS ONLY A BOOT IF SOMETHING WITNESSES A BOOT.** `[measured]` the first draft
    // quantified over every `run_*_*.log` and named `w224d (has: isolatefd)` and
    // `w224m (has: mtree)` — standalone probe artefacts that were never boots and have no
    // QEMU log to be missing. ⚠ This narrowing is the one move that has to be justified
    // rather than made: it is principled because the property under test is *"a boot's
    // GSP-RPC record is in the tree"*, and a tag with no boot-witness log did not boot. It
    // is **not** the gate being tuned until it is green — the known-positive `w290pdrain`
    // carries four of these suffixes and is still named when its QEMU log is absent.
    let booted: BTreeMap<&String, &BTreeSet<String>> = tags
        .iter()
        .filter(|(_, sfx)| sfx.iter().any(|s| BOOT_WITNESS_SUFFIXES.contains(&s.as_str())))
        .collect();
    assert!(
        booted.len() >= 40,
        "only {} boot tags found under {BOOT_LOGS} — this gate lost its universe",
        booted.len(),
    );
    let tags = booted;
    let blind: Vec<String> = tags
        .iter()
        .filter(|(_, sfx)| !sfx.contains("qemu"))
        .map(|(tag, sfx)| {
            let mut s: Vec<&str> = sfx.iter().map(String::as_str).collect();
            s.sort_unstable();
            format!("{tag}  (has: {})", s.join(", "))
        })
        .collect();
    assert!(
        blind.is_empty(),
        "★★★★★ {} boot tag(s) committed logs but NOT their `_qemu.log`. That file is the \
         ONLY instrument in this tree that records the GSP-RPC boundary (`unserviced fn 76 \
         cmd 0x…`), so every gate in this file is BLIND to those boots while reporting \
         healthy — which is exactly how `0x00802009` and `0x00802004` stayed unclassified. \
         ⊘ Do not delete the tag's other logs to satisfy this: commit the QEMU log.\n  {}",
        blind.len(),
        blind.join("\n  "),
    );
}

/// ★★★★★ **w294 — TWO BOUNDARIES, TWO IDS, ONE EVENT: `0x00801909` MUST NOT BE SERVED AND
/// `0x00802009`/`0x00802004` MUST BE.**
///
/// The obvious reading of `traces/nvdiff_w292/serve_r1.jsonl.zst` record **412** — the
/// guest's `NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL` answered `0x56` where a native
/// GA106 answers `NV_OK` — is *"serve `0x00801909`"*. That change would have compiled,
/// passed every gate in this file, and **done nothing**, because the id cannot reach us:
/// `flags=0x118`, no `ROUTE_TO_PHYSICAL` (`ogkm-580: g_device_nvoc.c:920`), so the guest's
/// own kernel answers it and only its internal consequence is RPC'd to us.
///
/// ⇒ This test pins the direction of the fix, in both polarities, so a later lane cannot
/// quietly move the row to the id the ioctl recorder shows.
#[test]
fn the_cuda_limit_pair_is_served_and_the_ioctl_id_is_not() {
    let (ioctl_id, set_control, disable) = kayfabe_abi::submit::PERF_CUDA_LIMIT_THE_ID_THAT_ARRIVES;

    assert!(
        !is_served(ioctl_id),
        "★★★ 0x{ioctl_id:08x} is served, and it CANNOT ARRIVE — it is not \
         ROUTE_TO_PHYSICAL, so the guest's own kernel answers it. A row for it is an answer \
         to traffic that does not exist, which from outside is indistinguishable from a fix",
    );
    assert!(
        !kayfabe_rmrpc::OBJECT_CONTROLS.contains(&ioctl_id),
        "0x{ioctl_id:08x} is claimed by OBJECT_CONTROLS — see above",
    );

    let seen = ledger_ids();
    for cmd in [set_control, disable] {
        assert!(
            is_served(cmd),
            "★★★ 0x{cmd:08x} is unserved — w294 claimed it, and its 0x56 is what the guest \
             reports back on 0x{ioctl_id:08x}",
        );
        // ⊘ The evidence half: a served id whose arrival nobody measured is a guess.
        let boots = seen.get(&cmd).unwrap_or_else(|| {
            panic!(
                "0x{cmd:08x} is served but NO committed boot log records it reaching the \
                 unserviced ledger — either the evidence left the tree or this id never \
                 arrived. ⚠ Check `every_committed_boot_tag_has_its_qemu_log` first: a \
                 missing `_qemu.log` produces exactly this symptom."
            )
        });
        assert!(
            !boots.is_empty(),
            "0x{cmd:08x} has an empty boot set — an entry that records nothing",
        );
        assert!(
            GRADUATED.contains(&cmd) && !LEDGER.contains(&cmd),
            "0x{cmd:08x} is served: it belongs in GRADUATED and not in LEDGER",
        );
    }
}

/// ⊘ **The two ids w292 deliberately did NOT serve, asserted so a later lane cannot add
/// them without meeting the reason.**
///
/// - `0x2080200a` `PERF_BOOST` — `[measured]` **zero** occurrences in our QEMU log. Its
///   `0x56` is produced inside the guest's own `nvidia.ko`; it never reaches us, so
///   "serving" it would be building an answer for traffic that does not exist.
/// - `0x2080012f` `GPU_QUERY_ECC_STATUS` — a **real GA106 also refuses it** `0x56`
///   (`traces/host_reference_ga106`, the ONE non-OK in 608 records). Our refusal AGREES
///   with hardware; changing it would be the divergence.
#[test]
fn the_two_ids_w292_left_alone_are_not_in_the_input_only_table() {
    for (cmd, why) in [
        (0x2080_200au32, "PERF_BOOST never reaches us — the guest's own nvidia.ko refuses it"),
        (0x2080_012fu32, "a real GA106 also refuses it; our refusal AGREES with hardware"),
    ] {
        assert!(
            kayfabe_abi::submit::input_only_control(cmd).is_none(),
            "0x{cmd:08x} must NOT be served: {why}"
        );
    }
}
