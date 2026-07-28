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
use kayfabe_arch::ids::{ClassId, HClient, HObject};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{Boundaries, NO_CONDEMNED, project};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraphError};
use kayfabe_gsp::{RpcCommand, RpcFunction, Transition};
use kayfabe_mocks::{MockIsolateFactory, WireClassArch, mock_classes};
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

    // Non-vacuity for the ignoring itself: the same lie under a NON-root class is not
    // ignored — it is refused, because this stage has no decoder for that class.
    body[12..16].copy_from_slice(&w::NV01_DEVICE_0.to_le_bytes());
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_ALLOC, 1, &body)),
        Err(BridgeRefusal::UnmappedAllocClass {
            class: w::NV01_DEVICE_0
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
            w::NV01_DEVICE_0
        )),
        Err(BridgeRefusal::ReservedClient),
    );
    // Namespace fixed -> the encoding.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_SERIALIZED,
            999,
            w::NV01_DEVICE_0
        )),
        Err(BridgeRefusal::SerializedParams {
            class: w::NV01_DEVICE_0
        }),
    );
    // Encoding fixed -> the bounds.
    assert_eq!(
        xlate(&all_wrong(
            HEX_CLIENT,
            w::RMAPI_RPC_FLAGS_NONE,
            999,
            w::NV01_DEVICE_0
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
            w::NV01_DEVICE_0
        )),
        Err(BridgeRefusal::UnmappedAllocClass {
            class: w::NV01_DEVICE_0
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
                    w::NV01_DEVICE_0,
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
                    w::NV01_DEVICE_0,
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
