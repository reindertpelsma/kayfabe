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
use kayfabe_arch::ids::{ClassId, ControlCmd, HClient, HObject, VChid};
use kayfabe_arch::{
    Arch, ClientKind, DoorbellTarget, GmmuFmt, ObjectKind, PushbufferAbi, UserdModel,
};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::project::{Boundaries, NO_CONDEMNED, project};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_gsp::{RpcCommand, RpcFunction};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_rmrpc::{BridgeRefusal, Translation, translate};
use kayfabe_tests::gspworld::FUNCTIONS;
use kayfabe_tests::rpcwire::{self as w, fn_id};
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
// ★ An `Arch` that speaks NVIDIA's real class ids.
//
// `MockArch`'s class plan is deliberately unlike NVIDIA's ("any core code that secretly
// assumes a real bit layout fails these tests"), which is right — and it means the mock
// classifies `NV01_ROOT` (0x0) as `Unknown`, so a graph driven from real wire bytes would
// declare no namespace at all. The design doc's B1 row says "a real `Gpu` (mock arch /
// isolate)" without noticing that gap.
//
// The shim is the smallest thing that closes it: **only `classify` is overridden**, and
// only for the classes this stage's fixtures carry. Every other seam — MMU, USERD,
// doorbell, pushbuffer, Case-2 controls — still comes from `MockArch`, so nothing about a
// real GPU has crept into the test either.
// ---------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct WireClassArch(MockArch);

impl Arch for WireClassArch {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn classify(&self, class: ClassId) -> ObjectKind {
        match class.0 {
            w::NV01_ROOT | w::NV01_ROOT_CLIENT => ObjectKind::Client,
            w::NV01_DEVICE_0 => ObjectKind::Device,
            other => self.0.classify(ClassId(other)),
        }
    }
    fn vchid_from_userd_flags(&self, flags: u32) -> VChid {
        self.0.vchid_from_userd_flags(flags)
    }
    fn decode_doorbell(&self, token: u64) -> Option<DoorbellTarget> {
        self.0.decode_doorbell(token)
    }
    fn mmu(&self) -> &dyn GmmuFmt {
        self.0.mmu()
    }
    fn userd(&self) -> &dyn UserdModel {
        self.0.userd()
    }
    fn is_case2_control(&self, cmd: ControlCmd) -> bool {
        self.0.is_case2_control(cmd)
    }
    fn pushbuffer(&self) -> &dyn PushbufferAbi {
        self.0.pushbuffer()
    }
}

fn fresh_gpu() -> kayfabe_tests::Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    kayfabe_tests::Guarded::new(
        "rmrpc_bridge::fresh_gpu",
        Gpu::new(Box::new(WireClassArch::default()), Box::new(factory), gpa)
            .expect("device realizes"),
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
