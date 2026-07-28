//! ★★ **The GSP → core bridge, stage B1** (`docs/design/gsp_core_bridge.md`): the first
//! time a decoded `RpcCommand` reaches `Gpu::apply` for real, from **wire bytes**.
//!
//! Everything here runs with no GPU, no guest, no hypervisor and no OS — the bridge is a
//! pure function from bytes to events, and the core it feeds is a pure state machine.
//!
//! # The oracle discipline (§5.1), because this seam invites exactly one mistake
//!
//! The trap is *build the bytes with a helper, decode them with the bridge, assert the
//! round trip* — which tests nothing but the helper's agreement with itself. It is the
//! same shape as the GSP oracle bug this rule was written from: `gspworld::Guest::recv`
//! derives the length under test **out of the element under test**, so `encode_message`'s
//! `elem_count` write is unobserved by its own oracle.
//!
//! So there are **three independent transcriptions** of every message here:
//!
//! 1. `kayfabe_tests::rpcwire` — a builder written from `ogkm: g_rpc-structures.h` in a
//!    file that imports **nothing**, with each offset a literal beside its header line;
//! 2. the **hand-written hex arrays** below, offset-annotated, unreadable on purpose;
//! 3. `kayfabe_abi`'s decoders, whose offsets came from the same headers via a different
//!    human, and which `crates/kayfabe-abi/tests/mean_wire.rs` additionally pins against
//!    the **C artifact's** independently-transcribed element-relative offsets.
//!
//! The expected `RmEvent` of each fixture is written out by hand, not derived.
//!
//! # And the property most likely to be lost later (§3.3)
//!
//! [`a_recycled_hclient_is_accepted_and_lands_in_a_different_component`] is the
//! statelessness canary. RM recycles `hClient`/`hObject` values **by design**, so a bridge
//! that grew a handle table, a seen-set or a dedup cache would refuse, dedup or
//! mis-attribute a legal recycle — re-opening the §12.41/§12.42 identity bug class from
//! the one place in the stack that currently cannot have it, because it holds no identity
//! at all.

#![allow(clippy::unusual_byte_groupings)]

use std::collections::BTreeMap;

use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_abi::wire::AbiError;
use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{Boundaries, NO_CONDEMNED, ProjectionError, project};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, ResourceKey, RmEvent, RmGraphError};
use kayfabe_fwd::{FwdFault, handle_doorbell};
use kayfabe_gsp::{RpcCommand, RpcFunction, Transition};
use kayfabe_mocks::{MockArch, MockIsolateFactory, WireClassArch, mock_classes};
use kayfabe_rmrpc::{BridgeRefusal, GraphPolicy, RefusalCensus, Translation, translate};
use kayfabe_tests::Scenario;
use kayfabe_tests::gspworld::{
    FUNCTIONS, GspWorld, GuestMsg, MODEL_A, P580, Profile, REAL_QUEUE_SIZE,
};
use kayfabe_tests::rpcwire::{self as w, RpcScript, fn_id};
use kayfabe_trace::{FaultTag, Faulted};

// =================================================================================
// Harness
// =================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// Turn a whole RPC **message** (envelope + body) into the decoded command the transport
/// would hand us — the same two steps `RpcCommand::from_incoming` performs after the
/// element layer has validated the run: classify `function`, and take the payload as
/// *everything after the 32-byte envelope*.
///
/// # Panics
/// If the envelope is malformed. Every fixture here is a well-formed envelope carrying a
/// possibly-hostile *body*; envelope-level hostility is `kayfabe-abi`'s and
/// `kayfabe-gsp`'s own suite, and duplicating it here would test a different seam.
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

/// `translate` over a whole message.
fn xlate(msg: &[u8]) -> Result<Translation, BridgeRefusal> {
    translate(abi(), &command(msg))
}

/// A `GSP_RM_ALLOC` message declaring a client root, built by the independent builder.
fn root_alloc_msg(class: u32, h_client: u32, process_id: u32) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::client_root_alloc_body(class, h_client, process_id),
    )
}

/// A `FREE` message shaped the way `rpcRmApiFree_GSP` shapes one.
fn free_msg(h_client: u32, h_object: u32) -> Vec<u8> {
    w::message(fn_id::FREE, 2, &w::driver_free_body(h_client, h_object))
}

/// The event a client-root alloc of `(client, pid)` must produce — **written by hand**,
/// from `RmEvent::Alloc`'s own doc plus the §2.2a normalisation rule, never derived.
fn expected_root_event(client: u32, class: u32, kind: ClientKind) -> RmEvent {
    RmEvent::Alloc {
        client: HClient(client),
        // ★ The normalisation: the wire says `hParent = hObject = 0`, and RM's own rule
        // (`rs_server.c:625`, `hResource = hClient`) says the root's handle IS the client.
        parent: HObject(client),
        handle: HObject(client),
        class: ClassId(class),
        facts: AllocFacts {
            client_kind: Some(kind),
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------------------------
// ★ The `Arch` that speaks NVIDIA's real class ids now lives in `kayfabe-mocks`.
//
// It was a test-local shim at B1, with a note that it recurs at B2/B3/B5. It does: this
// file needs it in six more places, so it is promoted rather than copied a third time.
// `kayfabe_mocks::WireClassArch` carries the whole argument for why overriding ONLY
// `classify` is sound, and why the fall-through to `MockArch` is load-bearing — the
// projection-equality oracle below depends on a wire-class graph and a mock-class graph
// classifying identically under one arch.
// ---------------------------------------------------------------------------------

fn fresh_gpu() -> kayfabe_tests::Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    kayfabe_tests::Guarded::new(
        "rmrpc_bridge::fresh_gpu",
        Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes"),
        rec,
    )
}

fn boundaries(gpu: &Gpu) -> Boundaries {
    project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED).expect("projects")
}

/// Translate and apply one message, returning whatever refused first.
fn drive(gpu: &mut Gpu, msg: &[u8]) -> Result<(), BridgeRefusal> {
    match xlate(msg)? {
        Translation::Event(ev) => {
            gpu.apply(ev).expect("the graph accepts this fixture");
            Ok(())
        }
        Translation::Inert => Ok(()),
    }
}

// =================================================================================
// 1. The hand-written hex fixtures — transcription #2
// =================================================================================

/// A complete `GSP_RM_ALLOC` message declaring client `0xc1d0_0069` as a **user** client
/// with `processID = 0xdd13`, written byte by byte.
///
/// ```text
/// ── rpc_message_header_v03_00 (32 B) ──────────────────────────────────────────
/// +0   header_version      00 00 00 03   -> 0x03000000 (MAJOR 3 / MINOR 0)
/// +4   signature           56 52 50 43   -> 0x43505256 = "VRPC" LE
/// +8   length              48 00 00 00   -> 72 = 32 envelope + 40 body
/// +12  function            67 00 00 00   -> 103 = GSP_RM_ALLOC
/// +16  rpc_result          00 00 00 00
/// +20  rpc_result_private  00 00 00 00
/// +24  sequence            01 00 00 00
/// +28  u                   00 00 00 00
/// ── rpc_gsp_rm_alloc_v03_00 (32 B header + 8 B params) ────────────────────────
/// +32  hClient             69 00 d0 c1   -> 0xc1d00069
/// +36  hParent             00 00 00 00   -> NV01_NULL_OBJECT   ★ the guest sends 0
/// +40  hObject             00 00 00 00   -> NV01_NULL_OBJECT   ★ and 0
/// +44  hClass              00 00 00 00   -> NV01_ROOT
/// +48  status              00 00 00 00   [OUT]
/// +52  paramsSize          08 00 00 00   -> 8
/// +56  flags               00 00 00 00   -> RMAPI_RPC_FLAGS_NONE
/// +60  reserved[4]         00 00 00 00
/// ── NV0000_ALLOC_PARAMETERS, 8-byte prefix contract ───────────────────────────
/// +64  hClient             69 00 d0 c1   -> 0xc1d00069  (the same fact, twice)
/// +68  processID           13 dd 00 00   -> 0xdd13      (a real pid => User)
/// ```
const HEX_ROOT_ALLOC: [u8; 72] = [
    0x00, 0x00, 0x00, 0x03, 0x56, 0x52, 0x50, 0x43, 0x48, 0x00, 0x00, 0x00, 0x67, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x69, 0x00, 0xd0, 0xc1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x69, 0x00, 0xd0, 0xc1, 0x13, 0xdd, 0x00, 0x00,
];

/// A complete `FREE` message: client `0xc1d0_0069` frees object `0x5c00_0019`.
///
/// ```text
/// ── rpc_message_header_v03_00 (32 B) ──────────────────────────────────────────
/// +0   header_version      00 00 00 03
/// +4   signature           56 52 50 43   "VRPC"
/// +8   length              30 00 00 00   -> 48 = 32 + 16
/// +12  function            0a 00 00 00   -> 10 = FREE
/// +24  sequence            02 00 00 00
/// ── rpc_free_v03_00 == NVOS00_PARAMETERS_v03_00 (16 B) ────────────────────────
/// +32  hRoot               69 00 d0 c1   -> 0xc1d00069
/// +36  hObjectParent       00 00 00 00   -> NV01_NULL_OBJECT (always, on this path)
/// +40  hObjectOld          19 00 00 5c   -> 0x5c000019
/// +44  status              00 00 00 00   [OUT]
/// ```
const HEX_FREE: [u8; 48] = [
    0x00, 0x00, 0x00, 0x03, 0x56, 0x52, 0x50, 0x43, 0x30, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x69, 0x00, 0xd0, 0xc1, 0x00, 0x00, 0x00, 0x00, 0x19, 0x00, 0x00, 0x5c, 0x00, 0x00, 0x00, 0x00,
];

const HEX_CLIENT: u32 = 0xc1d0_0069;
const HEX_PID: u32 = 0x0000_dd13;
const HEX_OBJECT: u32 = 0x5c00_0019;

/// ★ **Transcription #1 vs #2.** The hand-written hex and the ogkm-derived builder must
/// produce the identical message. Two humans, two methods, one byte string — and if they
/// ever disagree, one of them read the header wrong, which is the entire value of writing
/// the hex out by hand.
#[test]
fn the_hand_written_hex_and_the_independent_builder_agree_byte_for_byte() {
    assert_eq!(
        root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID),
        HEX_ROOT_ALLOC.to_vec(),
        "GSP_RM_ALLOC: the hand hex and the ogkm-derived builder disagree",
    );
    assert_eq!(
        free_msg(HEX_CLIENT, HEX_OBJECT),
        HEX_FREE.to_vec(),
        "FREE: the hand hex and the ogkm-derived builder disagree",
    );
}

// =================================================================================
// 2. The two translations B1 exists for
// =================================================================================

/// The headline: a hand-written hex `GSP_RM_ALLOC` becomes exactly the hand-written
/// `RmEvent::Alloc`, normalisation and all.
#[test]
fn the_hand_hex_client_root_alloc_becomes_the_declared_event() {
    assert_eq!(
        xlate(&HEX_ROOT_ALLOC),
        Ok(Translation::Event(expected_root_event(
            HEX_CLIENT,
            w::NV01_ROOT,
            ClientKind::User { pid: HEX_PID },
        ))),
    );
}

/// And a hand-written hex `FREE` becomes exactly the hand-written `RmEvent::Free`.
#[test]
fn the_hand_hex_free_becomes_the_declared_event() {
    assert_eq!(
        xlate(&HEX_FREE),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_OBJECT),
        })),
    );
}

/// ★ **Non-vacuity, per field.** Changing one field of the fixture must change exactly
/// that field of the event — so the decode is a real read of a real offset and not a
/// constant that happens to match.
#[test]
fn one_changed_field_moves_exactly_one_field_of_the_event() {
    let base = expected_root_event(HEX_CLIENT, w::NV01_ROOT, ClientKind::User { pid: HEX_PID });

    // A different client: the namespace AND both normalised handles move together,
    // because they are one fact.
    let other = root_alloc_msg(w::NV01_ROOT, 0xdead_0001, HEX_PID);
    assert_eq!(
        xlate(&other),
        Ok(Translation::Event(expected_root_event(
            0xdead_0001,
            w::NV01_ROOT,
            ClientKind::User { pid: HEX_PID },
        ))),
    );
    assert_ne!(xlate(&other), Ok(Translation::Event(base)));

    // A different processID: ONLY the client kind moves.
    let kernel = root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, w::KERNEL_PID);
    assert_eq!(
        xlate(&kernel),
        Ok(Translation::Event(expected_root_event(
            HEX_CLIENT,
            w::NV01_ROOT,
            ClientKind::Kernel,
        ))),
    );

    // A different root class: ONLY the class moves — `NV01_ROOT_CLIENT` is the same
    // resource kind and takes the same normalisation branch.
    assert_eq!(
        xlate(&root_alloc_msg(w::NV01_ROOT_CLIENT, HEX_CLIENT, HEX_PID)),
        Ok(Translation::Event(expected_root_event(
            HEX_CLIENT,
            w::NV01_ROOT_CLIENT,
            ClientKind::User { pid: HEX_PID },
        ))),
    );
}

/// ★ The `hParent`/`hObject` a client root carries on the wire are **ignored**, not
/// trusted — and the normalisation is what makes that safe. A hostile guest that fills
/// them in with something else gets the same event.
///
/// This is the arm that would fail if the normalisation were written as "if the fields
/// are zero, substitute the client" rather than "for this class, the handle IS the
/// client".
#[test]
fn a_client_roots_wire_parent_and_handle_are_ignored_whatever_they_say() {
    let mut body = w::alloc_body(
        HEX_CLIENT,
        0xbaad_f00d, // hParent — a lie
        0x1234_5678, // hObject — another lie
        w::NV01_ROOT,
        8,
        w::RMAPI_RPC_FLAGS_NONE,
        &w::client_root_params(HEX_CLIENT, HEX_PID),
    );
    let msg = w::message(fn_id::GSP_RM_ALLOC, 1, &body);
    assert_eq!(
        xlate(&msg),
        Ok(Translation::Event(expected_root_event(
            HEX_CLIENT,
            w::NV01_ROOT,
            ClientKind::User { pid: HEX_PID },
        ))),
        "the class decides the shape of a root alloc; the guest's parent/handle do not",
    );

    // ★★ Non-vacuity for the ignoring itself, and the B3 half of it: under a MAPPED
    // non-root class the very same two words are carried through **verbatim**. Before B3
    // there was no such class and this arm could only show a refusal, which proves the
    // normalisation did not fire but not that the alternative is the wire's own values.
    let dev = w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            HEX_CLIENT,
            0xbaad_f00d,
            0x1234_5678,
            w::NV01_DEVICE_0,
            56,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::device_params(0, 0, 0),
        ),
    );
    assert_eq!(
        xlate(&dev),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(HEX_CLIENT),
            parent: HObject(0xbaad_f00d),
            handle: HObject(0x1234_5678),
            class: ClassId(w::NV01_DEVICE_0),
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        })),
        "★ the normalisation is the client root's alone — every other class's edge is \
         whatever the wire said, unexamined",
    );

    // And a class with no entry in the table is still refused rather than defaulted.
    body[12..16].copy_from_slice(&w::NV01_MEMORY_SYSTEM.to_le_bytes());
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &body)),
        Err(BridgeRefusal::UnmappedAllocClass {
            class: w::NV01_MEMORY_SYSTEM
        }),
    );
}

/// ★ **The C's anti-oracle.** `nvkvm_gpu_emul.c:1796` derives "is this a client-root
/// free?" from `fClient == fObj`; `HandleRef`'s doc is a written warning against exactly
/// that equality. The bridge must emit a **flat** `Free` either way — the graph already
/// recorded the declaration and is the only thing entitled to answer that question.
#[test]
fn a_free_is_flat_whether_or_not_the_handle_equals_the_client() {
    // The shape the C would call a "root free".
    assert_eq!(
        xlate(&free_msg(HEX_CLIENT, HEX_CLIENT)),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_CLIENT),
        })),
    );
    // The shape it would call an object free. Same variant, same shape, no branch.
    assert_eq!(
        xlate(&free_msg(HEX_CLIENT, HEX_OBJECT)),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_OBJECT),
        })),
    );
    // And `hObjectParent` is discarded rather than smuggled in: a guest that fills it
    // produces the identical event.
    let msg = w::message(
        fn_id::FREE,
        2,
        &w::free_body(HEX_CLIENT, 0xffff_ffff, HEX_OBJECT),
    );
    assert_eq!(
        xlate(&msg),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_OBJECT),
        })),
    );
}

// =================================================================================
// 3. The refusal surface — every arm, by variant
// =================================================================================

/// Every function id, and the **exact** answer it earns. Three states, never two:
/// translated, known-and-inert, refused — and the refusals are themselves three
/// different variants for three different reasons.
#[test]
fn every_function_id_lands_on_its_own_arm() {
    // Known and inert: no object-model content this vocabulary could express.
    for (code, what) in [
        (fn_id::SET_GUEST_SYSTEM_INFO, "guest system description"),
        (
            fn_id::GET_GSP_STATIC_INFO,
            "a query; its REPLY is the data model's job",
        ),
        (
            fn_id::UNLOADING_GUEST_DRIVER,
            "the FSM owns it; not a graph teardown",
        ),
        (fn_id::GSP_SET_SYSTEM_INFO, "init, no reply"),
        (fn_id::SET_REGISTRY, "init, no reply"),
    ] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Ok(Translation::Inert),
            "fn {code} is inert ({what})",
        );
    }

    // Known, mapped by the design, arm not built — B4/B5/B6. NOT inert: the fact matters.
    for code in [
        fn_id::GSP_RM_CONTROL,
        fn_id::DUP_OBJECT,
        fn_id::CONTINUATION_RECORD,
    ] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 48])),
            Err(BridgeRefusal::NotYetTranslated { code }),
        );
    }

    // Ours to send, never to receive.
    for code in [fn_id::GSP_INIT_DONE, fn_id::POST_EVENT] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Err(BridgeRefusal::EventFromGuest { code }),
        );
    }

    // Not in the table at all — the third state.
    for code in [0u32, 2, 4, 14, 15, 27, 64, 70, 999, 0x1002, u32::MAX] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Err(BridgeRefusal::UnknownFunction { code }),
            "fn {code} is not a function this port names",
        );
    }
}

/// ★ fn 14 / fn 15 deserve their own assertion, because the temptation to "just add them"
/// is real and the answer is that they **cannot arrive**: `rpcMapMemoryDma` and
/// `rpcUnmapMemoryDma` are HAL **stubs** on every GSP-client part, the C artifact's
/// complete snoop set never contained them, and three C-era design docs say so flatly.
/// `RmEvent::MapMemoryDma` therefore has no producer on this path, ever.
#[test]
fn map_memory_dma_is_an_unknown_function_here_and_that_is_deliberate() {
    for code in [14u32, 15] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 64])),
            Err(BridgeRefusal::UnknownFunction { code }),
        );
    }
}

/// `hClient == 0` is refused on **both** verbs, before anything else about the message is
/// believed. `NV01_NULL_OBJECT` is not a namespace.
#[test]
fn a_zero_hclient_is_refused_on_both_verbs() {
    assert_eq!(
        xlate(&root_alloc_msg(w::NV01_ROOT, 0, HEX_PID)),
        Err(BridgeRefusal::ReservedClient),
    );
    assert_eq!(
        xlate(&free_msg(0, HEX_OBJECT)),
        Err(BridgeRefusal::ReservedClient),
    );
    // Non-vacuity: 1 is fine. The refusal is about zero, not about small handles.
    assert!(matches!(
        xlate(&free_msg(1, HEX_OBJECT)),
        Ok(Translation::Event(_)),
    ));
}

/// The two declarations of a client root's own handle must agree. RM stamps the params
/// copy itself, so a disagreement means **we** mis-decoded — never that the guest meant
/// something clever, and never a pick between them.
#[test]
fn the_params_hclient_must_agree_with_the_rpc_headers_hclient() {
    let body = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        8,
        w::RMAPI_RPC_FLAGS_NONE,
        &w::client_root_params(0xdead_beef, HEX_PID),
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &body)),
        Err(BridgeRefusal::ClientHandleDisagrees {
            header: HEX_CLIENT,
            params: 0xdead_beef,
        }),
    );
}

/// ★ The serialization refusal fires on the **declared bit**, not on a length heuristic:
/// the message below is otherwise perfect, and it is still refused.
#[test]
fn a_serialized_alloc_is_refused_by_name_even_though_it_is_otherwise_wellformed() {
    let params = w::client_root_params(HEX_CLIENT, HEX_PID);
    let body = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        params.len() as u32,
        w::RMAPI_RPC_FLAGS_SERIALIZED,
        &params,
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &body)),
        Err(BridgeRefusal::SerializedParams {
            class: w::NV01_ROOT
        }),
    );

    // The bit is bit 1 and nothing else: COPYOUT_ON_ERROR (bit 0) is not it.
    let ok = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        params.len() as u32,
        1,
        &params,
    );
    assert!(matches!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &ok)),
        Ok(Translation::Event(_)),
    ));
    // …and it is found under noise.
    let noisy = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        params.len() as u32,
        0xffff_ffff,
        &params,
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &noisy)),
        Err(BridgeRefusal::SerializedParams {
            class: w::NV01_ROOT
        }),
    );
}

/// `paramsSize` is the guest's **assertion about its own message**, so every impossible
/// one is refused with both numbers — never clamped to the smaller, which is how a
/// truncated struct gets zero-extended into a plausible-looking one.
#[test]
fn a_hostile_params_size_is_refused_with_both_numbers() {
    let params = w::client_root_params(HEX_CLIENT, HEX_PID);
    for declared in [9u32, 16, 120, 4096, 0x8000_0000, u32::MAX] {
        let body = w::alloc_body(
            HEX_CLIENT,
            0,
            0,
            w::NV01_ROOT,
            declared,
            w::RMAPI_RPC_FLAGS_NONE,
            &params,
        );
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &body)),
            Err(BridgeRefusal::ParamsSizeExceedsPayload {
                declared,
                available: 8,
            }),
            "declared {declared} against 8 bytes of params",
        );
    }

    // Exactly what arrived is fine (the boundary).
    let exact = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        8,
        w::RMAPI_RPC_FLAGS_NONE,
        &params,
    );
    assert!(matches!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &exact)),
        Ok(Translation::Event(_)),
    ));

    // ★ And the DECLARED window is what gets decoded, not the whole tail: a guest that
    // sends 120 bytes of params but declares 4 has declared too few for the prefix
    // contract, and is refused — reading the extra bytes anyway would be trusting bytes
    // the guest did not vouch for.
    let under = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        4,
        w::RMAPI_RPC_FLAGS_NONE,
        &w::client_root_params_sized(HEX_CLIENT, HEX_PID, 120),
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &under)),
        Err(BridgeRefusal::Abi(AbiError::Truncated {
            c_name: "NV0000_ALLOC_PARAMETERS",
            need: 8,
            got: 4,
        })),
    );

    // A full-size params block decodes from its prefix and ignores the rest.
    let full = w::alloc_body(
        HEX_CLIENT,
        0,
        0,
        w::NV01_ROOT,
        120,
        w::RMAPI_RPC_FLAGS_NONE,
        &w::client_root_params_sized(HEX_CLIENT, HEX_PID, 120),
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &full)),
        Ok(Translation::Event(expected_root_event(
            HEX_CLIENT,
            w::NV01_ROOT,
            ClientKind::User { pid: HEX_PID },
        ))),
    );
}

/// A body shorter than its fixed header is refused at **every** length, naming the struct
/// and both numbers — never zero-extended into a plausible alloc or free.
#[test]
fn a_truncated_body_is_refused_at_every_length_on_both_verbs() {
    let alloc = w::client_root_alloc_body(w::NV01_ROOT, HEX_CLIENT, HEX_PID);
    for n in 0..32usize {
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &alloc[..n])),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "rpc_gsp_rm_alloc_v03_00",
                need: 32,
                got: n,
            })),
            "alloc body of {n} bytes",
        );
    }
    let free = w::driver_free_body(HEX_CLIENT, HEX_OBJECT);
    for n in 0..16usize {
        assert_eq!(
            xlate(&w::message(fn_id::FREE, 2, &free[..n])),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "NVOS00_PARAMETERS",
                need: 16,
                got: n,
            })),
            "free body of {n} bytes",
        );
    }
    // ★ The 32-byte boundary is the sharp one, and it has TWO distinct answers.
    //
    // (a) The header survives the truncation but still declares `paramsSize = 8`, so what
    //     is caught is the guest's own claim outrunning its message — the bounds check,
    //     not a struct-length check. A decoder that clamped `paramsSize` to what arrived
    //     would sail past this and then decode 8 bytes of nothing.
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &alloc[..32])),
        Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 8,
            available: 0,
        }),
    );
    // (b) A header that HONESTLY declares no params at all. The bounds check passes (0
    //     of 0), and the refusal comes from the params decode instead: a client root that
    //     declares no `processID` cannot be classified, and `UndeclaredClientKind` is a
    //     hard refusal in the core by design — so it must never reach the core as a
    //     `None`.
    let empty = w::alloc_body(HEX_CLIENT, 0, 0, w::NV01_ROOT, 0, 0, &[]);
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &empty)),
        Err(BridgeRefusal::Abi(AbiError::Truncated {
            c_name: "NV0000_ALLOC_PARAMETERS",
            need: 8,
            got: 0,
        })),
    );
}

/// ★★ §3.4 — the bridge **must not pre-empt** the graph's own MISS/DEFER/FAULT taxonomy,
/// and **must not swallow** its answer either.
///
/// A `FREE` naming an object that was never declared is a perfectly well-formed message:
/// the bridge translates it without complaint, because "does this object exist?" is a
/// *lookup*, and the bridge performs none. The graph then refuses it by name. Both halves
/// are the point — a bridge that checked first would be a second, weaker copy of a rule
/// that already has an owner, and one that dropped the error would be the C's `NV_OK`.
#[test]
fn a_free_of_an_undeclared_object_translates_cleanly_and_the_graph_refuses_it() {
    let msg = free_msg(HEX_CLIENT, HEX_OBJECT);
    assert_eq!(
        xlate(&msg),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_OBJECT),
        })),
        "the bridge resolves nothing and refuses nothing here",
    );

    let mut gpu = fresh_gpu();
    drive(&mut gpu, &root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID)).expect("root");
    let Translation::Event(ev) = xlate(&msg).expect("translates") else {
        panic!("a FREE is not inert");
    };
    assert_eq!(
        gpu.apply(ev),
        Err(kayfabe_core::gpu::GpuError::Graph(
            kayfabe_core::rmgraph::RmGraphError::FreeUnknown(kayfabe_core::rmgraph::NodeKey::new(
                HClient(HEX_CLIENT),
                HObject(HEX_OBJECT)
            ))
        )),
        "the graph's refusal is the answer, and it is named",
    );

    // ★ B2's obligation, recorded here rather than in prose: that error has no
    // `BridgeRefusal` home yet, because `translate` never applies. The adapter that does
    // (`GraphPolicy`) is what turns it into a non-zero `rpc_result` — and until it exists,
    // nothing in this crate may quietly absorb it.
}

/// ★ **The refusal ORDER is deliberate**, and a message that trips several checks at once
/// is the only way to pin it. Peeled one at a time, worst-first:
///
/// 1. the namespace, because it is the one fact the bridge is responsible for;
/// 2. the params ENCODING, because if it is serialized every offset below is meaningless;
/// 3. the params BOUNDS, because you cannot read a window you do not have;
/// 4. the class, because that is what selects the params decoder;
/// 5. the params content.
#[test]
fn the_refusal_order_is_namespace_then_encoding_then_bounds_then_class() {
    let params = w::client_root_params(0xdead_beef, HEX_PID); // ALSO disagrees (check 5)
    let all_wrong = |client: u32, flags: u32, size: u32, class: u32| {
        w::message(
            fn_id::GSP_RM_ALLOC,
            1,
            &w::alloc_body(client, 0, 0, class, size, flags, &params),
        )
    };

    // Everything wrong at once -> the namespace.
    assert_eq!(
        xlate(&all_wrong(
            0,
            w::RMAPI_RPC_FLAGS_SERIALIZED,
            999,
            w::NV01_MEMORY_SYSTEM
        )),
        Err(BridgeRefusal::ReservedClient),
    );
    // Namespace fixed -> the encoding.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_SERIALIZED,
            999,
            w::NV01_MEMORY_SYSTEM
        )),
        Err(BridgeRefusal::SerializedParams {
            class: w::NV01_MEMORY_SYSTEM
        }),
    );
    // Encoding fixed -> the bounds.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_NONE,
            999,
            w::NV01_MEMORY_SYSTEM
        )),
        Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 999,
            available: 8,
        }),
    );
    // Bounds fixed -> the class.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_NONE,
            8,
            w::NV01_MEMORY_SYSTEM
        )),
        Err(BridgeRefusal::UnmappedAllocClass {
            class: w::NV01_MEMORY_SYSTEM
        }),
    );
    // Class fixed -> the content.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_NONE,
            8,
            w::NV01_ROOT
        )),
        Err(BridgeRefusal::ClientHandleDisagrees {
            header: HEX_CLIENT,
            params: 0xdead_beef,
        }),
    );
}

/// Every refusal is **countable** (a typed tag, so an invariant can be a bound rather than
/// an absence) and **answerable** (a non-zero `rpc_result`, because the guest is blocked
/// in `_issueRpcAndWait` and a drop hangs it for the whole RPC timeout).
///
/// The tags are asserted to be distinct, which is what makes a per-variant census — the
/// composed run below — mean anything.
#[test]
fn every_refusal_carries_a_distinct_tag_and_a_nonzero_rpc_result() {
    let all = [
        BridgeRefusal::UnknownFunction { code: 7 },
        BridgeRefusal::NotYetTranslated { code: 76 },
        BridgeRefusal::EventFromGuest { code: 0x1001 },
        BridgeRefusal::UnmappedAllocClass { class: 0x80 },
        BridgeRefusal::SerializedParams { class: 0 },
        BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 9,
            available: 8,
        },
        BridgeRefusal::ClientHandleDisagrees {
            header: 1,
            params: 2,
        },
        BridgeRefusal::ReservedClient,
        BridgeRefusal::Abi(AbiError::Truncated {
            c_name: "NVOS00_PARAMETERS",
            need: 16,
            got: 0,
        }),
        // ★ B2's arm, and it appears TWICE with different inner errors on purpose: the
        // `Faulted` impl **delegates**, so a graph refusal is countable by the protocol
        // rule it broke rather than by one flat "the graph said no". If it were
        // flattened, these two would collide and the assertion below would fail — which
        // is the whole reason they are both here.
        BridgeRefusal::Graph(GpuError::Graph(RmGraphError::FreeUnknown(NodeKey::new(
            HClient(HEX_CLIENT),
            HObject(HEX_OBJECT),
        )))),
        BridgeRefusal::Graph(GpuError::Graph(RmGraphError::ConflictingAlloc(
            NodeKey::new(HClient(HEX_CLIENT), HObject(HEX_CLIENT)),
        ))),
    ];
    let tags: std::collections::BTreeSet<FaultTag> = all.iter().map(Faulted::fault_tag).collect();
    assert_eq!(
        tags.len(),
        all.len(),
        "two refusals share a tag, so a census cannot tell them apart",
    );
    for r in all {
        assert_ne!(
            r.rpc_result(),
            0,
            "{:?} must not be answered NV_OK — that is the C's echo",
            r.fault_tag(),
        );
        // B1 sends ONE value and names it (§4.2's `[open]`; B4 revisits).
        assert_eq!(r.rpc_result(), 0x56, "NV_ERR_NOT_SUPPORTED");
    }
}

// =================================================================================
// 4. Statelessness — the property most likely to be silently lost
// =================================================================================

/// `translate` is a **pure function of one message**: the same bytes always translate to
/// the same event, however many times they arrive.
///
/// This is not pedantry. A *replayed* RPC must map to the identical event, because that is
/// what makes `RmGraph`'s idempotent-retry tolerance reachable at all — and a bridge with
/// a dedup cache would answer differently the second time and break it.
#[test]
fn the_same_message_always_translates_to_the_same_event() {
    let want = xlate(&HEX_ROOT_ALLOC);
    for _ in 0..8 {
        assert_eq!(xlate(&HEX_ROOT_ALLOC), want);
        assert_eq!(
            xlate(&free_msg(HEX_CLIENT, HEX_OBJECT)),
            Ok(Translation::Event(RmEvent::Free {
                client: HClient(HEX_CLIENT),
                handle: HObject(HEX_OBJECT),
            })),
        );
    }
    // Interleaving other messages — including refusals — changes nothing.
    let _ = xlate(&root_alloc_msg(w::NV01_ROOT, 0, HEX_PID));
    let _ = xlate(&w::message(999, 1, &[0u8; 8]));
    assert_eq!(xlate(&HEX_ROOT_ALLOC), want);
}

/// ★★ **The recycle canary.** Allocate a client root, free it, allocate the **same
/// `hClient`** again — and the second declaration must be accepted and land in a
/// different component than the first.
///
/// RM recycles `hClient` values by design (no free list, no quarantine, caller-supplied
/// handles honoured verbatim), so refusing or deduplicating a recycle hangs a **legal**
/// guest. The bridge's protection against ever doing so is that it holds no identity at
/// all; this test is what fails if that ever stops being true.
///
/// The two declarations are deliberately given different **privileges** — a user process
/// exits, and the guest kernel later reuses the same handle value — because that makes
/// "distinct" observable in the projection: one lands in a user `Proc`, the other in the
/// system component. A bridge that remembered the first would either refuse the second or
/// hand back the first's classification.
#[test]
fn a_recycled_hclient_is_accepted_and_lands_in_a_different_component() {
    let mut gpu = fresh_gpu();

    // (1) A user process declares the namespace.
    drive(&mut gpu, &root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID)).expect("first root");
    let b = boundaries(&gpu);
    assert_eq!(b.procs.len(), 1, "one user process");
    assert!(b.procs[0].client_values().contains(&HClient(HEX_CLIENT)));
    assert!(b.system.clients.is_empty(), "no kernel client yet");

    // (2) It exits: RM frees the client root, and the value becomes recyclable.
    drive(&mut gpu, &free_msg(HEX_CLIENT, HEX_CLIENT)).expect("root free");
    let b = boundaries(&gpu);
    assert!(b.procs.is_empty(), "the user process is gone");
    assert!(b.system.clients.is_empty());

    // (3) The SAME handle value is declared again — this time by a kernel client.
    drive(
        &mut gpu,
        &root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, w::KERNEL_PID),
    )
    .expect("★ a recycled hClient must be accepted — RM recycles by design");
    let b = boundaries(&gpu);
    assert!(
        b.procs.is_empty(),
        "the recycled declaration is a KERNEL client: no user proc",
    );
    assert!(
        b.system.client_values().contains(&HClient(HEX_CLIENT)),
        "…and it is a distinct declaration, in the system component",
    );

    // (4) ★ No residue: the graph is now byte-for-byte what a FRESH device driven by the
    // second declaration alone would be. A bridge (or a graph) that remembered the first
    // incarnation would differ here, and `Boundaries` compares whole.
    let mut clean = fresh_gpu();
    drive(
        &mut clean,
        &root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, w::KERNEL_PID),
    )
    .expect("clean root");
    assert_eq!(
        boundaries(&gpu),
        boundaries(&clean),
        "alloc→free→alloc must leave exactly what a single alloc leaves",
    );
}

/// An **identical re-send** of a client root is accepted idempotently by the graph — the
/// retried-RPC tolerance. Reachable only because the bridge is pure: it maps the replay to
/// the identical event.
#[test]
fn an_identical_resend_of_a_client_root_is_accepted_idempotently() {
    let mut gpu = fresh_gpu();
    let msg = root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID);
    drive(&mut gpu, &msg).expect("first");
    let after_one = boundaries(&gpu);
    for _ in 0..3 {
        drive(&mut gpu, &msg).expect("★ a retried RPC is not a protocol violation");
    }
    assert_eq!(
        boundaries(&gpu),
        after_one,
        "a replayed alloc changes nothing",
    );
}

// =================================================================================
// 5. The composed run — malformed traffic interleaved with a live stream
// =================================================================================

/// ★ **Mean, not happy path.** A guest's real client stream with hostile and malformed
/// messages interleaved between its valid ones, driven end to end into a real `Gpu`.
///
/// Two assertions, and the first is the load-bearing one:
///
/// 1. **the valid stream is unaffected** — the final projection is *identical* to the one
///    a device driven by the valid subset alone reaches. Refusals are inert to the graph,
///    which is the whole claim of "a refusal is a named answer, not a partial apply";
/// 2. **every refusal is counted, by variant** — a census, not a total, because a total
///    is satisfied by any nine refusals and would not notice a message refusing for the
///    wrong reason.
#[test]
fn malformed_traffic_between_valid_messages_leaves_the_valid_stream_untouched() {
    const A: u32 = 0xc1d0_0069; // a user process
    const B: u32 = 0xc1d0_006a; // a second user process, adjacent handle value
    const K: u32 = 0xdead_c0de; // the guest kernel's own client

    let valid: Vec<Vec<u8>> = vec![
        root_alloc_msg(w::NV01_ROOT, A, 0xdd13),
        root_alloc_msg(w::NV01_ROOT_CLIENT, B, 0xdd14),
        root_alloc_msg(w::NV01_ROOT, K, w::KERNEL_PID),
        // Two processes exit, in the order the guest's RM would emit.
        free_msg(A, A),
        free_msg(B, B),
        w::message(fn_id::SET_REGISTRY, 9, &[0xab; 32]), // inert
    ];

    let hostile: Vec<(Vec<u8>, FaultTag)> = vec![
        (
            w::message(999, 1, &[0u8; 8]),
            FaultTag("BridgeRefusal::UnknownFunction"),
        ),
        (
            w::message(fn_id::GSP_RM_CONTROL, 1, &[0u8; 48]),
            FaultTag("BridgeRefusal::NotYetTranslated"),
        ),
        (
            w::message(fn_id::GSP_INIT_DONE, 1, &[0u8; 8]),
            FaultTag("BridgeRefusal::EventFromGuest"),
        ),
        (
            root_alloc_msg(w::NV01_ROOT, 0, 1),
            FaultTag("BridgeRefusal::ReservedClient"),
        ),
        (
            w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(
                    A,
                    0,
                    0,
                    w::NV01_ROOT,
                    8,
                    w::RMAPI_RPC_FLAGS_SERIALIZED,
                    &w::client_root_params(A, 1),
                ),
            ),
            FaultTag("BridgeRefusal::SerializedParams"),
        ),
        (
            w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(A, 0, 0, w::NV01_ROOT, 4096, 0, &w::client_root_params(A, 1)),
            ),
            FaultTag("BridgeRefusal::ParamsSizeExceedsPayload"),
        ),
        (
            w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(
                    A,
                    0,
                    0,
                    w::NV01_MEMORY_SYSTEM,
                    8,
                    0,
                    &w::client_root_params(A, 1),
                ),
            ),
            FaultTag("BridgeRefusal::UnmappedAllocClass"),
        ),
        (
            w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(A, 0, 0, w::NV01_ROOT, 8, 0, &w::client_root_params(B, 1)),
            ),
            FaultTag("BridgeRefusal::ClientHandleDisagrees"),
        ),
        (
            w::message(fn_id::FREE, 1, &[0u8; 3]),
            FaultTag("BridgeRefusal::Abi"),
        ),
    ];

    // The interleaved run: one hostile message between every pair of valid ones, and the
    // leftovers appended, so no valid message is adjacent to only clean traffic.
    let mut gpu = fresh_gpu();
    let mut census: BTreeMap<FaultTag, usize> = BTreeMap::new();
    let mut answered = 0usize;
    let mut hostile_iter = hostile.iter();
    let mut run = |gpu: &mut Gpu, msg: &[u8], expect: Option<FaultTag>| {
        let got = xlate(msg);
        match (got, expect) {
            (Ok(t), None) => {
                if let Translation::Event(ev) = t {
                    gpu.apply(ev).expect("a valid message applies");
                }
            }
            (Err(r), Some(tag)) => {
                assert_eq!(r.fault_tag(), tag, "refused for the wrong reason");
                assert_ne!(r.rpc_result(), 0, "a refusal still owes the guest a reply");
                *census.entry(r.fault_tag()).or_default() += 1;
                answered += 1;
            }
            (other, expect) => panic!("expected {expect:?}, got {other:?}"),
        }
    };
    for v in &valid {
        run(&mut gpu, v, None);
        if let Some((h, tag)) = hostile_iter.next() {
            run(&mut gpu, h, Some(*tag));
        }
    }
    for (h, tag) in hostile_iter {
        run(&mut gpu, h, Some(*tag));
    }

    // (2) The census: every hostile message refused, each for its own declared reason.
    assert_eq!(
        answered,
        hostile.len(),
        "every hostile message was answered"
    );
    let want: BTreeMap<FaultTag, usize> =
        hostile.iter().fold(BTreeMap::new(), |mut m, (_, tag)| {
            *m.entry(*tag).or_default() += 1;
            m
        });
    assert_eq!(census, want, "refusal census by variant");

    // (1) The valid stream is untouched: a device that saw ONLY the valid messages ends
    // in the identical projection.
    let mut clean = fresh_gpu();
    for v in &valid {
        drive(&mut clean, v).expect("the valid subset is valid");
    }
    assert_eq!(
        boundaries(&gpu),
        boundaries(&clean),
        "★ hostile traffic must be inert to the graph, not partially applied",
    );

    // Non-vacuity for that comparison: it is not trivially true of any two devices.
    let mut different = fresh_gpu();
    for v in valid.iter().take(2) {
        drive(&mut different, v).expect("prefix");
    }
    assert_ne!(
        boundaries(&clean),
        boundaries(&different),
        "the projection comparison would notice a missing valid message",
    );
}

/// The `RpcFunction` classification the bridge dispatches on is the transport's, not one
/// this file invented: the two ids B1 needed had to be **added** to `FunctionCodes`, and
/// this pins that they classify as themselves rather than falling through to `Other`.
#[test]
fn free_and_dup_object_classify_as_themselves_not_as_unknown() {
    assert_eq!(FUNCTIONS.classify(fn_id::FREE), RpcFunction::Free);
    assert_eq!(
        FUNCTIONS.classify(fn_id::DUP_OBJECT),
        RpcFunction::DupObject
    );
    assert_eq!(FUNCTIONS.classify(103), RpcFunction::RmAlloc);
    // And the table is still internally consistent after the two additions.
    assert!(FUNCTIONS.validated().is_ok());
}

// =================================================================================
// ★★ 6. Stage B2 — `GraphPolicy`: the bridge on the guest's OWN path
// =================================================================================
//
// B1 joined the two ends **in a test body**: a `#[test]` called `translate`, matched the
// `Translation` and called `Gpu::apply` itself. That proves the types compose. It does
// not put the bridge where a guest can reach it.
//
// B2 is `kayfabe_rmrpc::GraphPolicy`, the `CommandPolicy` the boot FSM calls from inside
// `service_command_queue` for every command a guest posts. So from here down the graph is
// driven by the **transport** — a real msgq ring, a real element run, a real envelope —
// and the tests below are allowed to say "a guest did this", which nothing before them
// was.

/// The shared fixture: three namespaces and one exit, expressed **twice**.
///
/// Two user processes and the guest kernel declare client roots; process A then exits.
/// Small enough to write out by hand on both sides, which is the entire requirement.
mod x {
    /// Process A's client handle.
    pub const A: u32 = 0xc1d0_0069;
    /// Process B's client handle — adjacent to A's, so a bridge that confused them would
    /// still produce a plausible-looking graph.
    pub const B: u32 = 0xc1d0_006a;
    /// The guest kernel's own client (UVM's session shape).
    pub const K: u32 = 0xdead_c0de;
    /// Process A's pid. Deliberately unequal to its client handle: the two are different
    /// facts, and a decoder that read one for the other would pass if they matched.
    pub const PID_A: u32 = 0x0000_dd13;
    /// Process B's pid.
    pub const PID_B: u32 = 0x0000_dd14;
}

/// **Transcription #1** of the fixture: bytes, from `ogkm`'s struct definitions.
///
/// ★ Note the two roots use **different classes** — `NV01_ROOT` and `NV01_ROOT_CLIENT`.
/// RM treats them as one resource kind, so the projection must not be able to tell, and
/// making them differ here is what tests that rather than assuming it.
fn script_x() -> RpcScript {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, x::A, x::PID_A)
        .client_root(w::NV01_ROOT_CLIENT, x::B, x::PID_B)
        .client_root(w::NV01_ROOT, x::K, w::KERNEL_PID)
        // Process A exits: RM frees its client root, which drops every handle in the
        // namespace.
        .free(x::A, x::A);
    s
}

/// **Transcription #2** of the same fixture: `RmEvent`s, written out by hand.
///
/// ★★ This is the oracle's independent half (`gsp_core_bridge.md` §5.1), and its
/// independence is deliberate in three ways:
///
/// - the class ids are `mock_classes::CLIENT`, **not** NVIDIA's. `Boundaries` carries no
///   class id, so if the two agree it is because both classified to `ObjectKind::Client`,
///   not because the same number travelled down two paths;
/// - the client kind is written as the `ClientKind` the core groups on, never as a
///   `processID` — so `client_kind_from_process_id`'s sentinel rule is exercised on the
///   byte side and asserted on the domain side;
/// - the normalisation (`parent == handle == HObject(client)`) is written out, so the
///   wire's `hParent = hObject = 0` has to be recovered rather than passed through.
fn scenario_x() -> Scenario {
    let root = |client: u32, kind: ClientKind| RmEvent::Alloc {
        client: HClient(client),
        parent: HObject(client),
        handle: HObject(client),
        class: mock_classes::CLIENT,
        facts: AllocFacts {
            client_kind: Some(kind),
            ..Default::default()
        },
    };
    let mut s = Scenario::new();
    s.push(root(x::A, ClientKind::User { pid: x::PID_A }))
        .push(root(x::B, ClientKind::User { pid: x::PID_B }))
        .push(root(x::K, ClientKind::Kernel))
        .push(RmEvent::Free {
            client: HClient(x::A),
            handle: HObject(x::A),
        });
    s
}

/// Apply a `Scenario`'s events to a fresh device — the reference side of the oracle.
fn boundaries_of_scenario(s: &Scenario) -> Boundaries {
    let mut gpu = fresh_gpu();
    for ev in &s.events {
        gpu.apply(*ev).expect("the reference scenario is legal");
    }
    boundaries(&gpu)
}

/// Drive whole RPC **messages** through the policy, with no ring and no FSM — the direct
/// form, for the tests whose subject is the policy rather than the transport.
fn deliver_all(
    policy: &mut GraphPolicy<'_>,
    msgs: &[Vec<u8>],
) -> Vec<Result<Translation, BridgeRefusal>> {
    msgs.iter().map(|m| policy.deliver(&command(m))).collect()
}

// ---------------------------------------------------------------------------------
// 6.1 The policy itself
// ---------------------------------------------------------------------------------

/// The headline: the policy translates **and applies**, and its counters say which of
/// the three outcomes each command took.
///
/// ★ The projection is checked **between** the messages, not only at the end. An
/// end-state-only assertion here is satisfiable by "nothing ever applied" — the graph is
/// empty before the alloc and empty again after the free — which a mutation that turned
/// `deliver` into a pure `translate` sailed straight through. Alloc-then-free is exactly
/// the shape where the end state proves nothing, so the intermediate one is the assertion
/// that carries the claim.
#[test]
fn the_policy_translates_and_applies_and_counts_what_it_did() {
    let mut gpu = fresh_gpu();
    let (census, applied, inert, out, midway) = {
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        let mut out = vec![
            policy.deliver(&command(&HEX_ROOT_ALLOC)),
            policy.deliver(&command(&w::message(
                fn_id::SET_GUEST_SYSTEM_INFO,
                7,
                &[0xab; 16],
            ))),
        ];
        // ★ The namespace exists RIGHT NOW, before the free undoes it.
        let midway = boundaries(policy.gpu());
        out.push(policy.deliver(&command(&free_msg(HEX_CLIENT, HEX_CLIENT))));
        (
            policy.census().clone(),
            policy.applied(),
            policy.inert(),
            out,
            midway,
        )
    };

    assert_eq!(
        out,
        vec![
            Ok(Translation::Event(expected_root_event(
                HEX_CLIENT,
                w::NV01_ROOT,
                ClientKind::User { pid: HEX_PID },
            ))),
            Ok(Translation::Inert),
            Ok(Translation::Event(RmEvent::Free {
                client: HClient(HEX_CLIENT),
                handle: HObject(HEX_CLIENT),
            })),
        ],
    );
    // ★ Two counters, not one total: a regression that turned every alloc inert would
    // leave a single total unchanged.
    assert_eq!(applied, 2, "the alloc and the free declared facts");
    assert_eq!(inert, 1, "the system-info RPC carried none");
    assert_eq!(census, RefusalCensus::default(), "nothing was refused");

    // The graph really moved — one user process existed, and then did not.
    assert_eq!(
        midway.procs.len(),
        1,
        "★ the alloc APPLIED, it did not merely translate"
    );
    assert_eq!(
        midway.procs[0].client_values(),
        [HClient(HEX_CLIENT)].into_iter().collect(),
    );
    assert_eq!(
        boundaries(&gpu),
        Boundaries::default(),
        "and the free applied too"
    );
}

/// The policy's answer on the wire: `None` for anything it accepted (the FSM's own
/// `ack(NV_OK)`), and `Some(Reply)` with a non-zero status for anything it refused.
#[test]
fn an_accepted_command_is_acked_by_the_fsm_and_a_refused_one_is_answered_here() {
    use kayfabe_gsp::{CommandPolicy, Reply};

    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), &mut gpu);

    assert_eq!(
        policy.respond(&command(&HEX_ROOT_ALLOC)),
        None,
        "an accepted fact needs no reply BODY — the FSM acks (function, sequence)",
    );
    assert_eq!(
        policy.respond(&command(&w::message(999, 1, &[0u8; 8]))),
        Some(Reply {
            rpc_result: 0x56,
            body: Vec::new(),
        }),
        "★ a refusal is answered, never dropped — the guest blocks in _issueRpcAndWait",
    );
    // ★ The body is EMPTY, not the request echoed back. `memcpy(resp, cmd, 4096)` is the
    // C's behaviour (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2737`) and the
    // thing this deviation exists to refuse; `RpcCommand::reply` zero-fills to the
    // request's own length, which is the M9 clamp.
    assert_eq!(
        policy.respond(&command(&w::message(999, 1, &[0xff; 64]))),
        Some(Reply {
            rpc_result: 0x56,
            body: Vec::new(),
        }),
        "a 64-byte hostile body is not reflected back at its sender",
    );
}

// ---------------------------------------------------------------------------------
// 6.2 ★★ The graph-refusal path — `BridgeRefusal::Graph`, B2's whole new surface
// ---------------------------------------------------------------------------------

/// ★★ **The arm B1 could not construct.** The bridge resolves nothing and refuses
/// nothing about a well-formed `FREE`; the graph then refuses it by name; and the policy
/// must neither pre-empt the first nor swallow the second.
///
/// `RmGraph` does **not** tolerate every `Free` — only the teardown-verb exemption in
/// `undeclared_namespace` lets one name an undeclared *namespace*. A free of an
/// undeclared **object** inside a live namespace is `FreeUnknown`, which is faithful RM
/// behaviour and must reach the guest.
#[test]
fn a_free_of_an_undeclared_object_is_refused_by_the_graph_and_named_on_the_wire() {
    use kayfabe_gsp::{CommandPolicy, Reply};

    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), &mut gpu);
    let _ = policy
        .deliver(&command(&root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID)))
        .expect("the namespace is declared");

    let stray = command(&free_msg(HEX_CLIENT, HEX_OBJECT));

    // (a) The exact variant, all the way down. `Option<Reply>` cannot carry this, which
    //     is why `deliver` exists beside `respond`.
    assert_eq!(
        policy.deliver(&stray),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::FreeUnknown(NodeKey::new(HClient(HEX_CLIENT), HObject(HEX_OBJECT)))
        ))),
    );

    // (b) The same refusal, as the guest sees it.
    assert_eq!(
        policy.respond(&stray),
        Some(Reply {
            rpc_result: 0x56,
            body: Vec::new(),
        }),
    );

    // (c) Countable, and by the rule that was broken — not by a flat "the graph said no".
    assert_eq!(
        policy.census().of(FaultTag("RmGraphError::FreeUnknown")),
        2,
        "both attempts were counted",
    );
    assert_eq!(policy.census().total(), 2, "and nothing else was");
    assert_eq!(policy.applied(), 1, "only the root ever applied");
}

/// A **double free of a client root** is the same refusal, and it is the one that says
/// the exemption is about the *namespace*, not about frees in general: the first free
/// destroys the namespace, and the second names an object that is gone.
#[test]
fn a_double_free_of_a_client_root_is_refused_by_name() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), &mut gpu);
    let _ = policy
        .deliver(&command(&root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID)))
        .expect("root");
    let free = command(&free_msg(HEX_CLIENT, HEX_CLIENT));
    assert_eq!(
        policy.deliver(&free),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(HEX_CLIENT),
            handle: HObject(HEX_CLIENT),
        })),
        "the first free is the namespace's teardown",
    );
    assert_eq!(
        policy.deliver(&free),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::FreeUnknown(NodeKey::new(HClient(HEX_CLIENT), HObject(HEX_CLIENT)))
        ))),
        "★ the second names a handle that no longer exists",
    );
}

/// A **re-declaration** of a live client root with different facts is
/// `ConflictingAlloc` — and the non-vacuity arm is the one that matters: an *identical*
/// re-send is still accepted, so this refusal is about the disagreement and not about
/// having seen the handle before.
///
/// ★ That distinction is what a stateful bridge would destroy. A dedup cache would answer
/// the identical re-send differently the second time and break the graph's retried-RPC
/// tolerance; a per-handle memo would answer the *conflicting* one from the first
/// declaration and never reach the graph at all.
#[test]
fn a_conflicting_client_root_is_refused_while_an_identical_resend_is_not() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), &mut gpu);
    let first = command(&root_alloc_msg(w::NV01_ROOT, HEX_CLIENT, HEX_PID));
    let want = Ok(Translation::Event(expected_root_event(
        HEX_CLIENT,
        w::NV01_ROOT,
        ClientKind::User { pid: HEX_PID },
    )));
    assert_eq!(policy.deliver(&first), want);
    assert_eq!(
        policy.deliver(&first),
        want,
        "an identical re-send is legal"
    );
    assert_eq!(policy.deliver(&first), want);

    // The same handle, a different declared privilege — an ambiguity, never a tie-break.
    assert_eq!(
        policy.deliver(&command(&root_alloc_msg(
            w::NV01_ROOT,
            HEX_CLIENT,
            w::KERNEL_PID
        ))),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::ConflictingAlloc(NodeKey::new(HClient(HEX_CLIENT), HObject(HEX_CLIENT)))
        ))),
    );
    assert_eq!(
        policy
            .census()
            .of(FaultTag("RmGraphError::ConflictingAlloc")),
        1,
    );
    assert_eq!(
        policy.applied(),
        3,
        "all three re-sends applied idempotently"
    );
}

// ---------------------------------------------------------------------------------
// 6.3 ★★ The oracle — `Boundaries(RpcScript) == Boundaries(Scenario)`
// ---------------------------------------------------------------------------------

/// ★★ **The strongest oracle available** (`gsp_core_bridge.md` §5.1): a device driven
/// from wire bytes and a device driven from hand-written `RmEvent`s must reach the
/// **identical** projection.
///
/// `Boundaries` derives `PartialEq` and is already compared whole across shuffled orders
/// (decision #4), so this is exact rather than a spot check — and it retires "the tests
/// synthesise `RmEvent`" without deleting the synthesiser: `Scenario` becomes the
/// reference implementation instead of the only implementation.
#[test]
fn the_projection_from_wire_bytes_equals_the_projection_from_hand_written_events() {
    let mut gpu = fresh_gpu();
    {
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        for out in deliver_all(&mut policy, &script_x().messages()) {
            let _ = out.expect("every message of the fixture is legal");
        }
        assert_eq!(policy.applied(), 4, "three roots and one free");
        assert!(policy.census().is_empty(), "a clean script refuses nothing");
    }

    let from_bytes = boundaries(&gpu);
    assert_eq!(from_bytes, boundaries_of_scenario(&scenario_x()));

    // Non-vacuity for the comparison itself: it is not trivially true, and it is not
    // trivially true of an EMPTY projection either.
    assert_ne!(from_bytes, Boundaries::default());
    assert_eq!(from_bytes.procs.len(), 1, "A exited; B remains");
    assert_eq!(
        from_bytes.procs[0].client_values(),
        [HClient(x::B)].into_iter().collect(),
    );
    assert_eq!(
        from_bytes.system.client_values(),
        [HClient(x::K)].into_iter().collect(),
    );
}

/// ★ **Non-vacuity, per §5.1's rule:** mutate one field of the script's bytes and the
/// projections must **differ**. Three mutations, each moving a different thing.
#[test]
fn one_changed_field_of_the_script_changes_the_projection() {
    let reference = boundaries_of_scenario(&scenario_x());

    // (a) B's pid becomes the kernel sentinel: B stops being a user process and joins the
    //     system component. One word, and the whole grouping decision moves.
    let mut kernelised = RpcScript::new();
    kernelised
        .client_root(w::NV01_ROOT, x::A, x::PID_A)
        .client_root(w::NV01_ROOT_CLIENT, x::B, w::KERNEL_PID)
        .client_root(w::NV01_ROOT, x::K, w::KERNEL_PID)
        .free(x::A, x::A);

    // (b) The free names B instead of A: a different namespace survives.
    let mut other_exit = RpcScript::new();
    other_exit
        .client_root(w::NV01_ROOT, x::A, x::PID_A)
        .client_root(w::NV01_ROOT_CLIENT, x::B, x::PID_B)
        .client_root(w::NV01_ROOT, x::K, w::KERNEL_PID)
        .free(x::B, x::B);

    // (c) The free is simply absent.
    let mut no_exit = RpcScript::new();
    no_exit
        .client_root(w::NV01_ROOT, x::A, x::PID_A)
        .client_root(w::NV01_ROOT_CLIENT, x::B, x::PID_B)
        .client_root(w::NV01_ROOT, x::K, w::KERNEL_PID);

    for (what, script) in [
        ("B declared the kernel pid", kernelised),
        ("B exited instead of A", other_exit),
        ("nobody exited", no_exit),
    ] {
        let mut gpu = fresh_gpu();
        {
            let mut policy = GraphPolicy::new(abi(), &mut gpu);
            for out in deliver_all(&mut policy, &script.messages()) {
                let _ = out.expect("the mutated script is still legal");
            }
        }
        assert_ne!(
            boundaries(&gpu),
            reference,
            "★ the projection comparison would not notice: {what}",
        );
    }
}

// ---------------------------------------------------------------------------------
// 6.4 ★★ Through the real transport — a scripted boot that drives the graph
// ---------------------------------------------------------------------------------

/// What one scripted boot produced.
struct Run {
    /// Every status message the guest accepted, after `GSP_INIT_DONE`.
    replies: Vec<GuestMsg>,
    /// The FSM transitions that fired.
    transitions: Vec<Transition>,
    census: RefusalCensus,
    applied: u64,
    inert: u64,
}

/// Boot a world, post `steps` through the **real command ring**, ring the doorbell, and
/// let the guest drain — with `gpu`'s object model behind the policy the whole time.
///
/// Everything the guest does here is `gspworld::Guest`'s independent re-implementation of
/// the driver's own msgq path: its own checksum fold, its own acceptance predicate, its
/// own element counts. So a message that reaches the policy reached it by surviving a
/// transport we did not write twice.
fn run_through_transport(profile: Profile, steps: &[w::Step], gpu: &mut Gpu) -> Run {
    let mut world = GspWorld::new_sized(profile, MODEL_A, REAL_QUEUE_SIZE);
    let mut policy = GraphPolicy::new(profile.table(), gpu);

    let mut transitions = world.boot_with(&mut policy);
    // The guest links its status queue and consumes `GSP_INIT_DONE` — which is also what
    // makes the FSM's `Running` edge observable rather than assumed.
    let init = world.link_and_drain();
    assert_eq!(init.len(), 1, "the bind posts exactly GSP_INIT_DONE");

    for (i, s) in steps.iter().enumerate() {
        world
            .guest
            .send(&mut world.ram, s.function, 0x1000 + i as u32, &s.body)
            .expect("the 63-slot ring has room for this script");
    }
    transitions.extend(
        world
            .doorbell_with(&mut policy)
            .expect("the doorbell services the ring")
            .transitions,
    );
    let replies = world
        .guest
        .recv(&mut world.ram)
        .expect("a clean status stream");

    Run {
        replies,
        transitions,
        census: policy.census().clone(),
        applied: policy.applied(),
        inert: policy.inert(),
    }
}

/// ★★ **B2's headline.** A guest boots, posts its client-root stream through a real
/// msgq ring, and the RM object model on the other side reaches exactly the projection
/// the hand-written reference reaches.
///
/// No GPU, no hypervisor, no OS — and, for the first time, no test body in the middle:
/// `Gpu::apply` is called by the FSM, from bytes the guest itself encoded.
#[test]
fn a_scripted_boot_drives_the_object_model_end_to_end() {
    let script = script_x();
    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    // The transport did its job: the FSM reached `Running` by observing the guest's
    // drain, and every command was answered on `(function, sequence)`.
    assert!(
        run.transitions.contains(&Transition::Running),
        "the guest drained INIT_DONE, so the RPC plane is live: {:?}",
        run.transitions,
    );
    assert_eq!(
        run.replies
            .iter()
            .map(|m| (m.function, m.sequence, m.rpc_result))
            .collect::<Vec<_>>(),
        vec![
            (fn_id::GSP_RM_ALLOC, 0x1000, 0),
            (fn_id::GSP_RM_ALLOC, 0x1001, 0),
            (fn_id::GSP_RM_ALLOC, 0x1002, 0),
            (fn_id::FREE, 0x1003, 0),
        ],
        "one NV_OK reply per command, matched on (function, sequence)",
    );

    // The policy saw all four and refused none.
    assert_eq!(run.applied, 4);
    assert_eq!(run.inert, 0);
    assert!(run.census.is_empty(), "a clean boot script refuses nothing");

    // ★ And the object model is the reference one, byte for byte.
    assert_eq!(boundaries(&gpu), boundaries_of_scenario(&scenario_x()));
}

/// ★ The **other** element layout, for free. `gsp_boot.rs` already drives both profiles;
/// the bridge inherits that, and this pins the inheritance rather than assuming it — the
/// object model must be reachable through a transport whose element header, checksum
/// offset and element-count derivation are all different.
#[test]
fn the_same_script_reaches_the_same_graph_through_the_other_element_layout() {
    use kayfabe_tests::gspworld::P610;

    let script = script_x();
    let mut a = fresh_gpu();
    let mut b = fresh_gpu();
    let run_580 = run_through_transport(P580, script.steps(), &mut a);
    let run_610 = run_through_transport(P610, script.steps(), &mut b);

    assert_eq!(run_580.applied, 4);
    assert_eq!(run_610.applied, 4);
    assert_eq!(boundaries(&a), boundaries(&b));
    assert_eq!(boundaries(&a), boundaries_of_scenario(&scenario_x()));
}

/// ★ **The pre-bind backlog reaches the policy too**, and this is the 580 boot order
/// rather than a contrived one.
///
/// At 580 the guest queues its init RPCs from `kgspInitRm_IMPL` **before** `_kgspBootGspRm`
/// ever runs, and `rpcSendMessage` rings `QUEUE_HEAD(0)` after each one — so the command
/// ring is already non-empty at bind time and the door has already rung twice against an
/// unbound queue (`Transition::E12`, the healthy arm). `GspFsm::publish` drains that
/// backlog on the bind instead of waiting for another doorbell.
///
/// So a policy passed to `boot_with` must see those commands. Without this test the
/// `boot_with`/`wr_with` distinction is unobserved on the boot path: every other test here
/// queues its traffic *after* the bind, and would pass just as well if the boot handed the
/// FSM an `EchoOk`.
#[test]
fn commands_queued_before_the_bind_reach_the_policy_when_it_publishes() {
    let script = script_x();
    let mut gpu = fresh_gpu();
    let mut world = GspWorld::new_sized(P580, MODEL_A, REAL_QUEUE_SIZE);

    // The guest queues its stream and rings the door — all before anything is bound.
    for (i, s) in script.steps().iter().enumerate() {
        world
            .guest
            .send(&mut world.ram, s.function, 0x2000 + i as u32, &s.body)
            .expect("ring space");
    }

    let (transitions, applied, census) = {
        let mut policy = GraphPolicy::new(P580.table(), &mut gpu);
        // The pre-bootstrap doorbell: the healthy arm, and it must read no guest RAM and
        // reach no policy.
        let early = world
            .doorbell_with(&mut policy)
            .expect("an unbound doorbell before any bind has ever existed is not an attack");
        assert_eq!(early.transitions, vec![Transition::E12]);
        assert_eq!(policy.applied(), 0, "nothing is serviced while unbound");

        let t = world.boot_with(&mut policy);
        (t, policy.applied(), policy.census().clone())
    };

    // ★ The bind drained the backlog: E6 (publish) fired and the four commands applied,
    // with no second doorbell anywhere in this test.
    assert!(transitions.contains(&Transition::E6), "{transitions:?}");
    assert_eq!(applied, 4, "★ the pre-bind backlog reached the policy");
    assert!(census.is_empty());
    assert_eq!(boundaries(&gpu), boundaries_of_scenario(&scenario_x()));

    // And the guest gets its replies, all four, once it links.
    let drained = world.link_and_drain();
    assert_eq!(
        drained
            .iter()
            .map(|m| (m.function, m.sequence, m.rpc_result))
            .collect::<Vec<_>>(),
        vec![
            (fn_id::GSP_INIT_DONE, 0, 0),
            (fn_id::GSP_RM_ALLOC, 0x2000, 0),
            (fn_id::GSP_RM_ALLOC, 0x2001, 0),
            (fn_id::GSP_RM_ALLOC, 0x2002, 0),
            (fn_id::FREE, 0x2003, 0),
        ],
    );
}

/// ★★ **The B2 recycle canary**, driven through the transport.
///
/// RM recycles `hClient` values by design, so a bridge that grew a handle table, a
/// seen-set or a dedup cache would refuse, deduplicate or mis-attribute a **legal**
/// recycle. B1 pinned that against `translate`; this pins it against the whole path —
/// ring, FSM, policy, graph — because that is where a cache would actually be added for
/// "performance".
///
/// The two declarations are given different privileges on purpose, so "distinct" is
/// observable in the projection rather than asserted about internals.
#[test]
fn a_recycled_hclient_survives_the_whole_transport() {
    let mut recycled = RpcScript::new();
    recycled
        .client_root(w::NV01_ROOT, x::A, x::PID_A)
        .free(x::A, x::A)
        // The same handle VALUE, declared again — this time by the guest kernel.
        .client_root(w::NV01_ROOT, x::A, w::KERNEL_PID);

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, recycled.steps(), &mut gpu);
    assert!(
        run.census.is_empty(),
        "★ a recycle is legal traffic — refusing it hangs a conforming guest: {:?}",
        run.census.tags().collect::<Vec<_>>(),
    );
    assert_eq!(run.applied, 3);
    assert_eq!(
        run.replies.iter().map(|m| m.rpc_result).collect::<Vec<_>>(),
        vec![0, 0, 0],
        "and every one of them was answered NV_OK",
    );

    let after = boundaries(&gpu);
    assert!(
        after.procs.is_empty(),
        "the recycled declaration is a kernel client"
    );
    assert_eq!(
        after.system.client_values(),
        [HClient(x::A)].into_iter().collect(),
    );

    // ★ No residue: the graph is what a device that saw ONLY the second declaration
    // would be. A per-handle memo would have answered the third message out of the
    // first's classification and left this comparison unequal.
    let mut clean = RpcScript::new();
    clean.client_root(w::NV01_ROOT, x::A, w::KERNEL_PID);
    let mut fresh = fresh_gpu();
    let clean_run = run_through_transport(P580, clean.steps(), &mut fresh);
    assert_eq!(clean_run.applied, 1);
    assert_eq!(after, boundaries(&fresh));
}

/// ★ **A finding, pinned:** the two `Disposition::NoReply` functions never reach the
/// policy at all.
///
/// `GspFsm::answer` returns **before** calling `policy.respond` for 72/73, because
/// echoing an `_issueRpcAsync` RPC surfaces in the driver as an unexpected event and
/// desyncs the stream. So the bridge is not on their path: `translate` calls them
/// `Inert`, and that arm is unreachable through this adapter. Harmless today — they
/// carry no object-model content — and load-bearing to know, because a future
/// NoReply function that *did* carry a fact would be dropped silently.
///
/// The non-vacuity arm is `SET_GUEST_SYSTEM_INFO`, which is equally inert to the object
/// model but **is** answered, and therefore does reach the policy.
#[test]
fn the_no_reply_functions_never_reach_the_policy_and_the_answered_one_does() {
    // What `translate` alone says about all three: identical.
    for code in [
        fn_id::GSP_SET_SYSTEM_INFO,
        fn_id::SET_REGISTRY,
        fn_id::SET_GUEST_SYSTEM_INFO,
    ] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Ok(Translation::Inert),
        );
    }

    let mut script = RpcScript::new();
    script
        .raw(fn_id::GSP_SET_SYSTEM_INFO, vec![0xab; 16])
        .raw(fn_id::SET_REGISTRY, vec![0xcd; 16])
        .raw(fn_id::SET_GUEST_SYSTEM_INFO, vec![0xef; 16]);

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    assert_eq!(
        run.replies
            .iter()
            .map(|m| (m.function, m.sequence))
            .collect::<Vec<_>>(),
        vec![(fn_id::SET_GUEST_SYSTEM_INFO, 0x1002)],
        "★ 72 and 73 are answered by nobody — that is Disposition::NoReply",
    );
    assert_eq!(
        run.inert, 1,
        "★ and only ONE of the three inert RPCs ever reached the policy",
    );
    assert_eq!(run.applied, 0);
    assert!(run.census.is_empty());
    assert_eq!(boundaries(&gpu), Boundaries::default());
}

// ---------------------------------------------------------------------------------
// 6.5 ★★ MEAN — hostile traffic interleaved with a live stream, through the transport
// ---------------------------------------------------------------------------------

/// ★★ The composed run, and the bar for "done" at this stage.
///
/// B1's version of this test drove `translate` directly. This one drives the **whole
/// path**: a guest encodes hostile and valid messages into one command ring, the FSM
/// decodes them, the policy translates and applies them, and the guest drains the
/// replies. Four assertions, and the first two are the load-bearing ones:
///
/// 1. **the valid stream is unaffected** — the final projection is identical to the one a
///    device driven by the valid subset alone reaches. Refusals are inert to the graph,
///    which is the whole claim of "a refusal is a named answer, not a partial apply";
/// 2. **every command is answered** — including every refused one, on its own
///    `(function, sequence)`, because the guest is blocked in `_issueRpcAndWait` and a
///    drop hangs it for the whole RPC timeout;
/// 3. **the refusals are counted by variant** — a census, not a total: a total is
///    satisfied by any N refusals and would not notice one refusing for the wrong reason;
/// 4. **the transport itself never faulted** — a `GspFault` would mean the ring stopped,
///    which is a different failure from a policy refusal and must not be confused with
///    one.
#[test]
fn hostile_traffic_through_the_ring_leaves_the_valid_stream_untouched() {
    // The valid stream: the shared fixture.
    let valid = script_x();

    // Nine hostile messages, one per refusal reason this stage can produce — including
    // the two the GRAPH produces, which B1 had no way to reach.
    let hostile: Vec<(w::Step, FaultTag)> = vec![
        (
            w::Step {
                function: 999,
                body: vec![0u8; 8],
            },
            FaultTag("BridgeRefusal::UnknownFunction"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                body: vec![0u8; 48],
            },
            FaultTag("BridgeRefusal::NotYetTranslated"),
        ),
        (
            w::Step {
                function: fn_id::GSP_INIT_DONE,
                body: vec![0u8; 8],
            },
            FaultTag("BridgeRefusal::EventFromGuest"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::client_root_alloc_body(w::NV01_ROOT, 0, 1),
            },
            FaultTag("BridgeRefusal::ReservedClient"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::alloc_body(
                    x::A,
                    0,
                    0,
                    w::NV01_ROOT,
                    8,
                    w::RMAPI_RPC_FLAGS_SERIALIZED,
                    &w::client_root_params(x::A, 1),
                ),
            },
            FaultTag("BridgeRefusal::SerializedParams"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::alloc_body(
                    x::A,
                    0,
                    0,
                    w::NV01_ROOT,
                    4096,
                    0,
                    &w::client_root_params(x::A, 1),
                ),
            },
            FaultTag("BridgeRefusal::ParamsSizeExceedsPayload"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::alloc_body(
                    x::A,
                    0,
                    0,
                    w::NV01_MEMORY_SYSTEM,
                    8,
                    0,
                    &w::client_root_params(x::A, 1),
                ),
            },
            FaultTag("BridgeRefusal::UnmappedAllocClass"),
        ),
        (
            w::Step {
                function: fn_id::FREE,
                body: vec![0u8; 3],
            },
            FaultTag("BridgeRefusal::Abi"),
        ),
        // ★ The graph's own refusals, reached from the wire for the first time.
        (
            w::Step {
                function: fn_id::FREE,
                body: w::driver_free_body(x::B, 0x5c00_0019),
            },
            FaultTag("RmGraphError::FreeUnknown"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::client_root_alloc_body(w::NV01_ROOT, x::B, 0xffff_0000),
            },
            FaultTag("RmGraphError::ConflictingAlloc"),
        ),
    ];

    // Interleave: one hostile message between every pair of valid ones, leftovers
    // appended, so no valid message is adjacent only to clean traffic. ★ The two
    // graph-refusal messages name B, which the SECOND valid message declares — so they
    // land after it and genuinely reach the graph rather than short-circuiting on an
    // undeclared namespace.
    let mut interleaved: Vec<w::Step> = Vec::new();
    let mut tags: Vec<FaultTag> = Vec::new();
    let mut h = hostile.iter();
    for v in valid.steps() {
        interleaved.push(v.clone());
        if let Some((step, tag)) = h.next() {
            interleaved.push(step.clone());
            tags.push(*tag);
        }
    }
    for (step, tag) in h {
        interleaved.push(step.clone());
        tags.push(*tag);
    }
    assert_eq!(
        tags.len(),
        hostile.len(),
        "every hostile message was posted"
    );

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, &interleaved, &mut gpu);

    // (2) Every command answered, in order, on its own (function, sequence) — and the
    //     status is non-zero for exactly the hostile ones.
    let want_replies: Vec<(u32, u32, bool)> = interleaved
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let refused = !valid.steps().iter().any(|v| v == s);
            (s.function, 0x1000 + i as u32, refused)
        })
        .collect();
    assert_eq!(
        run.replies
            .iter()
            .map(|m| (m.function, m.sequence, m.rpc_result != 0))
            .collect::<Vec<_>>(),
        want_replies,
        "★ every command is answered, and only the hostile ones carry a failure",
    );
    for m in &run.replies {
        if m.rpc_result != 0 {
            assert_eq!(m.rpc_result, 0x56, "NV_ERR_NOT_SUPPORTED (B1's one value)");
        }
    }

    // (3) The census, by variant.
    let want_counts: std::collections::BTreeMap<FaultTag, usize> =
        tags.iter()
            .fold(std::collections::BTreeMap::new(), |mut m, t| {
                *m.entry(*t).or_default() += 1;
                m
            });
    assert_eq!(
        run.census
            .tags()
            .collect::<std::collections::BTreeMap<_, _>>(),
        want_counts,
        "refusal census by variant",
    );
    assert_eq!(run.applied, 4, "the four valid facts, and only those");

    // (1) ★ The valid stream is untouched.
    let mut clean = fresh_gpu();
    let clean_run = run_through_transport(P580, valid.steps(), &mut clean);
    assert!(clean_run.census.is_empty());
    assert_eq!(
        boundaries(&gpu),
        boundaries(&clean),
        "★ hostile traffic must be inert to the graph, not partially applied",
    );
    assert_eq!(boundaries(&gpu), boundaries_of_scenario(&scenario_x()));
}

// =================================================================================
// ★★ 7. **Stage B3 — the class table**: a CUDA process's whole subgraph, from bytes
//
// B1/B2 could declare and free a client NAMESPACE. Nothing below the root existed,
// because `translate_alloc` refused every class but the root by name. B3 is the class
// table (`DriverAbiTable::alloc_params`) plus the four params decoders it selects, and
// what it buys is the first *shaped* subgraph: client → device → VASpace → TSG →
// CtxShare → two channels → two engine objects.
//
// Three things stay refused, each for a reason recorded at the site rather than skipped:
// a memory object (§6's `mem_phys` row is unbuildable in this direction — see
// `w::NV01_MEMORY_SYSTEM`), a channel's `engineType` (nowhere to put it in `RmEvent`),
// and everything past the channel-params prefix (the two vendored trees disagree there).
// =================================================================================

/// Handles for one CUDA-process-shaped subgraph. Written as a module of constants rather
/// than a builder so the byte side and the event side can be read against each other on
/// one screen — the same reason `mod x` exists above.
mod cp {
    /// The compute client.
    pub const C: u32 = 0xc1d0_0071;
    /// Its pid, deliberately unequal to its handle.
    pub const PID: u32 = 0x0000_ab21;
    /// `NV01_DEVICE_0`.
    pub const DEV: u32 = 0x5c00_0001;
    /// `FERMI_VASPACE_A`.
    pub const VAS: u32 = 0x5c00_0010;
    /// `KEPLER_CHANNEL_GROUP_A`.
    pub const TSG: u32 = 0x5c00_0012;
    /// `FERMI_CONTEXT_SHARE_A`.
    pub const CTXSHARE: u32 = 0x5c00_0013;
    /// The GR channel.
    pub const GR: u32 = 0x5c00_0019;
    /// The CE channel — adjacent to the GR handle, so a bridge that mixed the two up
    /// would still produce a plausible graph.
    pub const CE: u32 = 0x5c00_001a;
    /// `AMPERE_COMPUTE_B` on the GR channel.
    pub const GR_OBJ: u32 = 0x5c00_0020;
    /// `AMPERE_DMA_COPY_B` on the CE channel.
    pub const CE_OBJ: u32 = 0x5c00_0021;
    /// The physical GPU the Device declares. Non-zero on purpose: `Some(0)` is also what
    /// a dropped `deviceId` would look like.
    pub const DEVICE_INSTANCE: u32 = 0;
}

/// The two channels' vChids, through the arch's own encoding — the guest declares the
/// *encoded* word and the arch recovers the id, which is the seam B3 explicitly cannot
/// prove against real silicon.
const CP_GR_VCHID: VChid = VChid(0x21);
const CP_CE_VCHID: VChid = VChid(0x22);

fn gr_flags() -> u32 {
    MockArch::userd_flags_for(CP_GR_VCHID)
}
fn ce_flags() -> u32 {
    MockArch::userd_flags_for(CP_CE_VCHID)
}

/// **Transcription #1** of the compute subgraph: bytes, from `ogkm`'s struct definitions.
///
/// ★ The GR channel declares its VASpace **directly**; the CE channel declares
/// `hVASpace = 0` and reaches the same VAS **through its context share**. That is not
/// decoration: `resolve_channel_vas`'s precedence (own handle → CtxShare's → parent TSG's)
/// has three arms, and a script that only ever used the first would leave two of them
/// unobserved on this path.
fn script_compute() -> RpcScript {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .ctxshare(cp::C, cp::VAS, cp::CTXSHARE, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .channel(cp::C, cp::TSG, cp::CE, ce_flags(), cp::CTXSHARE, 0)
        .engine_object(cp::C, cp::GR, cp::GR_OBJ, w::AMPERE_COMPUTE_B)
        .engine_object(cp::C, cp::CE, cp::CE_OBJ, w::AMPERE_DMA_COPY_B);
    s
}

/// **Transcription #2** of the same subgraph: `RmEvent`s, written by hand, in
/// `mock_classes` — so an agreement between the two is an agreement about
/// *classification and facts*, never about a class number travelling twice.
fn scenario_compute() -> Scenario {
    let alloc = |parent: u32, handle: u32, class, facts| RmEvent::Alloc {
        client: HClient(cp::C),
        parent: HObject(parent),
        handle: HObject(handle),
        class,
        facts,
    };
    let vas_only = |vas: u32| AllocFacts {
        h_vaspace: Some(HObject(vas)),
        ..Default::default()
    };
    let mut s = Scenario::new();
    s.push(alloc(
        cp::C,
        cp::C,
        mock_classes::CLIENT,
        AllocFacts {
            client_kind: Some(ClientKind::User { pid: cp::PID }),
            ..Default::default()
        },
    ))
    .push(alloc(
        cp::C,
        cp::DEV,
        mock_classes::DEVICE,
        AllocFacts {
            device_instance: Some(cp::DEVICE_INSTANCE),
            ..Default::default()
        },
    ))
    .push(alloc(
        cp::DEV,
        cp::VAS,
        mock_classes::VASPACE,
        AllocFacts::default(),
    ))
    .push(alloc(
        cp::DEV,
        cp::TSG,
        mock_classes::TSG,
        vas_only(cp::VAS),
    ))
    .push(alloc(
        cp::VAS,
        cp::CTXSHARE,
        mock_classes::CTXSHARE,
        vas_only(cp::VAS),
    ))
    .push(alloc(
        cp::TSG,
        cp::GR,
        // ★ The wire says `AMPERE_CHANNEL_GPFIFO_A` for BOTH channels; the reference says
        // `CHANNEL_GR` for both, because that is what a class-id-only `classify` can
        // answer. The CE-ness of the second one arrives with its engine object.
        mock_classes::CHANNEL_GR,
        AllocFacts {
            h_vaspace: Some(HObject(cp::VAS)),
            userd_flags: gr_flags(),
            ..Default::default()
        },
    ))
    .push(alloc(
        cp::TSG,
        cp::CE,
        mock_classes::CHANNEL_GR,
        AllocFacts {
            h_ctx_share: Some(HObject(cp::CTXSHARE)),
            userd_flags: ce_flags(),
            ..Default::default()
        },
    ))
    .push(alloc(
        cp::GR,
        cp::GR_OBJ,
        mock_classes::COMPUTE,
        AllocFacts::default(),
    ))
    .push(alloc(
        cp::CE,
        cp::CE_OBJ,
        mock_classes::DMA_COPY,
        AllocFacts::default(),
    ));
    s
}

/// The compute client's root, as an event — written by hand from the normalisation rule.
fn root_of_c() -> RmEvent {
    RmEvent::Alloc {
        client: HClient(cp::C),
        parent: HObject(cp::C),
        handle: HObject(cp::C),
        class: ClassId(w::NV01_ROOT),
        facts: AllocFacts {
            client_kind: Some(ClientKind::User { pid: cp::PID }),
            ..Default::default()
        },
    }
}

/// The PDB the compute VAS gets. Applied as a **raw event on both sides** — translating
/// `GSP_RM_CONTROL` is B4's, and this stage may not pretend otherwise. Holding it
/// constant on both sides is what keeps the comparison about the alloc translation.
const CP_PDB: Pdb = Pdb(0x0034_1000);

fn set_page_dir() -> RmEvent {
    RmEvent::SetPageDir {
        client: HClient(cp::C),
        vaspace: HObject(cp::VAS),
        pdb: CP_PDB,
    }
}

/// Drive a script through the policy onto a fresh device, then bind the PDB.
fn gpu_from_script(script: &RpcScript) -> kayfabe_tests::Guarded<Gpu> {
    let mut gpu = fresh_gpu();
    {
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        for (i, out) in deliver_all(&mut policy, &script.messages())
            .into_iter()
            .enumerate()
        {
            let _ = out.unwrap_or_else(|e| panic!("message {i} of the script refused: {e:?}"));
        }
        assert!(policy.census().is_empty(), "a clean script refuses nothing");
    }
    gpu.apply(set_page_dir()).expect("the PDB binds");
    gpu
}

// ---------------------------------------------------------------------------------
// 7.0 Transcription #2 for the new shape — a channel alloc, written byte by byte
// ---------------------------------------------------------------------------------

/// A complete `GSP_RM_ALLOC` message allocating the GR channel of the compute subgraph,
/// written out by hand. The third transcription of the channel layout, sharing no code
/// path with the builder or with the decoder.
///
/// ```text
/// ── rpc_message_header_v03_00 (32 B) ──────────────────────────────────────────
/// +0   header_version      00 00 00 03   -> 0x03000000
/// +4   signature           56 52 50 43   -> "VRPC" LE
/// +8   length              60 00 00 00   -> 96 = 32 envelope + 32 alloc hdr + 32 params
/// +12  function            67 00 00 00   -> 103 = GSP_RM_ALLOC
/// +16  rpc_result          00 00 00 00
/// +20  rpc_result_private  00 00 00 00
/// +24  sequence            03 00 00 00
/// +28  u                   00 00 00 00
/// ── rpc_gsp_rm_alloc_v03_00 (32 B) ────────────────────────────────────────────
/// +32  hClient             71 00 d0 c1   -> 0xc1d00071  (cp::C)
/// +36  hParent             12 00 00 5c   -> 0x5c000012  (cp::TSG)  ★ carried VERBATIM
/// +40  hObject             19 00 00 5c   -> 0x5c000019  (cp::GR)   ★ carried VERBATIM
/// +44  hClass              6f c5 00 00   -> 0xc56f = AMPERE_CHANNEL_GPFIFO_A
/// +48  status              00 00 00 00   [OUT]
/// +52  paramsSize          20 00 00 00   -> 32 = the agreed prefix
/// +56  flags               00 00 00 00   -> RMAPI_RPC_FLAGS_NONE
/// +60  reserved[4]         00 00 00 00
/// ── NV_CHANNEL_ALLOC_PARAMS, the 32-byte agreed prefix ────────────────────────
/// +64  hObjectError        00 00 00 00                       (params +0)
/// +68  hObjectBuffer       00 00 00 00                       (params +4)
/// +72  gpFifoOffset        00 … 00  (NvU64, 8-aligned)       (params +8)
/// +80  gpFifoEntries       00 00 00 00                       (params +16)
/// +84  flags               85 10 00 00   -> 0x1085           (params +20) ★
/// +88  hContextShare       00 00 00 00   -> none declared    (params +24) ★
/// +92  hVASpace            10 00 00 5c   -> 0x5c000010       (params +28) ★
/// ── and NOTHING follows: +96 is where 610 would put `hHandleVASpace` and 580
///    would put `hUserdMemory[0]`, which is exactly why the message stops here.
/// ```
const HEX_CHANNEL_ALLOC: [u8; 96] = [
    0x00, 0x00, 0x00, 0x03, 0x56, 0x52, 0x50, 0x43, 0x60, 0x00, 0x00, 0x00, 0x67, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x71, 0x00, 0xd0, 0xc1, 0x12, 0x00, 0x00, 0x5c, 0x19, 0x00, 0x00, 0x5c, 0x6f, 0xc5, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x85, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x5c,
];

/// ★ **Transcription #1 vs #2, for the channel.** Two humans, two methods, one byte
/// string. The hex above was written from `ogkm`'s header with a ruler; the builder was
/// written from the same header independently; if they disagree, one reading is wrong,
/// which is the only reason to write ninety-six bytes out by hand.
#[test]
fn the_hand_written_hex_channel_and_the_independent_builder_agree_byte_for_byte() {
    // The encoded flags word is part of the fixture, so it is pinned as a literal too —
    // otherwise the hex would silently follow the mock arch's packing wherever it went.
    assert_eq!(gr_flags(), 0x1085, "MockArch::userd_flags_for(VChid(0x21))");
    assert_eq!(
        w::message(
            fn_id::GSP_RM_ALLOC,
            3,
            &w::alloc_body(
                cp::C,
                cp::TSG,
                cp::GR,
                w::AMPERE_CHANNEL_GPFIFO_A,
                32,
                w::RMAPI_RPC_FLAGS_NONE,
                &w::channel_params(gr_flags(), 0, cp::VAS),
            ),
        ),
        HEX_CHANNEL_ALLOC.to_vec(),
    );
}

/// …and it becomes the event written out by hand from `AllocFacts`' own documentation.
#[test]
fn the_hand_hex_channel_alloc_becomes_the_declared_event() {
    assert_eq!(
        xlate(&HEX_CHANNEL_ALLOC),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(cp::C),
            parent: HObject(cp::TSG),
            handle: HObject(cp::GR),
            class: ClassId(w::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                // `hContextShare` was `NV01_NULL_OBJECT` on the wire, which is absence.
                h_ctx_share: None,
                userd_flags: 0x1085,
                ..Default::default()
            },
        })),
    );
}

// ---------------------------------------------------------------------------------
// 7.1 Each class, on its own, decoded from bytes
// ---------------------------------------------------------------------------------

/// Every class B3 added produces **exactly** the `AllocFacts` its params declare — and
/// exactly nothing else. Written as one table so a decoder that filled the wrong field
/// fails on the row it belongs to.
///
/// ★ The expected events are written out by hand from the core's own `AllocFacts` docs,
/// never derived from the builder.
#[test]
fn every_class_in_the_table_decodes_its_declared_facts_and_only_those() {
    const P: u32 = 0x5c00_00aa; // an arbitrary parent
    const H: u32 = 0x5c00_00bb; // an arbitrary handle
    let msg = |class: u32, params: &[u8]| {
        w::message(
            fn_id::GSP_RM_ALLOC,
            1,
            &w::alloc_body(
                cp::C,
                P,
                H,
                class,
                params.len() as u32,
                w::RMAPI_RPC_FLAGS_NONE,
                params,
            ),
        )
    };
    let want = |class: u32, facts: AllocFacts| {
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(cp::C),
            parent: HObject(P),
            handle: HObject(H),
            class: ClassId(class),
            facts,
        }))
    };

    assert_eq!(
        xlate(&msg(w::NV01_DEVICE_0, &w::device_params(3, 0xbeef, 7))),
        want(
            w::NV01_DEVICE_0,
            AllocFacts {
                device_instance: Some(3),
                ..Default::default()
            }
        ),
        "Device: `deviceId` @ +0 and nothing else — `hClientShare` and `vaMode` are set \
         here precisely so a decoder that read one of them fails",
    );
    assert_eq!(
        xlate(&msg(w::FERMI_VASPACE_A, &[0xff; 56])),
        want(w::FERMI_VASPACE_A, AllocFacts::default()),
        "★ VASpace: 56 bytes of 0xff declare NOTHING. Its params are geometry, and a \
         decoder that invented a fact from them would be inventing it from garbage",
    );
    assert_eq!(
        xlate(&msg(w::KEPLER_CHANNEL_GROUP_A, &w::tsg_params(cp::VAS, 9))),
        want(
            w::KEPLER_CHANNEL_GROUP_A,
            AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                ..Default::default()
            }
        ),
        "TSG: `hVASpace` @ +8. `engineType` @ +12 is set to 9 and dropped — declared, \
         with nowhere in `AllocFacts` to go",
    );
    assert_eq!(
        xlate(&msg(
            w::FERMI_CONTEXT_SHARE_A,
            &w::ctxshare_params(cp::VAS, 1, 2)
        )),
        want(
            w::FERMI_CONTEXT_SHARE_A,
            AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                ..Default::default()
            }
        ),
        "CtxShare: `hVASpace` @ +0 — a DIFFERENT offset from the TSG's, which is the \
         mistake this pair invites",
    );
    assert_eq!(
        xlate(&msg(
            w::AMPERE_CHANNEL_GPFIFO_A,
            &w::channel_params(gr_flags(), cp::CTXSHARE, cp::VAS)
        )),
        want(
            w::AMPERE_CHANNEL_GPFIFO_A,
            AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                h_ctx_share: Some(HObject(cp::CTXSHARE)),
                userd_flags: gr_flags(),
                ..Default::default()
            }
        ),
        "Channel: all three, from +20/+24/+28",
    );
    for class in [w::AMPERE_COMPUTE_B, w::AMPERE_DMA_COPY_B] {
        assert_eq!(
            xlate(&msg(class, &[0xff; 64])),
            want(class, AllocFacts::default()),
            "an engine object declares only its edge, whatever its params say",
        );
    }
    // …and the class the table does NOT have stays refused, by name.
    assert_eq!(
        xlate(&msg(w::NV01_MEMORY_SYSTEM, &[0u8; 64])),
        Err(BridgeRefusal::UnmappedAllocClass {
            class: w::NV01_MEMORY_SYSTEM
        }),
        "★ `mem_phys` is `gsp_core_bridge.md` §6's B3 row and is not buildable in this \
         direction — the refusal is the record of that, not a gap",
    );
}

/// ★ `NV01_NULL_OBJECT` in a declared-handle field means **nothing is declared**, which
/// the core spells `None`. `Some(HObject(0))` would be a handle the guest never allocated
/// and can never allocate, so every resolution of it would MISS forever.
#[test]
fn a_zero_handle_field_declares_nothing_rather_than_object_zero() {
    let msg = |class: u32, params: &[u8]| {
        w::message(
            fn_id::GSP_RM_ALLOC,
            1,
            &w::alloc_body(
                cp::C,
                cp::TSG,
                cp::GR,
                class,
                params.len() as u32,
                w::RMAPI_RPC_FLAGS_NONE,
                params,
            ),
        )
    };
    let facts_of = |m: &[u8]| match xlate(m) {
        Ok(Translation::Event(RmEvent::Alloc { facts, .. })) => facts,
        other => panic!("expected an Alloc, got {other:?}"),
    };

    // A channel that declares neither: both `None`, and `userd_flags` still carried.
    assert_eq!(
        facts_of(&msg(
            w::AMPERE_CHANNEL_GPFIFO_A,
            &w::channel_params(gr_flags(), 0, 0)
        )),
        AllocFacts {
            h_vaspace: None,
            h_ctx_share: None,
            userd_flags: gr_flags(),
            ..Default::default()
        },
        "★ hVASpace = 0 is the GSP-managed VAS, which `AllocFacts` models as absence",
    );
    // Non-vacuity: the same fields non-zero are `Some`, so the `None` above is the zero's
    // doing and not the decoder's.
    assert_eq!(
        facts_of(&msg(
            w::AMPERE_CHANNEL_GPFIFO_A,
            &w::channel_params(gr_flags(), 1, 2)
        )),
        AllocFacts {
            h_vaspace: Some(HObject(2)),
            h_ctx_share: Some(HObject(1)),
            userd_flags: gr_flags(),
            ..Default::default()
        },
    );
    assert_eq!(
        facts_of(&msg(w::KEPLER_CHANNEL_GROUP_A, &w::tsg_params(0, 0))).h_vaspace,
        None,
    );
    assert_eq!(
        facts_of(&msg(w::FERMI_CONTEXT_SHARE_A, &w::ctxshare_params(0, 0, 0))).h_vaspace,
        None,
    );
    // ★ And `userd_flags` is NOT given the same treatment: it is a `u32`, not an
    // `Option`, because zero is a legal encoded flags word and absence is not
    // expressible. Asserted so the asymmetry is a decision rather than an oversight.
    assert_eq!(
        facts_of(&msg(
            w::AMPERE_CHANNEL_GPFIFO_A,
            &w::channel_params(0, cp::CTXSHARE, cp::VAS)
        ))
        .userd_flags,
        0,
    );
}

/// ★★ **The 580-vs-610 fork, made a test.** `NV_CHANNEL_ALLOC_PARAMS` diverges at +32:
/// 610 has `hHandleVASpace` there, 580 — the bench driver — has `hUserdMemory[0]`. The
/// decoder must therefore read the agreed prefix and **never one byte past it**.
#[test]
fn nothing_past_the_channel_prefix_is_read_however_hostile_it_is() {
    let facts_of = |params: &[u8]| match xlate(&w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            cp::C,
            cp::TSG,
            cp::GR,
            w::AMPERE_CHANNEL_GPFIFO_A,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            params,
        ),
    )) {
        Ok(Translation::Event(RmEvent::Alloc { facts, .. })) => facts,
        other => panic!("expected an Alloc, got {other:?}"),
    };

    let want = AllocFacts {
        h_vaspace: Some(HObject(cp::VAS)),
        h_ctx_share: Some(HObject(cp::CTXSHARE)),
        userd_flags: gr_flags(),
        ..Default::default()
    };
    let exact = w::channel_params(gr_flags(), cp::CTXSHARE, cp::VAS);
    assert_eq!(facts_of(&exact), want, "the prefix alone decodes");

    // The same prefix followed by 0xff to the 610 length, to the 580 length, and to an
    // absurd length. All three must decode identically — the tail is not consulted.
    for extra in [4usize, 8, 104, 108, 512] {
        let mut long = exact.clone();
        long.extend(std::iter::repeat_n(0xffu8, extra));
        assert_eq!(
            facts_of(&long),
            want,
            "★ {extra} bytes of tail changed a fact — the decoder is reading past the \
             region the two vendored trees agree on",
        );
    }
}

/// One byte short of the prefix is a **refusal carrying both numbers**, never a
/// zero-extended decode — a channel whose params stop before `hVASpace` has not declared
/// one, and reading absence as `hVASpace = 0` would silently manufacture a legal
/// "GSP-managed VAS" declaration out of a truncated message.
#[test]
fn a_channel_params_short_of_the_prefix_is_refused_at_every_length() {
    let full = w::channel_params(gr_flags(), cp::CTXSHARE, cp::VAS);
    for len in 0..full.len() {
        let params = &full[..len];
        assert_eq!(
            xlate(&w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(
                    cp::C,
                    cp::TSG,
                    cp::GR,
                    w::AMPERE_CHANNEL_GPFIFO_A,
                    len as u32,
                    w::RMAPI_RPC_FLAGS_NONE,
                    params,
                ),
            )),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "NV_CHANNEL_ALLOC_PARAMS",
                need: 32,
                got: len,
            })),
            "a {len}-byte channel params",
        );
    }
    // Non-vacuity: the full prefix is accepted, so the loop is refusing shortness rather
    // than refusing channels.
    assert!(matches!(
        xlate(&w::message(
            fn_id::GSP_RM_ALLOC,
            1,
            &w::alloc_body(
                cp::C,
                cp::TSG,
                cp::GR,
                w::AMPERE_CHANNEL_GPFIFO_A,
                full.len() as u32,
                w::RMAPI_RPC_FLAGS_NONE,
                &full,
            ),
        )),
        Ok(Translation::Event(_))
    ));
}

/// The other three classes refuse a short params too, each naming **its own** struct —
/// so a truncation is attributed to the class that was actually being decoded.
#[test]
fn every_new_decoder_names_its_own_struct_when_it_refuses() {
    let cases: [(u32, &str, usize, Vec<u8>); 3] = [
        (
            w::NV01_DEVICE_0,
            "NV0080_ALLOC_PARAMETERS",
            56,
            w::device_params(0, 0, 0),
        ),
        (
            w::KEPLER_CHANNEL_GROUP_A,
            "NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS",
            20,
            w::tsg_params(cp::VAS, 0),
        ),
        (
            w::FERMI_CONTEXT_SHARE_A,
            "NV_CTXSHARE_ALLOCATION_PARAMETERS",
            12,
            w::ctxshare_params(cp::VAS, 0, 0),
        ),
    ];
    for (class, c_name, need, full) in cases {
        assert_eq!(full.len(), need, "the builder emits sizeof for {c_name}");
        let short = &full[..need - 1];
        assert_eq!(
            xlate(&w::message(
                fn_id::GSP_RM_ALLOC,
                1,
                &w::alloc_body(
                    cp::C,
                    cp::DEV,
                    cp::VAS,
                    class,
                    short.len() as u32,
                    w::RMAPI_RPC_FLAGS_NONE,
                    short,
                ),
            )),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name,
                need,
                got: need - 1,
            })),
        );
    }
}

/// ★ Per-field non-vacuity (§5.1): change **one word** of a channel's params and exactly
/// one fact of the event moves. Three mutations, three facts, no overlap.
#[test]
fn one_changed_word_of_the_channel_params_moves_exactly_one_fact() {
    let base = w::channel_params(gr_flags(), cp::CTXSHARE, cp::VAS);
    let reference = AllocFacts {
        h_vaspace: Some(HObject(cp::VAS)),
        h_ctx_share: Some(HObject(cp::CTXSHARE)),
        userd_flags: gr_flags(),
        ..Default::default()
    };
    let facts_of = |params: &[u8]| match xlate(&w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            cp::C,
            cp::TSG,
            cp::GR,
            w::AMPERE_CHANNEL_GPFIFO_A,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            params,
        ),
    )) {
        Ok(Translation::Event(RmEvent::Alloc { facts, .. })) => facts,
        other => panic!("expected an Alloc, got {other:?}"),
    };
    assert_eq!(facts_of(&base), reference);

    let mutate = |off: usize, v: u32| {
        let mut p = base.clone();
        p[off..off + 4].copy_from_slice(&v.to_le_bytes());
        facts_of(&p)
    };
    assert_eq!(
        mutate(20, ce_flags()),
        AllocFacts {
            userd_flags: ce_flags(),
            ..reference
        },
        "+20 moves `userd_flags` and nothing else",
    );
    assert_eq!(
        mutate(24, 0x5c00_00ff),
        AllocFacts {
            h_ctx_share: Some(HObject(0x5c00_00ff)),
            ..reference
        },
        "+24 moves `h_ctx_share` and nothing else",
    );
    assert_eq!(
        mutate(28, 0x5c00_00ee),
        AllocFacts {
            h_vaspace: Some(HObject(0x5c00_00ee)),
            ..reference
        },
        "+28 moves `h_vaspace` and nothing else",
    );
}

// ---------------------------------------------------------------------------------
// 7.2 ★★ The composed subgraph — `Boundaries(RpcScript) == Boundaries(Scenario)`
// ---------------------------------------------------------------------------------

/// ★★ **B3's headline** (`gsp_core_bridge.md` §6): a compute-process-shaped subgraph,
/// from wire bytes, projecting identically to the hand-written reference.
///
/// The two sides share no class number, no offset and no decoder. What they share is the
/// *meaning*, and `Boundaries` is what must not be able to tell them apart.
#[test]
fn the_compute_subgraph_from_wire_bytes_equals_the_hand_written_scenario() {
    let mut gpu = gpu_from_script(&script_compute());
    let from_bytes = boundaries(&gpu);

    let mut reference = fresh_gpu();
    for ev in &scenario_compute().events {
        reference
            .apply(*ev)
            .expect("the reference scenario is legal");
    }
    reference.apply(set_page_dir()).expect("the PDB binds");

    assert_eq!(from_bytes, boundaries(&reference));

    // Non-vacuity: the equality is not between two empty projections, and the subgraph
    // really has the shape the name claims.
    assert_ne!(from_bytes, Boundaries::default());
    assert_eq!(from_bytes.procs.len(), 1, "one compute process");
    assert_eq!(
        from_bytes.procs[0].client_values(),
        [HClient(cp::C)].into_iter().collect(),
    );
    assert_eq!(
        from_bytes.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, CP_PDB)],
        "★ the VAS routes — which needed the Device's `deviceId` to resolve a target",
    );
    assert_eq!(
        from_bytes.by_vchid.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, CP_GR_VCHID), (GpuId::ZERO, CP_CE_VCHID),],
        "★★ BOTH channels routed, and their vChids came out of `userd_flags` — the one \
         thing in this stage that is a wire word all the way to the exec plane",
    );

    // ★ The engine refinement: the CE channel is a CE channel because its engine object
    // said so, NOT because its class id differed — the wire has one channel class.
    let engines: Vec<(VChid, EngineKind)> = from_bytes.procs[0]
        .channels
        .values()
        .map(|c| (c.vchid, c.engine))
        .collect();
    assert_eq!(
        engines,
        vec![
            (CP_GR_VCHID, EngineKind::GrCompute),
            (CP_CE_VCHID, EngineKind::Ce),
        ],
    );

    // And the doorbell actually works on both, which is what "the subgraph is real" means.
    for vchid in [CP_GR_VCHID, CP_CE_VCHID] {
        assert!(
            handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(vchid), &[]).is_ok(),
            "the {vchid:?} channel rings",
        );
    }
}

/// ★ Non-vacuity for the projection comparison, per §5.1: mutate one field of the
/// script's **bytes** and the projections must differ. One mutation per decoder B3 added,
/// so a decoder that quietly stopped reading its field would be caught here as well as at
/// its own unit test.
#[test]
fn one_changed_field_of_the_compute_script_changes_the_projection() {
    let reference = boundaries(&gpu_from_script(&script_compute()));

    // ★ (a) The Device decoder gets its own arm, and it is not a projection difference:
    //     this device was realized with ONE GPU, so a `deviceId` of 1 is refused by the
    //     graph with its exact variant. That is a stronger reading of "the field is
    //     read" than any boundary comparison — a decoder that dropped `deviceId` would
    //     declare instance 0 here and the alloc would sail through.
    let mut other_gpu = RpcScript::new();
    other_gpu
        .client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, 1);
    {
        let mut gpu = fresh_gpu();
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        let out = deliver_all(&mut policy, &other_gpu.messages());
        assert_eq!(out[0], Ok(Translation::Event(root_of_c())));
        assert_eq!(
            out[1],
            Err(BridgeRefusal::Graph(GpuError::Graph(
                RmGraphError::InvalidDeviceInstance { instance: 1 }
            ))),
        );
    }

    // ★ (b) The channel decoder's own arm, and it is also a refusal rather than a
    //     difference: give the GR channel the CE's `userd_flags` and the two channels
    //     claim one vChid. `userd_flags` is a wire word that reaches the exec plane
    //     unaltered, so a decoder that dropped it would make BOTH channels vChid 0 and
    //     collide anyway — which is why the surviving channel is asserted by handle.
    let mut swapped_vchid = RpcScript::new();
    swapped_vchid
        .client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .ctxshare(cp::C, cp::VAS, cp::CTXSHARE, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, ce_flags(), 0, cp::VAS)
        .channel(cp::C, cp::TSG, cp::CE, ce_flags(), cp::CTXSHARE, 0);
    {
        let mut gpu = fresh_gpu();
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        let out = deliver_all(&mut policy, &swapped_vchid.messages());
        for (i, o) in out.iter().enumerate().take(6) {
            assert!(o.is_ok(), "message {i} of the prefix refused: {o:?}");
        }
        assert_eq!(
            out[6],
            Err(BridgeRefusal::Graph(GpuError::Projection(
                ProjectionError::VchidCollision {
                    gpu: Some(GpuId::ZERO),
                    vchid: CP_CE_VCHID,
                    a: ResourceKey::first(NodeKey::new(HClient(cp::C), HObject(cp::GR))),
                    b: ResourceKey::first(NodeKey::new(HClient(cp::C), HObject(cp::CE))),
                }
            ))),
            "★ two channels at one vChid is a projection refusal, by exact variant",
        );
    }

    // (c) The CtxShare names no VASpace: the CE channel, which reaches its VAS ONLY
    //     through the context share, loses it.
    let mut ctxshare_unbound = RpcScript::new();
    ctxshare_unbound
        .client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .ctxshare(cp::C, cp::VAS, cp::CTXSHARE, 0)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .channel(cp::C, cp::TSG, cp::CE, ce_flags(), cp::CTXSHARE, 0);

    // (d) The two engine objects swap channels: the engines swap with them.
    let mut swapped_engines = RpcScript::new();
    swapped_engines
        .client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .ctxshare(cp::C, cp::VAS, cp::CTXSHARE, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .channel(cp::C, cp::TSG, cp::CE, ce_flags(), cp::CTXSHARE, 0)
        .engine_object(cp::C, cp::GR, cp::GR_OBJ, w::AMPERE_DMA_COPY_B)
        .engine_object(cp::C, cp::CE, cp::CE_OBJ, w::AMPERE_COMPUTE_B);

    for (what, script) in [
        ("the CtxShare declared no VASpace", ctxshare_unbound),
        ("the engine objects swapped channels", swapped_engines),
    ] {
        assert_ne!(
            boundaries(&gpu_from_script(&script)),
            reference,
            "★ the projection comparison would not notice: {what}",
        );
    }
}

/// ★★ **The design's own B3 non-vacuity arm** (`gsp_core_bridge.md` §5.2): *"with the
/// decoder removed, the channel gets no `Vas` and the doorbell takes `FwdFault::NoVas` —
/// assert that, so the decoder's value is measured, not assumed."*
///
/// "The decoder removed" is expressed exactly: the identical edge (client, parent,
/// handle, class) with `AllocFacts::default()`, which is what a class with no decoder
/// would have produced. Everything else is held constant, including the PDB — so the
/// difference between a served ring and a named fault is *the decoder*.
///
/// ★ It takes **three** runs, not two, and the third is a finding the design's one-line
/// arm does not have: the channel decoder recovers `userd_flags` as well as the VAS
/// handles, and `userd_flags` is what the channel ROUTES by. So a wholly absent decoder
/// does not reach `NoVas` at all — the doorbell's token resolves to no channel and the
/// fault is `UnknownVchid`, one plane earlier. Only stripping the handle facts while
/// keeping the flags isolates the VAS, and that is the run the design is describing.
#[test]
fn with_the_channel_decoder_removed_the_doorbell_takes_no_vas() {
    let channel_msg = w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            cp::C,
            cp::TSG,
            cp::GR,
            w::AMPERE_CHANNEL_GPFIFO_A,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::channel_params(gr_flags(), 0, cp::VAS),
        ),
    );
    let decoded = match xlate(&channel_msg) {
        Ok(Translation::Event(ev)) => ev,
        other => panic!("expected an Alloc, got {other:?}"),
    };
    // The SAME edge, with the facts a class with no decoder behind it would carry.
    let with_facts = |facts: AllocFacts| match decoded {
        RmEvent::Alloc {
            client,
            parent,
            handle,
            class,
            ..
        } => RmEvent::Alloc {
            client,
            parent,
            handle,
            class,
            facts,
        },
        other => panic!("expected an Alloc, got {other:?}"),
    };
    let undecoded = with_facts(AllocFacts::default());
    let flags_only = with_facts(AllocFacts {
        userd_flags: gr_flags(),
        ..Default::default()
    });
    assert_ne!(decoded, undecoded, "the two differ only in `facts`");
    assert_ne!(decoded, flags_only);

    // ★ The TSG declares NO VASpace, so the channel's own `hVASpace` is the only path to
    // one — which is what makes this a measurement of the decoder rather than of the TSG.
    let prefix = {
        let mut s = RpcScript::new();
        s.client_root(w::NV01_ROOT, cp::C, cp::PID)
            .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
            .vaspace(cp::C, cp::DEV, cp::VAS)
            .tsg(cp::C, cp::DEV, cp::TSG, 0);
        s
    };
    let run = |channel: RmEvent| {
        let mut gpu = fresh_gpu();
        {
            let mut policy = GraphPolicy::new(abi(), &mut gpu);
            for out in deliver_all(&mut policy, &prefix.messages()) {
                let _ = out.expect("the prefix is legal");
            }
        }
        gpu.apply(channel).expect("the channel applies either way");
        gpu.apply(set_page_dir()).expect("the PDB binds");
        let outcome = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(CP_GR_VCHID), &[]);
        let vas_pdb = gpu
            .spine
            .by_vchid
            .get(&(GpuId::ZERO, CP_GR_VCHID))
            .map(|&(pid, cid)| gpu.procs[&pid].channels[&cid].vas_pdb);
        (outcome.map(|_| ()), vas_pdb)
    };

    // (1) With the decoder: the channel routes at the declared vChid, finds its VAS, and
    //     the ring is served.
    let (with_decoder, with_pdb) = run(decoded);
    assert_eq!(with_pdb, Some(Some(CP_PDB)), "the channel found its VAS");
    assert_eq!(with_decoder, Ok(()), "…and the ring is served");

    // (2) ★ With NO decoder at all: `userd_flags` is 0 too, so the channel is not even at
    //     this vChid. The absence surfaces one plane earlier, by name.
    let (no_decoder, no_pdb) = run(undecoded);
    assert_eq!(
        no_pdb, None,
        "★ no decoder ⇒ no `userd_flags` ⇒ the channel routes at a different vChid",
    );
    assert_eq!(
        no_decoder,
        Err(FwdFault::UnknownVchid {
            gpu: GpuId::ZERO,
            vchid: CP_GR_VCHID,
        }),
        "the EXACT fault, and it is not `NoVas` — this is the arm the design's one-liner \
         folds together",
    );

    // (3) ★★ The design's arm proper: the flags survive, the declared handles do not.
    //     The channel routes, materializes with no VAS, and the doorbell faults by name.
    let (vas_stripped, stripped_pdb) = run(flags_only);
    assert_eq!(
        stripped_pdb,
        Some(None),
        "★ no declared VASpace ⇒ the channel materializes with no VAS — deferred, not \
         guessed",
    );
    let cid = gpu_cid_at(CP_GR_VCHID, &prefix, flags_only);
    assert_eq!(
        vas_stripped,
        Err(FwdFault::NoVas(cid)),
        "★★ at ring time there is no 'later' — the EXACT `NoVas`, never a served ring",
    );
}

/// The `ChanId` a channel materializes at, for the exact-variant assertion above.
fn gpu_cid_at(vchid: VChid, prefix: &RpcScript, channel: RmEvent) -> kayfabe_core::ChanId {
    let mut gpu = fresh_gpu();
    {
        let mut policy = GraphPolicy::new(abi(), &mut gpu);
        for out in deliver_all(&mut policy, &prefix.messages()) {
            let _ = out.expect("the prefix is legal");
        }
    }
    gpu.apply(channel).expect("the channel applies");
    gpu.spine.by_vchid[&(GpuId::ZERO, vchid)].1
}

/// The three arms of `resolve_channel_vas`, each driven from bytes: a channel finds its
/// VASpace through its **own** handle, through its **CtxShare**, or through its **parent
/// TSG** — and through nothing at all when none of the three declares one.
#[test]
fn a_channels_vaspace_resolves_through_all_three_declared_paths() {
    // (own, ctxshare, tsg) -> does the channel end up with the PDB?
    let case = |chan_vas: u32, chan_ctxshare: u32, tsg_vas: u32, ctxshare_vas: u32| {
        let mut s = RpcScript::new();
        s.client_root(w::NV01_ROOT, cp::C, cp::PID)
            .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
            .vaspace(cp::C, cp::DEV, cp::VAS)
            .tsg(cp::C, cp::DEV, cp::TSG, tsg_vas)
            .ctxshare(cp::C, cp::VAS, cp::CTXSHARE, ctxshare_vas)
            .channel(cp::C, cp::TSG, cp::GR, gr_flags(), chan_ctxshare, chan_vas);
        let gpu = gpu_from_script(&s);
        let (pid, cid) = gpu.spine.by_vchid[&(GpuId::ZERO, CP_GR_VCHID)];
        gpu.procs[&pid].channels[&cid].vas_pdb
    };

    assert_eq!(
        case(cp::VAS, 0, 0, 0),
        Some(CP_PDB),
        "own `hVASpace` — first in the precedence",
    );
    assert_eq!(
        case(0, cp::CTXSHARE, 0, cp::VAS),
        Some(CP_PDB),
        "through the CtxShare",
    );
    assert_eq!(
        case(0, 0, cp::VAS, 0),
        Some(CP_PDB),
        "through the parent TSG",
    );
    assert_eq!(
        case(0, 0, 0, 0),
        None,
        "★ nothing declares a VASpace ⇒ nothing is resolved, and no path invents one",
    );
}

/// ★★ **The statelessness canary, extended below the root** (§3.3). RM recycles object
/// handles by design, so the same `hObject` VALUE is re-declared as a **different class**
/// after its predecessor is freed — a per-handle memo would answer the first class for
/// the second alloc, and a seen-set would refuse the recycle outright.
///
/// It also carries the idempotent-resend arm: the identical channel message twice is
/// accepted, which a dedup cache would break in the other direction.
#[test]
fn an_object_handle_recycled_as_a_different_class_is_translated_afresh() {
    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), &mut gpu);

    let mut boot = RpcScript::new();
    boot.client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS);
    for out in deliver_all(&mut policy, &boot.messages()) {
        let _ = out.expect("the boot prefix is legal");
    }

    // The value 0x5c00_0012 is first a TSG…
    let mut as_tsg = RpcScript::new();
    as_tsg.tsg(cp::C, cp::DEV, cp::TSG, cp::VAS);
    let tsg_msgs = as_tsg.messages();
    assert_eq!(
        policy.deliver(&command(&tsg_msgs[0])),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(cp::C),
            parent: HObject(cp::DEV),
            handle: HObject(cp::TSG),
            class: ClassId(w::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                ..Default::default()
            },
        })),
    );
    // …an identical re-send is accepted idempotently (a dedup cache would refuse it)…
    assert_eq!(
        policy.deliver(&command(&tsg_msgs[0])),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(cp::C),
            parent: HObject(cp::DEV),
            handle: HObject(cp::TSG),
            class: ClassId(w::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                ..Default::default()
            },
        })),
        "★ the identical message maps to the identical event — that is what makes \
         `RmGraphError::ConflictingAlloc`'s retried-RPC tolerance reachable",
    );

    // …then it is freed and re-declared as a CHANNEL, with different facts.
    let mut recycle = RpcScript::new();
    recycle
        .free(cp::C, cp::TSG)
        .channel(cp::C, cp::VAS, cp::TSG, ce_flags(), 0, cp::VAS);
    let recycle_msgs = recycle.messages();
    assert_eq!(
        policy.deliver(&command(&recycle_msgs[0])),
        Ok(Translation::Event(RmEvent::Free {
            client: HClient(cp::C),
            handle: HObject(cp::TSG),
        })),
    );
    assert_eq!(
        policy.deliver(&command(&recycle_msgs[1])),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(cp::C),
            parent: HObject(cp::VAS),
            handle: HObject(cp::TSG),
            class: ClassId(w::AMPERE_CHANNEL_GPFIFO_A),
            facts: AllocFacts {
                h_vaspace: Some(HObject(cp::VAS)),
                userd_flags: ce_flags(),
                ..Default::default()
            },
        })),
        "★★ the recycled value is translated from THIS message's bytes — a memo keyed on \
         `hObject` would still be answering `KEPLER_CHANNEL_GROUP_A`",
    );
    assert!(policy.census().is_empty(), "nothing here is a refusal");
}

/// ★ The whole compute subgraph, through the **real command ring** and the boot FSM —
/// the B2 transport, now carrying B3's traffic. Nine allocs, one doorbell, no refusals.
#[test]
fn the_compute_subgraph_reaches_the_graph_through_the_real_transport() {
    let script = script_compute();
    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    assert!(
        run.census.is_empty(),
        "a conforming compute stream refuses nothing: {:?}",
        run.census
    );
    assert_eq!(run.applied, 9, "nine allocs, all applied");
    assert_eq!(run.inert, 0);
    assert!(
        run.transitions.contains(&Transition::Running),
        "the FSM reached Running"
    );
    assert_eq!(
        run.replies.len(),
        script.steps().len(),
        "every command was answered on (function, sequence)"
    );

    gpu.apply(set_page_dir()).expect("the PDB binds");
    assert_eq!(
        boundaries(&gpu),
        boundaries(&gpu_from_script(&script)),
        "★ the ring and the direct path reach the same object model",
    );
}
