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
                    Box::new(kayfabe_isolate::StillbornIsolates::new("admitted_is_served")),
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
const PROBE_SIZES: &[usize] = &[0, 3, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 128, 256, 560, 1024];

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

/// The committed boot logs this gate reads. ⊘ **The whole directory, every suffix** —
/// enumerating one file is how `s43`'s alloc failures were missed (they were in the dmesg
/// log while the probe log, a `dmesg | tail -40`, had scrolled past them:
/// `execution_plane_increments.md` §16.55.4).
const BOOT_LOGS: &str = "traces/guest_boots";

/// The newest boot this file is calibrated against. Named, not inferred: a gate whose
/// universe depends on lexical filename order changes meaning when a tag is added.
const NEWEST_BOOT: &str = "s44_b17381c_rmtrace";

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
    0x2081_0108,
    0x2081_0110,
    0x208f_1105,
    0x402c_0101,
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
    assert!(
        boots.iter().any(|t| t == NEWEST_BOOT),
        "{NEWEST_BOOT} does not record 0xa06c0101 — this file is calibrated against the \
         wrong boot",
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
        "0x20800102", // InitTablePolicy
        "0x2080012b", // NV2080_CTRL_CMD_GPU_PROMOTE_CTX — ObjectPolicy, §14.25
        "0x20800301", // NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION — InitTablePolicy
        "0x20800a9f", // a publication control — InitTablePolicy
        "0x20801303", // InitTablePolicy
        "0x20801803", // InitTablePolicy
        "0x20801823", // InitTablePolicy
        "0x2080182a", // InitTablePolicy
        "0x2080182b", // InitTablePolicy
        "0x20802a02", // InitTablePolicy
        "0x20803601", // InitTablePolicy
        "0x20803801", // InitTablePolicy
        "0x90f10106", // the gvaspace PDE publication — InitTablePolicy
        "0xa06c0101", // ★★★★ NVA06C_CTRL_CMD_GPFIFO_SCHEDULE — ObjectPolicy, §16.56
        "0xa06f0103", // NVA06F_CTRL_CMD_GPFIFO_SCHEDULE — ObjectPolicy, #177
        "0xa06f0104", // NVA06F_CTRL_CMD_BIND — ObjectPolicy, E9/§13.6
    ];
    assert_eq!(
        served,
        expected,
        "the set of ADMITTED controls the chain answers changed. Adding one is progress and \
         belongs in this list; LOSING one is a control this port stopped deciding",
    );
}

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
