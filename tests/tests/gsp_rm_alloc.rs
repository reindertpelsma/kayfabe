//! ★★★ **`GspRmAlloc` — the wall a LIVE BOOT measured, and the rung that serves it.**
//!
//! `docs/design/boot_measured_2026_08_01.md` §3 is the subject of this file. A stock
//! 580.159.04 driver bound to the emulated GPU, reached `RmInitAdapter`, and was refused
//! `0x56` (`NV_ERR_NOT_SUPPORTED`) for **every** class it asked for:
//!
//! ```text
//! rpcRmApiAlloc_GSP: hClient=0xc1e00004 hParent=0 hObject=0 hClass=0x00000000 paramsSize=0x78
//! rpcRmApiAlloc_GSP: … hClass=0x00000080 (NV01_DEVICE_0)
//! rpcRmApiAlloc_GSP: … hClass=0x00002080 (NV20_SUBDEVICE_0)
//! rpcRmApiAlloc_GSP: … hClass=0x0000007e (NV01_EVENT_KERNEL_CALLBACK_EX)
//!    → pHeap != NULL @ mem_desc.c:152 → kbusInitBar2_HAL → RmInitAdapter failed! (0x24:0x40:1220)
//! ```
//!
//! # ★★ The controlling discipline here is the BEFORE/AFTER pair, not the green
//!
//! Every assertion that the four classes are served is run **beside** its own negative
//! control: the identical byte sequence through the identical chain built *without* the
//! object-model link, asserted to still produce `0x56`. That pair is what makes the green
//! mean something. `suspect_the_instrument_first`: a test that only ever saw the served
//! path could be green because the chain never ran, and the ledger of this project records
//! seven separate occasions on which the instrument was the defect.
//!
//! ⊘ **What this file does not claim.** It is not a boot. It drives the same bytes through
//! the same policy chain the port installs, with no guest, no hypervisor and no GPU; only
//! a live boot says what happens (`only_live_boots_are_proof`). The boot that this rung
//! was verified against is reported separately.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_abi::GuestOs;
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_arch::Arch;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, HClient, HObject};
use kayfabe_arch::{ObjectKind, PteDecode, PushMethod};
use kayfabe_chips::Ga10xArch;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use kayfabe_isolate::{IsolateFactory, IsolateId, StillbornIsolates};
use kayfabe_rmrpc::{BridgeRefusal, OBJECT_VERBS, ObjectPolicy};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w, fn_id};
use kayfabe_trace::{FaultTag, Faulted};

// =================================================================================
// The wall as MEASURED by the 2026-08-01 boot at rev `55a106f`
// (`docs/design/boot_measured_2026_08_01.md` §3), transcribed ONCE, from its dmesg
// =================================================================================

/// The kernel RM client the 2026-08-01 boot's `rpcRmApiAlloc_GSP` lines all carry.
const BOOT_HCLIENT: u32 = 0xc1e0_0004;

/// `NV0000_ALLOC_PARAMETERS`'s `paramsSize` as the boot declared it: `0x78` = 120 bytes.
///
/// ★ Transcribed from the dmesg, **not** from `size_of` a Rust mirror. A test whose
/// expected size came from the same struct the decoder uses would be asserting the mirror
/// against itself; this number came off a real guest's own message.
const BOOT_ROOT_PARAMS_SIZE: usize = 0x78;

/// `KERNEL_PID` — the sentinel RM stamps on a kernel-privileged client's `processID`
/// under `RMCFG_FEATURE_PLATFORM_UNIX` (`ogkm-580: rpc.h:67-77`). The boot's client is the
/// guest's own kernel RM, so this is the value its root alloc carries.
const KERNEL_PID: u32 = 0xffff_ffff;

/// The four classes, in the order the boot asked for them. ★ Public and iterated over
/// rather than restated per test — `gates_quantified_over_a_list`: a list spelled out in
/// three places is a list that shrinks in one of them with nothing going red.
const BOOT_CLASSES: &[(u32, &str)] = &[
    (0x0000_0000, "NV01_ROOT"),
    (0x0000_0080, "NV01_DEVICE_0"),
    (0x0000_2080, "NV20_SUBDEVICE_0"),
    (0x0000_007e, "NV01_EVENT_KERNEL_CALLBACK_EX"),
];

// =================================================================================
// Harness
// =================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// A `Gpu` built the way the **port** builds it: the shipped [`Ga10xArch`], a
/// non-spawning isolate factory, a declared guest-physical window.
///
/// ⊘ Deliberately not `MockArch`/`MockIsolateFactory`. This file exists to say that the
/// composition the archive ships accepts the boot's traffic; a harness that swapped either
/// of those would be testing a different device.
fn port_gpu() -> Gpu {
    Gpu::new(
        Box::new(Ga10xArch::new()),
        Box::new(StillbornIsolates::new("test: no forwarding plane")),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes")
}

/// The port's policy chain, **with** the object-model link.
fn chain_with_objects() -> (Box<dyn CommandPolicy>, kayfabe_device::unserviced::UnservicedLog) {
    let log = kayfabe_device::unserviced::UnservicedLog::new();
    let policy = kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        *abi(),
        log.clone(),
        kayfabe_device::faultbuffer::FaultBufferLog::new(),
        Some(Box::new(ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu()))),
    );
    (policy, log)
}

/// ★★★ The **negative control** — master's chain, exactly: no object-model link. This is
/// what the 2026-08-01 boot ran against.
fn chain_without_objects() -> (Box<dyn CommandPolicy>, kayfabe_device::unserviced::UnservicedLog) {
    let log = kayfabe_device::unserviced::UnservicedLog::new();
    let policy = kayfabe_device::served_policy(
        kayfabe_device::default_chip(),
        *abi(),
        log.clone(),
        kayfabe_device::faultbuffer::FaultBufferLog::new(),
        None,
    );
    (policy, log)
}

fn command(msg: &[u8]) -> RpcCommand {
    let env = abi()
        .decode_rpc_envelope(msg)
        .expect("well-formed envelope");
    RpcCommand {
        function: FUNCTIONS.classify(env.function),
        code: env.function,
        sequence: env.sequence,
        payload: abi().rpc_payload(msg).expect("payload").to_vec(),
        elements: 1,
    }
}

/// The boot's root alloc: `hParent = hObject = 0`, `hClass = 0`, `paramsSize = 0x78`.
fn root_alloc(seq: u32) -> RpcCommand {
    let mut params = w::client_root_params(BOOT_HCLIENT, KERNEL_PID);
    params.resize(BOOT_ROOT_PARAMS_SIZE, 0);
    assert_eq!(
        params.len(),
        BOOT_ROOT_PARAMS_SIZE,
        "the fixture must carry the size the boot declared"
    );
    command(&w::message(
        fn_id::GSP_RM_ALLOC,
        seq,
        &w::alloc_body(
            BOOT_HCLIENT,
            w::NV01_NULL_OBJECT,
            w::NV01_NULL_OBJECT,
            0x0000_0000,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            &params,
        ),
    ))
}

/// A non-root alloc under `parent`, carrying `params`.
fn alloc_with(seq: u32, parent: u32, object: u32, class: u32, params: &[u8]) -> RpcCommand {
    command(&w::message(
        fn_id::GSP_RM_ALLOC,
        seq,
        &w::alloc_body(
            BOOT_HCLIENT,
            parent,
            object,
            class,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            params,
        ),
    ))
}

/// A non-root alloc under `parent`, with **no** params.
///
/// ★★ Correct for `NV20_SUBDEVICE_0` and `NV01_EVENT_KERNEL_CALLBACK_EX` and WRONG for
/// `NV01_DEVICE_0`, deliberately — the asymmetry is a property this rung establishes and
/// the first draft of this file got it wrong in exactly the way a reader would. The
/// subdevice and the event are `AllocParams::NoDeclaredFacts`: their params are never
/// read, so sending none is indistinguishable from sending any. A Device is **not**: it
/// must declare `deviceId`, and the object model refuses one that does not rather than
/// defaulting it to GPU 0 (`RmGraph::gpu_of`). ⊘ Use [`device_alloc`] for a Device; this
/// function on a Device produces `AbiError::Truncated`, and
/// [`a_device_that_declares_no_device_id_is_refused_rather_than_defaulted`] pins that.
fn alloc(seq: u32, parent: u32, object: u32, class: u32) -> RpcCommand {
    alloc_with(seq, parent, object, class, &[])
}

/// The boot's Device alloc: `NV0080_ALLOC_PARAMETERS` declaring `deviceId = 0`.
fn device_alloc(seq: u32, parent: u32, object: u32) -> RpcCommand {
    alloc_with(seq, parent, object, 0x0000_0080, &w::device_params(0, 0, 0))
}

fn free(seq: u32, object: u32) -> RpcCommand {
    command(&w::message(
        fn_id::FREE,
        seq,
        &w::driver_free_body(BOOT_HCLIENT, object),
    ))
}

/// Handles chosen to match the boot's shape: a client root whose handle **is** its
/// `hClient`, then a device, subdevice and event beneath it.
const H_DEVICE: u32 = 0xcaf0_0001;
const H_SUBDEVICE: u32 = 0xcaf0_0002;
const H_EVENT: u32 = 0xcaf0_0003;

/// The boot's whole alloc sequence, in order.
fn boot_allocs() -> Vec<RpcCommand> {
    vec![
        root_alloc(1),
        device_alloc(2, BOOT_HCLIENT, H_DEVICE),
        alloc(3, H_DEVICE, H_SUBDEVICE, 0x0000_2080),
        alloc(4, H_SUBDEVICE, H_EVENT, 0x0000_007e),
    ]
}

fn ok(reply: &Reply, what: &str) {
    assert_eq!(
        reply.rpc_result, 0,
        "{what}: expected NV_OK, got {:#x}",
        reply.rpc_result
    );
}

// =================================================================================
// 1. The wall, and that it is gone
// =================================================================================

/// ⊘ **The negative control, run FIRST in this file so a reader meets the wall before the
/// fix.** Master's chain refuses all four classes with `0x56`, which is the exact status
/// the 2026-08-01 dmesg printed.
///
/// ★ It also asserts the refusals reach the **unserviced ledger**, because that is what
/// made the wall diagnosable at all: without a host-side list, "what has this port not
/// built" is answerable only one boot at a time.
#[test]
fn without_the_object_link_every_class_the_boot_asked_for_is_refused_0x56() {
    let (mut chain, log) = chain_without_objects();
    for (cmd, (class, name)) in boot_allocs().into_iter().zip(BOOT_CLASSES) {
        let reply = chain.respond(&cmd);
        match reply {
            // The chain has no answer, so the FSM refuses by name. Both shapes are the
            // same statement; which one occurs is the chain's business and not this
            // test's, so both are accepted and an `NV_OK` is not.
            None => {}
            Some(r) => assert_ne!(
                r.rpc_result, 0,
                "{name} ({class:#010x}) must NOT be answered NV_OK by a chain with no \
                 object model — that is the C's `NV_OK` fall-through, the exact lie \
                 `#127` removed"
            ),
        }
    }
    assert_eq!(
        log.total(),
        BOOT_CLASSES.len() as u64,
        "all four allocs must land in the unserviced ledger"
    );
}

/// ★★★ **The rung.** The same four messages, through the port's chain, all `NV_OK`.
#[test]
fn the_four_classes_the_boot_asked_for_are_all_served() {
    let (mut chain, log) = chain_with_objects();
    for (cmd, (class, name)) in boot_allocs().into_iter().zip(BOOT_CLASSES) {
        let reply = chain
            .respond(&cmd)
            .unwrap_or_else(|| panic!("{name} ({class:#010x}) was not answered at all"));
        ok(&reply, name);
    }
    assert_eq!(
        log.total(),
        0,
        "no alloc may reach the unserviced ledger once the object link is installed"
    );
}

/// ★ **Non-vacuity for the test above**, and it is a different question from "did the
/// replies say `NV_OK`": a policy that accepted everything and declared nothing would pass
/// that test. This asserts the graph actually took four facts.
#[test]
fn serving_the_four_classes_declares_four_facts_into_the_object_model() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    for cmd in boot_allocs() {
        let _ = policy.deliver(&cmd).expect("the object model accepts it");
    }
    assert_eq!(policy.applied(), 4, "four allocs, four accepted events");
    assert!(
        policy.census().is_empty(),
        "no refusals: {:?}",
        policy.census()
    );
}

/// The matching frees — the second half of the dmesg (`rpcRmApiFree_GSP`), and the half a
/// port that only served allocs would leak on.
///
/// ★ Freed **leaf-first**, which is the order the guest's own teardown uses. Freeing the
/// client root last is what exercises the namespace-wide free path.
#[test]
fn the_matching_frees_are_served() {
    let (mut chain, log) = chain_with_objects();
    for cmd in boot_allocs() {
        ok(&chain.respond(&cmd).expect("alloc answered"), "alloc");
    }
    for (seq, h) in [H_EVENT, H_SUBDEVICE, H_DEVICE, BOOT_HCLIENT].into_iter().enumerate() {
        let reply = chain
            .respond(&free(10 + seq as u32, h))
            .unwrap_or_else(|| panic!("free of {h:#010x} was not answered"));
        ok(&reply, "free");
    }
    assert_eq!(log.total(), 0, "no free may reach the unserviced ledger");
}

/// ⊘ And the frees are refused without the link — the before/after pair for the second
/// half, so "the frees fail identically" (the dmesg's own words) is a thing this file
/// reproduces rather than quotes.
#[test]
fn without_the_object_link_the_frees_are_refused_too() {
    let (mut chain, log) = chain_without_objects();
    for (seq, h) in [H_EVENT, H_SUBDEVICE, H_DEVICE, BOOT_HCLIENT].into_iter().enumerate() {
        if let Some(r) = chain.respond(&free(10 + seq as u32, h)) {
            assert_ne!(r.rpc_result, 0, "a free must never be answered NV_OK by a chain that models no objects");
        }
    }
    assert_eq!(log.total(), 4);
}

// =================================================================================
// 2. What is served is NOT "everything"
// =================================================================================

/// ★★★ The single most important negative in this file. Serving four classes must not
/// have made the port serve **all** classes — that would be the C's behaviour, and the C's
/// behaviour is what `#127` measured to be a lie the driver then acts on (the
/// `kbusInitBarsSize_KERNEL` stack overrun, bench run `t126b` at `rev f2acb89`,
/// 2026-07-31 — `kayfabe_gsp::EchoOk`'s rustdoc carries the dmesg).
///
/// `0x1234` is on no allowlist and in no params table, so it is refused *before* a byte of
/// its params is decoded.
#[test]
fn a_class_this_port_does_not_model_is_still_refused_by_name() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let _ = policy.deliver(&root_alloc(1)).expect("root accepted");
    let e = policy
        .deliver(&alloc(2, BOOT_HCLIENT, 0xdead_0001, 0x1234))
        .expect_err("an unlisted class must be refused");
    assert!(
        matches!(e, BridgeRefusal::AllocClassNotPermitted { class: 0x1234, .. }),
        "expected the CAPABILITY gate to refuse it (before the params table), got {e:?}"
    );
    assert_eq!(
        policy.census().of(e.fault_tag()),
        1,
        "the refusal must be countable"
    );
}

/// ★★ And a class that is **allowlisted but unmapped** is refused by a *different* name —
/// the distinction `BridgeRefusal::UnmappedAllocClass` exists to make. `NV01_EVENT`
/// (`0x5`) is on the shared allowlist and has no `alloc_params` row.
///
/// ⊘ This is the assertion that would go red if somebody "fixed" the boot by making
/// `alloc_params` return `NoDeclaredFacts` for everything. That fix would work, on this
/// boot, and would silently give the guest a default answer for every class forever.
#[test]
fn an_allowlisted_but_unmapped_class_is_refused_as_unmapped_not_as_unpermitted() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let _ = policy.deliver(&root_alloc(1)).expect("root accepted");
    let e = policy
        .deliver(&alloc(2, BOOT_HCLIENT, 0xdead_0002, 0x0000_0005))
        .expect_err("an unmapped class must be refused");
    assert!(
        matches!(e, BridgeRefusal::UnmappedAllocClass { class: 0x5 }),
        "got {e:?}"
    );
}

/// ★★ The event class's params are **never read**, which is the property that lets this
/// port admit a class whose `NV0005_ALLOC_PARAMETERS.data` is a guest-kernel callback
/// pointer. Two allocs identical except for 64 bytes of hostile params must produce the
/// same answer.
#[test]
fn the_event_class_params_are_never_read_however_hostile_they_are() {
    let hostile = vec![0xffu8; 64];
    let poisoned = command(&w::message(
        fn_id::GSP_RM_ALLOC,
        4,
        &w::alloc_body(
            BOOT_HCLIENT,
            H_SUBDEVICE,
            H_EVENT,
            0x0000_007e,
            hostile.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            &hostile,
        ),
    ));

    let mut clean = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let mut dirty = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    for (i, cmd) in boot_allocs().into_iter().enumerate() {
        let _ = clean.deliver(&cmd).expect("clean sequence accepted");
        if i < 3 {
            let _ = dirty.deliver(&cmd).expect("dirty sequence prefix accepted");
        }
    }
    let _ = dirty
        .deliver(&poisoned)
        .expect("an event alloc with hostile params is accepted — its params are not read");
    assert_eq!(clean.applied(), dirty.applied());
    assert!(dirty.census().is_empty());
}

// =================================================================================
// 3. The chain — the link claims what it declares and nothing else
// =================================================================================

/// ★★★ The composability property, and the reason `ObjectPolicy` exists beside
/// `GraphPolicy` at all: the link must **decline** every function outside
/// [`OBJECT_VERBS`], so the recorders below it in the chain still see what nobody
/// answered.
///
/// Quantified over `OBJECT_VERBS` rather than restating it — shorten that list and this
/// gate shrinks with it *visibly*, which is the only kind of shrinking that is honest.
#[test]
fn the_object_link_claims_exactly_its_declared_verbs() {
    assert_eq!(
        OBJECT_VERBS,
        &[RpcFunction::RmAlloc, RpcFunction::Free],
        "the served verb set changed; the tests below and the port's chain-position \
         argument are both about THIS list"
    );
    for f in OBJECT_VERBS {
        assert!(ObjectPolicy::claims(*f), "{f:?} must be claimed");
    }
    // A representative of every other family this port sees on the wire.
    for f in [
        RpcFunction::RmControl,
        RpcFunction::DupObject,
        RpcFunction::UnloadingGuestDriver,
        RpcFunction::GetGspStaticInfo,
        RpcFunction::SetRegistry,
        RpcFunction::ContinuationRecord,
        RpcFunction::Other(0xdead),
    ] {
        assert!(
            !ObjectPolicy::claims(f),
            "{f:?} must NOT be claimed — claiming it silences a link or a recorder \
             below this one in the chain"
        );
    }
}

/// ★★ The consequence, asserted at the chain rather than at the predicate: installing the
/// object link must leave the **unserviced ledger** working. A link that answered
/// everything (which `GraphPolicy` does) would empty it permanently, and nothing else
/// would go red.
#[test]
fn the_object_link_does_not_silence_the_unserviced_ledger() {
    // A control nothing in this port models.
    let unmodelled = command(&w::message(
        fn_id::GSP_RM_CONTROL,
        7,
        &w::control_body(BOOT_HCLIENT, H_SUBDEVICE, 0x2080_0fff, 0, 0, &[]),
    ));
    let (mut with, log_with) = chain_with_objects();
    let (mut without, log_without) = chain_without_objects();
    let a = with.respond(&unmodelled);
    let b = without.respond(&unmodelled);
    assert_eq!(
        a.map(|r| r.rpc_result),
        b.map(|r| r.rpc_result),
        "the object link must not change the answer to a command it does not claim"
    );
    assert_eq!(log_with.total(), 1, "the ledger still records it");
    assert_eq!(log_with.total(), log_without.total());
}

/// ★ And a command an existing link **does** answer keeps its existing answer, byte for
/// byte. This is the "changes no byte" half of the chain-position argument in
/// `served_chain`'s comment, checked instead of asserted.
#[test]
fn installing_the_object_link_changes_no_reply_the_chain_already_had() {
    let teardown = command(&w::message(fn_id::UNLOADING_GUEST_DRIVER, 47, &[0u8; 12]));
    let (mut with, _) = chain_with_objects();
    let (mut without, _) = chain_without_objects();
    assert_eq!(with.respond(&teardown), without.respond(&teardown));
}

// =================================================================================
// 4. The three ports the port did NOT build — asserted to refuse, not to work
// =================================================================================

/// ★★★ `Ga10xArch` classifies NVIDIA's **real** ids. This is what `MockArch` cannot do:
/// its table is keyed on invented `0xF0xx` ids, so a real `NV01_ROOT` would be
/// [`ObjectKind::Unknown`] and the graph could enforce none of its parenting rules.
#[test]
fn the_shipped_arch_classifies_the_real_wire_class_ids() {
    let a = Ga10xArch::new();
    assert_eq!(a.classify(ClassId(0x0)), ObjectKind::Client);
    assert_eq!(a.classify(ClassId(0x41)), ObjectKind::Client);
    assert_eq!(a.classify(ClassId(0x80)), ObjectKind::Device);
    assert_eq!(a.classify(ClassId(0x2080)), ObjectKind::Subdevice);
    assert_eq!(a.classify(ClassId(0x7e)), ObjectKind::Event);
    assert_eq!(a.classify(ClassId(0x90f1)), ObjectKind::VaSpace);
    assert_eq!(
        a.classify(ClassId(0xc56f)),
        ObjectKind::Channel {
            engine: EngineKind::GrCompute
        }
    );
    // ⊘ And an id it does not name is Unknown, never a guess.
    assert_eq!(a.classify(ClassId(0x1234)), ObjectKind::Unknown);
}

/// ⊘ **The data plane REFUSES, and this test exists so that "unbuilt" cannot quietly
/// become "plausible".** The day somebody implements the GA10x GMMU, this test goes red
/// and they must delete it deliberately — which is exactly the review that should happen.
#[test]
fn the_shipped_arch_refuses_every_data_plane_seam() {
    let a = Ga10xArch::new();
    assert_eq!(a.mmu().levels(), 0, "no walk is possible");
    assert!(a.mmu().page_sizes().is_empty(), "no leaf size is enumerated");
    assert_eq!(a.mmu().level_shift(0), None);
    assert_eq!(a.mmu().decode_entry(0, u128::MAX), PteDecode::Invalid);
    assert_eq!(a.userd().userd_size(), 0);
    assert_eq!(a.decode_doorbell(0xd000_0000_0000_0000), None);
    assert_eq!(a.pushbuffer().decode_method(0xffff_ffff, &[]), PushMethod::Opaque);
    assert!(a.pushbuffer().gpfifo_entries(&[0xffu8; 64]).is_empty());
    assert!(
        a.gsp().is_none(),
        "the GSP REGISTER model is the ChipProfile's, never this Arch's"
    );
    assert!(
        a.name().contains("unbuilt"),
        "the name must say so: it is what the homogeneity guard and every Debug print"
    );
}

/// ⊘ `StillbornIsolates` spawns nothing and can issue no verb. It is the isolate plane's
/// `RefusingRam`, and the property that makes it safe in a shipped archive is that it
/// **answers** nothing — a mock would answer.
#[test]
fn the_shipped_isolate_factory_can_never_issue_a_verb() {
    let mut f = StillbornIsolates::new("test");
    let mut iso = f.spawn(IsolateId::new(1, GpuId::ZERO));
    assert_eq!(iso.pool_size(), 0);
    assert_eq!(iso.idle_workers(), 0);
    assert!(iso.checkout().is_none(), "no worker, ever");
    assert!(iso.checked_out().is_empty());
    assert!(iso.in_flight() == 0);
    assert!(iso.is_retired(), "retired at birth: the refusal is permanent");
    assert_eq!(f.spawned.len(), 1, "the witness records the id it was asked for");
}

/// ★ And the object model realizes on top of it — the composition the port performs, run
/// here so a failure is a unit failure and not a boot failure.
#[test]
fn the_ports_object_model_realizes_with_no_forwarding_plane() {
    let gpu = port_gpu();
    assert_eq!(gpu.procs.len(), 0, "no guest proc exists before any client root");
    // The system proc got its GpuId::ZERO isolate — a stillborn one.
    assert!(gpu.system.isolates.contains_key(&GpuId::ZERO));
}

// =================================================================================
// 5. The namespace, and the recycle canary
// =================================================================================

/// ★★ The client-root normalisation, checked at the level the boot exercises it: the boot
/// sends `hParent = hObject = 0`, and the object model requires `parent == handle` for a
/// root. RM's own rule is that the `hClient` **is** its root object's handle
/// (`serverAllocClient` writes `pParams->hResource = hClient`).
///
/// If that normalisation were dropped, this alloc would create a node at `(client, 0)` and
/// the device alloc beneath it would have no resolvable parent — which is a failure the
/// boot would show as a refusal on the *second* class, not the first.
#[test]
fn the_client_root_is_normalised_so_the_device_beneath_it_resolves() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let _ = policy.deliver(&root_alloc(1)).expect("root accepted");
    let _ = policy
        .deliver(&device_alloc(2, BOOT_HCLIENT, H_DEVICE))
        .expect("a device parented on the CLIENT HANDLE must resolve");
    let g = policy.gpu();
    assert!(
        g.spine
            .rmgraph
            .origin_of_kind(
                kayfabe_core::rmgraph::NodeKey {
                    client: HClient(BOOT_HCLIENT),
                    handle: HObject(BOOT_HCLIENT),
                },
                ObjectKind::Client,
            )
            .is_some(),
        "the root node must live at (hClient, hClient)"
    );
}

/// ★★★ The statelessness canary, on the verbs this rung just started serving. RM recycles
/// `hClient` values **by design**; a port that grew a seen-set or a dedup cache while
/// wiring this would refuse or mis-attribute a legal recycle, and no other test here would
/// notice.
#[test]
fn a_recycled_hclient_survives_alloc_free_alloc() {
    let (mut chain, _) = chain_with_objects();
    for round in 0..3u32 {
        ok(
            &chain.respond(&root_alloc(round * 10 + 1)).expect("answered"),
            "root",
        );
        ok(
            &chain
                .respond(&device_alloc(round * 10 + 2, BOOT_HCLIENT, H_DEVICE))
                .expect("answered"),
            "device",
        );
        ok(
            &chain
                .respond(&free(round * 10 + 3, BOOT_HCLIENT))
                .expect("answered"),
            "free of the client root",
        );
    }
}

/// A refusal must be **countable**, not merely returned: the census is what turns "zero
/// refusals over a clean boot" into a bound rather than an absence.
#[test]
fn refusals_are_countable_by_tag() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let _ = policy.deliver(&alloc(1, BOOT_HCLIENT, 0xdead_0003, 0x1234));
    let _ = policy.deliver(&alloc(2, BOOT_HCLIENT, 0xdead_0004, 0x1235));
    let tags: Vec<(FaultTag, usize)> = policy.census().tags().collect();
    assert_eq!(policy.census().total(), 2);
    assert_eq!(tags.len(), 1, "both refusals share one tag: {tags:?}");
}

/// ★★★ **A Device that declares no `deviceId` is REFUSED, never defaulted to GPU 0.**
///
/// This is the arm the first draft of this file tripped over, and it is worth a test of
/// its own rather than a comment: `NV01_DEVICE_0` is the one class in the boot's sequence
/// whose params the object model reads, because `deviceId` is the multi-GPU routing fact
/// and a Device with no declared target is unroutable. `alloc_params`'s
/// `NoDeclaredFacts` arm — which the subdevice and the event both take — would have made
/// this pass by silently routing every guest device to GPU 0.
#[test]
fn a_device_that_declares_no_device_id_is_refused_rather_than_defaulted() {
    let mut policy = ObjectPolicy::new(abi(), GuestOs::Linux, port_gpu());
    let _ = policy.deliver(&root_alloc(1)).expect("root accepted");
    let e = policy
        .deliver(&alloc(2, BOOT_HCLIENT, H_DEVICE, 0x0000_0080))
        .expect_err("a Device with empty params must be refused");
    assert!(
        matches!(e, BridgeRefusal::Abi(_)),
        "expected the params decoder to refuse a truncated NV0080_ALLOC_PARAMETERS, got {e:?}"
    );
    // …and the same alloc WITH its params is accepted, so the refusal is about the
    // declaration and not about the class.
    let _ = policy
        .deliver(&device_alloc(3, BOOT_HCLIENT, H_DEVICE))
        .expect("the same Device, declaring deviceId, is accepted");
}
