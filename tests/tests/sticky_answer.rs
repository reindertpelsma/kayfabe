//! ★★★ **Which of our accepted answers the guest can keep FOREVER — over the DERIVED
//! universe of everything that can produce one.**
//!
//! The subject is `kayfabe_device::sticky`, and the property is not *"the guard works"* but
//! *"there is nothing the guard does not cover"*.
//!
//! ## Why this file lives in the conformance suite and not beside the guard
//!
//! The universe is *every* `kayfabe_gsp::CommandPolicy` implementation in the workspace,
//! and they are spread over three crates: `kayfabe-device` (six), `kayfabe-gsp` (two) and
//! `kayfabe-rmrpc` (one). No single library crate can construct all nine, so a test that
//! lived in one of them could only ever quantify over its own — a smaller universe is a
//! smaller true statement (memory: `gates_quantified_over_a_list`). This crate depends on
//! all three.
//!
//! ## ★★★ The universe is DERIVED, in two independent ways
//!
//! 1. **From the source text.** [`impls_from_source`] runs `git ls-files`, keeps
//!    `crates/*/src/**.rs`, and greps every `impl … CommandPolicy for <T>` out of it. A
//!    policy added tomorrow is in scope tomorrow with no edit here. ⊘ Not a `find`: a file
//!    that is not tracked is not built by CI either, and `git ls-files` is the same
//!    universe `scripts/claim_ledger.py` and the ratchets use.
//! 2. **From the type system.** Every row of `POLICY_DISPOSITIONS` whose disposition is
//!    executable below is executed against the *real* type, through `dyn CommandPolicy` —
//!    so a row that names a type which no longer behaves as claimed is red, not stale.
//!
//! The two are then required to agree, in both directions. That is the only shape that
//! survives this repository's most-repeated defect: the list that quietly shrinks.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_device::inittables::WantedTable;
use kayfabe_device::sticky::{
    self, BRANCH_A_CACHEABLE, CONTROL_RMCTRL_ACCESS_RIGHT_OFF, CONTROL_RMCTRL_FLAGS_OFF,
    POLICY_DISPOSITIONS, RMCTRL_FLAGS_CACHEABLE, RMCTRL_FLAGS_CACHEABLE_BY_INPUT,
    RMCTRL_FLAGS_INTERNAL, StickyAnswerGuard, StickyDisposition,
};
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

// =====================================================================================
// Harness
// =====================================================================================

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the workspace root resolves")
}

fn abi() -> DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// A `GSP_RM_CONTROL` request: the 40-byte `rpc_gsp_rm_control_v03_00` header then params.
///
/// `flags`/`access_right` are written where a **stock** sender writes zero
/// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:10994-10995`), which is exactly the lever a
/// hostile guest has and the one this file exercises.
fn control(cmd: u32, params: usize, flags: u32, access_right: u32) -> RpcCommand {
    let mut payload = vec![0u8; 40 + params];
    payload[0..4].copy_from_slice(&0x0000_c1d0u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x0000_0b1eu32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params as u32).to_le_bytes());
    payload[CONTROL_RMCTRL_FLAGS_OFF..CONTROL_RMCTRL_FLAGS_OFF + 4]
        .copy_from_slice(&flags.to_le_bytes());
    payload[CONTROL_RMCTRL_ACCESS_RIGHT_OFF..CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 4]
        .copy_from_slice(&access_right.to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 11,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn reply_flags(r: &Reply) -> (u32, u32) {
    let f = |at: usize| {
        u32::from_le_bytes([r.body[at], r.body[at + 1], r.body[at + 2], r.body[at + 3]])
    };
    (
        f(CONTROL_RMCTRL_FLAGS_OFF),
        f(CONTROL_RMCTRL_ACCESS_RIGHT_OFF),
    )
}

/// The port's chain, and the same chain **without** the guard — so the difference the guard
/// makes is read off two runs rather than asserted about itself.
fn guarded() -> Box<dyn CommandPolicy> {
    kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        abi(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_device::census::ControlCensusLog::new(),
        // ⊘ Neither object-model seat. This file's subject is `GSP_RM_CONTROL` replies and
        // the two fields the guard zeroes; `kayfabe_rmrpc::ObjectPolicy` claims no control
        // at all and the publication seat cannot answer one, so including either would add
        // a link that cannot change a single assertion here — and would quietly make these
        // tests depend on the object model realizing.
        kayfabe_device::ObjectLinks::default(),
        // ⊘ No model name: this file's subject is not the static-info body, and declaring
        // one would make it depend on a value no assertion here reads.
        kayfabe_device::staticinfo::GpuNames::default(),
    )
}

fn unguarded() -> Box<dyn CommandPolicy> {
    kayfabe_device::served_chain(
        kayfabe_device::default_chip(),
        abi(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_abi::eventnotify::ProbeArmSet::default(),
        // ⊘ Neither object-model seat. This file's subject is `GSP_RM_CONTROL` replies and
        // the two fields the guard zeroes; `kayfabe_rmrpc::ObjectPolicy` claims no control
        // at all and the publication seat cannot answer one, so including either would add
        // a link that cannot change a single assertion here — and would quietly make these
        // tests depend on the object model realizing.
        kayfabe_device::ObjectLinks::default(),
        // ⊘ No model name: this file's subject is not the static-info body, and declaring
        // one would make it depend on a value no assertion here reads.
        kayfabe_device::staticinfo::GpuNames::default(),
    )
}

/// A control this port really serves, so the reply under test is an `NV_OK` one with a body.
fn a_served_control() -> u32 {
    WantedTable::FifoNumChannels.cmd_id()
}

// =====================================================================================
// 1. ★★★ The universe, derived from `git ls-files`
// =====================================================================================

/// Every `impl … CommandPolicy for <T>` in a tracked library source, as the source spells
/// the type.
fn impls_from_source() -> BTreeSet<String> {
    let root = repo_root();
    let out = Command::new("git")
        .arg("ls-files")
        .arg("--")
        .arg("crates")
        .current_dir(&root)
        .output()
        .expect("git ls-files runs in the workspace");
    assert!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let files: Vec<PathBuf> = String::from_utf8(out.stdout)
        .expect("git ls-files is utf-8")
        .lines()
        .filter(|p| p.ends_with(".rs"))
        // ⊘ LIBRARY sources only. A `CommandPolicy` written inside a `tests/` target is a
        // fixture for one file and cannot be installed in the port; requiring a ledger row
        // for it would make the ledger grow with every test that needs a stub.
        .filter(|p| p.contains("/src/"))
        .map(|p| root.join(p))
        .collect();
    assert!(
        files.len() > 100,
        "only {} tracked library sources — the sweep is not reaching the tree \
         (a `--exclude=.git` rsync fakes exactly this)",
        files.len(),
    );

    let mut found = BTreeSet::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for line in text.lines() {
            let line = line.trim();
            // `impl CommandPolicy for X {`, `impl kayfabe_gsp::CommandPolicy for X {`,
            // `impl<'a> CommandPolicy for X<'a> {` — the discriminator is the ` for ` that
            // follows the trait name, so a `use` line or a prose mention cannot match.
            if !line.starts_with("impl") {
                continue;
            }
            let Some(at) = line.find("CommandPolicy for ") else {
                continue;
            };
            let tail = &line[at + "CommandPolicy for ".len()..];
            let name = tail.trim_end_matches('{').trim();
            found.insert(name.to_string());
        }
    }
    found
}

/// ★★★ **The ledger and the source state the same set.**
///
/// Both directions, and each with its own message, because they are different mistakes: an
/// implementation with no row is an *unexamined* answering site, and a row with no
/// implementation is a ledger that has stopped describing the code.
#[test]
fn the_universe_of_answering_policies_is_derived_from_the_source() {
    let from_source = impls_from_source();
    let from_ledger: BTreeSet<String> = POLICY_DISPOSITIONS
        .iter()
        .map(|d| d.name.to_string())
        .collect();

    assert!(
        !from_source.is_empty(),
        "no CommandPolicy implementation found at all — the grep is broken, not the code",
    );
    let missing: Vec<&String> = from_source.difference(&from_ledger).collect();
    assert!(
        missing.is_empty(),
        "{missing:?} implement CommandPolicy and have no row in \
         kayfabe_device::sticky::POLICY_DISPOSITIONS. Each of these can hand the guest an \
         answer; say why it cannot hand out a STICKY one (see that module's §1).",
    );
    let phantom: Vec<&String> = from_ledger.difference(&from_source).collect();
    assert!(
        phantom.is_empty(),
        "{phantom:?} have a disposition row but no `impl CommandPolicy for` in any tracked \
         library source — the ledger has stopped describing the code",
    );
}

/// The rows also name a file that exists and really contains the implementation.
///
/// ★ Without this the `path` field is decoration, and a reader sent to the wrong file is
/// worse off than one sent nowhere.
#[test]
fn every_disposition_row_points_at_the_file_that_implements_it() {
    let root = repo_root();
    for d in POLICY_DISPOSITIONS {
        let p = root.join(d.path);
        let text =
            std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} ({}): {e}", d.name, d.path));
        assert!(
            text.contains(&format!("CommandPolicy for {}", d.name)),
            "{} does not implement CommandPolicy in {}",
            d.name,
            d.path,
        );
    }
}

// =====================================================================================
// 2. The dispositions, EXECUTED — a row is a claim, not a label
// =====================================================================================

/// `NeverAnswers` means exactly that, against the sharpest input available: a **bit-15
/// fn-76 control**, which is the only shape either cache branch can act on.
///
/// ★ `GvasPubRecorder` is given the control it actually decodes, with the GSS-legacy bit
/// set — so "it declined" is not "it did not recognise the command". That is load-bearing
/// beyond the cache question: it is seated FIRST in `served_chain`, ahead of every answering
/// link, so a `Some` of its own would short-circuit `find_map` and REPLACE
/// `InitTablePolicy`'s reply.
///
/// ⊘ **`FaultBufferRecorder` left this list at §14.41 and its departure is the point.** It
/// became a `CommandObserver`, so it has no `respond` to assert about — the property this
/// test used to *check* is now one the type cannot violate. `0x20800a9b` is answered by
/// `InitTablePolicy` (`Guarded`), and the recorder's own non-vacuity moved to
/// `crates/kayfabe-device/tests/fault_buffer_recorder.rs`.
#[test]
fn the_never_answers_rows_answer_nothing_even_for_a_gss_legacy_control() {
    let rows: Vec<&str> = POLICY_DISPOSITIONS
        .iter()
        .filter(|d| d.disposition == StickyDisposition::NeverAnswers)
        .map(|d| d.name)
        .collect();
    assert_eq!(
        rows,
        vec!["GvasPubRecorder", "UnservicedLedger", "Observing",],
        "a NeverAnswers row was added or removed without extending this test",
    );

    let unserviced_log = kayfabe_device::unserviced::UnservicedLog::new();
    let mut ledger =
        kayfabe_device::unserviced::UnservicedLedger::new(abi(), unserviced_log.clone());
    let gvas_log = kayfabe_device::gvaspub::GvasPubLog::new();
    let mut gvas = kayfabe_device::gvaspub::GvasPubRecorder::new(abi(), gvas_log.clone());
    // ★★★ §14.23 — the observer adapter, over an observer that COUNTS. If `Observing`
    // could answer, this is the seat where the answer would replace `InitTablePolicy`'s;
    // the counter below is what proves the declines are decisions and not ignorance.
    let counted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl kayfabe_gsp::CommandObserver for Counting {
        fn observe(&mut self, _cmd: &RpcCommand) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let mut observing = kayfabe_gsp::Observing(Box::new(Counting(counted.clone())));

    let gss = a_served_control() | 0x0000_8000;
    let its_own = kayfabe_abi::faultbuffer::NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER;
    // ⊘ `control()` zero-fills the params, and 184 zeros are NOT a legal publication
    // (`pageSize = 0` is refused by name). That is fine and is asserted as what it is
    // below: the recorder recognised the id, tried to decode, and counted an UNDECODABLE
    // — which is the non-vacuity this test needs (it saw the traffic) without pretending
    // a zeroed body is a driver's.
    let pub_cmd = kayfabe_abi::gvaspacepdes::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES;
    let pub_size = kayfabe_abi::gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE;
    for cmd in [gss, its_own, its_own | 0x0000_8000] {
        let c = control(cmd, 32, RMCTRL_FLAGS_CACHEABLE, 0);
        assert!(
            ledger.respond(&c).is_none(),
            "UnservicedLedger answered {cmd:#010x}",
        );
        assert!(
            gvas.respond(&c).is_none(),
            "GvasPubRecorder answered {cmd:#010x}",
        );
        assert!(
            observing.respond(&c).is_none(),
            "Observing answered {cmd:#010x}",
        );
    }
    for cmd in [pub_cmd, pub_cmd | 0x0000_8000] {
        let c = control(cmd, pub_size, RMCTRL_FLAGS_CACHEABLE, 0);
        assert!(
            gvas.respond(&c).is_none(),
            "GvasPubRecorder answered its OWN control {cmd:#010x} — seated first in the \
             chain, that reply would replace InitTablePolicy's",
        );
        assert!(
            ledger.respond(&c).is_none(),
            "UnservicedLedger answered {cmd:#010x}",
        );
        assert!(
            observing.respond(&c).is_none(),
            "Observing answered its front-seat control {cmd:#010x} — that reply would \
             replace InitTablePolicy's",
        );
    }
    // ★ Non-vacuity, and it is the whole reason this is not `assert!(true)`: both policies
    // SAW the traffic. A recorder that declined because it never ran would pass above.
    assert_eq!(
        unserviced_log.total(),
        5,
        "the ledger never saw the traffic"
    );
    // ⊘ ONE, for the reason stated below the fault recorder's own count: `pub_cmd | 0x8000`
    // is a DIFFERENT command word and names no publication. A zero here would mean the
    // decline above was ignorance rather than a decision.
    let gvas_snap = gvas_log.snapshot();
    assert_eq!(
        gvas_snap.undecodable, 1,
        "the publication recorder never SAW its own control, so its declines prove nothing"
    );
    // ⊘ ONE, for the reason stated below the fault recorder's own count: `pub_cmd | 0x8000`
    // is a DIFFERENT command word and names no publication.
    assert_eq!(
        gvas_snap.total, 0,
        "a zeroed body is not a legal publication"
    );
    // ⊘ The fault-buffer recorder's own count moved to
    // `crates/kayfabe-device/tests/fault_buffer_recorder.rs` when it became an observer at
    // §14.41. `its_own` is still posted above — it now exercises `InitTablePolicy`'s refusal
    // of a 32-byte body for a control whose params are 2064, which is the shape this file
    // cares about (a refusal is never cached).
    // ⊘ FIVE — every command the sweeps above posted reached the wrapped observer. A zero
    // would mean `Observing` short-circuits, and its `None`s would prove nothing.
    assert_eq!(
        counted.load(std::sync::atomic::Ordering::Relaxed),
        5,
        "Observing did not pass the traffic through to its observer",
    );
}

/// `NotAControl` means the policy returns `None` for **every** fn-76 command, which is what
/// makes both cache branches unreachable for it (`sticky` §1a: both call sites are inside
/// `rpcRmApiControl_GSP`).
///
/// ⊘ Quantified over `WantedTable::ALL` and over the GSS-legacy form of each, rather than
/// over one hand-picked id: these three policies must decline the whole control vocabulary,
/// not one example of it.
#[test]
fn the_not_a_control_rows_decline_every_control_command() {
    let rows: Vec<&str> = POLICY_DISPOSITIONS
        .iter()
        .filter(|d| d.disposition == StickyDisposition::NotAControl)
        .map(|d| d.name)
        .collect();
    assert_eq!(
        rows,
        vec![
            "StaticInfoPolicy",
            "GuestSystemInfoPolicy",
            "InertPolicy",
            "BarPdePolicy",
        ],
        "a NotAControl row was added or removed without extending this test",
    );

    let chip = kayfabe_device::default_chip();
    let mut policies: Vec<(&str, Box<dyn CommandPolicy>)> = vec![
        (
            "StaticInfoPolicy",
            Box::new(kayfabe_device::staticinfo::StaticInfoPolicy::new(
                chip,
                abi(),
            )),
        ),
        (
            "GuestSystemInfoPolicy",
            Box::new(kayfabe_device::guestsysinfo::GuestSystemInfoPolicy::new(
                abi(),
            )),
        ),
        (
            "InertPolicy",
            Box::new(kayfabe_device::inert::InertPolicy::new()),
        ),
        // ⊘⊘ **`ObjectPolicy` LEFT this list on 2026-08-08.** It was here claiming
        // `NotAControl`, and `#177` had already made that false — it answers
        // `kayfabe_rmrpc::OBJECT_CONTROLS` (`0xa06f0103`, `0xa06f0104`) with `NV_OK` and a
        // body. The sweep below could not see it: it quantifies over `WantedTable::ALL`,
        // which is **`InitTablePolicy`'s** id list and contains neither of them, so a false
        // claim sat under an executing test. Its row is `Guarded` now, and the lesson is
        // this file's own: a gate is only as true as the list it quantifies over.
        // ★★ `#149`. It answers exactly `UPDATE_BAR_PDE` (fn 70) and declines every other
        // function, so no reply of its can reach `rpcRmApiControl_GSP`'s two cache-populating
        // call sites however it is composed.
        (
            "BarPdePolicy",
            Box::new(kayfabe_device::bar2::BarPdePolicy::new(
                abi(),
                kayfabe_device::bar2::BarPdeLog::new(),
            )),
        ),
    ];
    assert_eq!(
        rows.len(),
        policies.len(),
        "the ledger names {} NotAControl rows and this test builds {} of them",
        rows.len(),
        policies.len(),
    );
    let mut asked = 0usize;
    for (name, p) in &mut policies {
        for w in WantedTable::ALL {
            for cmd in [w.cmd_id(), w.cmd_id() | 0x0000_8000] {
                asked += 1;
                assert!(
                    p.respond(&control(cmd, w.params_size(), RMCTRL_FLAGS_CACHEABLE, 0))
                        .is_none(),
                    "{name} answered control {cmd:#010x}",
                );
            }
        }
    }
    assert_eq!(
        asked,
        policies.len() * 2 * WantedTable::ALL.len(),
        "sweep arithmetic"
    );

    // ★ Non-vacuity: each of these DOES answer something, so "returns None" above is a
    // statement about fn 76 and not about a policy that answers nothing at all.
    let mut inert = kayfabe_device::inert::InertPolicy::new();
    assert!(
        inert
            .respond(&RpcCommand {
                function: RpcFunction::UnloadingGuestDriver,
                code: 47,
                sequence: 1,
                payload: vec![0u8; 12],
                elements: 1,
                delivered: Vec::new(),
            })
            .is_some(),
        "InertPolicy answers nothing at all — the sweep above is vacuous",
    );
    // ★ …and the same for the row `#149` added, because a policy that answered NOTHING
    // would satisfy the whole sweep above by accident.
    let mut bar_pde =
        kayfabe_device::bar2::BarPdePolicy::new(abi(), kayfabe_device::bar2::BarPdeLog::new());
    assert!(
        bar_pde
            .respond(&RpcCommand {
                function: RpcFunction::UpdateBarPde,
                code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
                sequence: 1,
                payload: vec![0u8; kayfabe_device::bar2::UPDATE_BAR_PDE_BODY_SIZE],
                elements: 1,
                delivered: Vec::new(),
            })
            .is_some(),
        "BarPdePolicy answers nothing at all — the sweep above is vacuous",
    );
}

// =====================================================================================
// 3. ★★★ The guard, and what it changes
// =====================================================================================

/// **The bite.** A guest that asks for its own answer to be cached gets the flags reflected
/// by the unguarded chain, and zeroed by the port's.
///
/// This is the whole mechanism in one assertion pair, and it is stated as a *difference*
/// between two chains rather than as a property of one — so a guard that stopped being
/// installed turns the second half red while the first half proves the traffic was real.
#[test]
fn the_port_never_lets_the_guest_mark_our_answer_cacheable() {
    let cmd = a_served_control();
    let req = control(
        cmd,
        WantedTable::FifoNumChannels.params_size(),
        RMCTRL_FLAGS_CACHEABLE,
        0,
    );

    let bare = unguarded().respond(&req).expect("the chain serves it");
    assert_eq!(bare.rpc_result, 0, "the premise: this control is ANSWERED");
    assert_eq!(
        reply_flags(&bare),
        (RMCTRL_FLAGS_CACHEABLE, 0),
        "the unguarded chain reflects the guest's own rmctrlFlags — the exposure",
    );

    let guard = guarded().respond(&req).expect("the port serves it");
    assert_eq!(
        guard.rpc_result, 0,
        "the guard must not turn it into a refusal"
    );
    assert_eq!(
        reply_flags(&guard),
        (0, 0),
        "the port let the guest keep our answer forever",
    );
    // Everything else about the reply is untouched: the guard writes eight bytes.
    assert_eq!(bare.body.len(), guard.body.len());
    let mut expect = bare.body.clone();
    expect[CONTROL_RMCTRL_FLAGS_OFF..CONTROL_RMCTRL_FLAGS_OFF + 8].fill(0);
    assert_eq!(
        guard.body, expect,
        "the guard changed more than the two fields"
    );
}

/// The guard is **total over the served universe**, not over one example — and it counts.
#[test]
fn every_served_control_leaves_the_port_non_cacheable() {
    let mut port = StickyAnswerGuard::new(abi(), unguarded());
    let mut answered = 0usize;
    for w in WantedTable::ALL {
        let req = control(w.cmd_id(), w.params_size(), RMCTRL_FLAGS_CACHEABLE, 0);
        let Some(r) = port.respond(&req) else {
            continue;
        };
        if r.rpc_result != 0 || r.body.is_empty() {
            continue;
        }
        answered += 1;
        assert_eq!(reply_flags(&r), (0, 0), "{w:?} left the port cacheable");
    }
    assert!(
        answered >= 6,
        "only {answered} of {} served controls produced an NV_OK body — the sweep is not \
         exercising the guard",
        WantedTable::ALL.len(),
    );
    assert_eq!(port.inspected() as usize, answered);
    assert_eq!(
        port.rewritten() as usize,
        answered,
        "the counter did not see the flags it rewrote",
    );
    // ★★★ **§14.36 turned this from 0 into 1, and that is the guard becoming load-bearing
    // on the REAL served set rather than only on a crafted fixture.**
    //
    // ⊘ It used to read *"`neutralised` is ZERO here, which is not a weaker statement — it is
    // what the run says. No control this port serves has bit 15 set, so branch (b) was never
    // the branch these would have taken"*. That sentence was true and is now false:
    // `0x20808159` is served, it **is** bit 15, and this sweep hands it
    // `RMCTRL_FLAGS_CACHEABLE` exactly as a hostile guest would. The count says branch (b)
    // was genuinely live for it and was genuinely closed.
    //
    // ⚠ Pinned as a SET, not as a count, for `gates_quantified_over_a_list`'s reason: a
    // second served GSS-legacy id must show up as a test to edit, with its argument, rather
    // than as a number that quietly went to 2.
    let neutralisable: BTreeSet<u32> = WantedTable::ALL
        .iter()
        .map(|w| w.cmd_id())
        .filter(|id| id & kayfabe_abi::capability::RM_GSS_LEGACY_MASK != 0)
        .collect();
    assert_eq!(
        neutralisable,
        kayfabe_abi::gsslegacy::SERVED
            .iter()
            .map(|(c, _)| *c)
            .collect::<BTreeSet<u32>>(),
        "the served controls that can reach branch (b) are exactly the GSS-legacy module's"
    );
    // ⚠⚠ §14.37 took this from one to TWO, and the second is the one that makes the guard
    // indispensable rather than merely correct. `0x20808159`'s reply is the identity on the
    // guest's own buffer, so a cache that kept it would replay what the guest sent;
    // `0x20808162` writes a byte the guest did not send, so a kept entry would be a real
    // answer persisting. ⊘ The guard is what closes both, and only the second one would
    // have been WRONG without it.
    assert_eq!(neutralisable.len(), 2);
    assert_eq!(
        port.neutralised() as usize,
        neutralisable.len(),
        "the guard must have closed branch (b) for every served control that could take it"
    );
}

/// ★★★ **Branch (b), end to end** — the one assertion in this file where
/// `rmapiControlCacheSetUnchecked` would really have been reached.
///
/// The inner policy is `EchoOk`, the C baseline: it reflects the request under `NV_OK`,
/// which is exactly how a crafted `rmctrlFlags` gets back to the guest. Behind the guard the
/// same request comes back with both fields zero and the event is **counted**, so "branch
/// (b) was live and was closed" is a number rather than an argument.
#[test]
fn a_gss_legacy_control_answered_ok_is_counted_and_neutralised() {
    let gss = 0x2080_8513u32; // NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL
    assert_ne!(gss & 0x0000_8000, 0, "the fixture must be GSS-legacy");
    let req = control(gss, 16, RMCTRL_FLAGS_CACHEABLE, 0);

    // Unguarded: the C baseline hands the bits straight back — the exposure, visible.
    let bare = kayfabe_gsp::EchoOk
        .respond(&req)
        .expect("EchoOk answers everything");
    assert_eq!(bare.rpc_result, 0);
    assert_eq!(reply_flags(&bare), (RMCTRL_FLAGS_CACHEABLE, 0));
    assert!(sticky::reply_would_be_cached(
        gss,
        RMCTRL_FLAGS_CACHEABLE,
        0
    ));

    let mut port = StickyAnswerGuard::new(abi(), Box::new(kayfabe_gsp::EchoOk));
    let r = port.respond(&req).expect("answered");
    assert_eq!(r.rpc_result, 0, "the guard must not turn it into a refusal");
    assert_eq!(reply_flags(&r), (0, 0));
    assert_eq!(port.inspected(), 1);
    assert_eq!(port.rewritten(), 1);
    assert_eq!(
        port.neutralised(),
        1,
        "branch (b) fired and was not counted"
    );

    // The access-right lever too: `rmapiControlIsCacheable` bails on a non-zero
    // `accessRight`, so this one is rewritten but NOT counted as a branch-(b) save.
    let mut port = StickyAnswerGuard::new(abi(), Box::new(kayfabe_gsp::EchoOk));
    let r = port
        .respond(&control(gss, 16, RMCTRL_FLAGS_CACHEABLE, 7))
        .expect("answered");
    assert_eq!(reply_flags(&r), (0, 0));
    assert_eq!(port.rewritten(), 1);
    assert_eq!(port.neutralised(), 0);
}

/// A **stock** guest changes nothing, and that is the compatibility claim.
///
/// ★ Stated as `neutralised() == 0` **with** `inspected() > 0`: the pair is the difference
/// between "the guard had nothing to do" and "the guard was not on the path".
#[test]
fn a_stock_guest_sees_byte_identical_replies() {
    let mut bare = unguarded();
    let mut port = StickyAnswerGuard::new(abi(), unguarded());
    let mut seen = 0usize;
    for w in WantedTable::ALL {
        let req = control(w.cmd_id(), w.params_size(), 0, 0);
        let a = bare.respond(&req);
        let b = port.respond(&req);
        assert_eq!(a, b, "{w:?}: the guard perturbed a stock reply");
        if a.is_some_and(|r| r.rpc_result == 0 && !r.body.is_empty()) {
            seen += 1;
        }
    }
    assert!(seen > 0, "no accepted reply was compared");
    assert_eq!(port.inspected() as usize, seen);
    assert_eq!(port.rewritten(), 0, "a stock request needed rewriting");
    assert_eq!(port.neutralised(), 0);
}

/// A refusal is passed through untouched, because a refusal cannot be cached at all
/// (`ogkm-580: rpc.c:1994` short-circuits ahead of the whole post-RPC block).
///
/// ⊘ And it must NOT be counted: `inspected()` is the non-vacuity instrument for the
/// accepted path, so folding refusals into it would make every "the guard was on the path"
/// assertion in this file pass for the wrong reason.
#[test]
fn a_refusal_crosses_the_guard_unchanged_and_uncounted() {
    struct Refuses;
    impl CommandPolicy for Refuses {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            Some(Reply {
                rpc_result: kayfabe_abi::NV_ERR_NOT_SUPPORTED,
                body: cmd.payload.clone(),
            })
        }
    }
    let mut port = StickyAnswerGuard::new(abi(), Box::new(Refuses));
    let req = control(a_served_control() | 0x8000, 16, RMCTRL_FLAGS_CACHEABLE, 0);
    let r = port.respond(&req).expect("a refusal is never a drop");
    assert_eq!(r.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert_eq!(reply_flags(&r), (RMCTRL_FLAGS_CACHEABLE, 0));
    assert_eq!(port.inspected(), 0);
    assert_eq!(port.rewritten(), 0);
    assert_eq!(port.neutralised(), 0);
}

/// An accepted control reply too short to hold the header the guest reads out of it is
/// **refused**, not passed.
#[test]
fn an_accepted_control_reply_that_cannot_hold_the_header_is_refused() {
    struct Short;
    impl CommandPolicy for Short {
        fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
            Some(Reply {
                rpc_result: 0,
                body: vec![0xabu8; 12],
            })
        }
    }
    let mut port = StickyAnswerGuard::new(abi(), Box::new(Short));
    let r = port
        .respond(&control(a_served_control(), 16, 0, 0))
        .expect("answered");
    assert_eq!(r.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(r.body.is_empty());
    assert_eq!(port.malformed(), 1);

    // An EMPTY body is not malformed — `RpcCommand::reply` zero-fills it, which is already
    // the value the guard would write.
    struct Empty;
    impl CommandPolicy for Empty {
        fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
            Some(Reply {
                rpc_result: 0,
                body: Vec::new(),
            })
        }
    }
    let mut port = StickyAnswerGuard::new(abi(), Box::new(Empty));
    let r = port
        .respond(&control(a_served_control(), 16, 0, 0))
        .expect("answered");
    assert_eq!(r.rpc_result, 0);
    assert_eq!(port.malformed(), 0);
    assert_eq!(port.inspected(), 1);
}

// =====================================================================================
// 4. The guest's own predicate, transcribed — the arithmetic the guard rests on
// =====================================================================================

/// `rmapiControlIsCacheable` and the branch-(b) conjunction, case by case
/// (`ogkm-580: rmapi_cache.c:152-172`, `rpc.c:11098-11103`).
///
/// ★ Every arm of the C function has a case here, including the two that make the guard's
/// *"zero is the honest value"* argument true: `flags == 0` fails the first test, and an
/// `accessRight != 0` fails the second even with the cacheable bit set.
#[test]
fn the_cacheability_predicate_matches_the_guests_own() {
    // `!(flags & CACHEABLE_ANY)` — the arm that makes a zeroed header safe.
    assert!(!sticky::rmapi_control_is_cacheable(0, 0, true));
    assert!(!sticky::rmapi_control_is_cacheable(
        // every bit EXCEPT the two that make up `RMCTRL_FLAGS_CACHEABLE_ANY`, spelled as a
        // literal so this case stays independent of the constant it is checking.
        !0x0002_0400u32,
        0,
        true
    ));
    // The plain cacheable arm.
    assert!(sticky::rmapi_control_is_cacheable(
        RMCTRL_FLAGS_CACHEABLE,
        0,
        true
    ));
    // `accessRight != 0` -> never.
    assert!(!sticky::rmapi_control_is_cacheable(
        RMCTRL_FLAGS_CACHEABLE,
        1,
        true
    ));
    // INTERNAL follows `bAllowInternal`, and the RPC path passes NV_TRUE.
    assert!(sticky::rmapi_control_is_cacheable(
        RMCTRL_FLAGS_CACHEABLE | RMCTRL_FLAGS_INTERNAL,
        0,
        true
    ));
    assert!(!sticky::rmapi_control_is_cacheable(
        RMCTRL_FLAGS_CACHEABLE | RMCTRL_FLAGS_INTERNAL,
        0,
        false
    ));

    let gss = 0x2080_8513u32; // NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2_PHYSICAL
    let plain = 0x2080_0a61u32;
    assert!(sticky::reply_would_be_cached(
        gss,
        RMCTRL_FLAGS_CACHEABLE,
        0
    ));
    // Bit 15 clear -> branch (b) is not even entered, whatever the flags say.
    assert!(!sticky::reply_would_be_cached(
        plain,
        RMCTRL_FLAGS_CACHEABLE,
        0
    ));
    // `!(rmctrlFlags & RMCTRL_FLAGS_CACHEABLE_BY_INPUT)` — the last conjunct, which is a
    // NEGATIVE: a BY_INPUT reply takes the other setter and is excluded here.
    assert!(!sticky::reply_would_be_cached(
        gss,
        RMCTRL_FLAGS_CACHEABLE_BY_INPUT,
        0
    ));
    assert!(!sticky::reply_would_be_cached(gss, 0, 0));
}

// =====================================================================================
// 5. ★★★ Branch (a) — the exposure that is NOT ours, pinned so it cannot be forgotten
// =====================================================================================

/// The four controls the **guest's own** export table marks cacheable are controls this
/// port really serves, and their answers really are build-time constants.
///
/// ⚠ This test cannot check the flag words — they live in the guest's generated source, not
/// here (`sticky` §3 records them with their `g_subdevice_nvoc.c` lines, read at
/// `ogkm-580` on 2026-08-01). What it *can* check, and does, is the relationship that makes
/// the exposure tolerable: each is served, so each is answered from a `ChipProfile` row.
/// The day one of them stops being served, or a served control's answer stops being a
/// constant, this test is where the argument has to be redone.
#[test]
fn branch_a_is_a_subset_of_what_this_port_serves_from_a_constant_row() {
    let served: BTreeSet<u32> = WantedTable::ALL.iter().map(|w| w.cmd_id()).collect();
    for cmd in BRANCH_A_CACHEABLE {
        assert!(
            served.contains(&cmd),
            "{cmd:#010x} is cacheable in the guest's table but this port does not serve it \
             — the row in `sticky` §3 is describing a control that is no longer ours",
        );
        // ⊘ And branch (a) has nothing to do with bit 15. Stated here because the whole
        // task that produced this file started from the belief that it did.
        assert_eq!(
            cmd & 0x0000_8000,
            0,
            "{cmd:#010x}: branch (a) does not go through the GSS-legacy mask",
        );
    }
    // ★ 4 -> 5 at §14.28: `0x20800102` GPU_GET_INFO_V2, the first row here that reaches
    // branch (a) through `CACHEABLE_BY_INPUT` rather than the blanket `CACHEABLE` bit — and
    // the first whose reply is not a constant. ⊘ The argument still holds and is checked one
    // level up: the reply is a pure function of the very params the guest keys its cache on.
    //
    // ★★★ 5 -> 6 at §14.35: `0x20803601` GSP_GET_FEATURES, `flags = 0x40549`, plain
    // `CACHEABLE`. ⊘⊘ **This is the first row for which this test's own title is FALSE**, and
    // that is worth more than the row: its `firmwareVersion` is not served from a
    // `ChipProfile` row at all, it is latched from the guest's own `SET_GUEST_SYSTEM_INFO`
    // (`kayfabe_device::inittables::InitTablePolicy::guest_firmware`). So the docstring's
    // *"each is served, so each is answered from a `ChipProfile` row"* no longer follows,
    // and the argument has to be redone here exactly as the docstring says it must be.
    //
    // ★ Redone, and it survives — on a LIFETIME argument rather than a constancy one. The
    // guest populates its control cache from a reply; we populate the latch from fn 1; the
    // guest sends fn 1 once per driver load, during the version handshake, and cannot issue
    // any control before it (a guest whose fn 1 fails never finishes `RmInitAdapter` —
    // `kayfabe_device::guestsysinfo`, run `t127a`). So within one driver load the latch is
    // written before the first cacheable answer and never again, which is exactly the
    // property caching needs. A driver reload rebuilds both sides together.
    //
    // ⚠ What would break it, stated so the next reader does not have to re-derive it: any
    // future served control in this list whose answer depends on state the guest can change
    // *after* it has issued that control once. That one is unserveable-as-answered and the
    // choice becomes refuse-or-be-wrong-forever.
    assert_eq!(BRANCH_A_CACHEABLE.len(), 6);
}

/// **Neither guard can help branch (a), and this states the only two levers there are.**
///
/// The guard rewrites the reply header; branch (a) never reads it. So a served control in
/// `BRANCH_A_CACHEABLE` is cached by the guest whatever we do — unless we refuse. This test
/// is the executable form of that sentence.
#[test]
fn the_guard_does_not_and_cannot_stop_branch_a() {
    let mut port = StickyAnswerGuard::new(abi(), unguarded());
    for cmd in BRANCH_A_CACHEABLE {
        let w = WantedTable::from_cmd(cmd).expect("served");
        let Some(r) = port.respond(&control(cmd, w.params_size(), 0, 0)) else {
            continue;
        };
        if r.rpc_result != 0 {
            continue;
        }
        // The reply is ACCEPTED. `bCacheable` was decided before it was sent, from the
        // guest's table, so this is the guest caching our answer — and the only fact we
        // control is that the answer came from a constant row.
        assert_eq!(reply_flags(&r), (0, 0));
        assert!(
            !sticky::reply_would_be_cached(cmd, 0, 0),
            "{cmd:#010x} would ALSO take branch (b) — two branches, one control",
        );
    }
}

// =====================================================================================
// 6. What the capture says: bit 15 is never asked in the cold-boot prefix
// =====================================================================================

/// ★★ **`cap1b` contains no GSS-legacy control at all**, so no trace this repository holds
/// can bite branch (b) — which is a fact about the evidence, not about the risk.
///
/// Derived from the capture rather than from the report's printed lines: the replay's own
/// decoded command list is walked and every fn-76 control word is tested. If a future
/// capture *does* carry one, this test goes red and the guard stops being prospective.
#[test]
fn no_gss_legacy_control_appears_in_the_cold_boot_capture() {
    let Ok(Ok(trace)) = kayfabe_crec::load_cap1b() else {
        // The capture is committed; an unreadable one is a tree problem, not a skip.
        panic!("cap1b did not load — traces/cap1b_coldboot_hermetic_d6.rec");
    };
    let report = kayfabe_crec::Replay::new(&trace, kayfabe_crec::bench_abi())
        .with_policy(kayfabe_crec::served_policy)
        .run(kayfabe_crec::Fill::Reconstructed);
    let driver = abi();
    let mut controls = 0usize;
    let mut gss = Vec::new();
    for (_txn, c) in &report.commands {
        if c.function != RpcFunction::RmControl {
            continue;
        }
        let Ok(req) = driver.decode_rpc_control(&c.payload) else {
            continue;
        };
        controls += 1;
        if req.cmd & 0x0000_8000 != 0 {
            gss.push(req.cmd);
        }
    }
    assert!(
        controls >= 30,
        "only {controls} controls decoded from cap1b — the replay is not reaching them",
    );
    assert!(
        gss.is_empty(),
        "cap1b carries GSS-legacy controls {gss:#010x?} — branch (b) is now EXERCISED by a \
         capture and the guard's reach must be measured against it rather than argued",
    );
}
