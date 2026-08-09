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
//! 1. `kayfabe_tests::rpcwire` — a builder written from
//!    `src/nvidia/generated/g_rpc-structures.h`, transcribed from **both** vendored tags
//!    (`ogkm-580:` and `ogkm-610:`; the path is the same in each tree, the line numbers
//!    are not), in a file that imports **nothing**, with each offset a literal beside its
//!    header line;
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

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_abi::capability::{Denial, DeniedBecause, PassthroughRule};
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_abi::wire::AbiError;
use kayfabe_abi::{ClientKindRuleUnknown, GuestOs};
use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{Boundaries, NO_CONDEMNED, ProjectionError, project};
use kayfabe_core::rmgraph::{AllocFacts, GpFifoRing, NodeKey, ResourceKey, RmEvent, RmGraphError};
use kayfabe_fwd::{FwdFault, handle_doorbell};
use kayfabe_gsp::{RpcCommand, RpcFunction, Transition};
use kayfabe_mocks::{MockArch, MockIsolateFactory, WireClassArch, mock_classes};
use kayfabe_rmrpc::{
    BridgeRefusal, GraphPolicy, ReasmLimits, Reassembled, Reassembler, RefusalCensus, Translation,
    translate,
};
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
        delivered: Vec::new(),
    }
}

/// `translate` over a whole message.
fn xlate(msg: &[u8]) -> Result<Translation, BridgeRefusal> {
    translate(abi(), GuestOs::Linux, &command(msg))
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

/// Handles for the one modelled control. Named apart from the other fixtures because a
/// `SET_PAGE_DIRECTORY` carries **three** handles in three different places, and the whole
/// class of bug this stage can have is putting one of them where another belongs.
mod spd {
    /// The client the control is issued in — the RPC body's `hClient`.
    pub const C: u32 = 0xc1d0_0071;
    /// The Device the control is issued **against** — the RPC body's `hObject`. Dropped:
    /// `RmEvent::SetPageDir` has nowhere to put it.
    pub const DEV: u32 = 0x5c00_0001;
    /// The VASpace the page directory belongs to — a **params** field, not a header one.
    pub const VAS: u32 = 0x5c00_0010;
    /// The page-directory base.
    pub const PDB: u64 = 0x0000_0003_4100_0000;
}

/// A `GSP_RM_CONTROL`/`SET_PAGE_DIRECTORY` message, built by the independent builder.
fn set_page_dir_msg(h_client: u32, h_device: u32, h_vaspace: u32, pdb: u64, flags: u32) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_CONTROL,
        3,
        &w::control_body(
            h_client,
            h_device,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(pdb, 512, flags, h_vaspace, 0, 1, 0),
        ),
    )
}

/// The event a `SET_PAGE_DIRECTORY` must produce — **written by hand**, from
/// `gsp_core_bridge.md` §2.5's mapping, never derived from the decoder.
fn expected_set_page_dir(client: u32, vaspace: u32, pdb: u64) -> RmEvent {
    RmEvent::SetPageDir {
        client: HClient(client),
        vaspace: HObject(vaspace),
        pdb: Pdb(pdb),
    }
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
    project(&gpu.spine.rmgraph, gpu.spine.arch(), &NO_CONDEMNED).expect("projects")
}

/// Translate and apply one message, returning whatever refused first.
fn drive(gpu: &mut Gpu, msg: &[u8]) -> Result<(), BridgeRefusal> {
    match xlate(msg)? {
        Translation::Event(ev) => {
            gpu.apply(ev).expect("the graph accepts this fixture");
            Ok(())
        }
        Translation::Inert => Ok(()),
        // ★ Unreachable, and asserted rather than swallowed: `drive` goes through the
        // free function, which holds no state and therefore cannot produce a `Held`.
        // `translate_never_holds` is the general form of this; a `_ =>` here would let a
        // regression that moved reassembly into `translate` pass silently.
        Translation::Held => panic!("`translate` has no state and cannot hold a fragment"),
        // ★ The ADDRESS plane, not the graph. Applied through the same core entry point
        // `GraphPolicy` uses, so a fixture driven through `drive` sees the same
        // bindings a guest would — and a promotion the join refuses surfaces here as a
        // panic rather than as a silently green fixture.
        Translation::CtxPromotion(p) => {
            gpu.promote_ctx(&p).expect("the join accepts this fixture");
            Ok(())
        }
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
        // ★ fn 64 joined the named set at task #127: `RmRpcSetGuestSystemInfo` tail-calls
        // it and returns ITS status (`ogkm-580: rpc.c:8825-8832`), so a port that names
        // fn 1 and not fn 64 fails `RmInitAdapter` one line further on. It carries no
        // object-model content either — a driver-branch string and a bus address — so it
        // is inert here and answered in `kayfabe_device::guestsysinfo`.
        (64, "the fn-1 tail call; a version string and a bus address"),
        // ★★★ fn 70 joined the named set at `#149`, and it is the sharpest entry in this
        // list: `UPDATE_BAR_PDE` is inert **to the object model** and very far from inert
        // to the device. It carries a bus aperture's ROOT PAGE-DIRECTORY ENTRY — no
        // client, no handle, no class — so an `RmGraph` has nothing to record; and
        // `kayfabe_device::bar2` latches it, without which the translated BAR2 window is
        // rooted at nothing and `kbusVerifyBar2` fails NV_ERR_MEMORY_ERROR. Two planes,
        // one message, and this one is not the plane that acts on it.
        (
            70,
            "a bus aperture's root PDE; the DEVICE acts on it, not the graph",
        ),
    ] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Ok(Translation::Inert),
            "fn {code} is inert ({what})",
        );
    }

    // ★★ B6 emptied the "known, mapped, arm not built" state, and this is where that is
    // visible: `GSP_RM_CONTROL` left it at B4, `DUP_OBJECT` at B5, and
    // `CONTINUATION_RECORD` at B6 — not by becoming a translating arm, but by becoming a
    // *transport* one. A continuation carries no function of its own, so from
    // `translate`'s one-message view there is never a head in flight and the answer is
    // the no-head refusal. It is emphatically **not** `UnknownFunction` (the id is known)
    // and not `Inert` (the fact matters).
    assert_eq!(
        xlate(&w::message(fn_id::CONTINUATION_RECORD, 5, &[0u8; 48])),
        Err(BridgeRefusal::ContinuationWithoutHead {
            code: fn_id::CONTINUATION_RECORD,
        }),
    );
    // fn 21 is a fact now. A well-formed body translates; the same body with the
    // *envelope's* client zeroed is the message-level refusal, not a function-level one.
    assert_eq!(
        xlate(&w::message(
            fn_id::DUP_OBJECT,
            5,
            &w::dup_body(x::A, x::A, 0x5c00_0031, x::K, 0x5c00_0019, 0),
        )),
        Ok(Translation::Event(RmEvent::Dup {
            src: NodeKey::new(HClient(x::K), HObject(0x5c00_0019)),
            dst: NodeKey::new(HClient(x::A), HObject(0x5c00_0031)),
        })),
    );
    // fn 76 is now dispatched on its `cmd`, so it needs a well-formed body to reach the
    // command table at all. Both refusals below are about the CONTROL, not about the
    // function — and they are two different controls, because the capability gate and
    // the params table are two different questions asked in that order.
    //
    // cmd 0 is on nobody's allowlist: the boundary refuses it before decoding anything.
    assert_eq!(
        xlate(&w::message(
            fn_id::GSP_RM_CONTROL,
            5,
            &w::control_body(spd::C, spd::DEV, 0, 0, w::RMAPI_RPC_FLAGS_NONE, &[]),
        )),
        Err(BridgeRefusal::ControlNotPermitted {
            cmd: 0,
            denial: Denial::NotOnAllowlist,
        }),
    );
    // …and a command that IS allowed but has no arm is still `UnknownControl`, which is
    // what keeps the two variants from collapsing into one.
    assert_eq!(
        xlate(&w::message(
            fn_id::GSP_RM_CONTROL,
            5,
            &w::control_body(
                spd::C,
                spd::DEV,
                UNMODELLED_CMD,
                0,
                w::RMAPI_RPC_FLAGS_NONE,
                &[]
            ),
        )),
        Err(BridgeRefusal::UnknownControl {
            cmd: UNMODELLED_CMD
        }),
    );

    // Ours to send, never to receive.
    for code in [fn_id::GSP_INIT_DONE, fn_id::POST_EVENT] {
        assert_eq!(
            xlate(&w::message(code, 5, &[0u8; 16])),
            Err(BridgeRefusal::EventFromGuest { code }),
        );
    }

    // Not in the table at all — the third state.
    // ⊘ 70 left this list at `#149` — it is named now, and it moved UP into the inert
    // block above rather than out of the test.
    for code in [0u32, 2, 4, 14, 15, 27, 999, 0x1002, u32::MAX] {
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

/// `hClient == 0` is refused on **all three** verbs, before anything else about the
/// message is believed. `NV01_NULL_OBJECT` is not a namespace.
#[test]
fn a_zero_hclient_is_refused_on_every_verb() {
    assert_eq!(
        xlate(&root_alloc_msg(w::NV01_ROOT, 0, HEX_PID)),
        Err(BridgeRefusal::ReservedClient),
    );
    assert_eq!(
        xlate(&free_msg(0, HEX_OBJECT)),
        Err(BridgeRefusal::ReservedClient),
    );
    assert_eq!(
        xlate(&set_page_dir_msg(0, spd::DEV, spd::VAS, spd::PDB, 0)),
        Err(BridgeRefusal::ReservedClient),
        "a control is issued IN a namespace too",
    );
    // ★ B5: the dup's *destination* client is this message's namespace. Its **source**
    // client is not checked here — that is a cross-namespace reference and the rule that
    // owns it is `RmGraph::apply`'s, which enumerates BOTH of a dup's clients. The arm
    // below proves the source zero survives translation, and
    // `a_zero_source_client_is_refused_by_the_rule_that_owns_it` proves it is still
    // refused, named, one level down.
    assert_eq!(
        xlate(&w::message(
            fn_id::DUP_OBJECT,
            5,
            &w::dup_body(0, 0, 0x5c00_0031, x::K, 0x5c00_0019, 0),
        )),
        Err(BridgeRefusal::ReservedClient),
        "a dup is issued IN a namespace too",
    );
    assert_eq!(
        xlate(&w::message(
            fn_id::DUP_OBJECT,
            5,
            &w::dup_body(x::A, 0, 0x5c00_0031, 0, 0x5c00_0019, 0),
        )),
        Ok(Translation::Event(RmEvent::Dup {
            src: NodeKey::new(HClient(0), HObject(0x5c00_0019)),
            dst: NodeKey::new(HClient(x::A), HObject(0x5c00_0031)),
        })),
        "★ a zero SOURCE client is carried, not pre-empted — the graph owns that question",
    );
    // Non-vacuity: 1 is fine. The refusal is about zero, not about small handles.
    assert!(matches!(
        xlate(&free_msg(1, HEX_OBJECT)),
        Ok(Translation::Event(_)),
    ));
    assert!(matches!(
        xlate(&set_page_dir_msg(1, spd::DEV, spd::VAS, spd::PDB, 0)),
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

/// ★ The **same** order on the control arm, peeled one step at a time. Written out rather
/// than asserted as "it is like the alloc arm", because the two are separate functions and
/// nothing but a test makes them agree: an inconsistency here would mean the same hostile
/// message earns different names on two verbs.
///
/// The last two steps are the ones only a control has: the **command table** (unknown vs
/// known-but-unmodelled) and then the exact params size.
#[test]
fn the_control_refusal_order_matches_the_alloc_arms() {
    let all_wrong = |client: u32, flags: u32, size: u32, cmd: u32| {
        w::message(
            fn_id::GSP_RM_CONTROL,
            3,
            &w::control_body(
                client,
                spd::DEV,
                cmd,
                size,
                flags,
                &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
            ),
        )
    };
    let unknown_cmd = 0x0080_1812u32;

    // Everything wrong at once -> the namespace.
    assert_eq!(
        xlate(&all_wrong(
            0,
            w::RMAPI_RPC_FLAGS_SERIALIZED,
            999,
            unknown_cmd
        )),
        Err(BridgeRefusal::ReservedClient),
    );
    // Namespace fixed -> the encoding.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_SERIALIZED,
            999,
            unknown_cmd
        )),
        Err(BridgeRefusal::SerializedControlParams { cmd: unknown_cmd }),
    );
    // Encoding fixed -> the bounds.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_NONE,
            999,
            unknown_cmd
        )),
        Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 999,
            available: 32,
        }),
    );
    // ★ Bounds fixed -> the CAPABILITY GATE, which is a step the alloc side has too and
    // which runs BEFORE the command table. `unknown_cmd` is on no allowlist, so this is
    // as far as it gets — the port never looks up its params shape at all.
    assert_eq!(
        xlate(&all_wrong(spd::C, w::RMAPI_RPC_FLAGS_NONE, 32, unknown_cmd)),
        Err(BridgeRefusal::ControlNotPermitted {
            cmd: unknown_cmd,
            denial: Denial::NotOnAllowlist,
        }),
    );
    // Gate passed -> the command table, and the command is not in it at all.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_NONE,
            32,
            UNMODELLED_CMD
        )),
        Err(BridgeRefusal::UnknownControl {
            cmd: UNMODELLED_CMD
        }),
    );
    // A command that IS in the table, and is one we cannot express, is a DIFFERENT
    // refusal at the same step — the whole reason the table has two arms. ★ The
    // *revocation* since §14.23: `0x90f10106` used to sit here and now has a decoder, so
    // the id that still means "in the table, inexpressible" is `0x00801814`. ⊘ It also
    // has no `params_size`, which is why it refuses here rather than at the size check
    // below.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_NONE,
            32,
            w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY
        )),
        Err(BridgeRefusal::PageDirControlNotModelled {
            cmd: w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY,
        }),
    );
    // Command fixed, size still a lie -> the size.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_NONE,
            16,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY
        )),
        Err(BridgeRefusal::ControlParamsSizeMismatch {
            cmd: w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            declared: 16,
            expected: 32,
        }),
    );
    // Everything fixed -> the fact.
    assert_eq!(
        xlate(&all_wrong(
            spd::C,
            w::RMAPI_RPC_FLAGS_NONE,
            32,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY
        )),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            spd::VAS,
            spd::PDB
        ))),
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
        BridgeRefusal::EventFromGuest { code: 0x1001 },
        BridgeRefusal::UnmappedAllocClass { class: 0x80 },
        BridgeRefusal::SerializedParams { class: 0 },
        // ★ B4's five. `SerializedControlParams` sits next to `SerializedParams` on
        // purpose: they fire on the same bit of two different words, and folding them
        // would make "which controls serialize" unanswerable from a census.
        BridgeRefusal::SerializedControlParams { cmd: 0x0080_1813 },
        BridgeRefusal::UnknownControl { cmd: 0x2080_012b },
        BridgeRefusal::PageDirControlNotModelled { cmd: 0x0080_1814 },
        // ★★★ §14.23's three, and they are three because they are three different
        // diagnoses about one message: nothing to attribute it to, impossible content, and
        // a root in memory this port cannot address.
        BridgeRefusal::PublishedPdesUnnamedVaspace { cmd: 0x90f1_0106 },
        BridgeRefusal::PublishedPdesMalformed {
            cmd: 0x90f1_0106,
            err: kayfabe_abi::gvaspacepdes::ServerReservedPdesError::LevelCountOutOfRange {
                got: 7,
            },
        },
        BridgeRefusal::PublishedPdesRootAperture {
            cmd: 0x90f1_0106,
            aperture: kayfabe_abi::gvaspacepdes::GMMU_APERTURE_SYS_COH,
        },
        BridgeRefusal::ControlParamsSizeMismatch {
            cmd: 0x0080_1813,
            declared: 16,
            expected: 32,
        },
        BridgeRefusal::ImplicitVaspace,
        BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 9,
            available: 8,
        },
        BridgeRefusal::ClientHandleDisagrees {
            header: 1,
            params: 2,
        },
        BridgeRefusal::ReservedClient,
        // ★★★ The fourth axis's, added 2026-07-29. It has to be here for the same reason
        // every other one is: the guest-OS fold was invisible precisely because nothing
        // counted it, and a refusal that is not in this list is a refusal a census cannot
        // report. Its tag must be distinct from `ReservedClient`'s and from
        // `ClientHandleDisagrees`'s — all three fire on the same `NV01_ROOT` message, and
        // a census that folded them could not tell "this guest is misconfigured" from
        // "this guest sent nonsense".
        BridgeRefusal::ClientKindRuleUnknown(ClientKindRuleUnknown {
            guest_os: GuestOs::Windows,
            process_id: 0x0000_dd13,
        }),
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
        // ★ B5's, and the third distinct inner error on purpose: `DUP_OBJECT` is the
        // only verb that names two namespaces, so it is the only one that can reach
        // these. If `Faulted` were flattened they would collide with the two above.
        BridgeRefusal::Graph(GpuError::Graph(RmGraphError::ConflictingDup(NodeKey::new(
            HClient(HEX_CLIENT),
            HObject(HEX_OBJECT),
        )))),
        BridgeRefusal::Graph(GpuError::Graph(RmGraphError::ReservedClient(HClient(0)))),
        BridgeRefusal::Graph(GpuError::Graph(RmGraphError::UndeclaredClient(HClient(
            HEX_CLIENT,
        )))),
        // ★ B6's five, and they are five rather than one because they answer five
        // different questions about a fragmented message: nothing was in flight; a
        // *different* message interrupted; the head's own declared total is beyond what
        // we hold; too many fragments; the fragments carried more than the head declared.
        // A census that folded them could not tell a hostile guest from a bound that is
        // set too low, which is the only thing this refusal surface is for.
        BridgeRefusal::ContinuationWithoutHead { code: 71 },
        BridgeRefusal::ContinuationInterleaved { code: 103 },
        BridgeRefusal::ContinuationOverflow {
            declared: 65_576,
            max: 65_536,
        },
        BridgeRefusal::ContinuationCountExceeded {
            continuations: 65,
            max: 64,
        },
        BridgeRefusal::ContinuationOverrun {
            have: 200,
            declared: 168,
        },
        // ★★★ The capability gate's four, and they are FOUR rather than two because the
        // `Denial` is part of the tag. "A control we refuse by name" and "a control
        // nobody has ever seen" are the two findings a security census exists to
        // separate — the first is a guest doing something we anticipated, the second is a
        // guest exploring the surface — and the same split holds on the alloc side.
        //
        // They must also be distinct from `UnknownControl`/`UnmappedAllocClass`, which
        // are one step LATER in the same function and mean the opposite thing: permitted,
        // not yet modelled. Folding either pair would make "is the boundary holding?"
        // and "is the port finished?" the same question.
        BridgeRefusal::ControlNotPermitted {
            cmd: 0x2080_0112,
            denial: Denial::NotOnAllowlist,
        },
        BridgeRefusal::ControlNotPermitted {
            cmd: 0x2080_0122,
            denial: Denial::Refused {
                name: "NV2080_CTRL_CMD_GPU_EXEC_REG_OPS",
                why: DeniedBecause::RegisterAccess,
            },
        },
        BridgeRefusal::AllocClassNotPermitted {
            class: 0x0000_f001,
            denial: Denial::NotOnAllowlist,
        },
        BridgeRefusal::AllocClassNotPermitted {
            class: 0x0000_0071,
            denial: Denial::Refused {
                name: "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR",
                why: DeniedBecause::CallerMemoryDescriptor,
            },
        },
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
        // ★★ B4 settled §4.2's `[open]`: still ONE value, now with the constraint that
        // decided it. `_issueRpcAndWait` returns an `rpc_result` verbatim only while it
        // is BELOW the VMIOP base; at or above it, every distinct value collapses to one
        // indistinguishable `NV_ERR_GENERIC` (the `DRF_BASE(NV_VGPU_MSG_RESULT__VMIOP)`
        // test and the fall-through, identical at both tags:
        // `ogkm-580: rpc.c:2004-2007`, `ogkm-610: :2023-2026`). So a status the
        // guest can actually read is a *property* of the choice, not a preference — and
        // it is the property that ruled out `NV_VGPU_MSG_RESULT_RPC_API_CONTROL_NOT_SUPPORTED`
        // (`0xFF100009`), whose translation back to an `NV_STATUS` exists only on the
        // vGPU `RM_API_CONTROL` path and not on fn 76.
        assert!(
            r.rpc_result() < kayfabe_abi::NV_VGPU_MSG_RESULT_VMIOP_BASE,
            "{:?} would collapse to NV_ERR_GENERIC before the guest could read it",
            r.fault_tag(),
        );
        assert_eq!(r.rpc_result(), 0x56, "NV_ERR_NOT_SUPPORTED");
    }
    // Non-vacuity for the bound: it is a real constraint, and these two values fail it.
    for collapsed in [0xFF00_0000u32, 0xFF10_0001] {
        assert!(collapsed >= kayfabe_abi::NV_VGPU_MSG_RESULT_VMIOP_BASE);
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
            // ★ B6 vacated this slot in turn. A bare continuation record with nothing in
            // flight is still hostile traffic — it is simply hostile for a *named*
            // reason now, and the "known, mapped, not built" state it used to represent
            // is empty.
            w::message(fn_id::CONTINUATION_RECORD, 1, &[0u8; 48]),
            FaultTag("BridgeRefusal::ContinuationWithoutHead"),
        ),
        (
            // A dup whose ENVELOPE client is `NV01_NULL_OBJECT`. The message has no
            // namespace to be attributed to, which is a property of the message and needs
            // no graph — so it is refused here, on the same rule as every other verb.
            w::message(
                fn_id::DUP_OBJECT,
                1,
                &w::dup_body(0, 0, 0x5c00_0031, K, 0x5c00_0019, 0),
            ),
            FaultTag("BridgeRefusal::ReservedClient"),
        ),
        // ★ B4's five, in the same stream. The point of putting them here rather than
        // only in their own tests is the *interleaving*: each one sits between two valid
        // messages, so a control arm that corrupted the run would show up as a broken
        // projection below rather than as a wrong variant here.
        (
            w::message(
                fn_id::GSP_RM_CONTROL,
                1,
                &w::control_body(
                    A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                    32,
                    w::RMAPI_RPC_FLAGS_SERIALIZED,
                    &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
                ),
            ),
            FaultTag("BridgeRefusal::SerializedControlParams"),
        ),
        (
            w::message(
                fn_id::GSP_RM_CONTROL,
                1,
                // ★ Was `0x2080_012b` until that control gained a decoder (`#93`); the
                // finding this row carries is "permitted, and no arm for it", which
                // `GR_GET_CTX_BUFFER_INFO` now supplies.
                &w::control_body(A, spd::DEV, 0x2080_1219, 0, w::RMAPI_RPC_FLAGS_NONE, &[]),
            ),
            FaultTag("BridgeRefusal::UnknownControl"),
        ),
        (
            w::message(
                fn_id::GSP_RM_CONTROL,
                1,
                // ★ The revocation since §14.23 — see the sibling test.
                &w::control_body(
                    A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY,
                    0,
                    w::RMAPI_RPC_FLAGS_NONE,
                    &[],
                ),
            ),
            FaultTag("BridgeRefusal::PageDirControlNotModelled"),
        ),
        (
            w::message(
                fn_id::GSP_RM_CONTROL,
                1,
                &w::control_body(
                    A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                    16,
                    w::RMAPI_RPC_FLAGS_NONE,
                    &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
                ),
            ),
            FaultTag("BridgeRefusal::ControlParamsSizeMismatch"),
        ),
        (
            set_page_dir_msg(A, spd::DEV, 0, spd::PDB, 0),
            FaultTag("BridgeRefusal::ImplicitVaspace"),
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
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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

/// The policy's answer on the wire: an **explicit** `NV_OK` acknowledgement for anything it
/// accepted, and `Some(Reply)` with a non-zero status for anything it refused.
///
/// ★★★ The accepted arm used to be `None`, because `None` used to mean *"the FSM posts its
/// own `ack(NV_OK)`"*. Task #127 gave `None` one meaning — **I decline**, answered by the
/// FSM with a named refusal — so a policy that accepts a command now has to say so. This
/// test is what fails if the two ever collapse back into one word: with `None` restored on
/// the accepted arm, every command this bridge applies would reach the guest as
/// `NV_ERR_NOT_SUPPORTED`.
#[test]
fn an_accepted_command_is_acked_explicitly_and_a_refused_one_is_answered_here() {
    use kayfabe_gsp::{CommandPolicy, Reply};

    let mut gpu = fresh_gpu();
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);

    let accepted = command(&HEX_ROOT_ALLOC);
    assert_eq!(
        policy.respond(&accepted),
        Some(Reply {
            rpc_result: 0,
            body: accepted.payload.clone(),
        }),
        "an accepted fact is acknowledged BY NAME — never by declining to answer",
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
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
            let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
    /// ★ B6's third counter — see `GraphPolicy::held`. The non-vacuity instrument for
    /// every fragmentation claim: a run that reached the same graph while holding
    /// nothing never fragmented anything.
    held: u64,
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
    let mut policy = GraphPolicy::new(profile.table(), GuestOs::Linux, gpu);

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
        held: policy.held(),
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
        let mut policy = GraphPolicy::new(P580.table(), GuestOs::Linux, &mut gpu);
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

    // One hostile message per refusal reason this stage can produce — including the two
    // the GRAPH produces, which B1 had no way to reach, and B4's five, which arrive
    // through a real ring for the first time here.
    let hostile: Vec<(w::Step, FaultTag)> = vec![
        (
            w::Step {
                function: 999,
                body: vec![0u8; 8],
            },
            FaultTag("BridgeRefusal::UnknownFunction"),
        ),
        (
            // ★ B6 vacated this slot — see the sibling test. Through the ring the
            // refusal is the same one, which is the point: a continuation with no head is
            // a property of the *stream*, and the stream is what a ring delivers.
            w::Step {
                function: fn_id::CONTINUATION_RECORD,
                body: vec![0u8; 48],
            },
            FaultTag("BridgeRefusal::ContinuationWithoutHead"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                body: w::control_body(
                    x::A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                    32,
                    w::RMAPI_RPC_FLAGS_SERIALIZED,
                    &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
                ),
            },
            FaultTag("BridgeRefusal::SerializedControlParams"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                // ★ Was `0x2080_012b`; see the sibling test — it is decoded now.
                body: w::control_body(x::A, spd::DEV, 0x2080_1219, 0, w::RMAPI_RPC_FLAGS_NONE, &[]),
            },
            FaultTag("BridgeRefusal::UnknownControl"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                // ★ The revocation since §14.23 — see the sibling test.
                body: w::control_body(
                    x::A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY,
                    0,
                    w::RMAPI_RPC_FLAGS_NONE,
                    &[],
                ),
            },
            FaultTag("BridgeRefusal::PageDirControlNotModelled"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                body: w::control_body(
                    x::A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                    16,
                    w::RMAPI_RPC_FLAGS_NONE,
                    &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
                ),
            },
            FaultTag("BridgeRefusal::ControlParamsSizeMismatch"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                body: w::control_body(
                    x::A,
                    spd::DEV,
                    w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                    32,
                    w::RMAPI_RPC_FLAGS_NONE,
                    // hVASpace = 0: the client/device pair's IMPLICIT VAS, which this
                    // port has no node for.
                    &w::set_page_dir_params(spd::PDB, 512, 0, 0, 0, 1, 0),
                ),
            },
            FaultTag("BridgeRefusal::ImplicitVaspace"),
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
        // ★★ B5's three, and all three are refusals **only the dup verb can reach**,
        // because it is the one event that names TWO client namespaces.
        (
            // The DESTINATION namespace was never declared. `undeclared_namespace` names
            // `dst` first on purpose — it is the squat vector — so this is the arm that
            // fires even though the source is undeclared too.
            w::Step {
                function: fn_id::DUP_OBJECT,
                body: w::dup_body(0xfeed_0001, 0, 0x5c00_0031, 0xfeed_0002, 0x5c00_0019, 0),
            },
            FaultTag("RmGraphError::UndeclaredClient"),
        ),
        (
            // ★ `hClientSrc == 0`. The bridge deliberately does NOT check this: the
            // envelope's client is the message's attribution and the source client is a
            // *reference*, whose validation belongs to the one rule that owns every
            // namespace question. It still reaches the guest, named — which is the whole
            // argument for not making a second local copy of the rule.
            w::Step {
                function: fn_id::DUP_OBJECT,
                body: w::dup_body(x::B, 0, 0x5c00_0031, 0, 0x5c00_0019, 0),
            },
            FaultTag("RmGraphError::ReservedClient"),
        ),
        (
            // A destination handle that is already bound to a DIFFERENT resource: B's
            // own client-root handle, aliased onto the kernel client's root. An identical
            // re-send would be idempotent; this is not one.
            w::Step {
                function: fn_id::DUP_OBJECT,
                body: w::dup_body(x::B, x::B, x::B, x::K, x::K, 0),
            },
            FaultTag("RmGraphError::ConflictingDup"),
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
        .engine_object(cp::C, cp::CE, cp::CE_OBJ, w::AMPERE_DMA_COPY_B)
        // ★★ B4. The VASpace acquires its `Pdb` from a real `SET_PAGE_DIRECTORY`, issued
        // against the **Device** with the VASpace named in a params field — which is how
        // the driver issues it (`ogkm-580: dma.c:508-518`) and is the shape that makes a
        // header/params mix-up visible.
        .set_page_dir(cp::C, cp::DEV, cp::VAS, CP_PDB.0, w::PDB_FLAGS_ALL_CHANNELS);
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
            // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
            // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
            // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
            // declared no ring at all, which no channel does.
            gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
            // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
            // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
            // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
            // declared no ring at all, which no channel does.
            gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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

/// The PDB the compute VAS gets.
///
/// ★★ **B4 moved this to the byte side.** At B3 it was applied as a raw `RmEvent` on both
/// sides of the oracle, because translating `GSP_RM_CONTROL` was a later stage and B3 was
/// not entitled to pretend otherwise. It is now a `SET_PAGE_DIRECTORY` message in
/// [`script_compute`], decoded like everything else — so `set_page_dir` below survives
/// only as the **hand-written reference**, which is exactly the role it should have.
const CP_PDB: Pdb = Pdb(0x0034_1000);

/// **Transcription #2** of the PDB declaration: the `RmEvent`, written out by hand from
/// `gsp_core_bridge.md` §2.5's mapping. The byte side is `script_compute`'s
/// `.set_page_dir(…)`, and neither is derived from the other.
fn set_page_dir() -> RmEvent {
    RmEvent::SetPageDir {
        client: HClient(cp::C),
        vaspace: HObject(cp::VAS),
        pdb: CP_PDB,
    }
}

/// Drive a script through the policy onto a fresh device.
///
/// ★ No raw `gpu.apply` any more: a compute script now carries its own
/// `SET_PAGE_DIRECTORY`, so every fact in the resulting graph — including the data-plane
/// identity the whole address plane routes on — arrived as **wire bytes** through the
/// bridge. That is B4's headline in one function.
fn gpu_from_script(script: &RpcScript) -> kayfabe_tests::Guarded<Gpu> {
    let mut gpu = fresh_gpu();
    {
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
        for (i, out) in deliver_all(&mut policy, &script.messages())
            .into_iter()
            .enumerate()
        {
            let _ = out.unwrap_or_else(|e| panic!("message {i} of the script refused: {e:?}"));
        }
        assert!(policy.census().is_empty(), "a clean script refuses nothing");
    }
    // ★ #177 — a real guest schedules every channel it allocates before ringing it
    // (`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`); this harness models the whole rest of a
    // script from wire bytes but has never modelled that control. Every caller of this
    // helper that goes on to ring a doorbell needs it, and a caller that only inspects
    // `boundaries(&gpu)` is unaffected — `exec.requested` plays no part in a projection.
    kayfabe_tests::guest_schedules_every_channel(&mut gpu);
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
/// +84  flags               00 41 20 00   -> 0x0020_4100      (params +20) ★
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
    0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x5c,
];

/// ★ **Transcription #1 vs #2, for the channel.** Two humans, two methods, one byte
/// string. The hex above was written from `ogkm`'s header with a ruler; the builder was
/// written from the same header independently; if they disagree, one reading is wrong,
/// which is the only reason to write ninety-six bytes out by hand.
#[test]
fn the_hand_written_hex_channel_and_the_independent_builder_agree_byte_for_byte() {
    // The encoded flags word is part of the fixture, so it is pinned as a literal too —
    // otherwise the hex would silently follow the mock arch's packing wherever it went.
    // ★★ 0x0020_4100 and not the old 0x1085: `MockArch` used to pack the chid into one
    // contiguous invented field, and now encodes exactly as CPU-RM does —
    // `_PAGE_FIXED` set, `_PAGE_VALUE = 0x21 / 8 = 4`, `_VALUE = 0x21 % 8 = 1`
    // (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2793-2802`, and
    // `tests/tests/userd_chid_oracle.rs` differentials both directions against that
    // span COMPILED). So this fixture became MORE faithful, not less.
    assert_eq!(
        gr_flags(),
        0x0020_4100,
        "MockArch::userd_flags_for(VChid(0x21))"
    );
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
                userd_flags: 0x0020_4100,
                // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
                // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
                // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
                // declared no ring at all, which no channel does.
                gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
                // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
                // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
                // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
                // declared no ring at all, which no channel does.
                gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
                ..Default::default()
            }
        ),
        "Channel: all three, from +20/+24/+28",
    );
    // ★ `AMPERE_B` joins the two on 2026-08-08 (`execution_plane_increments.md` §14.26).
    // Its params, when a caller supplies them at all, are `NV_GR_ALLOCATION_PARAMETERS`
    // — `{version, flags, size, caps}`, four NvU32, no handle and no pointer
    // (`ogkm-580: nvos.h:2716-2721`) — so `[0xff; 64]` is exactly as readable as a
    // well-formed one, and the assertion below is the statement that neither is read.
    for class in [w::AMPERE_COMPUTE_B, w::AMPERE_DMA_COPY_B, w::AMPERE_B] {
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
            // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
            // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
            // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
            // declared no ring at all, which no channel does.
            gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
            // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
            // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
            // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
            // declared no ring at all, which no channel does.
            gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
        // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
        // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
        // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
        // declared no ring at all, which no channel does.
        gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
            AllocFacts {
                error_notifier: None,
                // ★★★ §16.16 — masked for `error_notifier`'s reason and no other: both are
                // **version-pinned** past-prefix decoders that a *read* tree bought the
                // right to run (`ChannelNotifierWire` / `ChannelUserdWire`), so of course
                // they move with the tail — that is their job. ⊘ The mask would be a
                // WEAKENING if it stopped there, because a field excluded from an equality
                // is a field this test can no longer see at all. It does not stop there:
                // the assertion below requires `userd` to actually TRACK the tail, so the
                // exclusion here is paid for by a positive check rather than by a hole.
                userd: None,
                ..facts_of(&long)
            },
            want,
            "★ {extra} bytes of tail changed a PREFIX fact — the decoder is reading past \
             the region the two vendored trees agree on",
        );
    }

    // ★★★★ §16.16 — AND THE PAST-PREFIX DECODER MUST ACTUALLY BE REACHABLE.
    //
    // ⊘ This half exists because of what §16.15 was caught doing: an instrument built
    // complete, committed green, and wired to **nothing** — `write_tagged` had no caller
    // anywhere in the repo, so a boot of it would have measured only its own default. A
    // decoder that is masked out of the only test that exercises its input is one edit away
    // from the same state, and the mask above would hide it perfectly.
    //
    // So: a tail long enough to contain `userdOffset[0]` must produce the tail's bytes,
    // and a tail too short must produce `None`. Together they prove the field is read, is
    // value-dependent, and refuses rather than zero-extends.
    let mut short = exact.clone();
    short.extend(std::iter::repeat_n(0xffu8, 4));
    assert_eq!(
        facts_of(&short).userd,
        None,
        "⊘ params that stop before `userdOffset[0]` must yield None — a decoder that \
         zero-extended here would report a USERD the guest never declared",
    );
    let mut full = exact.clone();
    full.extend(std::iter::repeat_n(0xffu8, 512));
    let got = facts_of(&full)
        .userd
        .expect("a tail this long carries the fields");
    assert_eq!(
        (got.handle, got.offset),
        (0xffff_ffff, 0xffff_ffff_ffff_ffff),
        "★ the USERD decoder must read the TAIL's bytes — if this reports zeros it is not \
         reading the field at all, and every USERD line in a boot log would be an artefact \
         of the decoder rather than a fact about the guest",
    );
}

/// ★★★ The **one** field that is read past the prefix, and the exact terms it is read on.
///
/// `error_notifier` breaks the invariant above on purpose (`kayfabe_abi::notifier`): the
/// GSP is the component contracted to write a channel's error notifier, and
/// `errorNotifierMem` is how CPU-RM tells it where. So the prefix contract is not
/// *"nothing past +32 is ever read"* any more — it is:
///
/// - the three **protocol facts** the graph keys on are still prefix-only, which the test
///   above pins by masking exactly one field out and nothing else;
/// - the notifier is read **only from an offset a vendored tree was opened at**, and
/// - at a boundary with no such tree, **nothing past +32 is read at all** — the original
///   invariant, preserved precisely where the version disagreement lives.
///
/// That third clause is the one worth a test: without it a `None` at 550 would be
/// indistinguishable from a decode that happened to find zeros.
#[test]
fn the_notifier_is_the_only_field_past_the_prefix_and_only_where_a_tree_was_read() {
    use kayfabe_abi::notifier::ChannelNotifierWire;
    use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
    use kayfabe_arch::fault::ErrorNotifier;

    // A full-length 580 params image declaring a sysmem notifier at a recognisable GPA.
    let w580 = ChannelNotifierWire::V580;
    let mut params = w::channel_params(gr_flags(), cp::CTXSHARE, cp::VAS);
    params.resize(w580.needs(), 0);
    // internalFlags[3:2] = ERROR_NOTIFIER_TYPE_MEMORY (3)
    params[w580.internal_flags..w580.internal_flags + 4]
        .copy_from_slice(&(3u32 << 2).to_le_bytes());
    let m = w580.error_notifier_mem;
    params[m..m + 8].copy_from_slice(&0x7fee_0000u64.to_le_bytes());
    params[m + 8..m + 16].copy_from_slice(&64u64.to_le_bytes()); // size
    params[m + 16..m + 20].copy_from_slice(&1u32.to_le_bytes()); // NV_ADDR_SYSMEM

    // The bench boundary HAS a pinned layout, so it learns the address.
    let bench = table_for(BENCH_DRIVER).expect("bench");
    assert_eq!(
        bench.decode_channel_error_notifier(&params),
        Ok(Some(ErrorNotifier::Sysmem { gpa: 0x7fee_0000 })),
        "580.159.04's tree was read, so the field is readable"
    );

    // 550.54.04 has no pinned layout. The IDENTICAL bytes yield nothing — not a guess, and
    // not a zero.
    let old = table_for(kayfabe_abi::DriverVersion {
        major: 550,
        minor: 54,
        patch: 4,
    })
    .expect("550 is supported");
    assert_eq!(
        old.decode_channel_error_notifier(&params),
        Ok(None),
        "★★ no tree was opened at 550.54.04, so nothing past the prefix is read there"
    );

    // ★★★ And the same bytes, through the WHOLE bridge, land on the event the graph
    // consumes. Without this the decode could be correct and simply not wired: `verdict`
    // would then escalate every fault as `Undeclared` and nothing would look wrong.
    let Ok(Translation::Event(RmEvent::Alloc { facts, .. })) = xlate(&w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            cp::C,
            cp::TSG,
            cp::GR,
            w::AMPERE_CHANNEL_GPFIFO_A,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            &params,
        ),
    )) else {
        panic!("a full-length channel alloc is an Alloc event");
    };
    assert_eq!(
        facts.error_notifier,
        Some(ErrorNotifier::Sysmem { gpa: 0x7fee_0000 }),
        "★ the declared notifier reaches AllocFacts — the seam is wired, not merely correct"
    );
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
        // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
        // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
        // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
        // declared no ring at all, which no channel does.
        gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
    // ★★★ §8.2.2 — the two fields the prefix ALWAYS covered and nobody read until now.
    // `gpFifoOffset` is an 8-aligned `NvU64` at +8, so its two halves are +8 and +12 and
    // BOTH must move the same fact; `gpFifoEntries` is a separate `NvU32` at +16 and must
    // move a different one. A decoder that read the entry count at +12 — the u64's high
    // half, the nearest wrong answer — passes every other assertion in this file.
    assert_eq!(
        mutate(8, 0x1234_5000),
        AllocFacts {
            gp_fifo_ring: Some(GpFifoRing {
                va: 0x1234_5000,
                entries: 0
            }),
            ..reference
        },
        "+8 moves the ring VA's LOW half and nothing else",
    );
    assert_eq!(
        mutate(12, 0x0000_007F),
        AllocFacts {
            gp_fifo_ring: Some(GpFifoRing {
                va: 0x0000_007F_0000_0000,
                entries: 0
            }),
            ..reference
        },
        "+12 moves the ring VA's HIGH half — the same fact, not a different one",
    );
    assert_eq!(
        mutate(16, 4096),
        AllocFacts {
            gp_fifo_ring: Some(GpFifoRing {
                va: 0,
                entries: 4096
            }),
            ..reference
        },
        "+16 moves `gpFifoEntries` and nothing else",
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
///
/// ★★ **B4 widened it to cover the data-plane identity.** The byte side now declares the
/// VASpace's PDB with a real `SET_PAGE_DIRECTORY`; the reference side still applies the
/// hand-written `RmEvent`. So the projection's `pdb` — the value the whole address plane
/// routes on — is now something the *decoder* has to get right rather than something both
/// sides were handed.
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
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
/// ★ It takes **three** runs, not two, and the second is a finding the design's one-line
/// arm does not have: the channel decoder recovers `userd_flags` as well as the VAS
/// handles, and `userd_flags` is what the channel ROUTES by. So a wholly absent decoder
/// does not reach `NoVas` at all — it does not even PROJECT. A zero flags word forces no
/// USERD page, which is one of the two shapes RM's own reader answers with "no chid here"
/// (`kchannelAllocHwID_GM107`), so `Arch::vchid_from_userd_flags` returns `None` and the
/// projection refuses by name with `ProjectionError::UnnamedVchid` — **two** planes
/// earlier than `NoVas`, at graph time rather than at ring time.
///
/// ⊘ That is a CHANGE, and it is the point of widening the seam to `Option`: this arm used
/// to reach `FwdFault::UnknownVchid`, because `MockArch`'s invented encoding decoded a
/// zero word to `VChid(0)` — a channel number the guest never asked for, sitting in the
/// exec-plane index. Only stripping the handle facts while KEEPING the flags isolates the
/// VAS, and that is the run the design is describing.
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
        // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
        // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
        // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
        // declared no ring at all, which no channel does.
        gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
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
            let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
            for out in deliver_all(&mut policy, &prefix.messages()) {
                let _ = out.expect("the prefix is legal");
            }
        }
        gpu.apply(channel).expect("the channel applies either way");
        gpu.apply(set_page_dir()).expect("the PDB binds");
        // ★ #177 — the guest schedules before it rings; this local helper builds the
        // channel from an `RmEvent` rather than a script, so it needs the same step
        // `gpu_from_script` gets. Both arms below still reach and assert their ORIGINAL
        // fault (`NoVas`) unchanged: `plan_doorbell` returns `NoVas` before it ever
        // checks `exec.requested` when the channel has no VAS.
        kayfabe_tests::guest_schedules_every_channel(&mut gpu);
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

    // (2) ★★ With NO decoder at all: `userd_flags` is 0 too, and a zero word forces no
    //     USERD page — RM's own reader leaves the chid to the allocator there, so the word
    //     names no channel and the projection REFUSES it by name. There is no doorbell to
    //     take: the channel never enters the index at all.
    let no_decoder = {
        let mut gpu = fresh_gpu();
        {
            let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
            for out in deliver_all(&mut policy, &prefix.messages()) {
                let _ = out.expect("the prefix is legal");
            }
        }
        gpu.apply(undecoded)
    };
    assert_eq!(
        no_decoder,
        Err(kayfabe_core::gpu::GpuError::Projection(
            kayfabe_core::project::ProjectionError::UnnamedVchid {
                channel: ResourceKey::first(NodeKey::new(HClient(cp::C), HObject(cp::GR))),
                userd_flags: 0,
            }
        )),
        "★★ the EXACT refusal, and it is neither `NoVas` nor `UnknownVchid`: a word that \
         names no channel is refused at graph time rather than given a vChid the guest \
         never asked for",
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
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
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
            .channel(cp::C, cp::TSG, cp::GR, gr_flags(), chan_ctxshare, chan_vas)
            .set_page_dir(cp::C, cp::DEV, cp::VAS, CP_PDB.0, w::PDB_FLAGS_ALL_CHANNELS);
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
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);

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
                // ★ §8.2.2: a channel's params ALWAYS declare a ring, and this fixture's is
                // the deliberate zero (`GpFifoRing { va: 0 }` is a value, not an absence —
                // `ogkm-580: kernel_graphics.c:2420-2424`). `None` would mean the class
                // declared no ring at all, which no channel does.
                gp_fifo_ring: Some(GpFifoRing { va: 0, entries: 0 }),
                ..Default::default()
            },
        })),
        "★★ the recycled value is translated from THIS message's bytes — a memo keyed on \
         `hObject` would still be answering `KEPLER_CHANNEL_GROUP_A`",
    );
    assert!(policy.census().is_empty(), "nothing here is a refusal");
}

/// ★ The whole compute subgraph, through the **real command ring** and the boot FSM —
/// the B2 transport, now carrying B3's traffic **and B4's control**. Nine allocs plus one
/// `SET_PAGE_DIRECTORY`, one doorbell, no refusals.
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
    assert_eq!(
        run.applied, 10,
        "nine allocs and the page-directory declaration, all applied"
    );
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
    // ★ Including the control, and with NV_OK. An accepted `GSP_RM_CONTROL` is acked with
    // the request's own body preserved, and the field the guest actually reads the control
    // handler's status out of is `rpc_gsp_rm_control_v03_00.status` @ **body+12**, not the
    // envelope (`ogkm-580: rpc.c:11063-11070`, `ogkm-610: :10868-10875` — identical, only
    // relocated). The guest sent zero there because `rpcWriteCommonHeader` zeroes the
    // whole message buffer first (byte-identical AND at the same lines in both tags:
    // `ogkm-580: rpc_common.c:149-152`, `ogkm-610:` idem), so the echo is an `NV_OK`
    // control reply — a fact about the ack that nothing else in this file observes.
    let ctrl_reply = run
        .replies
        .iter()
        .find(|m| m.function == fn_id::GSP_RM_CONTROL)
        .expect("the control was answered");
    assert_eq!(ctrl_reply.rpc_result, 0, "envelope status NV_OK");
    assert_eq!(
        u32::from_le_bytes(ctrl_reply.payload[12..16].try_into().unwrap()),
        0,
        "★ the CONTROL BODY's own status word @ +12 is NV_OK too",
    );
    assert_eq!(
        u32::from_le_bytes(ctrl_reply.payload[8..12].try_into().unwrap()),
        w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        "and it is the reply to the command we think it is",
    );

    assert_eq!(
        boundaries(&gpu),
        boundaries(&gpu_from_script(&script)),
        "★ the ring and the direct path reach the same object model",
    );
}

// =================================================================================
// ★★ 8. **Stage B4 — the one modelled control**: where a VASpace gets its `Pdb`
//
// B3 could build a compute process's whole object graph from bytes but had to hand it
// the page-directory base as a raw `RmEvent`, because `GSP_RM_CONTROL` refused as a
// whole function. B4 translates one `cmd` out of it — `SET_PAGE_DIRECTORY` — and the
// facts above are now reachable end to end from the wire.
//
// ★★ AND IT IS NOT SUFFICIENT, WHICH IS THE STAGE'S REAL FINDING. `gsp_core_bridge.md`
// §7 item 1 asked which control actually carries the compute VAS's PDB and said to settle
// it *before* B4. Settled, from both vendored trees, and it went against the design:
//
//   - `SET_PAGE_DIRECTORY` reaches the wire only for a SHARED_MANAGEMENT /
//     IS_EXTERNALLY_OWNED VASpace — i.e. UVM's. `gvaspaceExternalRootDirCommit_IMPL`
//     asserts on exactly that (`ogkm-580: gpu_vaspace.c:3109`), and its only caller is
//     the UVM/gpu-ops path (`ogkm-580: nv_gpu_ops.c:8778, 8870`).
//   - Every ORDINARY RM-managed VASpace declares its root through
//     `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) at construct
//     time, as `levels[0].physAddress`, on a code path that is on by default for any GSP
//     client (`ogkm-580: gpu_vaspace.c:598-611, 395, 313, 378, 4039, 5161-5189`;
//     `gpu_registry.c:171-186`).
//
// So this section builds the arm the design specifies AND pins the gap by name, because a
// dropped page-directory declaration is otherwise completely silent downstream: the VAS
// simply never routes and every channel in it defers at its first doorbell, forever.
// =================================================================================

// ---------------------------------------------------------------------------------
// 8.0 Transcription #3 — a whole SET_PAGE_DIRECTORY message, written byte by byte
// ---------------------------------------------------------------------------------

/// A complete `GSP_RM_CONTROL` message carrying `SET_PAGE_DIRECTORY`, by hand.
///
/// 32-byte envelope + 40-byte control header + 32-byte params = **104**. Unreadable on
/// purpose: it shares no code path with `rpcwire`'s builder or with the decoder, so an
/// offset all three agree on has been read out of `ogkm` by three separate acts.
///
/// ★ The two easy mistakes are both visible here as *gaps*: `status` @ body+12 is zero
/// because it is `[OUT]`, and `params[]` starts at body+40 — after `reserved0`'s eight
/// aligned bytes — not at body+36.
///
/// ```text
///   0..4   header_version 0x03000000
///   4..8   signature "VRPC" 0x43505256
///   8..12  length 104
///  12..16  function 76 (GSP_RM_CONTROL)
///  16..20  rpc_result 0
///  20..24  rpc_result_private 0
///  24..28  sequence 3
///  28..32  u (union) 0
///  ── rpc_gsp_rm_control_v03_00 ──                     (body+N shown)
///  32..36  hClient            0xc1d00071   (+0)
///  36..40  hObject            0x5c000001   (+4)   the DEVICE, not the VASpace
///  40..44  cmd                0x00801813   (+8)
///  44..48  status             0            (+12)  [OUT]
///  48..52  paramsSize         32           (+16)
///  52..56  rmapiRpcFlags      0            (+20)
///  56..60  rmctrlFlags        0            (+24)
///  60..64  rmctrlAccessRight  0            (+28)
///  64..72  reserved0          0            (+32)  NvU64, 8-aligned
///  ── NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS ──     (params+N shown)
///  72..80  physAddress        0x341000000  (+0)
///  80..84  numEntries         512          (+8)
///  84..88  flags              0x9          (+12)  SYSMEM_COH | ALL_CHANNELS
///  88..92  hVASpace           0x5c000010   (+16)  a PARAMS field
///  92..96  chId               0            (+20)
///  96..100 subDeviceId        1            (+24)
/// 100..104 pasid              0            (+28)
/// ```
const HEX_SET_PAGE_DIR: [u8; 104] = [
    0x00, 0x00, 0x00, 0x03, 0x56, 0x52, 0x50, 0x43, 0x68, 0x00, 0x00, 0x00, 0x4c, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x71, 0x00, 0xd0, 0xc1, 0x01, 0x00, 0x00, 0x5c, 0x13, 0x18, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x03, 0x00, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x5c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The `flags` word `HEX_SET_PAGE_DIR` carries: `SYSMEM_COH | ALL_CHANNELS`, which is the
/// shape UVM actually sends (`ogkm-580: nv_gpu_ops.c:8857-8862`).
const HEX_SPD_FLAGS: u32 = w::PDB_APERTURE_SYSMEM_COH | w::PDB_FLAGS_ALL_CHANNELS;

/// The hand-written hex and the independent builder agree byte for byte — the third
/// transcription checked against the second, with the decoder taking no part.
#[test]
fn the_hand_written_hex_control_and_the_independent_builder_agree_byte_for_byte() {
    let built = set_page_dir_msg(spd::C, spd::DEV, spd::VAS, spd::PDB, HEX_SPD_FLAGS);
    assert_eq!(
        built.len(),
        HEX_SET_PAGE_DIR.len(),
        "32 envelope + 40 control header + 32 params",
    );
    assert_eq!(
        built,
        HEX_SET_PAGE_DIR.to_vec(),
        "the hand-written control message and the builder's disagree",
    );
}

/// ★ The headline: those bytes become the one fact this control declares.
#[test]
fn the_hand_hex_set_page_directory_becomes_the_declared_event() {
    assert_eq!(
        xlate(&HEX_SET_PAGE_DIR),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            spd::VAS,
            spd::PDB
        ))),
    );
}

// ---------------------------------------------------------------------------------
// 8.1 Which field is which — the three handles, and the two that are dropped
// ---------------------------------------------------------------------------------

/// ★★ Each field of the control moves **exactly one** field of the event, or none.
///
/// The non-vacuity arm §5.2 asks for, and the shape of the only bug this arm can really
/// have: `hClient`, `hObject` and `hVASpace` are three 32-bit handles in three places, and
/// picking the wrong one produces a graph that looks entirely plausible.
#[test]
fn one_changed_field_of_the_control_moves_exactly_one_field_of_the_event() {
    let base = expected_set_page_dir(spd::C, spd::VAS, spd::PDB);
    assert_eq!(xlate(&HEX_SET_PAGE_DIR), Ok(Translation::Event(base)));

    // The namespace comes from the RPC body's own `hClient` — never a params field.
    assert_eq!(
        xlate(&set_page_dir_msg(
            0xc1d0_00ff,
            spd::DEV,
            spd::VAS,
            spd::PDB,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(expected_set_page_dir(
            0xc1d0_00ff,
            spd::VAS,
            spd::PDB
        ))),
    );
    // The VASpace comes from params+16.
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            spd::DEV,
            0x5c00_00ee,
            spd::PDB,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            0x5c00_00ee,
            spd::PDB
        ))),
    );
    // The PDB comes from params+0, and it is 64 bits wide — a 32-bit read would truncate
    // this value, which is deliberately above 2^32.
    assert!(spd::PDB > u64::from(u32::MAX));
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            spd::DEV,
            spd::VAS,
            0xdead_beef_cafe_0000,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            spd::VAS,
            0xdead_beef_cafe_0000
        ))),
    );

    // ★ And `hObject` — the Device the control is issued AGAINST — moves NOTHING. It is a
    // declared fact this port drops, because `RmEvent::SetPageDir` has nowhere to put it.
    // Asserted rather than assumed: a bridge that used it as the VASpace would pass every
    // test above and fail this one.
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            0xdead_0001,
            spd::VAS,
            spd::PDB,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(base)),
        "hObject is dropped: two controls differing only in it are ONE event",
    );
}

/// ★★ The **aperture is dropped**, and here is what that costs, stated as a test rather
/// than as a comment.
///
/// `flags[1:0]` says whether the page directory lives in framebuffer or in guest RAM —
/// two different address spaces — and `kayfabe_abi::view::PdbAperture` decodes it
/// perfectly well. `RmEvent::SetPageDir` has nowhere to put it, so all three apertures
/// (and an undefined fourth) produce **the same event**.
///
/// That is safe exactly as long as `Pdb` is only ever a KEY, which today it is: nothing in
/// the tree dereferences one. The day a walker follows a PDB, this test is the one that
/// has to change, and it will say so by failing.
#[test]
fn the_aperture_is_dropped_so_a_vidmem_and_a_sysmem_root_are_the_same_event() {
    let want = Ok(Translation::Event(expected_set_page_dir(
        spd::C,
        spd::VAS,
        spd::PDB,
    )));
    for flags in [
        w::PDB_APERTURE_VIDMEM,
        w::PDB_APERTURE_SYSMEM_COH,
        w::PDB_APERTURE_SYSMEM_NONCOH,
        3, // the undefined fourth encoding — `PdbAperture::Undefined(3)`
        w::PDB_APERTURE_SYSMEM_COH | w::PDB_FLAGS_ALL_CHANNELS,
        u32::MAX, // every other flag bit set as well
    ] {
        assert_eq!(
            xlate(&set_page_dir_msg(
                spd::C,
                spd::DEV,
                spd::VAS,
                spd::PDB,
                flags
            )),
            want,
            "flags {flags:#x} must not change the fact — the aperture has nowhere to go",
        );
    }
}

/// The three tail fields — `chId`, `subDeviceId`, `pasid` — are declared and dropped too,
/// and nothing hostile in them reaches the event.
#[test]
fn nothing_past_hvaspace_in_the_control_params_is_read() {
    let want = Ok(Translation::Event(expected_set_page_dir(
        spd::C,
        spd::VAS,
        spd::PDB,
    )));
    for (ch_id, sub_device_id, pasid) in [
        (0u32, 1u32, 0u32),
        (u32::MAX, u32::MAX, u32::MAX),
        (spd::VAS, spd::DEV, spd::C),
    ] {
        let body = w::control_body(
            spd::C,
            spd::DEV,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(
                spd::PDB,
                512,
                HEX_SPD_FLAGS,
                spd::VAS,
                ch_id,
                sub_device_id,
                pasid,
            ),
        );
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
            want,
            "chId={ch_id:#x} subDeviceId={sub_device_id:#x} pasid={pasid:#x}",
        );
    }
    // `numEntries` likewise: declared, and not a fact the object model holds.
    for entries in [0u32, 1, 512, u32::MAX] {
        let body = w::control_body(
            spd::C,
            spd::DEV,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(spd::PDB, entries, HEX_SPD_FLAGS, spd::VAS, 0, 1, 0),
        );
        assert_eq!(xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)), want);
    }
}

// ---------------------------------------------------------------------------------
// 8.2 The control refusal surface
// ---------------------------------------------------------------------------------

/// ★★ `hVASpace == 0` is **not** "unspecified". NVIDIA's own header, in both vendored
/// trees verbatim: *"If it's 0, it assumes to use the implicit allocated VA space
/// associated with the client/device pair"* (`ogkm-610: ctrl0080dma.h:782-785`,
/// `ogkm-580: ctrl0080dma.h:812-815`).
///
/// That VAS is a real object this RPC does not name. `HObject(0)` would attach the PDB to
/// a node key the guest never declared, where the graph parks it **silently and forever**
/// — a fact landing in the wrong component. So it is refused by name.
///
/// ★ Contrast the alloc arm deliberately: there, a zero handle field means *nothing is
/// declared* and `declared_handle` maps it to `None`. Same zero, opposite meaning, because
/// NVIDIA documented them differently — which is exactly why neither may be inferred.
#[test]
fn a_zero_hvaspace_names_the_implicit_vas_and_is_refused() {
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            spd::DEV,
            0,
            spd::PDB,
            HEX_SPD_FLAGS
        )),
        Err(BridgeRefusal::ImplicitVaspace),
    );
    // Non-vacuity: 1 is a fine VASpace handle. The refusal is about zero.
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            spd::DEV,
            1,
            spd::PDB,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            1,
            spd::PDB
        ))),
    );
    // ★ And a zero PDB is NOT refused — it is a legal declaration this port has no
    // opinion about, and inventing a rule for it would be exactly the guess §4 forbids.
    assert_eq!(
        xlate(&set_page_dir_msg(
            spd::C,
            spd::DEV,
            spd::VAS,
            0,
            HEX_SPD_FLAGS
        )),
        Ok(Translation::Event(expected_set_page_dir(
            spd::C,
            spd::VAS,
            0
        ))),
    );
}

/// ★★ **The §7-item-1 gap, pinned by exact variant** — and it is now **one** command, not
/// three. The revocation moves a page-directory binding and `RmEvent` has no verb for it,
/// so it is refused *as that*, not as "unknown control", because the two say different
/// things to whoever reads the census.
///
/// ⊘ The other two ids left this test on 2026-08-08 (§14.23): they are **publications** and
/// they are now translated, which is what `a_publication_becomes_a_set_page_dir_event`
/// asserts. A test that still demanded a refusal from them would have been a green gate on
/// the exact defect the increment removed.
#[test]
fn a_page_dir_bearing_control_is_refused_as_itself_not_as_unknown() {
    let cmd = w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY;
    let body = w::control_body(spd::C, spd::DEV, cmd, 0, w::RMAPI_RPC_FLAGS_NONE, &[]);
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
        Err(BridgeRefusal::PageDirControlNotModelled { cmd }),
        "cmd {cmd:#x}",
    );
    // ★ The adjacency assertion. 0x801813 and 0x801814 differ in one bit and mean
    // opposite things; a table with an off-by-one would bind a VASpace to a page
    // directory that was just revoked.
    assert_eq!(
        w::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY,
        w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY + 1,
    );
}

/// Handles and values for a **publication**, kept apart from [`spd`] because the two
/// controls name their VA space in different places and that is the whole class of bug
/// here: `SET_PAGE_DIRECTORY` names it in a **params** field, a publication names it in the
/// RPC **header**'s `hObject`.
mod pubv {
    /// The client the publication is issued in — the RPC body's `hClient`.
    pub const C: u32 = 0xc1e0_0006;
    /// ★★★ The VA SPACE — the RPC body's `hObject`. Not a params field.
    /// `[measured 2026-08-08, boot ship3_d5369b5]`: `gvas cmd 0x90f10106 hClient
    /// 0xc1e00006 hObject 0x0000000a`.
    pub const VAS: u32 = 0x0000_000a;
    /// The root page directory that boot published for it, verbatim.
    pub const ROOT: u64 = 0x0000_0002_efa9_c000;
}

/// One 184-byte publication body, built from the ABI's own encoder so the test states
/// *values* and not *offsets*.
fn publication_params(root: u64, root_aperture: u32, num_levels: u32) -> Vec<u8> {
    use kayfabe_abi::gvaspacepdes::{GMMU_FMT_MAX_LEVELS, PdeLevel, ServerReservedPdes};
    let mut levels = [PdeLevel::default(); GMMU_FMT_MAX_LEVELS];
    // ★ The measured GA106 shape: four levels, shifts 47/38/29/21, each level one page
    // below the last. Only `levels[0]` is the root and only it may reach the event.
    for (i, lv) in levels.iter_mut().enumerate().take(4) {
        *lv = PdeLevel {
            phys_address: root - (i as u64) * 0x1000,
            size: if i == 0 { 0x20 } else { 0x1000 },
            aperture: if i == 0 {
                root_aperture
            } else {
                kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO
            },
            page_shift: [47u8, 38, 29, 21][i],
        };
    }
    kayfabe_abi::gvaspacepdes::encode_server_reserved_pdes(&ServerReservedPdes {
        h_subdevice: 0,
        subdevice_id: 0,
        page_size: 0x20_0000,
        virt_addr_lo: 0x1_0000_0000,
        virt_addr_hi: 0x1_1fff_ffff,
        num_levels,
        levels,
    })
}

/// A publication message on either of the two ids.
fn publication_msg(cmd: u32, client: u32, vaspace: u32, params: &[u8]) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_CONTROL,
        3,
        &w::control_body(
            client,
            vaspace,
            cmd,
            params.len() as u32,
            w::RMAPI_RPC_FLAGS_NONE,
            params,
        ),
    )
}

/// ★★★ **§14.23 — the publication becomes a fact, and the VA space comes from the
/// HEADER.**
///
/// The defect this replaces: `[measured 2026-08-08, boots ship_7a881a7 / ship3_d5369b5]`
/// the port answered `control 0x90f10106 result 0x00000000 x4`, logged all five
/// publications, and produced **no event** — so `Vas::pdb` was empty and every promote-ctx
/// could only refuse `ContextVasUndeclared`.
///
/// ⊘ The `hObject` assertion is the load-bearing half. A translator that read the VA space
/// from anywhere in the 184-byte body would pass every "an event was produced" test and
/// attribute four roots to nothing.
#[test]
fn a_publication_becomes_a_set_page_dir_event_keyed_on_the_headers_hobject() {
    for cmd in [
        w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        w::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
    ] {
        let params = publication_params(
            pubv::ROOT,
            kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO,
            4,
        );
        assert_eq!(
            xlate(&publication_msg(cmd, pubv::C, pubv::VAS, &params)),
            Ok(Translation::Event(expected_set_page_dir(
                pubv::C,
                pubv::VAS,
                pubv::ROOT
            ))),
            "cmd {cmd:#x}",
        );
    }
    // ★ And the VA space really does travel from the header: change ONLY `hObject` and the
    // event's VA space changes with it. Without this, a hard-coded or params-derived
    // handle passes the assertion above.
    let params = publication_params(
        pubv::ROOT,
        kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO,
        4,
    );
    assert_eq!(
        xlate(&publication_msg(
            w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            pubv::C,
            0x0000_000c,
            &params
        )),
        Ok(Translation::Event(expected_set_page_dir(
            pubv::C,
            0x0000_000c,
            pubv::ROOT
        ))),
    );
}

/// ★★★ **A publication rooted anywhere but the framebuffer is REFUSED, not reinterpreted.**
///
/// `Pdb` is documented as a per-GPU **FB** address; `GMMU_APERTURE_SYS_{COH,NONCOH}` roots
/// are guest-physical ones, and `c_ceutils_ring_resolution.md` §2 measured a sysmem-rooted
/// PDB as a live channel's own root on a real GA106. Decoding one into a `Pdb` would be the
/// same number meaning a different memory.
///
/// ⊘ `GMMU_APERTURE_INVALID` is refused too, and it is the trap in the pair: it is a *real
/// enum value* meaning "there is no sub-level here", so a translator that treated 0 as
/// "unset, assume vidmem" would publish a root at whatever address rode beside it.
#[test]
fn a_publication_rooted_outside_the_framebuffer_is_refused_by_name() {
    use kayfabe_abi::gvaspacepdes as g;
    for aperture in [
        g::GMMU_APERTURE_INVALID,
        g::GMMU_APERTURE_PEER,
        g::GMMU_APERTURE_SYS_NONCOH,
        g::GMMU_APERTURE_SYS_COH,
        // A value the header does not define at all.
        9,
    ] {
        let params = publication_params(pubv::ROOT, aperture, 4);
        assert_eq!(
            xlate(&publication_msg(
                w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
                pubv::C,
                pubv::VAS,
                &params
            )),
            Err(BridgeRefusal::PublishedPdesRootAperture {
                cmd: w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
                aperture,
            }),
            "aperture {aperture}",
        );
    }
    // ⊘ And the fork is on `levels[0]` ONLY. A publication whose ROOT is vidmem and whose
    // deeper levels are not is accepted, because only the root becomes the `Pdb` — the
    // deeper levels are §14.12's "for a different path, must not be reused".
    let mut params = publication_params(
        pubv::ROOT,
        kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO,
        4,
    );
    // `levels[1].aperture` — offset 0x28 + 24 + 16.
    params[0x28 + 24 + 16..0x28 + 24 + 20]
        .copy_from_slice(&kayfabe_abi::gvaspacepdes::GMMU_APERTURE_SYS_COH.to_le_bytes());
    assert!(matches!(
        xlate(&publication_msg(
            w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            pubv::C,
            pubv::VAS,
            &params
        )),
        Ok(Translation::Event(RmEvent::SetPageDir { .. })),
    ));
}

/// ★★ **A publication that names no VA space is refused, never attributed by guessing.**
///
/// `[measured 2026-08-08, boot ship3_d5369b5]` the GPU-group global arm arrives with
/// `hClient 0x00000000 hObject 0x00000000` — so this is not a hypothetical malformed
/// message, it is one real arm of the pair, every boot. ⊘ Its `hClient` of zero means it is
/// refused one step earlier as `ReservedClient`, which is asserted here as well so the two
/// refusals do not silently swap places.
#[test]
fn a_publication_naming_no_vaspace_is_refused_and_the_global_arm_is_reserved_client() {
    let params = publication_params(
        pubv::ROOT,
        kayfabe_abi::gvaspacepdes::GMMU_APERTURE_VIDEO,
        4,
    );
    assert_eq!(
        xlate(&publication_msg(
            w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
            pubv::C,
            0,
            &params
        )),
        Err(BridgeRefusal::PublishedPdesUnnamedVaspace {
            cmd: w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
        }),
    );
    // The real global arm, as the boot sends it: both handles zero.
    assert_eq!(
        xlate(&publication_msg(
            w::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER,
            0,
            0,
            &params
        )),
        Err(BridgeRefusal::ReservedClient),
    );
}

/// ★ **The guest's own ABI rules are the refusal, and the declared size is checked exactly.**
#[test]
fn a_malformed_publication_is_refused_as_malformed_and_a_lying_size_as_a_size() {
    use kayfabe_abi::gvaspacepdes as g;
    const CMD: u32 = w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES;
    // `numLevelsToCopy = 0` — a publication of no levels is not a publication
    // (`ctrl90f1.h`'s own rule, via `decode_server_reserved_pdes`).
    let params = publication_params(pubv::ROOT, g::GMMU_APERTURE_VIDEO, 0);
    assert_eq!(
        xlate(&publication_msg(CMD, pubv::C, pubv::VAS, &params)),
        Err(BridgeRefusal::PublishedPdesMalformed {
            cmd: CMD,
            err: g::ServerReservedPdesError::LevelCountOutOfRange { got: 0 },
        }),
    );
    // Past the array bound.
    let params = publication_params(pubv::ROOT, g::GMMU_APERTURE_VIDEO, 7);
    assert_eq!(
        xlate(&publication_msg(CMD, pubv::C, pubv::VAS, &params)),
        Err(BridgeRefusal::PublishedPdesMalformed {
            cmd: CMD,
            err: g::ServerReservedPdesError::LevelCountOutOfRange { got: 7 },
        }),
    );
    // ⊘ A declared `paramsSize` that is not the struct's own size is refused as a SIZE,
    // before the body is looked at — §4.3's exact check, not a lower bound.
    let params = publication_params(pubv::ROOT, g::GMMU_APERTURE_VIDEO, 4);
    let body = w::control_body(
        pubv::C,
        pubv::VAS,
        CMD,
        32,
        w::RMAPI_RPC_FLAGS_NONE,
        &params,
    );
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
        Err(BridgeRefusal::ControlParamsSizeMismatch {
            cmd: CMD,
            declared: 32,
            expected: g::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE,
        }),
    );
}

/// ★ **§7 item 6's decision, made and pinned.** An unmodelled control is a *refusal*, not
/// a `Translation::Forward` nobody consumes — and it is a different variant from an
/// unknown *function*, because "we do not know this RPC" and "we know this RPC and not
/// this command" are different findings.
///
/// ★★ `GPU_PROMOTE_CTX` **left this list on 2026-07-30**, and that is the point of the
/// comment rather than a tidy-up: it was the canonical permitted-but-unmodelled control,
/// and modelling it is `#93`. Its own assertions are in
/// `tests/tests/promote_ctx.rs`. `GR_GET_CTX_BUFFER_INFO` inherits the role.
#[test]
fn an_unmodelled_control_is_refused_as_unknown_control_not_unknown_function() {
    // ★★ The list SPLIT when the capability gate landed, and the split is the finding.
    // Every one of these used to answer `UnknownControl`; only the ones the boundary
    // actually permits still do. `GR_GET_CTX_BUFFER_INFO` is here on purpose: it is
    // permitted only because this port added it as `Origin::Mode2Rpc` (the C's
    // ioctl-era list has it nowhere) and it is deliberately not modelled, so it is the
    // canonical permitted-but-unmodelled control now that `GPU_PROMOTE_CTX` is decoded.
    for cmd in [
        0x2080_1219, // GR_GET_CTX_BUFFER_INFO — Mode-2 only, listed, unmodelled
        UNMODELLED_CMD,
    ] {
        let body = w::control_body(spd::C, spd::DEV, cmd, 0, w::RMAPI_RPC_FLAGS_NONE, &[]);
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
            Err(BridgeRefusal::UnknownControl { cmd }),
            "cmd {cmd:#x}",
        );
    }
    // ★★ `u32::MAX` was in that list, annotated *"permitted by the GSS-legacy rule, not
    // by a row"* — and that annotation is now the variant. A control admitted by a RULE
    // rather than a row was admitted on nvproxy's premise that a GSP services it, which
    // in Mode 2 names our own fake GSP; `BridgeRefusal::GspRuleControlUnserviced` carries
    // the argument and the guest-side control flow. The split is asserted here rather
    // than merged away because the two are separately countable in the census, and
    // `crates/kayfabe-rmrpc/tests/gss_legacy_answer.rs` is where it is pinned in full.
    let body = w::control_body(spd::C, spd::DEV, u32::MAX, 0, w::RMAPI_RPC_FLAGS_NONE, &[]);
    assert_eq!(
        xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
        Err(BridgeRefusal::GspRuleControlUnserviced {
            cmd: u32::MAX,
            rule: PassthroughRule::GssLegacy,
        }),
    );
    // …and the ones the boundary refuses outright never reach the params table. This is
    // the half of the old list that moved, and it must not be able to move back: a
    // `ControlNotPermitted` that quietly became `UnknownControl` would mean the gate had
    // stopped running.
    for cmd in [
        0x0000_0000u32,
        0x0080_1812, // one BELOW the modelled command
        0x0080_1815,
        0x0080_180f, // UPDATE_PDE_2 — one PDE, not the root
        0x90f1_0105, // one below COPY_SERVER_RESERVED_PDES
        UNPERMITTED_CMD,
    ] {
        let body = w::control_body(spd::C, spd::DEV, cmd, 0, w::RMAPI_RPC_FLAGS_NONE, &[]);
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &body)),
            Err(BridgeRefusal::ControlNotPermitted {
                cmd,
                denial: Denial::NotOnAllowlist,
            }),
            "cmd {cmd:#x}",
        );
    }
    // ★ A control the port refuses BY NAME is a third answer, distinct from both — and
    // the `Denial` it carries is what a census counts it under.
    let regops = 0x2080_0122u32;
    assert_eq!(
        xlate(&w::message(
            fn_id::GSP_RM_CONTROL,
            3,
            &w::control_body(spd::C, spd::DEV, regops, 0, w::RMAPI_RPC_FLAGS_NONE, &[]),
        )),
        Err(BridgeRefusal::ControlNotPermitted {
            cmd: regops,
            denial: Denial::Refused {
                name: "NV2080_CTRL_CMD_GPU_EXEC_REG_OPS",
                why: DeniedBecause::RegisterAccess,
            },
        }),
    );
    // The function itself is emphatically KNOWN — this is not `UnknownFunction`, and the
    // two carry different numbers (a wire fn id vs an RM command).
    assert_ne!(
        FaultTag("BridgeRefusal::UnknownControl"),
        FaultTag("BridgeRefusal::UnknownFunction"),
    );
}

/// ★ The serialization refusal on the control side fires on **one bit**, and its
/// neighbour must not trip it.
///
/// `rmapiRpcFlags` carries `COPYOUT_ON_ERROR` = `NVBIT(0)` and `SERIALIZED` = `NVBIT(1)`,
/// set independently by `rpcRmApiControl_GSP` (identical at both tags:
/// `ogkm-580: rpc.c:10998-11001`, `ogkm-610: :10803-10806`). A `!= 0` test
/// would refuse every control that merely asked for copy-out-on-error — which is a large
/// and entirely ordinary class of control.
#[test]
fn a_serialized_control_is_refused_but_copyout_on_error_alone_is_not() {
    let with_flags = |flags: u32| {
        w::message(
            fn_id::GSP_RM_CONTROL,
            3,
            &w::control_body(
                spd::C,
                spd::DEV,
                w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                32,
                flags,
                &w::set_page_dir_params(spd::PDB, 512, HEX_SPD_FLAGS, spd::VAS, 0, 1, 0),
            ),
        )
    };
    let want = Ok(Translation::Event(expected_set_page_dir(
        spd::C,
        spd::VAS,
        spd::PDB,
    )));

    assert_eq!(xlate(&with_flags(w::RMAPI_RPC_FLAGS_NONE)), want);
    // ★ The neighbour bit alone: ordinary, and it must translate.
    assert_eq!(
        xlate(&with_flags(w::RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR)),
        want
    );
    // The serialized bit, alone and with its neighbour: refused both ways.
    for flags in [
        w::RMAPI_RPC_FLAGS_SERIALIZED,
        w::RMAPI_RPC_FLAGS_SERIALIZED | w::RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR,
        u32::MAX,
    ] {
        assert_eq!(
            xlate(&with_flags(flags)),
            Err(BridgeRefusal::SerializedControlParams {
                cmd: w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            }),
            "flags {flags:#x}",
        );
    }
}

/// A control's `paramsSize` is checked twice, against two different things, and the two
/// refusals are distinct: **can it be read** (bounds) and **is it the right struct**
/// (size).
///
/// ★ The exactness is `gsp_core_bridge.md` §4.3's rule and it is `[src]`-backed: the
/// driver passes `sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS)` verbatim
/// (`ogkm-580: dma.c:508-518`), so 33 is as wrong as 31 — and "take the first 32 bytes"
/// is `abi_struct_truncation` with extra steps.
#[test]
fn a_control_params_size_is_bounded_by_the_payload_and_pinned_to_the_struct() {
    let params = w::set_page_dir_params(spd::PDB, 512, HEX_SPD_FLAGS, spd::VAS, 0, 1, 0);
    let declaring = |size: u32, params: &[u8]| {
        w::message(
            fn_id::GSP_RM_CONTROL,
            3,
            &w::control_body(
                spd::C,
                spd::DEV,
                w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                size,
                w::RMAPI_RPC_FLAGS_NONE,
                params,
            ),
        )
    };

    // Beyond what arrived -> bounds, with BOTH numbers, never clamped to the smaller.
    for size in [33u32, 64, 4096, u32::MAX] {
        assert_eq!(
            xlate(&declaring(size, &params)),
            Err(BridgeRefusal::ParamsSizeExceedsPayload {
                declared: size,
                available: 32,
            }),
            "declared {size}",
        );
    }
    // Readable, but not the struct's own size -> the size refusal, with both numbers.
    for size in [0u32, 1, 16, 31] {
        assert_eq!(
            xlate(&declaring(size, &params)),
            Err(BridgeRefusal::ControlParamsSizeMismatch {
                cmd: w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                declared: size,
                expected: 32,
            }),
            "declared {size}",
        );
    }
    // ★ Longer params AND an honest oversize declaration is still a mismatch: 33 bytes of
    // params is a different struct, not a padded one.
    let mut long = params.clone();
    long.push(0xAA);
    assert_eq!(
        xlate(&declaring(33, &long)),
        Err(BridgeRefusal::ControlParamsSizeMismatch {
            cmd: w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            declared: 33,
            expected: 32,
        }),
    );
    // Exactly 32 is the one that translates.
    assert!(matches!(
        xlate(&declaring(32, &params)),
        Ok(Translation::Event(_))
    ));
}

/// The control **header** is refused below 40 bytes at every length, naming its own
/// struct — never zero-extended into a plausible-looking control.
///
/// ★ The window that matters is `[36, 40)`: `reserved0` is a `NvU64` at +32, so a reader
/// who forgot its alignment would accept exactly those four lengths and then slice
/// `params[]` four bytes early — decoding `physAddress` out of the tail of `reserved0`.
#[test]
fn a_truncated_control_body_is_refused_at_every_length() {
    let full = w::control_body(
        spd::C,
        spd::DEV,
        w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        32,
        w::RMAPI_RPC_FLAGS_NONE,
        &w::set_page_dir_params(spd::PDB, 512, HEX_SPD_FLAGS, spd::VAS, 0, 1, 0),
    );
    for len in 0..40usize {
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &full[..len])),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "rpc_gsp_rm_control_v03_00",
                need: 40,
                got: len,
            })),
            "len {len}",
        );
    }
    // 40..72 decodes the header and then fails on the params window, which is the NEXT
    // refusal and a different one — so the boundary is observed from both sides.
    for len in 40..72usize {
        assert_eq!(
            xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &full[..len])),
            Err(BridgeRefusal::ParamsSizeExceedsPayload {
                declared: 32,
                available: len - 40,
            }),
            "len {len}",
        );
    }
    assert!(matches!(
        xlate(&w::message(fn_id::GSP_RM_CONTROL, 3, &full)),
        Ok(Translation::Event(_))
    ));
}

// ---------------------------------------------------------------------------------
// 8.3 Statelessness, carried onto the control arm
// ---------------------------------------------------------------------------------

/// The same control message always translates to the same event — the property a memo, a
/// seen-set or a dedup cache would break, and the one that makes the graph's
/// idempotent-retry tolerance reachable.
#[test]
fn the_same_control_always_translates_to_the_same_event() {
    let first = xlate(&HEX_SET_PAGE_DIR);
    for _ in 0..8 {
        assert_eq!(xlate(&HEX_SET_PAGE_DIR), first);
    }
    // Interleaved with other traffic, so a cache keyed on "the last message" is caught
    // too — not only one keyed on a handle.
    for _ in 0..4 {
        let _ = xlate(&root_alloc_msg(w::NV01_ROOT, spd::C, 0x1234));
        let _ = xlate(&free_msg(spd::C, spd::VAS));
        assert_eq!(xlate(&HEX_SET_PAGE_DIR), first);
    }
}

/// ★★ **A re-declared PDB is accepted, and the LAST one wins** — which is the exact
/// opposite of what `gsp_core_bridge.md` §5.2's B4 row asks for, and the design is wrong.
///
/// The row demands *"a second, **different** PDB for one VASpace refuses (never a second
/// candidate)"*. Three things are wrong with it:
///
/// 1. **The bridge structurally cannot.** Refusing a second PDB means remembering the
///    first, and `translate` is a stateless free function by construction. The one rule
///    the crate is founded on and the one rule this row asks for are incompatible.
/// 2. **`RmGraph` already decided, and decided the other way.** Its `SetPageDir` arm
///    documents *"re-binding a VASpace to a new PDB is protocol-legal
///    (UNSET/SET_PAGE_DIRECTORY); last declaration wins"* — and it is right: the two
///    commands are documented as symmetric operations in both vendored trees, so
///    UNSET-then-SET with a different root is a legal guest sequence and refusing it
///    would hang a conforming guest.
/// 3. **The concern it came from is already met.** The C's actual defect was keeping
///    *both* roots as candidates and probing at use (`C:2538-2545`, `C:5070-5133`). The
///    core keeps exactly ONE. "Never a second candidate" holds; "must be a refusal" was
///    never the way to get it.
///
/// ★ And the research explains the C's own comment. For a SHARED_MANAGEMENT VAS the two
/// roots genuinely differ: `0x90f10106` carries RM's internal root at construct time and
/// `0x00801813` carries UVM's external root later, with no re-emission of the first
/// (`ogkm-580: gpu_vaspace.c:378` is its only call site). Last-declaration-wins is not a
/// tolerance here — it is the correct rule.
#[test]
fn a_second_different_pdb_for_one_vaspace_is_accepted_and_the_last_one_wins() {
    const PDB_A: u64 = 0x0000_0003_1140_0000;
    const PDB_B: u64 = 0x0000_0003_4000_0000;

    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .engine_object(cp::C, cp::GR, cp::GR_OBJ, w::AMPERE_COMPUTE_B)
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_A, w::PDB_APERTURE_VIDMEM)
        // The second declaration, with a DIFFERENT root for the SAME VASpace.
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_B, w::PDB_APERTURE_SYSMEM_COH);

    let gpu = gpu_from_script(&s);
    let b = boundaries(&gpu);
    assert_eq!(
        b.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(PDB_B))],
        "★ exactly ONE root, and it is the LAST declared — never two candidates",
    );
    assert_eq!(
        gpu.procs[&gpu.spine.by_vchid[&(GpuId::ZERO, CP_GR_VCHID)].0].channels
            [&gpu.spine.by_vchid[&(GpuId::ZERO, CP_GR_VCHID)].1]
            .vas_pdb,
        Some(Pdb(PDB_B)),
        "and the channel routes on the new one",
    );

    // Non-vacuity: an IDENTICAL re-send is also accepted, and changes nothing — which is
    // what makes the "last wins" above a real observation rather than a tautology.
    let mut same = RpcScript::new();
    same.client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .engine_object(cp::C, cp::GR, cp::GR_OBJ, w::AMPERE_COMPUTE_B)
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_B, w::PDB_APERTURE_SYSMEM_COH)
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_B, w::PDB_APERTURE_SYSMEM_COH);
    assert_eq!(boundaries(&gpu_from_script(&same)), b);
}

/// ★★ The recycle canary, carried onto the control arm — the statefulness check that has
/// been load-bearing since B1.
///
/// A VASpace handle is declared, given a PDB, **freed**, and then the *same* `hObject`
/// value is declared again as a fresh VASpace with a different PDB. A bridge holding any
/// per-handle memory — a memo, a seen-set, a dedup cache — answers the second declaration
/// with the first one's PDB, or refuses it. Both are wrong, and both hang a legal guest:
/// RM recycles handles by design, with no free list and no quarantine.
#[test]
fn a_recycled_vaspace_handle_gets_a_fresh_page_directory() {
    const PDB_1: u64 = 0x0000_0002_0000_0000;
    const PDB_2: u64 = 0x0000_0005_0000_0000;

    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, cp::C, cp::PID)
        .device(cp::C, cp::C, cp::DEV, cp::DEVICE_INSTANCE)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_1, w::PDB_APERTURE_VIDMEM)
        // The VASpace goes away, and the SAME handle value comes back.
        .free(cp::C, cp::VAS)
        .vaspace(cp::C, cp::DEV, cp::VAS)
        .set_page_dir(cp::C, cp::DEV, cp::VAS, PDB_2, w::PDB_APERTURE_VIDMEM)
        .tsg(cp::C, cp::DEV, cp::TSG, cp::VAS)
        .channel(cp::C, cp::TSG, cp::GR, gr_flags(), 0, cp::VAS)
        .engine_object(cp::C, cp::GR, cp::GR_OBJ, w::AMPERE_COMPUTE_B);

    let gpu = gpu_from_script(&s);
    let b = boundaries(&gpu);
    assert_eq!(
        b.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(PDB_2))],
        "★ the second incarnation routes on ITS OWN root, not the dead one's",
    );

    // And the translation of the second control is byte-identical to what it would be
    // with no history at all — the statelessness statement, made directly.
    assert_eq!(
        xlate(&set_page_dir_msg(
            cp::C,
            cp::DEV,
            cp::VAS,
            PDB_2,
            w::PDB_APERTURE_VIDMEM
        )),
        Ok(Translation::Event(expected_set_page_dir(
            cp::C,
            cp::VAS,
            PDB_2
        ))),
    );
}

// ---------------------------------------------------------------------------------
// 8.4 ★★ The composed run — a routable VAS, entirely from bytes
// ---------------------------------------------------------------------------------

/// ★★ **B4's headline** (`gsp_core_bridge.md` §6): *"a routable `Vas` with a PDB, from
/// bytes"* — and now literally every fact in that sentence arrived as wire bytes.
///
/// The B3 version of this had to hand the graph its PDB as a raw `RmEvent`. This one does
/// not: the script's last message is a real `SET_PAGE_DIRECTORY`, and the doorbell that
/// proves the channel routes is resolving a `Pdb` that came off the wire through the
/// bridge.
#[test]
fn the_whole_compute_process_including_its_page_directory_comes_from_wire_bytes() {
    let script = script_compute();
    let mut gpu = gpu_from_script(&script);
    let b = boundaries(&gpu);

    // The control is genuinely in the byte stream, and it is the only one.
    let controls = script
        .steps()
        .iter()
        .filter(|s| s.function == fn_id::GSP_RM_CONTROL)
        .count();
    assert_eq!(controls, 1, "one SET_PAGE_DIRECTORY in the script");

    assert_eq!(
        b.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, CP_PDB)],
        "★ the VAS routes, on a PDB decoded from a control message",
    );
    for vchid in [CP_GR_VCHID, CP_CE_VCHID] {
        assert!(
            handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(vchid), &[]).is_ok(),
            "the {vchid:?} channel rings on a wire-declared page directory",
        );
    }

    // ★ Non-vacuity, and the strongest form of it: strike the control out of the script
    // and the SAME doorbell takes the EXACT `NoVas` fault. So the control is not
    // decoration — it is the thing that makes the exec plane work.
    let mut without = RpcScript::new();
    for s in script.steps() {
        if s.function != fn_id::GSP_RM_CONTROL {
            without.raw(s.function, s.body.clone());
        }
    }
    let mut stripped = gpu_from_script(&without);
    assert!(
        boundaries(&stripped).by_pdb.is_empty(),
        "with no SET_PAGE_DIRECTORY the VAS has no root at all",
    );
    let (pid, cid) = stripped.spine.by_vchid[&(GpuId::ZERO, CP_GR_VCHID)];
    assert_eq!(stripped.procs[&pid].channels[&cid].vas_pdb, None);
    assert_eq!(
        handle_doorbell(
            &mut stripped,
            GpuId::ZERO,
            MockArch::token_for(CP_GR_VCHID),
            &[]
        ),
        Err(FwdFault::NoVas(cid)),
        "★★ the exact fault — a channel whose VAS never declared a root defers, forever",
    );
}

// =================================================================================
// ★★ 9. **Stage B5 — `DUP_OBJECT`**: the only cross-client edge in the object model
//
// B1–B4 built four one-namespace verbs: allocate, free, and one control, each acting
// wholly inside the namespace its envelope names. `DUP_OBJECT` is the first that names
// **two**, and that is the entire reason it is a stage of its own rather than a fifth
// arm:
//
//   * it is the protocol-correct source of **process grouping** — how the guest kernel's
//     UVM session aliases a CUDA process's VASpace, and how two guest processes that
//     genuinely share end up in one blast radius (`project`'s §12.27 predicate);
//   * it makes a whole family of graph refusals reachable from the wire for the first
//     time (`ConflictingDup`, and `UndeclaredClient`/`ReservedClient` on *either* end);
//   * and it is the verb where the crate's own namespace-attribution rule looks like it
//     has an exception and does not. `hClientSrc` is a params field naming a client — but
//     it is an **additional** namespace, not a substitute for the envelope's, and
//     `RmEvent::Dup` has a `NodeKey` for each. The C's `GPU_PROMOTE_CTX` handler is the
//     real counter-example: it substitutes.
// =================================================================================

/// Handles for the dup fixtures, named apart because a `DUP_OBJECT` carries **five**
/// handles across **two** namespaces and the whole class of bug this stage can have is
/// putting one of them where another belongs.
mod dp {
    /// The **source** namespace: a user process's client.
    pub const SRC_C: u32 = 0xc1d0_0071;
    /// The source client's Device.
    pub const SRC_DEV: u32 = 0x5c00_0001;
    /// The object being aliased: the source client's VASpace.
    pub const SRC_H: u32 = 0x5c00_0010;

    /// The **destination** namespace: the guest kernel's UVM session client.
    pub const DST_C: u32 = 0xdead_c0de;
    /// The destination client's Device — and the alias's declared parent, which the
    /// event **drops**.
    pub const DST_P: u32 = 0x5d00_0001;
    /// The alias's handle in the destination namespace. Deliberately unequal to
    /// [`SRC_H`]: RM assigns it out of the destination client's own generator, so a
    /// bridge that reflected the source handle would look plausible.
    pub const DST_H: u32 = 0x5d00_0031;
}

/// A `DUP_OBJECT` message, built by the independent builder.
fn dup_msg(
    dst_client: u32,
    dst_parent: u32,
    dst_handle: u32,
    src_client: u32,
    src_handle: u32,
    flags: u32,
) -> Vec<u8> {
    w::message(
        fn_id::DUP_OBJECT,
        4,
        &w::dup_body(
            dst_client, dst_parent, dst_handle, src_client, src_handle, flags,
        ),
    )
}

/// The event a `DUP_OBJECT` must produce — **written by hand**, from
/// `gsp_core_bridge.md` §2.4's mapping, never derived from the decoder.
fn expected_dup(dst_client: u32, dst_handle: u32, src_client: u32, src_handle: u32) -> RmEvent {
    RmEvent::Dup {
        src: NodeKey::new(HClient(src_client), HObject(src_handle)),
        dst: NodeKey::new(HClient(dst_client), HObject(dst_handle)),
    }
}

// ---------------------------------------------------------------------------------
// 9.0 Transcription #2 — a dup, written byte by byte
// ---------------------------------------------------------------------------------

/// A complete `DUP_OBJECT` message: the guest kernel's UVM client aliases the compute
/// client's VASpace. Written out by hand — the third transcription of the NVOS55 layout,
/// sharing no code path with the builder or with the decoder.
///
/// ```text
/// ── rpc_message_header_v03_00 (32 B) ──────────────────────────────────────────
/// +0   header_version      00 00 00 03   -> 0x03000000
/// +4   signature           56 52 50 43   -> "VRPC" LE
/// +8   length              3c 00 00 00   -> 60 = 32 envelope + 28 body
/// +12  function            15 00 00 00   -> 21 = DUP_OBJECT
/// +16  rpc_result          00 00 00 00
/// +20  rpc_result_private  00 00 00 00
/// +24  sequence            04 00 00 00
/// +28  u                   00 00 00 00
/// ── rpc_dup_object_v03_00 == NVOS55_PARAMETERS_v03_00 (28 B) ──────────────────
/// +32  hClient             de c0 ad de   -> 0xdeadc0de  ★ the DESTINATION namespace
/// +36  hParent             01 00 00 5d   -> 0x5d000001  ── declared, and DROPPED
/// +40  hObject             31 00 00 5d   -> 0x5d000031  ★ the new alias handle
/// +44  hClientSrc          71 00 d0 c1   -> 0xc1d00071  ★ the SOURCE namespace
/// +48  hObjectSrc          10 00 00 5c   -> 0x5c000010  ★ the source object
/// +52  flags               00 00 00 00   -> NV04_DUP_HANDLE_FLAGS_NONE — DROPPED
/// +56  status              00 00 00 00   [OUT]
/// ```
const HEX_DUP: [u8; 60] = [
    0x00, 0x00, 0x00, 0x03, 0x56, 0x52, 0x50, 0x43, 0x3c, 0x00, 0x00, 0x00, 0x15, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xde, 0xc0, 0xad, 0xde, 0x01, 0x00, 0x00, 0x5d, 0x31, 0x00, 0x00, 0x5d, 0x71, 0x00, 0xd0, 0xc1,
    0x10, 0x00, 0x00, 0x5c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// ★ **Transcription #1 vs #2, for the dup.** Two humans, two methods, one byte string.
#[test]
fn the_hand_written_hex_dup_and_the_independent_builder_agree_byte_for_byte() {
    assert_eq!(
        dup_msg(
            dp::DST_C,
            dp::DST_P,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H,
            w::NV04_DUP_HANDLE_FLAGS_NONE
        ),
        HEX_DUP.to_vec(),
    );
    // And the two flag values are the constants NVIDIA defines, not zero-by-accident.
    assert_eq!(w::NV04_DUP_HANDLE_FLAGS_NONE, 0);
    assert_eq!(w::NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE, 1);
}

/// The hand-written bytes become the hand-written event.
#[test]
fn the_hand_hex_dup_becomes_the_declared_event() {
    assert_eq!(
        xlate(&HEX_DUP),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H
        ))),
    );
}

// ---------------------------------------------------------------------------------
// 9.1 The mapping: what moves, what is carried verbatim, and what is dropped
// ---------------------------------------------------------------------------------

/// ★ One changed field moves **exactly one** field of the event — swept over every one of
/// the seven members of `NVOS55_PARAMETERS_v03_00`, not witnessed at one.
///
/// Three of the seven must move *nothing*: `hParent` and `flags` are declared facts
/// `RmEvent::Dup` has nowhere to put, and `status` is `[OUT]`. A decoder that read
/// `hObject` out of `hParent` — the two are adjacent — would pass a single-witness test
/// and fail here.
#[test]
fn one_changed_field_of_the_dup_moves_exactly_one_field_of_the_event() {
    let base = expected_dup(dp::DST_C, dp::DST_H, dp::SRC_C, dp::SRC_H);
    assert_eq!(xlate(&HEX_DUP), Ok(Translation::Event(base)));

    const NEW: u32 = 0x1111_2222;

    // hClient @ body+0 -> dst.client, and NOTHING else.
    assert_eq!(
        xlate(&dup_msg(NEW, dp::DST_P, dp::DST_H, dp::SRC_C, dp::SRC_H, 0)),
        Ok(Translation::Event(expected_dup(
            NEW,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H
        ))),
    );
    // hParent @ body+4 -> nothing.
    assert_eq!(
        xlate(&dup_msg(dp::DST_C, NEW, dp::DST_H, dp::SRC_C, dp::SRC_H, 0)),
        Ok(Translation::Event(base)),
        "★ the alias's declared parent is dropped — `RmEvent::Dup` has nowhere to put it",
    );
    // hObject @ body+8 -> dst.handle.
    assert_eq!(
        xlate(&dup_msg(dp::DST_C, dp::DST_P, NEW, dp::SRC_C, dp::SRC_H, 0)),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            NEW,
            dp::SRC_C,
            dp::SRC_H
        ))),
    );
    // hClientSrc @ body+12 -> src.client.
    assert_eq!(
        xlate(&dup_msg(dp::DST_C, dp::DST_P, dp::DST_H, NEW, dp::SRC_H, 0)),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            dp::DST_H,
            NEW,
            dp::SRC_H
        ))),
    );
    // hObjectSrc @ body+16 -> src.handle.
    assert_eq!(
        xlate(&dup_msg(dp::DST_C, dp::DST_P, dp::DST_H, dp::SRC_C, NEW, 0)),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            dp::DST_H,
            dp::SRC_C,
            NEW
        ))),
    );
    // flags @ body+20 -> nothing.
    assert_eq!(
        xlate(&dup_msg(
            dp::DST_C,
            dp::DST_P,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H,
            NEW
        )),
        Ok(Translation::Event(base)),
    );
    // status @ body+24 -> nothing. It is [OUT]; the guest sends zero and a guest that
    // does not is still telling us nothing.
    let mut poked = HEX_DUP.to_vec();
    poked[32 + 24..32 + 28].copy_from_slice(&NEW.to_le_bytes());
    assert_eq!(xlate(&poked), Ok(Translation::Event(base)));
}

/// ★ The two drops, **swept** rather than witnessed. `hParent` and `flags` are real
/// declared facts; if either ever acquires a home in `RmEvent`, this is the test that
/// must change, and it is written so that it cannot pass by accident in the meantime.
///
/// `NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` is in the sweep by name because it
/// is the one flag NVIDIA defines and it is a **privilege** assertion — the core models
/// privilege as `ClientKind`, declared once at the client root, so there is genuinely
/// nowhere for it to go and the drop is a decision rather than an omission.
#[test]
fn the_dups_parent_and_flags_are_dropped_and_no_value_of_either_moves_the_event() {
    let want = Ok(Translation::Event(expected_dup(
        dp::DST_C,
        dp::DST_H,
        dp::SRC_C,
        dp::SRC_H,
    )));
    for parent in [0u32, 1, dp::DST_P, dp::DST_H, dp::SRC_C, 0xffff_ffff] {
        for flags in [
            w::NV04_DUP_HANDLE_FLAGS_NONE,
            w::NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE,
            0x8000_0000,
            u32::MAX,
        ] {
            assert_eq!(
                xlate(&dup_msg(
                    dp::DST_C,
                    parent,
                    dp::DST_H,
                    dp::SRC_C,
                    dp::SRC_H,
                    flags
                )),
                want,
                "parent {parent:#x}, flags {flags:#x}",
            );
        }
    }
}

/// ★★ **The asymmetry between an edge handle and a params handle, stated as a test.**
///
/// B3 established that a zero in an alloc's *params* handle field means "nothing is
/// declared here" and maps to `None` (`declared_handle`), while B4 established that a
/// zero `hVASpace` in a control's params names the client/device pair's **implicit** VAS
/// and is a refusal. Neither generalises to a dup's `hObject`/`hObjectSrc`, because those
/// are **edge** fields — the node the message creates and the node it references — and
/// the guest's zero is the guest's own choice of key, landing exactly where the guest put
/// it.
///
/// `[src]` RM reads a zero *destination* handle as "generate one"
/// (`clientAssignResourceHandle` → `clientGenResourceHandle`,
/// `ogkm-580: rs_client.c:998-1001`), but that runs on the guest's own CPU-side RM at
/// `serverCopyResource` (`rs_server.c:1725`) **before** the copy-constructor issues the
/// RPC with the already-assigned handle (`ogkm-580: mem.c:1116`). So a conforming guest
/// cannot send zero here at all — and the identical argument holds for `GSP_RM_ALLOC`'s
/// `hObject` (`rs_server.c:898`), which this crate has carried verbatim since B1.
/// Refusing it on one verb and not the other would be a rule with no principle behind it.
#[test]
fn a_dups_handles_are_carried_verbatim_including_zero() {
    for handle in [0u32, 1, dp::DST_H, 0xffff_ffff] {
        assert_eq!(
            xlate(&dup_msg(
                dp::DST_C,
                dp::DST_P,
                handle,
                dp::SRC_C,
                dp::SRC_H,
                0
            )),
            Ok(Translation::Event(expected_dup(
                dp::DST_C,
                handle,
                dp::SRC_C,
                dp::SRC_H
            ))),
            "dst handle {handle:#x} is an EDGE, not a declaration",
        );
        assert_eq!(
            xlate(&dup_msg(
                dp::DST_C,
                dp::DST_P,
                dp::DST_H,
                dp::SRC_C,
                handle,
                0
            )),
            Ok(Translation::Event(expected_dup(
                dp::DST_C,
                dp::DST_H,
                dp::SRC_C,
                handle
            ))),
            "src handle {handle:#x} likewise",
        );
    }
    // ★ Non-vacuity against the sibling rule: the SAME zero, in a params handle field of
    // a class that declares one, is `None` and not `HObject(0)`. Two opposite readings of
    // one byte pattern, and neither may be inferred from the other.
    assert_eq!(
        xlate(&w::message(
            fn_id::GSP_RM_ALLOC,
            1,
            &w::alloc_body(
                dp::SRC_C,
                dp::SRC_DEV,
                0x5c00_0012,
                w::KEPLER_CHANNEL_GROUP_A,
                20,
                w::RMAPI_RPC_FLAGS_NONE,
                &w::tsg_params(0, 0),
            ),
        )),
        Ok(Translation::Event(RmEvent::Alloc {
            client: HClient(dp::SRC_C),
            parent: HObject(dp::SRC_DEV),
            handle: HObject(0x5c00_0012),
            class: ClassId(w::KEPLER_CHANNEL_GROUP_A),
            facts: AllocFacts::default(),
        })),
        "a TSG declaring hVASpace = 0 declares NOTHING — `h_vaspace: None`",
    );
}

/// ★★ The namespace rule, at the one verb that looks like its exception.
///
/// The message's **attribution** is the envelope's `hClient` and nothing may replace it;
/// `hClientSrc` is a *second, additional* namespace the message names, which is a
/// different fact with its own slot. The test drives that home by making the two clients
/// differ in every case and asserting which end each lands on — including the case a
/// substitution bug produces, where both ends carry the same client.
#[test]
fn the_source_client_is_a_second_namespace_not_a_substitute_for_the_first() {
    let t = xlate(&HEX_DUP);
    let Ok(Translation::Event(RmEvent::Dup { src, dst })) = t else {
        panic!("the fixture translates: {t:?}");
    };
    assert_eq!(
        dst.client,
        HClient(dp::DST_C),
        "attribution = the envelope's"
    );
    assert_eq!(
        src.client,
        HClient(dp::SRC_C),
        "the reference = the params'"
    );
    assert_ne!(
        src.client, dst.client,
        "the fixture is a genuine cross-namespace edge",
    );

    // ★ The two substitution bugs, each of which would produce a *plausible* event, and
    // neither of which this fixture can be confused with.
    assert_ne!(
        xlate(&HEX_DUP),
        Ok(Translation::Event(expected_dup(
            dp::SRC_C,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H
        ))),
        "the C's GPU_PROMOTE_CTX bug: the params client substituted for the envelope's",
    );
    assert_ne!(
        xlate(&HEX_DUP),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            dp::DST_H,
            dp::DST_C,
            dp::SRC_H
        ))),
        "the inverse: the envelope's client used for the reference too",
    );

    // A same-namespace dup is legal RM (a client may alias its own object) and must
    // translate to an event whose two ends genuinely share a client.
    assert_eq!(
        xlate(&dup_msg(
            dp::SRC_C,
            dp::SRC_DEV,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H,
            0
        )),
        Ok(Translation::Event(expected_dup(
            dp::SRC_C,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H
        ))),
    );
}

// ---------------------------------------------------------------------------------
// 9.2 The dup arm's own refusal surface
// ---------------------------------------------------------------------------------

/// A short body is refused at **every** length below the struct, with the struct named
/// and both numbers — never zero-extended into a plausible dup.
///
/// ★ And a *longer* payload is accepted: `sizeof(rpc_dup_object_v03_00)` is what the
/// driver writes into the common header (`rpcWriteCommonHeader(.., DUP_OBJECT,
/// sizeof(rpc_dup_object_v03_00))`, identical at both tags: `ogkm-580: rpc.c:11287`,
/// `ogkm-610: :11093`), but the element the
/// transport delivers is padded to its own granularity, so refusing on `len != 28` would
/// refuse a conforming guest.
#[test]
fn a_truncated_dup_is_refused_at_every_length_and_a_longer_body_is_not() {
    let full = w::dup_body(dp::DST_C, dp::DST_P, dp::DST_H, dp::SRC_C, dp::SRC_H, 0);
    assert_eq!(full.len(), 28, "sizeof(NVOS55_PARAMETERS_v03_00)");

    for got in 0..full.len() {
        assert_eq!(
            xlate(&w::message(fn_id::DUP_OBJECT, 4, &full[..got])),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "NVOS55_PARAMETERS",
                need: 28,
                got,
            })),
            "a {got}-byte dup body",
        );
    }
    let want = Ok(Translation::Event(expected_dup(
        dp::DST_C,
        dp::DST_H,
        dp::SRC_C,
        dp::SRC_H,
    )));
    for extra in [0usize, 1, 4, 36] {
        let mut padded = full.clone();
        padded.extend(std::iter::repeat_n(0xAAu8, extra));
        assert_eq!(
            xlate(&w::message(fn_id::DUP_OBJECT, 4, &padded)),
            want,
            "{extra} trailing bytes are not the bridge's business",
        );
    }
}

/// ★ The refusal **order** on the dup arm, peeled one step at a time — written out rather
/// than asserted as "it is like the other arms", because they are separate functions and
/// nothing but a test makes them agree.
///
/// There are only two steps here, and that is itself the finding: a dup carries no
/// `paramsSize`, no serialization bit and no class, so the encoding and the namespace are
/// the whole surface. Everything else about a dup is a *lookup*, and §3.4 gives every
/// lookup to `RmGraph::apply`.
#[test]
fn the_dup_refusal_order_is_encoding_then_namespace() {
    // Both wrong at once -> the encoding, because a namespace read out of a buffer that
    // is not there is not a fact about the guest.
    assert_eq!(
        xlate(&w::message(fn_id::DUP_OBJECT, 4, &[0u8; 12])),
        Err(BridgeRefusal::Abi(AbiError::Truncated {
            c_name: "NVOS55_PARAMETERS",
            need: 28,
            got: 12,
        })),
    );
    // Encoding fixed -> the namespace.
    assert_eq!(
        xlate(&dup_msg(0, dp::DST_P, dp::DST_H, dp::SRC_C, dp::SRC_H, 0)),
        Err(BridgeRefusal::ReservedClient),
    );
    // Namespace fixed -> the fact.
    assert_eq!(
        xlate(&dup_msg(
            dp::DST_C,
            dp::DST_P,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H,
            0
        )),
        Ok(Translation::Event(expected_dup(
            dp::DST_C,
            dp::DST_H,
            dp::SRC_C,
            dp::SRC_H
        ))),
    );
}

// ---------------------------------------------------------------------------------
// 9.3 Through the graph — the refusals only a two-namespace verb can reach
// ---------------------------------------------------------------------------------

/// Handles for the two-process dup fixtures. Two user processes and one guest-kernel
/// client, because §12.27's grouping predicate reads **both** ends' `ClientKind` and a
/// fixture with only one shape would leave half of it unobserved.
mod tp {
    /// User process 1's client.
    pub const C1: u32 = 0xc1d0_0069;
    /// Its pid.
    pub const PID1: u32 = 0x0000_dd13;
    /// Its Device.
    pub const DEV1: u32 = 0x5c00_0001;
    /// Its VASpace.
    pub const VAS1: u32 = 0x5c00_0010;
    /// Its page-directory base.
    pub const PDB1: u64 = 0x0000_0000_0034_1000;

    /// User process 2's client — adjacent to process 1's, so a bridge that confused them
    /// would still produce a plausible graph.
    pub const C2: u32 = 0xc1d0_006a;
    /// Its pid.
    pub const PID2: u32 = 0x0000_dd14;
    /// Its Device.
    pub const DEV2: u32 = 0x5e00_0001;
    /// Its VASpace.
    pub const VAS2: u32 = 0x5e00_0010;
    /// Its page-directory base.
    pub const PDB2: u64 = 0x0000_0000_0035_1000;
    /// The alias handle process 2 gives to process 1's VASpace.
    pub const ALIAS2: u32 = 0x5e00_0031;

    /// The guest kernel's own client — UVM's session shape.
    pub const K: u32 = 0xdead_c0de;
    /// The kernel client's alias of a user VASpace: the measured case, every guest CUDA
    /// process dups into this one client.
    pub const KALIAS: u32 = 0x5d00_0031;
    /// A second page-directory base for process 1, after its VASpace handle is recycled.
    pub const PDB1B: u64 = 0x0000_0000_0036_1000;
}

/// **Transcription #1** of one user process's routable subgraph: bytes.
fn push_process_bytes(s: &mut RpcScript, client: u32, pid: u32, dev: u32, vas: u32, pdb: u64) {
    s.client_root(w::NV01_ROOT, client, pid)
        .device(client, client, dev, cp::DEVICE_INSTANCE)
        .vaspace(client, dev, vas)
        .set_page_dir(client, dev, vas, pdb, w::PDB_FLAGS_ALL_CHANNELS);
}

/// **Transcription #2** of the same subgraph: `RmEvent`s in `mock_classes`, written by
/// hand, sharing no class number and no offset with the byte side.
fn push_process_events(s: &mut Scenario, client: u32, pid: u32, dev: u32, vas: u32, pdb: u64) {
    s.push(RmEvent::Alloc {
        client: HClient(client),
        parent: HObject(client),
        handle: HObject(client),
        class: mock_classes::CLIENT,
        facts: AllocFacts {
            client_kind: Some(ClientKind::User { pid }),
            ..Default::default()
        },
    })
    .push(RmEvent::Alloc {
        client: HClient(client),
        parent: HObject(client),
        handle: HObject(dev),
        class: mock_classes::DEVICE,
        facts: AllocFacts {
            device_instance: Some(cp::DEVICE_INSTANCE),
            ..Default::default()
        },
    })
    .push(RmEvent::Alloc {
        client: HClient(client),
        parent: HObject(dev),
        handle: HObject(vas),
        class: mock_classes::VASPACE,
        facts: AllocFacts::default(),
    })
    .push(RmEvent::SetPageDir {
        client: HClient(client),
        vaspace: HObject(vas),
        pdb: Pdb(pdb),
    });
}

/// Drive a whole script through the policy, **keeping** every outcome and the census —
/// the form the tests below need, because their subject is what gets refused.
fn deliver_script(
    gpu: &mut Gpu,
    script: &RpcScript,
) -> (Vec<Result<Translation, BridgeRefusal>>, RefusalCensus) {
    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, gpu);
    let out = deliver_all(&mut policy, &script.messages());
    let census = policy.census().clone();
    (out, census)
}

/// ★ A dup into a namespace that was never declared is a **FAULT**, not a defer — and
/// `dst` is named first, because it is the squat vector: an alias planted in a namespace
/// the guest has not opened is how an unrelated later process gets merged into the
/// planter's `Proc`.
///
/// The bridge does **not** answer this. It emits the declared fact and lets
/// `RmGraph::apply` decide (§3.4) — so the assertion is layered: `translate` says `Ok`,
/// and the *policy* says the exact graph refusal.
#[test]
fn a_dup_into_an_undeclared_namespace_is_refused_by_the_graph_naming_dst_first() {
    let mut s = RpcScript::new();
    push_process_bytes(&mut s, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    let mut gpu = gpu_from_script(&s);

    // The bridge is untroubled: it resolves nothing and asks nothing.
    let undeclared = dup_msg(tp::C2, 0, tp::ALIAS2, tp::C1, tp::VAS1, 0);
    assert_eq!(
        xlate(&undeclared),
        Ok(Translation::Event(expected_dup(
            tp::C2,
            tp::ALIAS2,
            tp::C1,
            tp::VAS1
        ))),
        "translate never pre-empts the graph's namespace rule",
    );

    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    assert_eq!(
        policy.deliver(&command(&undeclared)),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::UndeclaredClient(HClient(tp::C2))
        ))),
        "the DESTINATION namespace does not exist",
    );
    // ★ Both ends undeclared: still `dst`. Order is a decision, not an accident.
    assert_eq!(
        policy.deliver(&command(&dup_msg(
            tp::C2,
            0,
            tp::ALIAS2,
            tp::K,
            tp::VAS1,
            0
        ))),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::UndeclaredClient(HClient(tp::C2))
        ))),
    );
    // Destination declared, SOURCE not: the other end, named.
    assert_eq!(
        policy.deliver(&command(&dup_msg(
            tp::C1,
            0,
            tp::ALIAS2,
            tp::K,
            tp::VAS1,
            0
        ))),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::UndeclaredClient(HClient(tp::K))
        ))),
    );
    assert_eq!(
        policy.census().tags().collect::<Vec<_>>(),
        vec![(FaultTag("RmGraphError::UndeclaredClient"), 3)],
        "★ by EXACT CONTENT: three refusals and nothing under any other tag. A count \
         cannot tell 'three of the right kind' from 'two right and one wrong'",
    );
    assert_eq!(
        policy.applied(),
        0,
        "not one of them reached the graph state"
    );
}

/// ★★ **`hClientSrc == 0`, and the argument for not checking it in the bridge.**
///
/// The envelope's client is the message's *attribution*, and a message with no namespace
/// is malformed on its face — which needs no graph state, so the bridge refuses it. The
/// source client is a *reference*, and a reference is exactly what §3.4 forbids the
/// bridge to resolve or pre-validate. `RmGraph::apply`'s central gate enumerates **both**
/// of a dup's clients (`clients_named`) precisely so this arm does not have to, and the
/// answer it gives is strictly more informative than a second local copy: the census
/// records `RmGraphError::ReservedClient`, the rule that was actually broken.
#[test]
fn a_zero_source_client_is_refused_by_the_rule_that_owns_it() {
    let mut s = RpcScript::new();
    push_process_bytes(&mut s, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    let mut gpu = gpu_from_script(&s);

    let msg = dup_msg(tp::C1, tp::DEV1, tp::ALIAS2, 0, tp::VAS1, 0);
    assert_eq!(
        xlate(&msg),
        Ok(Translation::Event(expected_dup(
            tp::C1,
            tp::ALIAS2,
            0,
            tp::VAS1
        ))),
        "★ the bridge carries it — the zero SOURCE client is not its question",
    );

    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    assert_eq!(
        policy.deliver(&command(&msg)),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::ReservedClient(HClient(0))
        ))),
    );
    assert_eq!(
        policy.census().tags().collect::<Vec<_>>(),
        vec![(FaultTag("RmGraphError::ReservedClient"), 1)],
        "★ counted by the rule that was broken, not by 'the graph said no'",
    );
    // And the two zeros are DIFFERENT findings: the envelope's is the bridge's.
    assert_eq!(
        policy.deliver(&command(&dup_msg(
            0,
            tp::DEV1,
            tp::ALIAS2,
            tp::C1,
            tp::VAS1,
            0
        ))),
        Err(BridgeRefusal::ReservedClient),
    );
    assert_ne!(
        FaultTag("BridgeRefusal::ReservedClient"),
        FaultTag("RmGraphError::ReservedClient"),
    );
}

/// A destination handle may be bound once. An **identical** re-send is accepted
/// idempotently — retried-RPC tolerance, and the property a dedup cache in the bridge
/// would destroy from the other side — while a *different* source at the same handle is
/// `ConflictingDup`, by exact variant.
#[test]
fn a_conflicting_dup_is_refused_while_an_identical_resend_is_not() {
    let mut s = RpcScript::new();
    push_process_bytes(&mut s, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut s, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    let mut gpu = gpu_from_script(&s);

    let alias_vas1 = dup_msg(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::VAS1, 0);
    // The same alias handle, pointing at a DIFFERENT resource.
    let alias_dev1 = dup_msg(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::DEV1, 0);

    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    assert_eq!(
        policy.deliver(&command(&alias_vas1)),
        Ok(Translation::Event(expected_dup(
            tp::C2,
            tp::ALIAS2,
            tp::C1,
            tp::VAS1
        ))),
    );
    // ★ Byte-identical re-send: accepted, and it must not double-count.
    assert_eq!(
        policy.deliver(&command(&alias_vas1)),
        Ok(Translation::Event(expected_dup(
            tp::C2,
            tp::ALIAS2,
            tp::C1,
            tp::VAS1
        ))),
        "a retried RPC is the SAME fact — the bridge is stateless, so it maps to the \
         same event and the graph's idempotence is reachable",
    );
    // ★ The same destination, a different source: loud.
    assert_eq!(
        policy.deliver(&command(&alias_dev1)),
        Err(BridgeRefusal::Graph(GpuError::Graph(
            RmGraphError::ConflictingDup(NodeKey::new(HClient(tp::C2), HObject(tp::ALIAS2)))
        ))),
    );
    assert_eq!(policy.applied(), 2, "both accepted deliveries applied");
    assert_eq!(
        policy.census().tags().collect::<Vec<_>>(),
        vec![(FaultTag("RmGraphError::ConflictingDup"), 1)],
    );
}

/// ★ **DEFER, not FAULT: a dup may legitimately precede its source.**
///
/// `[measured]` only 25 of 82 observed dups reach the GSP wire, so a dup's source can be
/// an object RM saw and we did not — faulting it would hang a legal guest. The edge parks
/// and resolves the moment the source lands, and the two orders must reach the **same**
/// projection, which is the whole-core determinism property stated for one verb.
#[test]
fn a_dup_that_precedes_its_source_parks_and_resolves_when_the_source_lands() {
    // Both namespaces declared first — that part is NOT deferrable (§12.38).
    let mut prefix = RpcScript::new();
    push_process_bytes(&mut prefix, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    prefix.client_root(w::NV01_ROOT, tp::C2, tp::PID2);

    // Order A: the dup names an object of C2 that does not exist yet, then it arrives.
    let mut early = prefix.clone();
    early
        .dup(tp::C1, tp::DEV1, tp::ALIAS2, tp::C2, tp::VAS2)
        .device(tp::C2, tp::C2, tp::DEV2, cp::DEVICE_INSTANCE)
        .vaspace(tp::C2, tp::DEV2, tp::VAS2)
        .set_page_dir(
            tp::C2,
            tp::DEV2,
            tp::VAS2,
            tp::PDB2,
            w::PDB_FLAGS_ALL_CHANNELS,
        );

    // Order B: the same facts, source first.
    let mut late = prefix.clone();
    late.device(tp::C2, tp::C2, tp::DEV2, cp::DEVICE_INSTANCE)
        .vaspace(tp::C2, tp::DEV2, tp::VAS2)
        .set_page_dir(
            tp::C2,
            tp::DEV2,
            tp::VAS2,
            tp::PDB2,
            w::PDB_FLAGS_ALL_CHANNELS,
        )
        .dup(tp::C1, tp::DEV1, tp::ALIAS2, tp::C2, tp::VAS2);

    let a = gpu_from_script(&early);
    let b = gpu_from_script(&late);
    assert_eq!(
        boundaries(&a),
        boundaries(&b),
        "★ the same facts in either order reach the same object model",
    );

    // Non-vacuity: the dup is what joins them, and the parked-then-resolved edge really
    // did resolve — one boundary holding BOTH clients and BOTH page directories.
    let joined = boundaries(&a);
    assert_eq!(joined.procs.len(), 1);
    assert_eq!(
        joined.procs[0].client_values(),
        [HClient(tp::C1), HClient(tp::C2)].into_iter().collect(),
    );
    assert_eq!(
        joined.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(tp::PDB1)), (GpuId::ZERO, Pdb(tp::PDB2))],
    );

    // ★ And while the source is still missing the edge is INERT, never a fault: the
    // prefix-plus-dup device answers cleanly and groups nothing.
    let mut parked_only = prefix.clone();
    parked_only.dup(tp::C1, tp::DEV1, tp::ALIAS2, tp::C2, tp::VAS2);
    let parked = gpu_from_script(&parked_only);
    assert_eq!(
        boundaries(&parked)
            .procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1)].into_iter().collect(),
            [HClient(tp::C2)].into_iter().collect(),
        ],
        "an unresolved dup groups nothing — MISS is never a silent wrong grouping",
    );
}

// ---------------------------------------------------------------------------------
// 9.4 ★★ What the dup is FOR: grouping, and the lifetime it creates
// ---------------------------------------------------------------------------------

/// ★★ **The §5.1 oracle, for the verb it was hardest to state.** Two user processes that
/// genuinely share — one aliases the other's VASpace — are **one** blast radius, and the
/// projection reached from wire bytes must equal the projection reached from hand-written
/// `RmEvent`s in mock classes.
///
/// The two sides share no class number, no offset and no decoder; what they share is the
/// meaning, and `Boundaries` is what must not be able to tell them apart.
#[test]
fn two_user_processes_joined_by_a_dup_project_the_same_from_bytes_as_from_events() {
    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut script, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    script.dup(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::VAS1);

    let mut reference = Scenario::new();
    push_process_events(
        &mut reference,
        tp::C1,
        tp::PID1,
        tp::DEV1,
        tp::VAS1,
        tp::PDB1,
    );
    push_process_events(
        &mut reference,
        tp::C2,
        tp::PID2,
        tp::DEV2,
        tp::VAS2,
        tp::PDB2,
    );
    reference.push(RmEvent::Dup {
        src: NodeKey::new(HClient(tp::C1), HObject(tp::VAS1)),
        dst: NodeKey::new(HClient(tp::C2), HObject(tp::ALIAS2)),
    });

    let from_bytes = boundaries(&gpu_from_script(&script));
    assert_eq!(from_bytes, boundaries_of_scenario(&reference));

    // Non-vacuity, and the claim spelled out: ONE boundary, holding both namespaces and
    // both page directories.
    assert_ne!(from_bytes, Boundaries::default());
    assert_eq!(
        from_bytes.procs.len(),
        1,
        "a shared resource is one blast radius"
    );
    assert_eq!(
        from_bytes.procs[0].client_values(),
        [HClient(tp::C1), HClient(tp::C2)].into_iter().collect(),
    );
    assert_eq!(
        from_bytes.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(tp::PDB1)), (GpuId::ZERO, Pdb(tp::PDB2))],
    );
    let anchors: BTreeSet<_> = from_bytes.by_pdb.values().map(|(a, _)| *a).collect();
    assert_eq!(anchors.len(), 1, "both VASes route to the one component");

    // ★★ The non-vacuity arm §5.1 demands: strike the dup out of the BYTES and the
    // projection differs — two boundaries, one per process. So the dup is the thing
    // doing the grouping, not a message the graph happened to tolerate.
    let mut without = RpcScript::new();
    for s in script.steps() {
        if s.function != fn_id::DUP_OBJECT {
            without.raw(s.function, s.body.clone());
        }
    }
    let split = boundaries(&gpu_from_script(&without));
    assert_ne!(from_bytes, split);
    assert_eq!(
        split
            .procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1)].into_iter().collect(),
            [HClient(tp::C2)].into_iter().collect(),
        ],
    );
}

/// ★★ **The measured case, and it must merge NOTHING.**
///
/// Every guest CUDA process dups into the one UVM session client (`[measured]`: two
/// concurrent processes issued 82 dups each, *every one* into the kernel client). If a
/// dup into a kernel client merged, the whole guest would collapse into a single `Proc` —
/// #14 un-fixed — and the second process would be a `LateMerge` refusal.
///
/// So the grouping predicate requires **both** ends to be declared *user* clients, and
/// this is the byte-driven proof: two processes, each aliasing into the kernel client,
/// stay two boundaries, and the aliases live in the system component.
#[test]
fn a_dup_into_the_kernel_client_merges_nothing_and_the_alias_lands_in_the_system_component() {
    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut script, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    script
        .client_root(w::NV01_ROOT, tp::K, w::KERNEL_PID)
        .dup(tp::K, tp::K, tp::KALIAS, tp::C1, tp::VAS1)
        .dup(tp::K, tp::K, tp::KALIAS + 1, tp::C2, tp::VAS2);

    let b = boundaries(&gpu_from_script(&script));
    assert_eq!(
        b.procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1)].into_iter().collect(),
            [HClient(tp::C2)].into_iter().collect(),
        ],
        "★★ 164 dups into one UVM client must still be two processes",
    );
    assert_eq!(
        b.system.client_values(),
        [HClient(tp::K)].into_iter().collect(),
    );
    // The two user VASes still route, each to its OWN component — the aliases are
    // references, and a reference moves nothing.
    let owners: Vec<_> = b.by_pdb.iter().map(|(k, (a, _))| (*k, *a)).collect();
    assert_eq!(
        owners,
        vec![
            ((GpuId::ZERO, Pdb(tp::PDB1)), b.procs[0].anchor),
            ((GpuId::ZERO, Pdb(tp::PDB2)), b.procs[1].anchor),
        ],
    );

    // ★ Non-vacuity for "kernel-ness is what stopped it": declare the SAME client with a
    // real pid instead of the KERNEL_PID sentinel, and the identical dup stream now
    // merges all three into one boundary. One `processID` word decides it.
    let mut as_user = RpcScript::new();
    push_process_bytes(&mut as_user, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut as_user, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    as_user
        .client_root(w::NV01_ROOT, tp::K, 0x0000_dd15)
        .dup(tp::K, tp::K, tp::KALIAS, tp::C1, tp::VAS1)
        .dup(tp::K, tp::K, tp::KALIAS + 1, tp::C2, tp::VAS2);
    // ★ The two scripts differ in EXACTLY ONE message, and inside it in exactly the four
    // bytes of `NV0000_ALLOC_PARAMETERS.processID`. Asserted rather than assumed, because
    // "one word decides it" is the whole claim and a second difference would make the
    // comparison below prove something weaker.
    assert_eq!(script.steps().len(), as_user.steps().len());
    assert_eq!(
        script
            .steps()
            .iter()
            .zip(as_user.steps())
            .filter(|(a, b)| a != b)
            .count(),
        1,
    );
    let merged = boundaries(&gpu_from_script(&as_user));
    assert_eq!(
        merged
            .procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1), HClient(tp::C2), HClient(tp::K)]
                .into_iter()
                .collect()
        ],
    );
    assert!(merged.system.client_values().is_empty());
}

// =================================================================================
// 8.5 ★★★ The FOURTH AXIS — the guest OS, and the privilege fold it used to hide
//
// `four_axes_of_variation.md` §1 lists the guest OS as the one axis with *"nowhere yet"*
// as its home, and warns that Windows-as-a-guest *"only stays true if nothing bakes in
// 'the guest is Linux'"*. Until 2026-07-29 something did, in the single worst place: the
// wire→`ClientKind` translation, i.e. the function that decides which RM clients share a
// host isolate.
//
// The rule it applied — `processID == 0xFFFF_FFFF` means kernel-privileged — is gated in
// the guest driver on `RMCFG_FEATURE_PLATFORM_UNIX`
// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` / `ogkm-610: rpc.h:67-77`,
// byte-identical). On a non-UNIX guest the `else` arm runs for kernel clients too, so
// they declare a *real* pid — and the old code answered `ClientKind::User { pid }` for
// them, silently, with no refusal and nothing counted.
//
// These tests are about the SHAPE of that answer, so they are driven end to end through
// the policy from wire bytes and asserted at the projection: what a refusal must not do
// is leave a partial graph, and what a profile must not do is change anything else.
// =================================================================================

/// Drive a whole script through the policy under a **named** guest OS, and report
/// everything an assertion could want: the per-message outcomes, the census, how many
/// commands the graph actually accepted, and the resulting boundaries.
///
/// ★ Deliberately does **not** `unwrap` (unlike [`gpu_from_script`], which asserts a
/// clean run): the subject here is what happens when a message is refused, so a helper
/// that panicked on a refusal could not express any of it.
fn run_script_under(
    guest_os: GuestOs,
    script: &RpcScript,
) -> (
    Vec<Result<Translation, BridgeRefusal>>,
    RefusalCensus,
    u64,
    Boundaries,
) {
    let mut gpu = fresh_gpu();
    let (out, census, applied) = {
        let mut policy = GraphPolicy::new(abi(), guest_os, &mut gpu);
        let out = deliver_all(&mut policy, &script.messages());
        (out, policy.census().clone(), policy.applied())
    };
    let b = boundaries(&gpu);
    (out, census, applied, b)
}

/// The measured #14 fixture: two user processes, the guest kernel's client, and the 2×
/// dup into it that every guest CUDA process performs.
fn two_process_plus_kernel_script() -> RpcScript {
    let mut s = RpcScript::new();
    push_process_bytes(&mut s, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut s, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    s.client_root(w::NV01_ROOT, tp::K, w::KERNEL_PID)
        .dup(tp::K, tp::K, tp::KALIAS, tp::C1, tp::VAS1)
        .dup(tp::K, tp::K, tp::KALIAS + 1, tp::C2, tp::VAS2);
    s
}

/// ★★★ **The whole measured fixture, under a guest whose privilege rule we do not have:
/// every client root REFUSES, and the graph is left with NOTHING.**
///
/// This is the mean form of the fix, not the unit form. The unit question ("does the
/// function return `Err`?") is answered in `kayfabe-abi`; the question here is what a
/// refusal does to a *run*: nine messages, three namespaces, two page directories and two
/// dups, and the assertion is that not one byte of it reached the object model. A refusal
/// that stopped the client root but let the Device, VASpace and `SET_PAGE_DIRECTORY`
/// through would leave a graph with routable page directories owned by nobody — which is
/// a worse outcome than the fold, because it is a fold with no client to name.
///
/// The three assertions are deliberately independent:
/// 1. every `NV01_ROOT` message carries the typed refusal, naming the profile **and** the
///    `processID` it would not interpret;
/// 2. every *dependent* message refuses too, and by the graph's own rule
///    (`UndeclaredClient`), not by an OS rule — the profile decides one thing and the
///    graph notices the consequence, which is what "one seam" means;
/// 3. the projection is empty: no `Proc`, no system component, no routable PDB.
#[test]
fn a_guest_os_without_a_rule_refuses_every_client_root_and_leaves_no_partial_graph() {
    let script = two_process_plus_kernel_script();
    let (out, census, applied, b) = run_script_under(GuestOs::Windows, &script);

    // 1. The three client roots — two user pids and the sentinel — all refuse, and the
    //    sentinel is NOT special-cased back into `Kernel`. "The sentinel still means
    //    kernel, we just also accept pids" is the most tempting wrong rule available, and
    //    it is wrong for the same reason the whole thing is: on a non-UNIX guest RM never
    //    writes the sentinel, so a message carrying it is not evidence of anything.
    let refused_roots: Vec<_> = out
        .iter()
        .filter_map(|r| match r {
            Err(BridgeRefusal::ClientKindRuleUnknown(e)) => Some(*e),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused_roots,
        vec![
            ClientKindRuleUnknown {
                guest_os: GuestOs::Windows,
                process_id: tp::PID1,
            },
            ClientKindRuleUnknown {
                guest_os: GuestOs::Windows,
                process_id: tp::PID2,
            },
            ClientKindRuleUnknown {
                guest_os: GuestOs::Windows,
                process_id: w::KERNEL_PID,
            },
        ],
        "★ each refusal must name the profile AND the value it would not interpret — a \
         refusal that only said \"no\" cannot tell a misconfigured guest from a driver \
         whose rule changed",
    );

    // 2. Nothing applied, and the dependent messages refused for the GRAPH's reason.
    assert_eq!(
        applied, 0,
        "★ NON-VACUITY IN THE OTHER DIRECTION: a run that applied something would mean \
         part of this fixture got in",
    );
    assert_eq!(
        out.iter().filter(|r| r.is_err()).count(),
        script.steps().len(),
        "every message in the script must be accounted for as a refusal",
    );
    assert!(
        census.of(FaultTag("BridgeRefusal::ClientKindRuleUnknown")) == 3
            && census.of(FaultTag("RmGraphError::UndeclaredClient")) > 0,
        "the census must show BOTH the OS refusal and its downstream consequence, not \
         one flat count: {:?}",
        census.tags().collect::<Vec<_>>(),
    );

    // 3. The graph is empty. Not "mostly empty" — empty.
    assert!(b.procs.is_empty(), "no Proc may exist: {:?}", b.procs);
    assert!(b.system.client_values().is_empty());
    assert!(
        b.by_pdb.is_empty(),
        "★ a routable page directory owned by no client is the worst outcome available \
         here: {:?}",
        b.by_pdb.keys().collect::<Vec<_>>(),
    );
}

/// ★★ **Non-vacuity, and the reason the test above is not simply "Windows breaks
/// everything": the identical bytes are fully served under Linux.**
///
/// Same script, same builder, same policy, one parameter different — and it produces the
/// measured #14 answer: two `Proc`s that do not merge despite 2 dups into one kernel
/// client, and the kernel client in the system component. So the refusal above is a
/// property of the *profile*, not of the fixture.
#[test]
fn the_same_bytes_are_fully_served_under_linux_and_that_is_the_only_difference() {
    let script = two_process_plus_kernel_script();
    let (out, census, applied, b) = run_script_under(GuestOs::Linux, &script);

    assert!(
        out.iter().all(Result::is_ok),
        "the measured fixture is legal under the profile it was measured on: {:?}",
        out.iter().filter(|r| r.is_err()).collect::<Vec<_>>(),
    );
    assert!(census.is_empty());
    assert_eq!(
        applied,
        script.steps().len() as u64,
        "★ every message declared a fact — 'no refusals' over a run that applied nothing \
         is the green-instrument-on-an-unexercised-path failure",
    );
    assert_eq!(
        b.procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1)].into_iter().collect(),
            [HClient(tp::C2)].into_iter().collect(),
        ],
    );
    assert_eq!(
        b.system.client_values(),
        [HClient(tp::K)].into_iter().collect(),
    );
}

/// ★★★ **The fold itself, reachable from bytes — and it is bigger than "one process's
/// blast radius": it collapses the WHOLE GUEST into one isolate.**
///
/// ★ This test was written assuming the isolate key was the pid, i.e. that two client
/// roots declaring the same `processID` would merge. **They do not** — and the correction
/// is the finding. `ClientKind::User` is not the key; it is the *eligibility predicate*
/// for a merge, and merges are driven by `DUP_OBJECT`
/// (`a_dup_into_the_kernel_client_merges_nothing_and_the_alias_lands_in_the_system_component`
/// is the Linux-side proof: the grouping requires **both** ends of a dup to be declared
/// user clients).
///
/// Run that through the measured traffic and the consequence is larger than the seam
/// audit's "folds the WDDM kernel into a guest process's blast radius":
///
/// - every guest CUDA process dups into the one kernel/UVM session client — `[measured]`,
///   two concurrent processes, 82 dups **each**, every one into that client;
/// - on a UNIX guest that client declares `KERNEL_PID`, is `ClientKind::Kernel`, is
///   therefore **not** merge-eligible, and the dups merge nothing — which is exactly what
///   fixes #14;
/// - on a WDDM guest `RMCFG_FEATURE_PLATFORM_UNIX` is false, so it declares a real pid
///   (`ogkm-580: rpc.h:74` / `ogkm-610: rpc.h:74`), becomes merge-eligible, and every
///   process's dups now join it — and through it, each other.
///
/// So the old code did not leak one guest process into the kernel's clients. It put
/// **every process in the guest, plus the guest kernel, into a single host isolate** —
/// #14 un-fixed, silently, on a guest nobody had run yet.
///
/// The bytes are genuinely ambiguous: a client root declaring pid 4 is a normal user
/// client on Linux and the `System` process on Windows, and *nothing on the wire
/// distinguishes them*. Only the declared profile can, which is the whole argument for
/// configuration over detection. Both halves are pinned here.
#[test]
fn a_kernel_client_that_declares_a_real_pid_collapses_the_whole_guest_and_only_the_profile_stops_it()
 {
    /// The Windows `System` process, so the fixture reads as what it models.
    const SYSTEM_PID: u32 = 4;

    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut script, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    script
        // ★ The ONE message that differs from the measured fixture, and inside it the
        // four bytes of `NV0000_ALLOC_PARAMETERS.processID`: a real pid where a UNIX
        // guest would have written the sentinel. That is what a WDDM guest sends.
        .client_root(w::NV01_ROOT, tp::K, SYSTEM_PID)
        .dup(tp::K, tp::K, tp::KALIAS, tp::C1, tp::VAS1)
        .dup(tp::K, tp::K, tp::KALIAS + 1, tp::C2, tp::VAS2);

    // Under the Linux rule those bytes are three user clients joined by dups — ONE Proc.
    // Correct on Linux; a total privilege collapse on WDDM. No care taken in the grouping
    // rule could have caught it, because the grouping rule is right either way.
    let (out, _, applied, linux) = run_script_under(GuestOs::Linux, &script);
    assert!(out.iter().all(Result::is_ok));
    assert_eq!(applied, script.steps().len() as u64);
    assert_eq!(
        linux
            .procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![
            [HClient(tp::C1), HClient(tp::C2), HClient(tp::K)]
                .into_iter()
                .collect::<BTreeSet<_>>()
        ],
        "★★★ one word of one message, and the entire guest is one isolate",
    );
    assert!(
        linux.system.client_values().is_empty(),
        "★ and the system component is EMPTY — there is no privileged client left at all",
    );

    // ★ The measured fixture differs in exactly that one word, and keeps the two
    // processes apart. Asserted here rather than assumed, because "one word decides it"
    // is the claim and a second difference would make the comparison prove something
    // weaker.
    let measured = two_process_plus_kernel_script();
    assert_eq!(measured.steps().len(), script.steps().len());
    assert_eq!(
        measured
            .steps()
            .iter()
            .zip(script.steps())
            .filter(|(a, b)| a != b)
            .count(),
        1,
    );

    // Under a profile with no rule the question is refused, not answered — and there is
    // no Proc to collapse into.
    let (out, census, applied, other) = run_script_under(GuestOs::Windows, &script);
    assert_eq!(
        out.iter()
            .filter_map(|r| match r {
                Err(BridgeRefusal::ClientKindRuleUnknown(e)) => Some(e.process_id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![tp::PID1, tp::PID2, SYSTEM_PID],
    );
    assert_eq!(applied, 0);
    assert_eq!(
        census.of(FaultTag("BridgeRefusal::ClientKindRuleUnknown")),
        3
    );
    assert!(other.procs.is_empty() && other.system.client_values().is_empty());
}

/// ★★ **The seam is NARROW: the profile changes exactly one decision and nothing else.**
///
/// The failure mode of a new axis is that it becomes a second switch on everything, and
/// the failure mode of a refusal is that it quietly becomes a kill switch. Both are
/// checked here by driving traffic that declares no client kind at all — inert messages,
/// an unmapped alloc class, an unmodelled control, a malformed free — under **both**
/// profiles and requiring the outcomes to be **identical**, refusals included.
///
/// ★ The non-vacuity guard matters more than usual: a version of this that happened to
/// send only inert traffic would compare two empty censuses and pass forever. So it
/// asserts that the shared traffic produces refusals of at least three distinct tags,
/// i.e. that the two profiles are being compared on a real refusal surface.
#[test]
fn the_guest_os_profile_changes_nothing_that_does_not_declare_a_client_kind() {
    let msgs = vec![
        w::message(fn_id::SET_GUEST_SYSTEM_INFO, 1, &[0xab; 16]),
        w::message(fn_id::GSP_RM_CONTROL, 2, &[0u8; 8]),
        w::message(fn_id::FREE, 3, &[0u8; 4]),
        w::message(fn_id::GSP_RM_ALLOC, 4, &[0u8; 32]),
        w::message(0x0000_7fff, 5, &[]),
    ];

    let run = |guest_os: GuestOs| {
        let mut gpu = fresh_gpu();
        let mut policy = GraphPolicy::new(abi(), guest_os, &mut gpu);
        let out = deliver_all(&mut policy, &msgs);
        let census: Vec<_> = policy.census().tags().collect();
        (out, census, policy.applied(), policy.inert())
    };
    let linux = run(GuestOs::Linux);
    let other = run(GuestOs::Windows);

    assert_eq!(
        linux, other,
        "★ the fourth axis must be one decision, not a mode. Anything that differs here \
         is a second place the guest OS leaked into the bridge",
    );
    assert!(
        linux.1.len() >= 3,
        "★ NON-VACUITY: the shared traffic must actually exercise a refusal surface, or \
         this test compares two empty censuses forever ({:?})",
        linux.1,
    );
    assert!(
        !linux
            .1
            .iter()
            .any(|(t, _)| *t == FaultTag("BridgeRefusal::ClientKindRuleUnknown")),
        "★ none of this traffic declares a client kind, so the OS refusal must not appear \
         under EITHER profile — if it does, the refusal has become a kill switch",
    );
}

/// ★★★ **§12.41, driven from wire bytes for the first time.** A dup keeps a resource
/// alive past the free of the handle that allocated it, and the *handle value* is then
/// legally recycled — so one `(client, handle)` names two live resources.
///
/// `Alloc (C,V) → Dup to K → Free (C,V) → Alloc (C,V)`. Both VASpaces are live, both
/// route, and they are **different resources** — which is why the projection is keyed by
/// `ResourceKey` and not by handle. Keyed by handle the second `insert` silently
/// overwrote the first, the dup-kept ghost vanished from the projection, its `Pdb` left
/// `by_pdb`, and the alias holder's every address op took `FwdFault::UnknownPdb`.
#[test]
fn an_alias_keeps_a_freed_origins_resource_alive_and_the_recycled_handle_is_a_new_one() {
    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    script.client_root(w::NV01_ROOT, tp::K, w::KERNEL_PID).dup(
        tp::K,
        tp::K,
        tp::KALIAS,
        tp::C1,
        tp::VAS1,
    );

    // Stage 1: the alias exists, one VASpace, one PDB.
    let staged = boundaries(&gpu_from_script(&script));
    assert_eq!(
        staged.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(tp::PDB1))],
    );

    // Stage 2: the origin handle is freed. The ALIAS still holds the resource, so the
    // page directory must still route — this is the "never free host memory RM says is
    // live" direction, observed at the projection.
    script.free(tp::C1, tp::VAS1);
    let after_free = boundaries(&gpu_from_script(&script));
    assert_eq!(
        after_free.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(tp::PDB1))],
        "★ a freed origin whose alias is live is STILL a live resource",
    );

    // Stage 3: the guest recycles the handle VALUE for a brand-new VASpace with its own
    // page directory. Two live resources, one handle value.
    script.vaspace(tp::C1, tp::DEV1, tp::VAS1).set_page_dir(
        tp::C1,
        tp::DEV1,
        tp::VAS1,
        tp::PDB1B,
        w::PDB_FLAGS_ALL_CHANNELS,
    );
    let recycled = boundaries(&gpu_from_script(&script));
    assert_eq!(
        recycled.by_pdb.keys().copied().collect::<Vec<_>>(),
        vec![(GpuId::ZERO, Pdb(tp::PDB1)), (GpuId::ZERO, Pdb(tp::PDB1B)),],
        "★★ BOTH page directories route — the ghost was not overwritten by its successor",
    );
    let vas_keys: BTreeSet<ResourceKey> = recycled.by_pdb.values().map(|(_, r)| *r).collect();
    assert_eq!(
        vas_keys.len(),
        2,
        "and they are two DIFFERENT resources at one handle value",
    );
    assert_eq!(
        recycled.procs.len(),
        1,
        "the kernel alias grouped nothing (§12.27)",
    );
    assert_eq!(
        recycled.procs[0]
            .vases
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        vas_keys,
        "★ both belong to the ONE component that declared them, by exact content",
    );
}

/// ★ **The statelessness canary, at the dup.** The same `hObject` value is used as an
/// alias handle, freed, and used again as an alias of a *different* resource. A
/// per-handle memo in the bridge would answer the first source for the second message;
/// a seen-set would refuse the recycle outright. Both hang a conforming guest, because RM
/// recycles handle values by design.
#[test]
fn an_alias_handle_recycled_against_a_different_source_is_translated_afresh() {
    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut script, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    script
        .dup(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::VAS1)
        .free(tp::C2, tp::ALIAS2)
        // The same alias handle VALUE, now naming a different source object entirely.
        .dup(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::DEV1);

    let mut gpu = fresh_gpu();
    let (out, census) = deliver_script(&mut gpu, &script);
    assert!(
        census.is_empty(),
        "★ a recycle is legal traffic — refusing it hangs a conforming guest: {census:?}",
    );
    assert_eq!(
        out.last().cloned(),
        Some(Ok(Translation::Event(expected_dup(
            tp::C2,
            tp::ALIAS2,
            tp::C1,
            tp::DEV1
        )))),
        "★ the second dup is translated from its OWN bytes — a memo would have answered \
         VAS1 here",
    );

    // ★ No residue: the graph is what a device that saw only the surviving facts would
    // be. The two processes are still joined (the second dup is also a user↔user share),
    // and only one alias exists.
    let b = boundaries(&gpu);
    assert_eq!(b.procs.len(), 1);
    assert_eq!(
        b.procs[0].client_values(),
        [HClient(tp::C1), HClient(tp::C2)].into_iter().collect(),
    );
}

/// The whole dup stream through the **real command ring** and the boot FSM: two
/// processes, a kernel client, three dups, every command answered `NV_OK` on its own
/// `(function, sequence)`, and the same object model the direct path reaches.
#[test]
fn the_dup_stream_reaches_the_graph_through_the_real_transport() {
    let mut script = RpcScript::new();
    push_process_bytes(&mut script, tp::C1, tp::PID1, tp::DEV1, tp::VAS1, tp::PDB1);
    push_process_bytes(&mut script, tp::C2, tp::PID2, tp::DEV2, tp::VAS2, tp::PDB2);
    script
        .client_root(w::NV01_ROOT, tp::K, w::KERNEL_PID)
        .dup(tp::K, tp::K, tp::KALIAS, tp::C1, tp::VAS1)
        .dup(tp::K, tp::K, tp::KALIAS + 1, tp::C2, tp::VAS2)
        .dup(tp::C2, tp::DEV2, tp::ALIAS2, tp::C1, tp::VAS1);

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    assert!(
        run.census.is_empty(),
        "a conforming dup stream refuses nothing: {:?}",
        run.census,
    );
    assert_eq!(run.applied, script.steps().len() as u64);
    assert_eq!(run.inert, 0);
    assert!(run.transitions.contains(&Transition::Running));
    assert_eq!(
        run.replies
            .iter()
            .filter(|m| m.function == fn_id::DUP_OBJECT)
            .map(|m| (m.sequence, m.rpc_result))
            .collect::<Vec<_>>(),
        // Nine messages precede the dups (four per process, then the kernel root), and
        // `run_through_transport` numbers the stream from 0x1000 — so 0x1009..=0x100b,
        // written out rather than derived, because a sequence the test computed from the
        // same list it posted would agree with itself.
        vec![(0x1009, 0), (0x100a, 0), (0x100b, 0)],
        "★ each dup answered on its own (function, sequence), with NV_OK",
    );
    assert_eq!(
        boundaries(&gpu),
        boundaries(&gpu_from_script(&script)),
        "★ the ring and the direct path reach the same object model",
    );
    // The composed claim, by exact content: the two user processes share, the kernel
    // client does not pull anyone in.
    let b = boundaries(&gpu);
    assert_eq!(
        b.procs
            .iter()
            .map(|p| p.client_values())
            .collect::<Vec<_>>(),
        vec![[HClient(tp::C1), HClient(tp::C2)].into_iter().collect()],
    );
    assert_eq!(
        b.system.client_values(),
        [HClient(tp::K)].into_iter().collect(),
    );
}

// =================================================================================
// 7. ★★ B6 — continuation records: reassembly, its two bounds, and the reply question
//
// `gsp_core_bridge.md` §2.6 / §5.2's B6 row. Everything here is still bytes-in, and the
// one stateful piece is a value: `Reassembler` holds ONE partial message, keyed by
// nothing, bounded two ways, dropped on completion and on every refusal.
//
// The oracle discipline is unchanged and gains a fourth strand: `rpcwire::fragment` is a
// hand transcription of `_issueRpcLarge`'s split loop (`ogkm-580: rpc.c:2053-2122`,
// `ogkm-610: :2074-2143` — same loop, same arithmetic, 580 reaching the buffer through
// the `vgpu_rpc_message_header_v` / `rpc_message` macros where 610 uses accessors) in a file
// that imports nothing, so the bytes the reassembler joins were split by a re-reading of
// the driver rather than by the reassembler's own inverse.
// =================================================================================

/// Handles for the fragmented-control fixtures. Deliberately the `spd` set, because the
/// hand-hex fragments below spell exactly those numbers.
mod frag {
    /// ★★ **The smallest `maxRpcSize` a `GSP_RM_CONTROL` can legally be split at**, and it
    /// is not a fixture convenience — it is the driver's own arithmetic.
    /// `rpcRmApiControl_GSP` opens with
    /// `message_buffer_remaining = pRpc->maxRpcSize - fixed_param_size`, an **unsigned**
    /// subtraction over `fixed_param_size = sizeof(rpc_message_header_v) +
    /// sizeof(rpc_gsp_rm_control_v03_00)` = 32 + 40 = **72**
    /// (`ogkm-610: rpc.c:10678-10679`, `ogkm-580: :10874-10875`). So a head always carries
    /// the whole 40-byte fixed header; a shorter one could only come from a guest that had
    /// already underflowed. In practice `maxRpcSize = RM_PAGE_SIZE` = 4096
    /// (`rpcConstruct_IMPL`, `ogkm-580: rpc.c:1000`, `ogkm-610: :1002`), fifty-six times
    /// this.
    ///
    /// ★ Splitting *at* 72 is therefore the most hostile **legal** split there is: the
    /// head declares a `paramsSize` and carries not one byte of it.
    pub const SPLIT_AT_PARAMS: usize = 72;
    /// A `maxRpcSize` that splits **inside** `params[]` instead — the case the boundary
    /// split cannot see, because a reassembler that dropped or duplicated a fragment
    /// boundary inside a struct still produces a 32-byte params block.
    pub const SPLIT_MID_PARAMS: usize = 88;
    /// A split leaving only eight bytes for the tail — the other end of the same sweep.
    pub const SPLIT_LATE: usize = 96;

    /// ★★ **`SET_PAGE_DIRECTORY` can never be split into more than two fragments**, and
    /// that is a fact about the modelled surface rather than about these fixtures.
    ///
    /// Its body is 40 + 32 = 72 bytes; a legal head carries at least 40 of them
    /// ([`SPLIT_AT_PARAMS`]) and the continuation stride is `maxRpcSize - 32` >= 40, so at
    /// most 32 bytes ever remain and they always fit in one record. At the *real*
    /// `maxRpcSize` of 4096 the whole message fits in one element and **nothing this port
    /// models fragments at all** — the reassembler exists for the control long tail and
    /// for the day a modelled control is larger, which is why the multi-record arms below
    /// are driven with a big-params control instead.
    pub const BIG_PARAMS: usize = 200;
}

/// A `GSP_RM_CONTROL` carrying [`frag::BIG_PARAMS`] bytes of params for a command this
/// port does not model — the only shape that fragments into **many** records under a legal
/// split. `translate` refuses it as `UnknownControl`, which is the point: reassembly is
/// what lets that refusal name the right command instead of a truncated one.
///
/// ★ It must be a command the capability gate **permits**, or the refusal these tests
/// pin would be `ControlNotPermitted` instead and they would be measuring the gate rather
/// than the reassembler. `NV2080_CTRL_CMD_GPU_GET_INFO` is on the ported allowlist
/// (`kayfabe_abi::capability`) and has no arm in `control_params` — the exact
/// permitted-but-unmodelled state `Translation::Forward` will one day occupy.
/// [`UNPERMITTED_CMD`] is its counterpart, and the two are asserted to differ.
const UNMODELLED_CMD: u32 = 0x2080_0110;

/// A `GSP_RM_CONTROL` command the capability gate refuses outright — one below
/// [`UNMODELLED_CMD`]'s neighbour and on nobody's list.
const UNPERMITTED_CMD: u32 = 0x2080_0112;

fn big_control_body(params_size: u32, carried: usize) -> Vec<u8> {
    w::control_body(
        spd::C,
        spd::DEV,
        UNMODELLED_CMD,
        params_size,
        w::RMAPI_RPC_FLAGS_NONE,
        &vec![0x5a; carried],
    )
}

/// The whole `SET_PAGE_DIRECTORY` message, unfragmented — the thing every fragment run
/// below must reassemble back into.
fn spd_whole(sequence: u32) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_CONTROL,
        sequence,
        &w::control_body(
            spd::C,
            spd::DEV,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            32,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
        ),
    )
}

/// **Hand-written hex, transcription #2** — the HEAD of a fragmented `GSP_RM_CONTROL`.
///
/// `_issueRpcLarge` sets `pVgpuRpcHeader->length = NV_MIN(bufSize, maxRpcSize)` and leaves
/// the real function in place (identical at both tags: `ogkm-580: rpc.c:2061-2068`,
/// `ogkm-610: :2082-2089`), so a head is an ordinary, well-formed fn-76 message that
/// happens to be **short of what its own `paramsSize`
/// declares**. That is the only signal there is, and it is the reason a head is
/// recognised by arithmetic rather than by a flag.
///
/// ```text
/// ── rpc_message_header_v03_00 (32 B) ──────────────────────────────────────────
/// +0   header_version      00 00 00 03   -> 0x03000000
/// +4   signature           56 52 50 43   -> "VRPC"
/// +8   length              48 00 00 00   -> 72 = maxRpcSize, NOT the total
/// +12  function            4c 00 00 00   -> 76 = GSP_RM_CONTROL  ★ the real function
/// +16  rpc_result          00 00 00 00
/// +20  rpc_result_private  00 00 00 00
/// +24  sequence            00 11 00 00   -> 0x1100 = firstSequence
/// +28  u                   00 00 00 00
/// ── rpc_gsp_rm_control_v03_00 (40 B fixed header, ZERO params bytes present) ───
/// +32  hClient             71 00 d0 c1   -> 0xc1d00071
/// +36  hObject             01 00 00 5c   -> 0x5c000001  (the Device)
/// +40  cmd                 13 18 80 00   -> 0x00801813  NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY
/// +44  status              00 00 00 00   -> [OUT], the guest sends zero
/// +48  paramsSize          20 00 00 00   -> 32   ★ declares 32 bytes that are NOT here
/// +52  rmapiRpcFlags       00 00 00 00
/// +56  rmctrlFlags         00 00 00 00
/// +60  rmctrlAccessRight   00 00 00 00
/// +64  reserved0 (NvU64)   00 …          -> 8 bytes
/// ```
#[rustfmt::skip]
const HEX_FRAG_HEAD: [u8; 72] = [
    0x00, 0x00, 0x00, 0x03,  0x56, 0x52, 0x50, 0x43,  0x48, 0x00, 0x00, 0x00,  0x4c, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,  0x00, 0x11, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
    0x71, 0x00, 0xd0, 0xc1,  0x01, 0x00, 0x00, 0x5c,  0x13, 0x18, 0x80, 0x00,  0x00, 0x00, 0x00, 0x00,
    0x20, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
];

/// **Hand-written hex, transcription #2** — the one CONTINUATION_RECORD that finishes it.
///
/// `length = entryLength + sizeof(rpc_message_header_v)` and
/// `function = NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD` (the two adjacent stores,
/// identical at both tags: `ogkm-580: rpc.c:2102-2103`, `ogkm-610: :2123-2124`), with
/// the sequence one past the head's (`ogkm-580: :2126` / `ogkm-610: :2147`,
/// `NV_ASSERT(lastSequence == firstSequence + recordCount)`).
///
/// ```text
/// +0   header_version      00 00 00 03
/// +4   signature           56 52 50 43
/// +8   length              40 00 00 00   -> 64 = 32 payload + 32 envelope
/// +12  function            47 00 00 00   -> 71 = CONTINUATION_RECORD
/// +16  rpc_result          00 00 00 00
/// +20  rpc_result_private  00 00 00 00
/// +24  sequence            01 11 00 00   -> 0x1101 = firstSequence + 1
/// +28  u                   00 00 00 00
/// ── the raw slice: NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS, 32 B ────────────
/// +32  physAddress lo      00 00 00 41   |
/// +36  physAddress hi      03 00 00 00   |-> 0x0000000341000000  ★ the PDB
/// +40  numEntries          00 02 00 00   -> 512
/// +44  flags               00 00 00 00   -> aperture VIDMEM
/// +48  hVASpace            10 00 00 5c   -> 0x5c000010  ★ the VASpace, a PARAMS field
/// +52  chId                00 00 00 00
/// +56  subDeviceId         01 00 00 00
/// +60  pasid               00 00 00 00
/// ```
#[rustfmt::skip]
const HEX_FRAG_TAIL: [u8; 64] = [
    0x00, 0x00, 0x00, 0x03,  0x56, 0x52, 0x50, 0x43,  0x40, 0x00, 0x00, 0x00,  0x47, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,  0x01, 0x11, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x41,  0x03, 0x00, 0x00, 0x00,  0x00, 0x02, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x5c,  0x00, 0x00, 0x00, 0x00,  0x01, 0x00, 0x00, 0x00,  0x00, 0x00, 0x00, 0x00,
];

// ---------------------------------------------------------------------------------
// 7.1 The splitter itself — transcription #3 of the driver's own loop
// ---------------------------------------------------------------------------------

/// The independent builder's split **is** `_issueRpcLarge`'s, checked against
/// hand-computed literals rather than against the reassembler.
///
/// ★ This is the strand the oracle rule (§5.1) demands: if `fragment` and `Reassembler`
/// were inverses of one another written from the same reading, every test below would
/// assert their agreement with themselves. So the split is pinned here, first, by
/// numbers computed on paper from the driver's four lines — and the two hand-hex arrays
/// above are a *fourth* transcription of one instance of it.
#[test]
fn the_fragment_builder_splits_the_way_issue_rpc_large_does() {
    let whole = spd_whole(0x1100);
    assert_eq!(
        whole.len(),
        104,
        "32 envelope + 40 fixed header + 32 params"
    );

    // The boundary split: entryLength = NV_MIN(104, 72) = 72, then stride = 72 - 32 = 40,
    // remaining = 104 - 72 = 32 -> ONE continuation of 32.
    let at_params = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &whole[w::ENVELOPE..],
        frag::SPLIT_AT_PARAMS,
    );
    assert_eq!(
        at_params.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![72, 64],
        "head = maxRpcSize; the tail = 32 remaining + a 32-byte envelope",
    );

    // The mid-struct split: entryLength = NV_MIN(104, 88) = 88, stride = 56,
    // remaining = 16 -> ONE continuation of 16.
    let mid = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &whole[w::ENVELOPE..],
        frag::SPLIT_MID_PARAMS,
    );
    assert_eq!(mid.iter().map(Vec::len).collect::<Vec<_>>(), vec![88, 48]);

    // Many records, at the tightest LEGAL split. A 200-byte params block makes the body
    // 240 and the message 272; entryLength = NV_MIN(272, 72) = 72, stride = 40,
    // remaining = 200 -> 40 x 5 = five continuations.
    let many = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &big_control_body(frag::BIG_PARAMS as u32, frag::BIG_PARAMS),
        frag::SPLIT_AT_PARAMS,
    );
    assert_eq!(
        many.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![72, 72, 72, 72, 72, 72],
    );

    // ★ The two facts a length list cannot show. For every split:
    for (what, run) in [
        ("at the params boundary", &at_params),
        ("inside params", &mid),
        ("five continuations", &many),
    ] {
        // (a) the function/sequence discipline — the head keeps the REAL function and
        //     every continuation is fn 71, sequences running firstSequence + i.
        assert_eq!(
            run.iter()
                .map(|m| {
                    let c = command(m);
                    (c.code, c.sequence)
                })
                .collect::<Vec<_>>(),
            std::iter::once((fn_id::GSP_RM_CONTROL, 0x1100u32))
                .chain((1..run.len()).map(|i| (fn_id::CONTINUATION_RECORD, 0x1100 + i as u32)))
                .collect::<Vec<_>>(),
            "the fragment run's (function, sequence) pairs, {what}",
        );
        // (b) the fragments are slices of `[envelope ++ body]`, and concatenating the
        //     PAYLOADS reproduces the original body exactly — no byte lost, none doubled.
        let source = if what == "five continuations" {
            big_control_body(frag::BIG_PARAMS as u32, frag::BIG_PARAMS)
        } else {
            whole[w::ENVELOPE..].to_vec()
        };
        let joined: Vec<u8> = run.iter().flat_map(|m| command(m).payload).collect();
        assert_eq!(joined, source, "the payloads rejoin, {what}");
    }

    // Non-vacuity: a body that fits produces no continuation at all, which is §5.2's
    // "a head with no continuations still translates" arm at the byte level.
    let short = w::fragment(fn_id::FREE, 4, &w::driver_free_body(spd::C, spd::DEV), 4096);
    assert_eq!(short.len(), 1);
    assert_eq!(
        short[0],
        w::message(fn_id::FREE, 4, &w::driver_free_body(spd::C, spd::DEV))
    );
}

/// The hand-hex fragments and the independent builder agree **byte for byte** — the third
/// and fourth transcriptions meeting.
#[test]
fn the_hand_written_hex_fragments_and_the_independent_builder_agree() {
    let whole = spd_whole(0x1100);
    let built = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &whole[w::ENVELOPE..],
        frag::SPLIT_AT_PARAMS,
    );
    assert_eq!(built.len(), 2);
    assert_eq!(built[0], HEX_FRAG_HEAD, "the head");
    assert_eq!(built[1], HEX_FRAG_TAIL, "the continuation");
}

// ---------------------------------------------------------------------------------
// 7.2 The reassembler, without a `Gpu`
// ---------------------------------------------------------------------------------

/// Feed a run of messages to a bare reassembler, returning every outcome.
fn absorb(r: &mut Reassembler, msgs: &[Vec<u8>]) -> Vec<Result<Reassembled, BridgeRefusal>> {
    msgs.iter().map(|m| r.accept(abi(), &command(m))).collect()
}

/// ★★ The headline: a fragmented `GSP_RM_CONTROL` reassembles into **exactly** the
/// message the guest would have sent unfragmented — and the reassembled command carries
/// the **head's** identity, not the last fragment's.
///
/// The expected value is the *unfragmented* message's decoded command, which no code path
/// under test produced: `spd_whole` is the builder, `fragment` is the splitter, and both
/// were written from `ogkm` rather than from `Reassembler`.
#[test]
fn a_fragmented_control_reassembles_into_the_unfragmented_command() {
    let whole = command(&spd_whole(0x1100));
    for (what, split) in [
        ("at the params boundary", frag::SPLIT_AT_PARAMS),
        ("inside params", frag::SPLIT_MID_PARAMS),
        ("eight bytes left over", frag::SPLIT_LATE),
    ] {
        let run = w::fragment(
            fn_id::GSP_RM_CONTROL,
            0x1100,
            &command(&spd_whole(0x1100)).payload,
            split,
        );
        let mut r = Reassembler::new();
        let out = absorb(&mut r, &run);
        let (last, held) = out.split_last().expect("at least one fragment");

        assert!(
            held.iter().all(|o| o == &Ok(Reassembled::Held)),
            "every fragment but the last is held, {what}: {held:?}",
        );
        assert_eq!(
            last,
            &Ok(Reassembled::Complete(RpcCommand {
                function: RpcFunction::RmControl,
                code: fn_id::GSP_RM_CONTROL,
                sequence: 0x1100,
                payload: whole.payload.clone(),
                // One element per fragment: `command` builds each with `elements: 1`, and
                // the reassembled fact must still measure the transport it cost.
                elements: run.len() as u32,
                delivered: Vec::new(),
            })),
            "★ the reassembled command is the head's identity and the whole body, {what}",
        );
        assert!(
            !r.in_flight() && r.held_bytes() == 0,
            "the head is released on completion, {what}",
        );
    }
}

/// ★ **Many records**, at the tightest legal split — the arm `SET_PAGE_DIRECTORY` cannot
/// reach (`frag::BIG_PARAMS`'s doc says why).
///
/// The contract asserted here is the reassembler's own and nothing else's: the joined
/// payload is **byte-identical** to the body the guest would have sent unfragmented. That
/// is checked against the builder's output, which was produced by neither the splitter's
/// inverse nor the decoder — `big_control_body` writes the struct from `ogkm`'s offsets.
#[test]
fn a_control_split_into_many_records_rejoins_byte_for_byte() {
    for split in [
        frag::SPLIT_AT_PARAMS,
        frag::SPLIT_AT_PARAMS + 8,
        frag::SPLIT_MID_PARAMS,
        frag::SPLIT_LATE,
        144,
    ] {
        let body = big_control_body(frag::BIG_PARAMS as u32, frag::BIG_PARAMS);
        let run = w::fragment(fn_id::GSP_RM_CONTROL, 0x2200, &body, split);
        assert!(
            run.len() >= 3,
            "split {split} produced only {} records",
            run.len()
        );
        let mut r = Reassembler::new();
        let out = absorb(&mut r, &run);
        let (last, held) = out.split_last().expect("a run");
        assert!(
            held.iter().all(|o| o == &Ok(Reassembled::Held)),
            "split {split}: {held:?}",
        );
        assert_eq!(
            last,
            &Ok(Reassembled::Complete(RpcCommand {
                function: RpcFunction::RmControl,
                code: fn_id::GSP_RM_CONTROL,
                sequence: 0x2200,
                payload: body.clone(),
                elements: run.len() as u32,
                delivered: Vec::new(),
            })),
            "split {split}",
        );
        // ★ And it translates as the WHOLE message: the command it names is the one the
        // head declared, not a truncated or a mis-offset one. That is the entire value of
        // reassembly for the control long tail.
        assert_eq!(
            translate(
                abi(),
                GuestOs::Linux,
                &command(&w::message(fn_id::GSP_RM_CONTROL, 0x2200, &body))
            ),
            Err(BridgeRefusal::UnknownControl {
                cmd: UNMODELLED_CMD
            }),
        );
    }
}

/// ★★ **A head too short to contain its own fixed header is malformed, not fragmented** —
/// and that distinction is load-bearing in the safe direction.
///
/// A `GSP_RM_CONTROL` body under 40 bytes declares no `paramsSize` at all: there is
/// nothing in it from which a total could be computed, so it cannot be recognised as a
/// head. It is refused **immediately** by the ABI decoder rather than held for a
/// continuation that would complete nothing.
///
/// `[src]` A conforming guest cannot produce one: `rpcRmApiControl_GSP`'s
/// `pRpc->maxRpcSize - fixed_param_size` is an unsigned subtraction over 72
/// (`ogkm-610: rpc.c:10678-10679`, `ogkm-580: :10874-10875`) and `maxRpcSize` is 4096
/// (`ogkm-580: rpc.c:1000`, `ogkm-610: :1002`). This is category 3 — refused because it
/// cannot happen.
#[test]
fn a_head_shorter_than_its_own_fixed_header_is_malformed_not_fragmented() {
    let mut r = Reassembler::new();
    for len in [0usize, 4, 16, 39] {
        let msg = w::message(fn_id::GSP_RM_CONTROL, 3, &vec![0xff; len]);
        assert_eq!(
            r.accept(abi(), &command(&msg)),
            Ok(Reassembled::Whole),
            "a {len}-byte control body is not a head",
        );
        assert!(!r.in_flight(), "a {len}-byte control body was held");
        assert_eq!(
            xlate(&msg),
            Err(BridgeRefusal::Abi(AbiError::Truncated {
                c_name: "rpc_gsp_rm_control_v03_00",
                need: 40,
                got: len,
            })),
            "and it is refused, not deferred",
        );
    }
    // 40 exactly — the first length that CAN be a head — is one, so the sweep above is
    // not vacuous.
    assert_eq!(
        r.accept(abi(), &command(&head_declaring(32))),
        Ok(Reassembled::Held),
    );
}

/// ★ **A head with no continuations still translates** — §5.2's B6 non-vacuity arm.
///
/// The complementary half of the same rule: `needed > payload.len()` is `>`, not `>=`, so
/// a control whose declared `paramsSize` is exactly satisfied is a whole message. Getting
/// that boundary wrong would hold **every** conforming control forever, waiting for a
/// continuation the guest has no reason to send — a hang with no refusal anywhere.
#[test]
fn a_control_whose_params_all_arrived_is_never_held() {
    let mut r = Reassembler::new();
    // Sweep the declared size across the boundary rather than witnessing one value: the
    // params block is 32 bytes, so 0..=32 are whole and 33.. are heads.
    for declared in [0u32, 1, 16, 31, 32] {
        let msg = w::message(
            fn_id::GSP_RM_CONTROL,
            9,
            &w::control_body(
                spd::C,
                spd::DEV,
                w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                declared,
                w::RMAPI_RPC_FLAGS_NONE,
                &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
            ),
        );
        assert_eq!(
            r.accept(abi(), &command(&msg)),
            Ok(Reassembled::Whole),
            "paramsSize={declared} is satisfied by the 32 bytes present",
        );
        assert!(!r.in_flight(), "nothing was held for paramsSize={declared}");
    }
    // And one past the boundary is a head — so the sweep above is not vacuously true of
    // every input.
    let head = w::message(
        fn_id::GSP_RM_CONTROL,
        9,
        &w::control_body(
            spd::C,
            spd::DEV,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            33,
            w::RMAPI_RPC_FLAGS_NONE,
            &w::set_page_dir_params(spd::PDB, 512, 0, spd::VAS, 0, 1, 0),
        ),
    );
    assert_eq!(r.accept(abi(), &command(&head)), Ok(Reassembled::Held));
    assert_eq!(r.held_bytes(), 72, "the head's whole body is held");
}

/// ★★ **`GSP_RM_ALLOC` cannot fragment, and a short one is still malformed.**
///
/// `rpcRmApiAlloc_GSP` copies params into the single message buffer under an explicit
/// remaining-space bound and returns `NV_ERR_BUFFER_TOO_SMALL` rather than calling
/// `_issueRpcAndWaitLarge` (`ogkm-610: rpc.c:11024-11029`, `ogkm-580: :11218-11223`). So the
/// reassembler must **not** treat an over-declared fn-103 as a head: doing so would
/// convert an immediate, named refusal into a message held forever.
///
/// The sweep runs every function the port names, because the recognition rule is
/// per-function and a single witness cannot see a table with one wrong row.
#[test]
fn only_the_control_path_can_be_a_head() {
    let mut r = Reassembler::new();
    // A fn-103 that declares far more params than it carries — B1's
    // `ParamsSizeExceedsPayload` fixture, verbatim.
    let short_alloc = w::message(
        fn_id::GSP_RM_ALLOC,
        1,
        &w::alloc_body(
            spd::C,
            0,
            0,
            w::NV01_ROOT,
            4096,
            0,
            &w::client_root_params(spd::C, 1),
        ),
    );
    assert_eq!(
        r.accept(abi(), &command(&short_alloc)),
        Ok(Reassembled::Whole)
    );
    assert!(!r.in_flight(), "a short alloc is malformed, not fragmented");
    // ...and reaches the refusal it always did.
    assert_eq!(
        xlate(&short_alloc),
        Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: 4096,
            available: 8,
        }),
    );

    // Every other id the table names: none of them is ever a head, whatever body it
    // carries. `SET_REGISTRY` is in this list deliberately — it *does* fragment in the
    // driver (`_issueRpcAsyncLarge`), and this port cannot compute its total, so it must
    // not pretend to.
    for code in [
        fn_id::SET_GUEST_SYSTEM_INFO,
        fn_id::FREE,
        fn_id::DUP_OBJECT,
        fn_id::UNLOADING_GUEST_DRIVER,
        fn_id::GET_GSP_STATIC_INFO,
        fn_id::GSP_SET_SYSTEM_INFO,
        fn_id::SET_REGISTRY,
        fn_id::GSP_RM_ALLOC,
        999,
    ] {
        for body in [vec![0u8; 0], vec![0xff; 16], vec![0xff; 200]] {
            assert_eq!(
                r.accept(abi(), &command(&w::message(code, 2, &body))),
                Ok(Reassembled::Whole),
                "fn {code} with a {}-byte body is never a head",
                body.len(),
            );
            assert!(!r.in_flight(), "fn {code} held something");
        }
    }
}

/// A `CONTINUATION_RECORD` with nothing in flight is refused — §5.2's other B6 arm — and
/// it is refused **identically** by the stateless free function and by the reassembler.
#[test]
fn a_continuation_with_no_head_refuses_the_same_way_both_ways() {
    let bare = w::message(fn_id::CONTINUATION_RECORD, 5, &[0xab; 48]);
    let refusal = BridgeRefusal::ContinuationWithoutHead {
        code: fn_id::CONTINUATION_RECORD,
    };
    // Written twice with the two return types on purpose: the assertion is that the
    // stateless function and the stateful one refuse with the SAME variant, not that one
    // was derived from the other.
    let want: Result<Reassembled, BridgeRefusal> = Err(refusal);
    assert_eq!(xlate(&bare), Err(refusal));
    assert_eq!(Reassembler::new().accept(abi(), &command(&bare)), want);

    // ★ And after a completed run, too: the head is released, so the fragment that
    // follows a finished message is exactly as headless as one that follows nothing.
    let run = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &command(&spd_whole(0x1100)).payload,
        frag::SPLIT_AT_PARAMS,
    );
    let mut r = Reassembler::new();
    absorb(&mut r, &run);
    assert_eq!(r.accept(abi(), &command(&bare)), want);
}

/// ★★ **`translate` never holds.** The statelessness canary for B6.
///
/// The whole crate's shape is that the free function cannot remember a previous message.
/// If reassembly ever migrated into it — the obvious "simplification" — this is what
/// notices: `translate` is called twice with a head and then a continuation, and the
/// continuation must still be a no-head refusal, because there is nowhere for the head to
/// have gone.
#[test]
fn translate_never_holds() {
    let run = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &big_control_body(frag::BIG_PARAMS as u32, frag::BIG_PARAMS),
        frag::SPLIT_AT_PARAMS,
    );
    assert!(run.len() >= 3, "a run worth checking");
    // The head, through the free function: to something that cannot know a continuation
    // is coming, a head is simply a control that declared more params than it carries.
    assert_eq!(
        xlate(&run[0]),
        Err(BridgeRefusal::ParamsSizeExceedsPayload {
            declared: frag::BIG_PARAMS as u32,
            available: 0,
        }),
        "★ a head is NOT specially recognised by the stateless function",
    );
    // Every continuation after it: still headless, in order, repeatedly.
    for (i, m) in run[1..].iter().enumerate() {
        assert_eq!(
            xlate(m),
            Err(BridgeRefusal::ContinuationWithoutHead {
                code: fn_id::CONTINUATION_RECORD,
            }),
            "continuation {i} was remembered by a stateless function",
        );
    }
    // And no input of any shape makes it produce a `Held`.
    for code in [
        fn_id::GSP_RM_CONTROL,
        fn_id::GSP_RM_ALLOC,
        fn_id::CONTINUATION_RECORD,
        fn_id::SET_REGISTRY,
        fn_id::FREE,
    ] {
        for body in [vec![0u8; 0], vec![0u8; 40], vec![0xff; 200]] {
            assert_ne!(
                xlate(&w::message(code, 1, &body)),
                Ok(Translation::Held),
                "fn {code} made the free function hold",
            );
        }
    }
}

// ---------------------------------------------------------------------------------
// 7.3 ★ The hostile-length matrix — the bounds, each proven load-bearing
// ---------------------------------------------------------------------------------

/// A `GSP_RM_CONTROL` head declaring `declared` bytes of params but carrying none.
fn head_declaring(declared: u32) -> Vec<u8> {
    w::message(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &w::control_body(
            spd::C,
            spd::DEV,
            w::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            declared,
            w::RMAPI_RPC_FLAGS_NONE,
            &[],
        ),
    )
}

/// ★ The size bound refuses **at the head**, before a byte is reserved — swept across it,
/// not witnessed at one point.
///
/// `declared` is a guest-supplied `u32` plus the 40-byte fixed header. A bound tested
/// after the buffer was reserved would be a four-gigabyte allocation on demand, so the
/// hostile end of the sweep is `u32::MAX` and the test's *completion* is the assertion.
#[test]
fn the_reassembly_size_bound_refuses_at_the_head() {
    let limits = ReasmLimits {
        max_body: 200,
        max_continuations: 64,
    };
    // The whole body is `params_at + paramsSize` = 40 + declared, so 160 is the last
    // accepted declaration and 161 the first refused one.
    for (declared, accept) in [
        (41u32, true),
        (159, true),
        (160, true),
        (161, false),
        (4096, false),
        (u32::MAX - 40, false),
        (u32::MAX, false),
    ] {
        let mut r = Reassembler::with_limits(limits);
        let got = r.accept(abi(), &command(&head_declaring(declared)));
        if accept {
            assert_eq!(got, Ok(Reassembled::Held), "declared={declared}");
            assert_eq!(r.held_bytes(), 40, "only the head's own 40 bytes are held");
        } else {
            assert_eq!(
                got,
                Err(BridgeRefusal::ContinuationOverflow {
                    declared: 40 + declared as usize,
                    max: 200,
                }),
                "declared={declared}",
            );
            assert!(
                !r.in_flight(),
                "nothing was reserved for declared={declared}"
            );
        }
    }
}

/// ★★ **The count bound is not implied by the size bound**, and this is the proof.
///
/// A **zero-length** continuation moves the accumulated size not at all, so under a size
/// bound alone a guest holds one head open across an unbounded number of messages —
/// bounded memory, unbounded work. `MAX_CONTINUATIONS` is the bound that refuses it, and
/// the sweep runs right across it rather than at it.
#[test]
fn a_head_cannot_be_held_open_by_empty_continuations() {
    let limits = ReasmLimits {
        max_body: 4096,
        max_continuations: 4,
    };
    let mut r = Reassembler::with_limits(limits);
    // A head that needs 32 more bytes than it carries, so no empty fragment can finish it.
    assert_eq!(
        r.accept(abi(), &command(&head_declaring(32))),
        Ok(Reassembled::Held),
    );
    let empty = w::message(fn_id::CONTINUATION_RECORD, 0x1101, &[]);
    for i in 1..=4 {
        assert_eq!(
            r.accept(abi(), &command(&empty)),
            Ok(Reassembled::Held),
            "empty continuation {i} is within the bound",
        );
        assert_eq!(r.held_bytes(), 40, "and moved the size not at all");
    }
    assert_eq!(
        r.accept(abi(), &command(&empty)),
        Err(BridgeRefusal::ContinuationCountExceeded {
            continuations: 5,
            max: 4,
        }),
    );
    assert!(!r.in_flight(), "the refusal dropped the head");

    // ★ Non-vacuity for the claim "the size bound could not have caught this": the same
    // five messages under a size bound of 40 — the tightest one that admits the head at
    // all — still never trip it.
    let mut tight = Reassembler::with_limits(ReasmLimits {
        max_body: 72,
        max_continuations: u32::MAX,
    });
    assert_eq!(
        tight.accept(abi(), &command(&head_declaring(32))),
        Ok(Reassembled::Held),
    );
    for _ in 0..64 {
        assert_eq!(tight.accept(abi(), &command(&empty)), Ok(Reassembled::Held));
    }
    assert!(tight.in_flight(), "★ a size bound alone never fires here");
}

/// ★ Fragments carrying **more** than the head declared are refused, never truncated.
///
/// The alternative — `body.truncate(declared)` — manufactures a struct the guest did not
/// send, which is `abi_struct_truncation` with extra steps. Swept by overshoot amount,
/// including the exact-fit boundary on both sides.
#[test]
fn fragments_that_overrun_the_declared_total_are_refused_not_clamped() {
    // needed = 40 + 32 = 72; the head carries 40, so 32 bytes are outstanding.
    for (extra, ok) in [(0usize, true), (1, false), (8, false), (1000, false)] {
        let mut r = Reassembler::new();
        assert_eq!(
            r.accept(abi(), &command(&head_declaring(32))),
            Ok(Reassembled::Held),
        );
        let tail = w::message(fn_id::CONTINUATION_RECORD, 0x1101, &vec![0xcd; 32 + extra]);
        let got = r.accept(abi(), &command(&tail));
        if ok {
            assert!(
                matches!(got, Ok(Reassembled::Complete(_))),
                "an exact fit completes: {got:?}",
            );
        } else {
            assert_eq!(
                got,
                Err(BridgeRefusal::ContinuationOverrun {
                    have: 72 + extra,
                    declared: 72,
                }),
                "overshoot by {extra}",
            );
            assert!(!r.in_flight(), "the refusal dropped the head");
        }
    }
}

/// ★ A **new head while one is in flight** is refused, the old head is dropped, and the
/// interrupting message is refused too rather than quietly starting a second run.
#[test]
fn a_new_message_mid_run_is_refused_and_the_old_head_is_dropped() {
    // Every function that could interrupt, including a second head of the same function.
    for interrupter in [
        w::message(fn_id::FREE, 7, &w::driver_free_body(spd::C, spd::DEV)),
        spd_whole(7),
        head_declaring(32),
        w::message(fn_id::SET_REGISTRY, 7, &[0xab; 16]),
        w::message(999, 7, &[0u8; 8]),
    ] {
        let mut r = Reassembler::new();
        assert_eq!(
            r.accept(abi(), &command(&head_declaring(32))),
            Ok(Reassembled::Held),
        );
        let code = command(&interrupter).code;
        assert_eq!(
            r.accept(abi(), &command(&interrupter)),
            Err(BridgeRefusal::ContinuationInterleaved { code }),
        );
        assert!(!r.in_flight(), "fn {code} did not drop the head");
        // ★ And the *continuation* that would have finished the abandoned head is now
        // headless — which is what "dropped" has to mean, and what a test asserting only
        // the refusal above could not tell from "kept".
        assert_eq!(
            r.accept(
                abi(),
                &command(&w::message(fn_id::CONTINUATION_RECORD, 8, &[0xcd; 32])),
            ),
            Err(BridgeRefusal::ContinuationWithoutHead {
                code: fn_id::CONTINUATION_RECORD,
            }),
        );
    }
}

/// ★★ **A refused fragment does not wedge the reassembler.** Asserted by exact content on
/// the *next* message, because the refusal itself cannot distinguish "the head was
/// dropped" from "the head was kept and happened not to matter".
///
/// Every one of the five refusals is driven, and after each the reassembler must accept a
/// fresh, conforming fragmented control and produce the identical event. A reassembler
/// that kept a head across any refusal would be permanently wedgeable by one hostile
/// message.
#[test]
fn no_refusal_wedges_the_reassembler() {
    let whole = command(&spd_whole(0x1100));
    let clean = w::fragment(
        fn_id::GSP_RM_CONTROL,
        0x1100,
        &whole.payload,
        frag::SPLIT_AT_PARAMS,
    );
    let limits = ReasmLimits {
        max_body: 200,
        max_continuations: 2,
    };

    // (name, the message run that provokes it, the tag it must carry)
    let provocations: Vec<(&str, Vec<Vec<u8>>, FaultTag)> = vec![
        (
            "no head",
            vec![w::message(fn_id::CONTINUATION_RECORD, 1, &[0u8; 8])],
            FaultTag("BridgeRefusal::ContinuationWithoutHead"),
        ),
        (
            "interleaved",
            vec![
                head_declaring(32),
                w::message(fn_id::FREE, 2, &w::driver_free_body(spd::C, spd::DEV)),
            ],
            FaultTag("BridgeRefusal::ContinuationInterleaved"),
        ),
        (
            "overflow",
            vec![head_declaring(4096)],
            FaultTag("BridgeRefusal::ContinuationOverflow"),
        ),
        (
            "count",
            vec![
                head_declaring(32),
                w::message(fn_id::CONTINUATION_RECORD, 2, &[]),
                w::message(fn_id::CONTINUATION_RECORD, 3, &[]),
                w::message(fn_id::CONTINUATION_RECORD, 4, &[]),
            ],
            FaultTag("BridgeRefusal::ContinuationCountExceeded"),
        ),
        (
            "overrun",
            vec![
                head_declaring(32),
                w::message(fn_id::CONTINUATION_RECORD, 2, &[0xcd; 33]),
            ],
            FaultTag("BridgeRefusal::ContinuationOverrun"),
        ),
    ];

    for (what, run, tag) in provocations {
        let mut r = Reassembler::with_limits(limits);
        let out = absorb(&mut r, &run);
        let last = out
            .last()
            .expect("a provocation posts at least one message");
        let err = last.clone().expect_err(what);
        assert_eq!(err.fault_tag(), tag, "{what} refused for the wrong reason");
        assert_ne!(err.rpc_result(), 0, "{what}: a refusal still owes a reply");
        assert!(!r.in_flight(), "{what} left a head in flight");
        assert_eq!(r.held_bytes(), 0, "{what} left bytes held");

        // ★ The recovery, by exact content: the very next conforming run completes, and
        // completes into the same command a virgin reassembler produces.
        let after = absorb(&mut r, &clean);
        assert_eq!(
            after,
            vec![
                Ok(Reassembled::Held),
                Ok(Reassembled::Complete(RpcCommand {
                    function: RpcFunction::RmControl,
                    code: fn_id::GSP_RM_CONTROL,
                    sequence: 0x1100,
                    payload: whole.payload.clone(),
                    elements: 2,
                    delivered: Vec::new(),
                })),
            ],
            "★ the reassembler is wedged after {what}",
        );
    }
}

// ---------------------------------------------------------------------------------
// 7.4 Through the policy and the graph
// ---------------------------------------------------------------------------------

/// The script that must exist before a `SET_PAGE_DIRECTORY` can apply: a client, its
/// Device, its VASpace.
fn spd_prerequisites() -> RpcScript {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, spd::C, 0xdd13)
        .device(spd::C, spd::C, spd::DEV, cp::DEVICE_INSTANCE)
        .vaspace(spd::C, spd::DEV, spd::VAS);
    s
}

/// ★★ **The B6 headline through the policy**: a `SET_PAGE_DIRECTORY` delivered in
/// fragments reaches the object model as the identical fact the unfragmented one does —
/// and the counters say a fragmentation actually happened.
///
/// The oracle is `Boundaries` from a device that saw the **unfragmented** stream, which
/// shares no code with the splitter. The `held` counter is the non-vacuity instrument:
/// without it, a reassembler that silently ignored fragmentation and a correct one look
/// identical from the end state.
#[test]
fn a_fragmented_control_reaches_the_graph_as_the_unfragmented_one_does() {
    for (what, split) in [
        ("at the params boundary", frag::SPLIT_AT_PARAMS),
        ("inside params", frag::SPLIT_MID_PARAMS),
        ("eight bytes left over", frag::SPLIT_LATE),
    ] {
        let prereq = spd_prerequisites();
        let body = command(&spd_whole(0x1100)).payload;
        let run = w::fragment(fn_id::GSP_RM_CONTROL, 0x1100, &body, split);

        let mut gpu = fresh_gpu();
        let (out, applied, held, census) = {
            let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
            for (i, o) in deliver_all(&mut policy, &prereq.messages())
                .into_iter()
                .enumerate()
            {
                let _ = o.unwrap_or_else(|e| panic!("prerequisite {i}: {e:?}"));
            }
            let out = deliver_all(&mut policy, &run);
            (
                out,
                policy.applied(),
                policy.held(),
                policy.census().clone(),
            )
        };

        // Every fragment but the last is `Held`; the last is the fact, by exact variant.
        let (last, rest) = out.split_last().expect("a run");
        assert!(
            rest.iter().all(|o| o == &Ok(Translation::Held)),
            "{what}: {rest:?}",
        );
        assert_eq!(
            last,
            &Ok(Translation::Event(expected_set_page_dir(
                spd::C,
                spd::VAS,
                spd::PDB
            ))),
            "★ {what}: the fact is the head's, recovered from bytes split across messages",
        );
        assert!(
            census.is_empty(),
            "{what}: a conforming run refuses nothing"
        );
        assert_eq!(applied, 4, "{what}: three prerequisites and the control");
        assert_eq!(
            held,
            run.len() as u64 - 1,
            "★ {what}: the fragmentation really happened",
        );

        // And the object model is the unfragmented one's, whole.
        let mut whole_script = spd_prerequisites();
        whole_script.set_page_dir(spd::C, spd::DEV, spd::VAS, spd::PDB, 0);
        assert_eq!(
            boundaries(&gpu),
            boundaries(&gpu_from_script(&whole_script)),
            "★ {what}: fragmentation is invisible to the object model",
        );
    }
}

/// ★★ **Two identical fragmented controls produce two identical events.** The
/// statefulness canary for B6, in the shape §3.3 and §4.3 care about.
///
/// A reassembler that grew a memo keyed by anything the guest supplies — the head's
/// handles, its sequence, its cmd — would answer the second run differently. It must not:
/// a *replayed* message maps to the *identical* event, which is exactly what makes
/// `RmGraphError::ConflictingAlloc`'s idempotent-retry tolerance reachable.
#[test]
fn two_identical_fragmented_controls_produce_two_identical_events() {
    let body = command(&spd_whole(0x1100)).payload;
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0x1100, &body, frag::SPLIT_AT_PARAMS);

    let mut gpu = fresh_gpu();
    let (first, second, third, held) = {
        let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
        for o in deliver_all(&mut policy, &spd_prerequisites().messages()) {
            let _ = o.expect("prerequisites");
        }
        let first = deliver_all(&mut policy, &run);
        let second = deliver_all(&mut policy, &run);
        // ★ And a run whose handles are RECYCLED from the same values but split
        // differently: the reassembler must not have learned anything from the first two.
        let third = deliver_all(
            &mut policy,
            &w::fragment(fn_id::GSP_RM_CONTROL, 0x2200, &body, frag::SPLIT_MID_PARAMS),
        );
        (first, second, third, policy.held())
    };

    let want = Ok(Translation::Event(expected_set_page_dir(
        spd::C,
        spd::VAS,
        spd::PDB,
    )));
    assert_eq!(first, vec![Ok(Translation::Held), want.clone()]);
    assert_eq!(
        second, first,
        "★ the second run is byte-identical in outcome"
    );
    assert_eq!(
        third.last(),
        Some(&want),
        "★ a different split of the same message is the same fact",
    );
    assert_eq!(
        held,
        1 + 1 + (third.len() as u64 - 1),
        "each run held its own fragments and remembered nothing between them",
    );
}

/// ★ The reassembler is **per policy**, not global or shared — two policies over two
/// devices interleaving their fragment runs must not see each other's heads.
///
/// This is the multi-process shape at the transport level: `GraphPolicy` is the per-device
/// object, and a reassembler hoisted into a `static` or a shared table (the obvious
/// "optimisation") is the #14 identical-handles collision one layer down.
#[test]
fn two_policies_interleaving_fragment_runs_do_not_share_a_head() {
    let body = command(&spd_whole(0x1100)).payload;
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0x1100, &body, frag::SPLIT_AT_PARAMS);
    assert_eq!(run.len(), 2);

    let mut a = fresh_gpu();
    let mut b = fresh_gpu();
    let (outs_a, outs_b) = {
        let mut pa = GraphPolicy::new(abi(), GuestOs::Linux, &mut a);
        let mut pb = GraphPolicy::new(abi(), GuestOs::Linux, &mut b);
        for p in [&mut pa, &mut pb] {
            for o in deliver_all(p, &spd_prerequisites().messages()) {
                let _ = o.expect("prerequisites");
            }
        }
        // Element-by-element interleave, which is the hostile order.
        let mut outs_a = Vec::new();
        let mut outs_b = Vec::new();
        for m in &run {
            outs_a.push(pa.deliver(&command(m)));
            outs_b.push(pb.deliver(&command(m)));
        }
        (outs_a, outs_b)
    };

    let want = Ok(Translation::Event(expected_set_page_dir(
        spd::C,
        spd::VAS,
        spd::PDB,
    )));
    assert_eq!(outs_a.last(), Some(&want));
    assert_eq!(
        outs_b, outs_a,
        "★ neither policy stole the other's fragments"
    );
    assert_eq!(boundaries(&a), boundaries(&b));
}

// ---------------------------------------------------------------------------------
// 7.5 ★★ Through the real transport — and the reply-per-fragment answer, on the wire
// ---------------------------------------------------------------------------------

/// ★★ **The settled reply question, asserted as wire bytes.**
///
/// `_issueRpcLarge` sends every fragment and then waits ONCE at
/// `(expectedFunc, firstSequence)`; but `rpcRmApiControl_GSP` issues fn 76 with
/// `bBidirectional = NV_TRUE` (`ogkm-610: rpc.c:10856`, `ogkm-580: :11051`), so the receive
/// side then polls `(CONTINUATION_RECORD, firstSequence + i)` for each record until the
/// reply bytes fill the request's own `bufSize` (identical at both tags:
/// `ogkm-580: rpc.c:2165-2205`, `ogkm-610: :2186-2226`). **A reply per
/// fragment is therefore required**, and each must echo that fragment's own
/// `(function, sequence)`.
///
/// So this test asserts the exact reply stream, by content: the head answered on fn 76 at
/// the first sequence, then one fn-71 reply per continuation at successive sequences —
/// which is precisely what the FSM already posts, and which is why B6 changed no
/// transport code.
#[test]
fn a_fragmented_control_is_answered_once_per_fragment_on_its_own_sequence() {
    let mut script = spd_prerequisites();
    let steps_before = script.steps().len();
    // Post the fragment run as raw steps so the ring carries the real function ids.
    let body = command(&spd_whole(0)).payload;
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0, &body, frag::SPLIT_AT_PARAMS);
    assert_eq!(
        run.len(),
        2,
        "★ `SET_PAGE_DIRECTORY` cannot make more — see `frag`"
    );
    for m in &run {
        let c = command(m);
        script.raw(c.code, c.payload);
    }

    let mut gpu = fresh_gpu();
    let out = run_through_transport(P580, script.steps(), &mut gpu);

    // `run_through_transport` numbers the stream from 0x1000, so the head is at
    // 0x1000 + steps_before and each continuation follows it — written out from the
    // driver's `firstSequence + i` rule rather than read back off the replies.
    let first = 0x1000 + steps_before as u32;
    let want: Vec<(u32, u32, u32)> = std::iter::once((fn_id::GSP_RM_CONTROL, first, 0))
        .chain((1..run.len()).map(|i| (fn_id::CONTINUATION_RECORD, first + i as u32, 0)))
        .collect();
    assert_eq!(
        out.replies
            .iter()
            .skip(steps_before)
            .map(|m| (m.function, m.sequence, m.rpc_result))
            .collect::<Vec<_>>(),
        want,
        "★ one reply per fragment, each on its own (function, sequence), all NV_OK",
    );
    assert_eq!(
        out.applied, 4,
        "three prerequisites and the reassembled control"
    );
    assert_eq!(out.held, run.len() as u64 - 1);
    assert!(out.census.is_empty());

    // And the fact really landed: the same object model the unfragmented script reaches.
    let mut whole_script = spd_prerequisites();
    whole_script.set_page_dir(spd::C, spd::DEV, spd::VAS, spd::PDB, 0);
    assert_eq!(
        boundaries(&gpu),
        boundaries(&gpu_from_script(&whole_script))
    );
}

/// ★★ **A refused fragmented control fails on its LAST fragment's reply — which is the
/// one the driver reads the status from.**
///
/// After the continuation loop the driver reads `rpc_result` out of the message header,
/// i.e. out of the last record it received (identical at both tags but for the accessor
/// name — `pVgpuRpcHeader` at 610, the `vgpu_rpc_message_header_v` macro at 580:
/// `ogkm-580: rpc.c:2209-2220`, `ogkm-610: :2230-2241`). Reassembly completes on
/// that same last fragment, so the head and every intermediate one ack `NV_OK` and the
/// real outcome rides the final reply. The two facts line up without either side
/// arranging it, which is exactly why it is worth pinning: a change to *either* breaks a
/// guest silently.
#[test]
fn a_refused_fragmented_control_carries_its_status_on_the_last_fragment() {
    // A fragmented control the port does not model: `UnknownControl` — refused by
    // `translate`, on the reassembled whole, and therefore not before the last fragment.
    let body = big_control_body(frag::BIG_PARAMS as u32, frag::BIG_PARAMS);
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0, &body, frag::SPLIT_AT_PARAMS);
    assert_eq!(run.len(), 6, "a head and five records with intermediates");

    let mut script = RpcScript::new();
    for m in &run {
        let c = command(m);
        script.raw(c.code, c.payload);
    }
    let mut gpu = fresh_gpu();
    let out = run_through_transport(P580, script.steps(), &mut gpu);

    let statuses: Vec<u32> = out.replies.iter().map(|m| m.rpc_result).collect();
    let (last, rest) = statuses.split_last().expect("replies");
    assert!(
        rest.iter().all(|&s| s == 0),
        "★ the head and every intermediate fragment ack NV_OK: {statuses:?}",
    );
    assert_eq!(
        *last,
        BridgeRefusal::UnknownControl {
            cmd: UNMODELLED_CMD
        }
        .rpc_result(),
        "★ the outcome rides the fragment the driver reads the status off",
    );
    assert_ne!(*last, 0);
    assert_eq!(
        out.census.of(FaultTag("BridgeRefusal::UnknownControl")),
        1,
        "counted ONCE — the fragments are one message, not three",
    );
    assert_eq!(out.census.total(), 1);
    assert_eq!(out.held, run.len() as u64 - 1);
    assert_eq!(out.applied, 0);
}

/// ★ Hostile fragment traffic through the **real ring**, interleaved with a valid stream:
/// the valid stream is unaffected and each refusal is counted by variant.
///
/// The §5.3 shape, narrowed to B6's surface: the point is that a continuation refusal is a
/// property of the *stream* and a ring is what delivers a stream, so the direct-path
/// results above must survive a transport that batches, wraps and checksums.
#[test]
fn hostile_fragment_traffic_through_the_ring_leaves_the_valid_stream_untouched() {
    let mut script = spd_prerequisites();
    // A bare continuation, then a head abandoned by a valid message, then the real thing.
    let body = command(&spd_whole(0)).payload;
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0, &body, frag::SPLIT_AT_PARAMS);
    let head = command(&run[0]);
    let tail = command(&run[1]);

    script
        .raw(fn_id::CONTINUATION_RECORD, vec![0xab; 48])
        .raw(head.code, head.payload.clone())
        .free(spd::C, 0xdead_0001)
        .raw(head.code, head.payload.clone())
        .raw(tail.code, tail.payload.clone());

    let mut gpu = fresh_gpu();
    let out = run_through_transport(P580, script.steps(), &mut gpu);

    assert_eq!(
        out.census.tags().collect::<Vec<_>>(),
        vec![
            (FaultTag("BridgeRefusal::ContinuationInterleaved"), 1),
            (FaultTag("BridgeRefusal::ContinuationWithoutHead"), 1),
            // ★ The `FREE` that interrupted the head is itself refused as the
            // interrupter — it never reaches the graph — so there is no `FreeUnknown`
            // here, and its absence is the assertion.
        ],
        "★ by exact tag and exact count",
    );
    assert_eq!(
        out.applied, 4,
        "three prerequisites and the reassembled control"
    );
    assert_eq!(out.held, 2, "the abandoned head and the surviving one");

    // The valid stream is untouched: the same object model as the clean script.
    let mut clean = spd_prerequisites();
    clean.set_page_dir(spd::C, spd::DEV, spd::VAS, spd::PDB, 0);
    assert_eq!(
        boundaries(&gpu),
        boundaries(&gpu_from_script(&clean)),
        "★ hostile fragment traffic is inert to the graph",
    );
}

// =================================================================================
// 4b. The capability gate — the ported default-deny boundary, driven from the wire
// =================================================================================

/// ★★★ **The gap, closed and observed end to end.** A guest that names an allocation
/// class outside the ported allowlist is answered on the wire with a non-zero
/// `rpc_result`, the refusal is counted, and **the object model is untouched**.
///
/// Driven through the real msgq ring rather than through `translate`, because the claim
/// is about the boundary a guest can actually reach: before this, `kayfabe_fwd` answered
/// `Forwarded` for anything that was not Case-2 and nothing anywhere asked whether a
/// class was permitted, so a guest could name **any** `hClass` at all
/// (`docs/design/eight_blockers_resolved.md` §6).
///
/// ★ The mean part: the refused alloc is in the *middle* of a legal stream, so the test
/// also says that a refusal does not poison what follows — the surrounding client roots
/// still land, and the projection is the one the clean script produces.
#[test]
fn an_unpermitted_alloc_class_is_refused_on_the_wire_and_declares_nothing() {
    let mut script = RpcScript::new();
    script
        .client_root(w::NV01_ROOT, x::A, x::PID_A)
        // ★ `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` — a class nvproxy deliberately omits and
        // the C omitted with it, because it pins a descriptor over the CALLER's own
        // address range, and in Mode 2 the caller is the guest kernel.
        .alloc(x::A, x::A, 0x5c00_0099, 0x0000_0071, &[0u8; 16])
        // …and one nobody has ever seen.
        .alloc(x::A, x::A, 0x5c00_009a, 0x0000_f001, &[0u8; 16])
        .client_root(w::NV01_ROOT_CLIENT, x::B, x::PID_B)
        .client_root(w::NV01_ROOT, x::K, w::KERNEL_PID)
        .free(x::A, x::A);

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    let statuses: Vec<u32> = run.replies.iter().map(|m| m.rpc_result).collect();
    assert_eq!(
        statuses,
        vec![0, 0x56, 0x56, 0, 0, 0],
        "the two refused allocs are answered NV_ERR_NOT_SUPPORTED and nothing else is",
    );

    // ★★ The status word alone CANNOT tell the gate from the decoder — both answer
    // `NV_ERR_NOT_SUPPORTED`, deliberately, because the guest must not learn which
    // (`BridgeRefusal::rpc_result` is one value for every variant). So the discriminating
    // assertion is on the *variant*, and it is a triple: a class the gate refuses by
    // name, one it refuses as unknown, and one it PERMITS and merely cannot decode.
    // Removing the gate collapses all three onto the third, and only this says so.
    let alloc = |class: u32| {
        xlate(&w::message(
            fn_id::GSP_RM_ALLOC,
            9,
            &w::alloc_body(
                x::A,
                x::A,
                0x5c00_0099,
                class,
                16,
                w::RMAPI_RPC_FLAGS_NONE,
                &[0u8; 16],
            ),
        ))
    };
    assert_eq!(
        alloc(0x0000_0071),
        Err(BridgeRefusal::AllocClassNotPermitted {
            class: 0x0000_0071,
            denial: Denial::Refused {
                name: "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR",
                why: DeniedBecause::CallerMemoryDescriptor,
            },
        }),
    );
    assert_eq!(
        alloc(0x0000_f001),
        Err(BridgeRefusal::AllocClassNotPermitted {
            class: 0x0000_f001,
            denial: Denial::NotOnAllowlist,
        }),
    );
    // A class on the allowlist with no decoder — permitted, unmodelled, and therefore the
    // OTHER refusal. This is the distinction that must not move.
    //
    // ★★ **This row used to be `NV20_SUBDEVICE_0` (`0x2080`), and moving it is the point.**
    // On 2026-08-01 the subdevice got an `alloc_params` row (the `GspRmAlloc` rung: a live
    // boot showed the guest's kernel RM allocating one during `RmInitAdapter`), so it is no
    // longer permitted-but-unmapped and this assertion went red — correctly. The exemplar
    // is now `NV01_EVENT_OS_EVENT` (`0x79`): nvproxy lists it, this port models it in no
    // params table, and it is a *different* class from the `0x7e` the same rung added, so
    // the two refusals stay distinguishable.
    //
    // ⚠ The day `0x79` gets a decoder too, move this row again — do NOT delete the
    // assertion. `UnmappedAllocClass` and `AllocClassNotPermitted` being different answers
    // is the whole finding this test carries.
    assert_eq!(
        alloc(0x0000_0079),
        Err(BridgeRefusal::UnmappedAllocClass { class: 0x0000_0079 }),
    );
    assert_eq!(
        run.census
            .of(FaultTag("BridgeRefusal::AllocClassNotPermitted::Refused")),
        1,
        "the named refusal is counted as itself",
    );
    assert_eq!(
        run.census.of(FaultTag(
            "BridgeRefusal::AllocClassNotPermitted::NotOnAllowlist"
        )),
        1,
        "…and the unknown one separately — a probe is not an unimplemented feature",
    );
    assert_eq!(run.census.total(), 2);
    // ★ Non-vacuity: the run really did apply the surrounding traffic. Zero refusals over
    // a run that applied nothing would prove nothing, and neither would two refusals over
    // a run that got no further.
    assert_eq!(run.applied, 4, "the four legal verbs still landed");
    // ★★ And the graph is *exactly* the clean script's graph: a refused alloc declared no
    // node, so the projection cannot tell this run from `script_x()`.
    assert_eq!(boundaries(&gpu), boundaries_of_scenario(&scenario_x()));
}

/// The control half, same shape: a command outside the ported allowlist is refused before
/// its params are decoded, and the two denial kinds are counted apart.
///
/// ★ The third command is the one that says the gate is a *gate* and not a rename: it is
/// `SET_PAGE_DIRECTORY`, which is permitted, modelled, and lands as a fact in the same
/// run. A gate that refused everything would pass every assertion above this line.
#[test]
fn an_unpermitted_control_is_refused_before_its_params_are_read() {
    let mut script = RpcScript::new();
    script
        .client_root(w::NV01_ROOT, spd::C, x::PID_A)
        .device(spd::C, spd::C, spd::DEV, 0)
        .vaspace(spd::C, spd::DEV, spd::VAS)
        // Refused by name: arbitrary register peek/poke.
        .control(spd::C, spd::DEV, 0x2080_0122, &[0u8; 32])
        // Refused as unknown: on nobody's list.
        .control(spd::C, spd::DEV, UNPERMITTED_CMD, &[0u8; 32])
        // …and the modelled one still gets through, in the same run.
        .set_page_dir(
            spd::C,
            spd::DEV,
            spd::VAS,
            spd::PDB,
            w::PDB_FLAGS_ALL_CHANNELS,
        );

    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    assert_eq!(
        run.replies.iter().map(|m| m.rpc_result).collect::<Vec<_>>(),
        vec![0, 0, 0, 0x56, 0x56, 0],
    );
    assert_eq!(
        run.census
            .of(FaultTag("BridgeRefusal::ControlNotPermitted::Refused")),
        1,
    );
    assert_eq!(
        run.census.of(FaultTag(
            "BridgeRefusal::ControlNotPermitted::NotOnAllowlist"
        )),
        1,
    );
    assert_eq!(run.census.total(), 2);
    assert_eq!(run.applied, 4, "three allocs and the page-directory fact");
}

/// ★★ **The known-good set still passes.** The whole compute bring-up sequence — client,
/// device, VASpace, TSG, subcontext, channel, engine object, page directory — runs
/// through the gate with **zero** refusals.
///
/// This is the half of the deliverable that a default-deny change is most likely to break
/// silently, and it is the half a "refuse everything" bug would fail. It is asserted as a
/// census *bound* rather than as an absence, with `applied` as the non-vacuity instrument.
#[test]
fn the_whole_compute_bringup_passes_the_gate_with_no_refusals() {
    let script = script_compute();
    let mut gpu = fresh_gpu();
    let run = run_through_transport(P580, script.steps(), &mut gpu);

    assert!(
        run.census.is_empty(),
        "the compute bring-up must not be refused anywhere: {:?}",
        run.census.tags().collect::<Vec<_>>(),
    );
    assert!(
        run.applied >= 7,
        "non-vacuity: the run has to have DONE something ({} applied)",
        run.applied,
    );
    assert!(
        run.replies.iter().all(|m| m.rpc_result == 0),
        "every reply is NV_OK",
    );
    // ★ And every class the script names really is on the ported allowlist — read from
    // the table rather than assumed, so this cannot pass because the script drifted.
    let caps = abi().capabilities();
    for class in [
        w::NV01_ROOT,
        w::NV01_DEVICE_0,
        w::FERMI_VASPACE_A,
        w::KEPLER_CHANNEL_GROUP_A,
        w::FERMI_CONTEXT_SHARE_A,
        w::AMPERE_CHANNEL_GPFIFO_A,
        w::AMPERE_COMPUTE_B,
        w::AMPERE_DMA_COPY_B,
    ] {
        assert!(
            caps.alloc_class(kayfabe_arch::ids::ClassId(class))
                .is_permitted(),
            "{class:#010x}",
        );
    }
}

/// ★ A **fragmented** control the gate refuses is refused on the last fragment, exactly
/// as an unmodelled one is — the reassembler runs first and decides nothing, so the gate
/// sees the whole message or none of it.
///
/// Without this, a guest could split an unpermitted command across records and the head's
/// `NV_OK` ack would be indistinguishable from acceptance.
#[test]
fn a_fragmented_unpermitted_control_is_refused_on_the_last_fragment() {
    let body = w::control_body(
        spd::C,
        spd::DEV,
        UNPERMITTED_CMD,
        frag::BIG_PARAMS as u32,
        w::RMAPI_RPC_FLAGS_NONE,
        &[0x5a; frag::BIG_PARAMS],
    );
    let run = w::fragment(fn_id::GSP_RM_CONTROL, 0, &body, frag::SPLIT_AT_PARAMS);
    assert!(run.len() >= 3, "a head and intermediates");

    let mut script = RpcScript::new();
    for m in &run {
        let c = command(m);
        script.raw(c.code, c.payload);
    }
    let mut gpu = fresh_gpu();
    let out = run_through_transport(P580, script.steps(), &mut gpu);

    let statuses: Vec<u32> = out.replies.iter().map(|m| m.rpc_result).collect();
    let (last, rest) = statuses.split_last().expect("replies");
    assert!(rest.iter().all(|&s| s == 0), "{statuses:?}");
    assert_eq!(*last, 0x56);
    assert_eq!(
        out.census.of(FaultTag(
            "BridgeRefusal::ControlNotPermitted::NotOnAllowlist"
        )),
        1,
        "counted ONCE — the fragments are one message",
    );
    assert_eq!(out.held, run.len() as u64 - 1);
    assert_eq!(out.applied, 0);
}
