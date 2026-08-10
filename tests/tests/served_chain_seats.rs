//! ★★★ **The two seats a composition root's object model takes in the served chain, and
//! what each is allowed to change** — `kayfabe_device::ObjectLinks`.
//!
//! # ⊘⊘ Why this file exists: the obligation it discharges was checked by a FILE THAT DID
//! NOT EXIST
//!
//! `kayfabe_device::served_chain` has carried this sentence since the object seat was
//! created:
//!
//! > ⚠ And the link must not claim more than it serves. … the obligation is stated here and
//! > `tests/served_chain_objects.rs` is where it is checked.
//!
//! `crates/kayfabe-device/tests/served_chain_objects.rs` has **never existed** in this
//! repository, and `kayfabe-crec` cites the same non-file a second time as the reason its
//! own replay chain is safe. The obligation was load-bearing, precisely worded, cited
//! twice, and unchecked for its whole life — the same species as the `#[should_panic]` that
//! matched the wrong site and the `grep`-read gate that could not fail: a claim whose
//! *reference* to a check was mistaken for the check.
//!
//! # What is actually asserted, and against which universe
//!
//! ⊘ Quantified over `kayfabe_rmrpc::OBJECT_VERBS` + `OBJECT_CONTROLS` +
//! `PUBLICATION_CONTROLS`, read from the crate rather than restated here
//! (`gates_quantified_over_a_list`). A list that shrinks tomorrow shrinks this gate with it
//! — but it cannot shrink *silently*, because the byte-identity direction below sweeps a
//! universe the object model does not own.
//!
//! | seat | property |
//! |---|---|
//! | `objects` | its presence changes the chain's reply **only** for the verbs and controls it claims; every other command is byte-identical with and without it |
//! | `publications` | its presence changes **nothing the guest can read, ever** — and that is a property of the *type*, not of this test |
//!
//! ★★★ The second row is why the front seat is a `kayfabe_gsp::CommandObserver` and not a
//! `CommandPolicy`. `observe` has no return value, so an observer *cannot* answer, cannot
//! re-route a reply and cannot short-circuit the `find_map` that would otherwise take the
//! answer away from `InitTablePolicy`. This file executes that anyway — on the exact ids
//! where it would matter — because "the type makes it impossible" is worth a test that
//! would go red if the seat's type were ever widened back to a policy.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_device::inittables::WantedTable;
use kayfabe_gsp::{CommandObserver, CommandPolicy, Reply, RpcCommand, RpcFunction};

// =====================================================================================
// Harness
// =====================================================================================

fn abi() -> DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// The object model the shipped composition root builds, minus the shell: same `Arch`,
/// same stillborn isolate plane, same GPA window shape.
fn port_gpu() -> kayfabe_core::gpu::Gpu {
    kayfabe_core::gpu::Gpu::new(
        Box::new(kayfabe_chips::Ga10xArch::new()),
        Box::new(kayfabe_isolate::StillbornIsolates::new(
            "served_chain_seats",
        )),
        kayfabe_core::gpa::GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes")
}

fn object_link() -> Box<dyn CommandPolicy> {
    Box::new(kayfabe_rmrpc::ObjectPolicy::new(
        &abi(),
        kayfabe_abi::GuestOs::Linux,
        port_gpu(),
        kayfabe_device::ga10x::GA106_ENGINES,
    ))
}

fn publication_link() -> Box<dyn CommandObserver> {
    Box::new(kayfabe_rmrpc::PublicationObserver::over(
        &abi(),
        kayfabe_abi::GuestOs::Linux,
        Box::new(port_gpu()),
        kayfabe_rmrpc::SharedRefusalCensus::default(),
    ))
}

/// The production chain, with whichever seats the caller wants filled.
fn chain(objects: bool, publications: bool) -> Box<dyn CommandPolicy> {
    kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        abi(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks {
            publications: publications.then(publication_link),
            objects: objects.then(object_link),
        },
        // ⊘ No model name: this file's subject is which SEATS the chain has, not what the
        // static-info body says.
        kayfabe_device::staticinfo::GpuNames::default(),
    )
}

/// A `GSP_RM_CONTROL` command with `paramsSize` bytes of zeros.
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

/// A command on a bare RPC function, body of zeros.
fn func(function: RpcFunction, code: u32, len: usize) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 1,
        payload: vec![0u8; len],
        elements: 1,
        delivered: Vec::new(),
    }
}

/// Every command this file sweeps: the whole served-control vocabulary, both claim lists,
/// and the non-control functions the object seat claims.
///
/// ⊘ Built from the crates' own lists. `WantedTable::ALL` is `InitTablePolicy`'s universe
/// and is deliberately *not* the object model's — sweeping only the object model's own list
/// is how a claim about `ObjectPolicy` sat false under an executing test for a year
/// (`sticky_answer.rs`'s `ObjectPolicy` row, corrected 2026-08-08).
fn every_command() -> Vec<(String, RpcCommand)> {
    let mut v: Vec<(String, RpcCommand)> = Vec::new();
    for w in WantedTable::ALL {
        v.push((
            format!("table {:#010x}", w.cmd_id()),
            control(w.cmd_id(), w.params_size()),
        ));
    }
    for &cmd in kayfabe_rmrpc::OBJECT_CONTROLS {
        v.push((format!("object-control {cmd:#010x}"), control(cmd, 8)));
    }
    for &cmd in kayfabe_rmrpc::PUBLICATION_CONTROLS {
        // ★★★★ §16.42 — the params size is PER ID, not one size for the list. It used to be
        // `COPY_SERVER_RESERVED_PDES_PARAMS_SIZE` for every member, which was correct only
        // while every member happened to be a `0x90f1`-shaped publication. `translate_control`
        // pins `paramsSize` to the struct's EXACT size and refuses a mismatch by name, so
        // posting `0x00801813` at the wrong size would exercise the size refusal instead of
        // the seat — a sweep that runs, goes green, and tests something else.
        let size = if cmd == kayfabe_abi::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY {
            kayfabe_abi::generated::ctrl::Nv0080CtrlDmaSetPageDirectoryParams::SIZE
        } else {
            kayfabe_abi::gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE
        };
        v.push((format!("publication {cmd:#010x}"), control(cmd, size)));
    }
    // A control nobody claims, so the ledger arm is in the sweep too.
    v.push(("unclaimed control".into(), control(0x2080_0a4b, 8)));
    for (f, code, len) in [
        (
            RpcFunction::RmAlloc,
            kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
            64usize,
        ),
        (
            RpcFunction::Free,
            kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_FREE,
            16,
        ),
        (RpcFunction::UnloadingGuestDriver, 47, 16),
        // ★★★★ §16.38 — `DUP_OBJECT`, `NVOS55_PARAMETERS` = seven words. A zero body makes
        // this a refusal-by-name (`BridgeRefusal::ReservedClient` — `hClient == 0`), which
        // is still an ANSWER and therefore still a difference from the `None` the chain
        // gave before. ⊘ That is all this sweep asks; the accepted path is exercised
        // against the graph in `tests/tests/gsp_rm_alloc.rs`.
        (RpcFunction::DupObject, 0x15, 28),
    ] {
        v.push((format!("fn {code}"), func(f, code, len)));
    }
    v
}

fn reply_bytes(r: &Option<Reply>) -> Option<(u32, Vec<u8>)> {
    r.as_ref().map(|r| (r.rpc_result, r.body.clone()))
}

// =====================================================================================
// The object seat
// =====================================================================================

/// ★★ **The object seat changes the chain's answer for exactly the commands it claims, and
/// for nothing else.**
///
/// This is the sentence `served_chain` has always carried. It is executed here for the
/// first time.
#[test]
fn the_object_seat_changes_only_the_verbs_and_controls_it_claims() {
    let claimed_controls: Vec<u32> = kayfabe_rmrpc::OBJECT_CONTROLS.to_vec();
    let mut differed: Vec<String> = Vec::new();
    for (name, cmd) in every_command() {
        let without = reply_bytes(&chain(false, false).respond(&cmd));
        let with = reply_bytes(&chain(true, false).respond(&cmd));
        if without != with {
            differed.push(name);
        }
    }
    // The commands whose answer the seat is ALLOWED to change: its two claim lists.
    let mut allowed: Vec<String> = claimed_controls
        .iter()
        .map(|c| format!("object-control {c:#010x}"))
        .collect();
    for f in kayfabe_rmrpc::OBJECT_VERBS {
        let code = match f {
            RpcFunction::RmAlloc => kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
            RpcFunction::Free => kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_FREE,
            // ★★★★ §16.38 — `DUP_OBJECT`, fn 21. Named here rather than defaulted, so this
            // sweep proves the seat's answer for fn 21 CHANGED (it is now in `differed`) —
            // which is the property `s31`'s `unserviced fn 21` row says was missing.
            RpcFunction::DupObject => kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_DUP_OBJECT,
            other => panic!("OBJECT_VERBS grew a member this test cannot name: {other:?}"),
        };
        allowed.push(format!("fn {code}"));
    }
    allowed.sort();
    differed.sort();
    assert_eq!(
        differed, allowed,
        "the object seat changed the chain's answer for a command outside its claim lists \
         (or stopped changing one inside them)",
    );
    // ⊘ Non-vacuity: it must change SOMETHING, or the assertion above is `[] == []` and the
    // seat could be unseated entirely with this test green.
    assert!(
        !differed.is_empty(),
        "the object seat changed nothing at all — the sweep never reached it",
    );
}

// =====================================================================================
// The publication seat
// =====================================================================================

/// ★★★ **The publication seat changes NOTHING the guest can read — on every command, not
/// just the ones it claims.**
///
/// It sits FIRST in the chain, ahead of `InitTablePolicy`, which is the only seat from
/// which it can see a control that link terminates the chain for. From that seat, a link
/// that could answer would *replace* the correct answer. It cannot, and this is the
/// execution of that.
#[test]
fn the_publication_seat_changes_no_reply_byte_of_any_command() {
    for (name, cmd) in every_command() {
        let without = reply_bytes(&chain(false, false).respond(&cmd));
        let with = reply_bytes(&chain(false, true).respond(&cmd));
        assert_eq!(
            without, with,
            "the publication seat changed the reply to {name}"
        );
        // …and the same with the object seat filled, because that is the shipped shape.
        let without = reply_bytes(&chain(true, false).respond(&cmd));
        let with = reply_bytes(&chain(true, true).respond(&cmd));
        assert_eq!(
            without, with,
            "the publication seat changed the reply to {name} in the shipped composition",
        );
    }
}

/// ⊘ **And the seat is REACHED** — otherwise the byte-identity above is a statement about a
/// link that was never installed.
///
/// ★★★ This is the specific non-vacuity that the whole increment turns on: the publication
/// controls are *answered by `InitTablePolicy`*, so an observer seated anywhere below it
/// would see nothing, every reply byte would be identical, and every assertion in this file
/// would still pass. `[measured 2026-08-08]` that is not hypothetical — the port answered
/// `control 0x90f10106 result 0x00000000 x4` for two full boots while the value reached
/// nothing.
#[test]
fn the_publication_seat_sees_the_commands_the_link_below_it_answers() {
    let seen = Arc::new(AtomicUsize::new(0));
    let pubs = Arc::new(AtomicUsize::new(0));

    struct Counting {
        all: Arc<AtomicUsize>,
        publications: Arc<AtomicUsize>,
        abi: DriverAbiTable,
    }
    impl CommandObserver for Counting {
        fn observe(&mut self, cmd: &RpcCommand) {
            self.all.fetch_add(1, Ordering::Relaxed);
            if cmd.function == RpcFunction::RmControl
                && let Ok(req) = self.abi.decode_rpc_control(&cmd.payload)
                && kayfabe_rmrpc::PUBLICATION_CONTROLS.contains(&req.cmd)
            {
                self.publications.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let mut c = kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        abi(),
        kayfabe_device::ChainLogs::default(),
        kayfabe_device::census::ControlCensusLog::new(),
        kayfabe_device::ObjectLinks {
            publications: Some(Box::new(Counting {
                all: Arc::clone(&seen),
                publications: Arc::clone(&pubs),
                abi: abi(),
            })),
            objects: None,
        },
        kayfabe_device::staticinfo::GpuNames::default(),
    );

    let all = every_command();
    let total = all.len();
    // ⊘ Counted from the SWEEP, not from `PUBLICATION_CONTROLS.len()`.
    //
    // ★★★★ §16.42 — and quantified PER ID, because the old `>= 2 * len()` encoded a hidden
    // assumption that stopped being true. It held only while EVERY member was *also* a
    // `WantedTable::ALL` entry, so the sweep posted each one twice. `0x00801813` is answered
    // by `SetPageDirPolicy`, not `InitTablePolicy`, so it appears once — and a list-wide
    // multiplier turned "one member is covered once" into "the sweep stopped covering both
    // ids", which names the wrong defect. ⊘ A gate whose message misdescribes its own
    // failure costs a reader the time the gate saved.
    //
    // The invariant this test actually needs is *"every publication id reaches the seat"*,
    // so that is what is asserted, per id, with the id in the message.
    let posted = |cmd: u32| {
        all.iter()
            .filter(|(_, c)| {
                abi()
                    .decode_rpc_control(&c.payload)
                    .ok()
                    .is_some_and(|r| c.function == RpcFunction::RmControl && r.cmd == cmd)
            })
            .count()
    };
    for &cmd in kayfabe_rmrpc::PUBLICATION_CONTROLS {
        assert!(
            posted(cmd) >= 1,
            "the sweep stopped covering publication id {cmd:#010x}",
        );
    }
    let want_publications: usize = kayfabe_rmrpc::PUBLICATION_CONTROLS
        .iter()
        .map(|&c| posted(c))
        .sum();
    for (_, cmd) in &all {
        // ★ The reply is discarded on purpose: this test is about REACH, and the bytes are
        // the previous test's subject.
        let _ = c.respond(cmd);
    }
    assert_eq!(
        seen.load(Ordering::Relaxed),
        total,
        "the front seat did not see every command — a chain that short-circuits above it \
         makes the whole seat conditional on which link answers",
    );
    assert_eq!(
        pubs.load(Ordering::Relaxed),
        want_publications,
        "the front seat did not see the publication controls — which is exactly what a seat \
         BELOW InitTablePolicy would look like, with every reply byte still identical",
    );
    // ⊘ And the link below still answered them, which is the other half: the observer saw
    // them *and* did not take them.
    for &cmd in kayfabe_rmrpc::PUBLICATION_CONTROLS {
        let r = c.respond(&control(
            cmd,
            kayfabe_abi::gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE,
        ));
        assert!(
            r.is_some(),
            "{cmd:#010x} went unanswered with the observer seated — the observer took it",
        );
    }
}
