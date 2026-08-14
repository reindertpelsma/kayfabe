//! ★★★★★ **w303 B — `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`) DECIDES, and the decision is
//! the test.**
//!
//! ## The defect, in one sentence
//!
//! `docs/audits/w301_cancellation_error_leaks.md` §1.4: this id was answered `NV_OK` with
//! the guest's own bytes echoed back, **unconditionally**, on a channel group that may have
//! a live, scheduled host twin executing on a real GA106. Nothing was preempted. That is a
//! forged completion, and it breaks §0 of `road_to_v1_after_cup2.md`.
//!
//! ## ⊘⊘ AND THE AUDIT MISPLACED IT, WHICH MADE IT LOOK HARMLESS
//!
//! §1.4 says record 457 of `ctx_r1` is *"inside `cuCtxCreate`, not teardown"*.
//! `[measured 2026-08-14, w303]` decoding
//! `../nvidia-gpu-passthrough/traces/host_reference_ga106/ctx_r1.jsonl.zst` puts it inside
//! an unbroken `RM_FREE` cascade — 450, 451, 453, 456, **457**, 459, 463, 465 — i.e.
//! **`cuCtxDestroy`**. `init_r1`/`dev_r1`, which create no context, never issue it.
//! ⇒ the lie is not a setup nicety; it is a **teardown** ack, answering *"is the engine
//! still running out of the pages I am about to free?"* with an unconditional yes.
//!
//! ## What is asserted here, and why each half is needed
//!
//! | | fixture | expected | if the decision were removed |
//! |---|---|---|---|
//! | [`a_group_with_no_host_twin_is_served_because_there_is_nothing_to_preempt`] | TSG whose channel has `host_channel: None` | `NV_OK` + echo | still green |
//! | ★ [`a_group_with_a_live_host_twin_is_refused_by_name`] | the SAME fixture, one field written | `PREEMPT_UNPERFORMED_STATUS` | **RED** |
//!
//! ★ **The two fixtures differ in exactly one field**, so the change in answer is
//! attributable to the host twin and to nothing else — the shape
//! `falsifier_blocker_vs_only_blocker` asks for. The second is the known-positive: it was
//! watched failing against the old unconditional echo before this file was committed.
//!
//! ⊘ **What this does NOT claim.** Nothing is preempted here either — the refusal is the
//! honest half, not the fix. Performing it needs a host `preempt` verb
//! (`HostRmBackend::schedule` already finds the host TSG as `channel_parts(raw).tsg`; the
//! verb is that function with a different command id) and a boot. Until then, an
//! unperformed preempt says so.

use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::{ClassId, HClient, HObject, Pdb};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const CLIENT: HClient = HClient(0x5c00_0000);
const PDB: Pdb = Pdb(0x4E60_0000);

fn h(off: u32) -> HObject {
    HObject(0x5c00_0000 + off)
}

fn abi() -> DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// One guest process with one TSG holding one channel — the shape `cuCtxCreate` leaves
/// behind and the shape `cuCtxDestroy` then preempts.
fn one_group_gpu() -> (kayfabe_core::gpu::Gpu, HObject) {
    use kayfabe_abi::generated::classes as nv;
    let mut gpu = kayfabe_core::gpu::Gpu::new(
        Box::new(kayfabe_chips::Ga10xArch::new()),
        Box::new(kayfabe_isolate::StillbornIsolates::new("preempt_is_decided")),
        kayfabe_core::gpa::GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes");
    let (root, device, vas, tsg, chan) = (h(0), h(1), h(0x10), h(0x12), h(0x19));
    for ev in [
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: root,
            class: ClassId(nv::NV01_ROOT),
            facts: AllocFacts {
                client_kind: Some(ClientKind::User { pid: CLIENT.0 }),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: root,
            handle: device,
            class: ClassId(nv::NV01_DEVICE_0),
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: vas,
            class: ClassId(nv::FERMI_VASPACE_A),
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: CLIENT,
            vaspace: vas,
            pdb: PDB,
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: device,
            handle: tsg,
            class: ClassId(nv::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: tsg,
            handle: chan,
            class: ClassId(nv::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(vas),
                userd_flags: kayfabe_mocks::MockArch::userd_flags_for(kayfabe_arch::ids::VChid(0)),
                ..Default::default()
            },
        },
    ] {
        gpu.apply(ev).expect("the object model accepts the chain");
    }
    (gpu, tsg)
}

/// Write a host twin onto the group's one channel.
///
/// ⊘ **A fabricated `HostHandle`, and it is declared as such** — exactly as
/// `c_bug_regressions.rs::cb14_host_channel_touch_alone_blocks_a_late_merge` does, and for
/// the same reason: the state under test is *"this channel has a live host twin"*, and
/// materializing one for real through `StillbornIsolates` is impossible by construction.
/// The census this arm reads (`Channel::host_channel.is_some()`) is exactly the field being
/// written, so nothing else in the fixture has to be true for the assertion to mean what it
/// says.
fn give_it_a_host_twin(gpu: &mut kayfabe_core::gpu::Gpu) {
    let pid = *gpu.procs.keys().next().expect("one proc");
    let proc = gpu.procs.get_mut(&pid).expect("the proc");
    let cid = *proc.channels.keys().next().expect("one channel");
    proc.channels
        .get_mut(&cid)
        .expect("the channel")
        .host_channel = Some(kayfabe_isolate::HostHandle::new(
        kayfabe_isolate::IsolateId::new(pid.0, kayfabe_arch::ids::GpuId::ZERO),
        0xC0FFEE,
    ));
}

/// A `GSP_RM_CONTROL` naming `object` in `CLIENT`'s namespace, carrying the
/// `NVA06C_CTRL_PREEMPT_PARAMS` body `bWait = 1, bManualTimeout = 0, timeoutUs = 0`.
///
/// ★ **That body is the guest's own value**, not a fixture's choice:
/// `[measured 2026-08-14, w303 —
/// ../nvidia-gpu-passthrough/traces/host_reference_ga106/ctx_r1.jsonl.zst i=457, and
/// traces/guest_boots/run_s50_9a446e9_probefix_probe.log:469]` both carry
/// `ppre = ppost = 0100000000000000`. `bWait = 1` is what makes an `NV_OK` a promise about a
/// postcondition rather than an acknowledgement of a hint.
fn preempt(object: HObject, params_size: usize) -> RpcCommand {
    let mut payload = vec![0u8; 40 + params_size];
    payload[0..4].copy_from_slice(&CLIENT.0.to_le_bytes());
    payload[4..8].copy_from_slice(&object.0.to_le_bytes());
    payload[8..12].copy_from_slice(&kayfabe_abi::submit::NVA06C_CTRL_CMD_PREEMPT.to_le_bytes());
    payload[16..20].copy_from_slice(&(params_size as u32).to_le_bytes());
    if params_size >= 1 {
        payload[40] = 1; // bWait
    }
    RpcCommand {
        function: RpcFunction::RmControl,
        code: kayfabe_abi::generated::rpc::NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn policy(gpu: kayfabe_core::gpu::Gpu) -> kayfabe_rmrpc::ObjectPolicy {
    kayfabe_rmrpc::ObjectPolicy::new(
        &abi(),
        kayfabe_abi::GuestOs::Linux,
        gpu,
        kayfabe_device::ga10x::GA106_ENGINES,
    )
}

// =====================================================================================
// The two arms
// =====================================================================================

/// **Nothing to preempt ⇒ `NV_OK`, and it is TRUE.**
///
/// A group no member of which has ever reached the host GPU is idle by construction, so the
/// preemption is vacuously complete. ⊘ This arm is *not* the gate — it would stay green
/// under the old unconditional echo. It is here so the gate below cannot pass by refusing
/// everything, which is the failure mode a one-sided refusal test has.
#[test]
fn a_group_with_no_host_twin_is_served_because_there_is_nothing_to_preempt() {
    let (gpu, tsg) = one_group_gpu();
    let mut p = policy(gpu);
    let cmd = preempt(tsg, kayfabe_abi::submit::PREEMPT_PARAMS_SIZE);
    let reply = p.respond(&cmd).expect("the id is claimed and decided");
    assert_eq!(
        reply.rpc_result, 0,
        "a group with no host twin has nothing a preemption could preempt, so NV_OK is a \
         true statement rather than a forged one"
    );
    assert_eq!(
        reply.body, cmd.payload,
        "★ the body is the guest's OWN bytes — native GA106 leaves ppost == ppre, so an \
         echo is the reply real hardware gives and there is no [OUT] field to get wrong"
    );
}

/// ★★★★★ **THE GATE, and the known-positive.** The same fixture with **one field**
/// written — a live host twin on the group's channel — must stop being answered `NV_OK`.
///
/// ★ *Watched failing before this file was committed*: against the previous behaviour
/// (`0xa06c0105` in `INPUT_ONLY_CONTROLS`, dispatched to `respond_input_only`) this
/// assertion reports `rpc_result 0x0`, because that arm never looks at the group at all.
/// ⊘ A gate nobody has seen fail is not a gate.
#[test]
fn a_group_with_a_live_host_twin_is_refused_by_name() {
    let (mut gpu, tsg) = one_group_gpu();
    give_it_a_host_twin(&mut gpu);
    let mut p = policy(gpu);
    let reply = p
        .respond(&preempt(tsg, kayfabe_abi::submit::PREEMPT_PARAMS_SIZE))
        .expect("the id is claimed and decided");
    assert_eq!(
        reply.rpc_result,
        kayfabe_abi::submit::PREEMPT_UNPERFORMED_STATUS,
        "★★★ THE FORGED SUCCESS. This group has a live host twin on real silicon and this \
         port has no preempt verb, so NV_OK would promise the guest a postcondition \
         (bWait=1) we did not deliver — and it would do so on the TEARDOWN path, while the \
         guest is about to free the pages the engine may still be reading"
    );
    assert!(
        reply.body.is_empty(),
        "a refusal carries an EMPTY body — never the caller's bytes wearing a non-OK \
         status, which reads as a partially-served call"
    );
}

/// The shape refusal stays a **different** status from the state refusal.
///
/// ⊘ *"You asked wrongly"* (`0x47`) and *"the object is not in a state where this can be
/// done"* (`NV_ERR_INVALID_STATE`) are two findings. Collapsing them would make a malformed
/// guest indistinguishable from a busy one at the only place a boot log can tell.
#[test]
fn a_malformed_preempt_is_refused_with_the_shape_status_not_the_state_status() {
    let (mut gpu, tsg) = one_group_gpu();
    give_it_a_host_twin(&mut gpu); // so the state refusal is the one being ruled out
    let mut p = policy(gpu);
    let reply = p
        .respond(&preempt(tsg, 12))
        .expect("the id is claimed even when malformed");
    assert_eq!(
        reply.rpc_result,
        kayfabe_abi::submit::INPUT_ONLY_REFUSED_STATUS,
        "a wrong paramsSize is refused as a SHAPE error, and the shape check runs before \
         the group is ever routed"
    );
    assert_ne!(
        reply.rpc_result,
        kayfabe_abi::submit::PREEMPT_UNPERFORMED_STATUS,
        "…and it is not the state status, even though this fixture would earn one"
    );
}

/// An unroutable group is **UNKNOWN**, never "empty".
///
/// ★ The trap this closes is `a_census_zero_needs_a_known_positive` in its exact form: a
/// group we cannot resolve has a host-twin count of zero *in our census* purely because we
/// could not look, and serving `NV_OK` off that zero would be inferring safety from our own
/// blindness.
#[test]
fn a_group_that_does_not_route_is_unknown_and_not_empty() {
    let (gpu, _tsg) = one_group_gpu();
    let mut p = policy(gpu);
    let reply = p
        .respond(&preempt(
            HObject(0xDEAD_0000),
            kayfabe_abi::submit::PREEMPT_PARAMS_SIZE,
        ))
        .expect("the id is claimed even when the object is not ours");
    assert_eq!(
        reply.rpc_result,
        kayfabe_abi::submit::PREEMPT_UNPERFORMED_STATUS,
        "a group that did not route has an UNKNOWN host-twin count; answering NV_OK off \
         that would be reading our own blindness as an empty set"
    );
}

// =====================================================================================
// The table's own discipline
// =====================================================================================

/// `0xa06c0105` is **out** of `INPUT_ONLY_CONTROLS` and still **claimed** by
/// `OBJECT_CONTROLS`.
///
/// ⊘ Both halves, because dropping only the first would send the id to the
/// `UnservicedLedger`'s blanket `0x56` — a *different* wrong answer (it asserts "no such
/// control" about a `ROUTE_TO_PHYSICAL` verb we recognise) reached by *removing* code,
/// which is the shape that looks like a cleanup.
#[test]
fn preempt_left_the_input_only_table_and_is_still_claimed() {
    assert!(
        kayfabe_abi::submit::input_only_control(kayfabe_abi::submit::NVA06C_CTRL_CMD_PREEMPT)
            .is_none(),
        "★ 0xa06c0105 must NOT be an input-only ack: its ack is not the whole verb — see \
         w301 §1.4"
    );
    assert!(
        kayfabe_rmrpc::OBJECT_CONTROLS.contains(&kayfabe_abi::submit::NVA06C_CTRL_CMD_PREEMPT),
        "…and it must still be CLAIMED, or it falls to the UnservicedLedger's 0x56, which \
         asserts 'no such control' about a control we do recognise"
    );
    // Non-vacuity: the sibling that legitimately IS an input-only ack is still one.
    assert!(
        kayfabe_abi::submit::input_only_control(0xa06c_0103).is_some(),
        "★ known-positive for the zero above — SET_TIMESLICE is still in the table, so the \
         lookup is live"
    );
}

/// ★★ Every remaining row states **why** its ack is the whole verb.
///
/// The w301 finding was not that a row was wrong; it was that membership carried no reason,
/// so a row that stopped being true looked exactly like one that never was. The kind now
/// lives **on the value** (`InputOnlyDisposition`), and a row cannot be added without
/// writing the argument down.
#[test]
fn every_input_only_row_states_why_its_ack_is_the_whole_verb() {
    use kayfabe_abi::submit::{INPUT_ONLY_CONTROLS, InputOnlyDisposition};
    assert!(!INPUT_ONLY_CONTROLS.is_empty(), "the table is non-empty");
    for row in INPUT_ONLY_CONTROLS {
        let InputOnlyDisposition::AckIsTheWholeVerb(why) = row.disposition;
        assert!(
            why.len() > 60,
            "0x{:08x} {} carries a disposition string too short to be an argument: {why:?}",
            row.cmd,
            row.name,
        );
        assert!(
            !row.authority.is_empty(),
            "0x{:08x} {} has no authority",
            row.cmd,
            row.name,
        );
    }
}
