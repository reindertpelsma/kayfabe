//! ★★★ **The `GSP_RM_ALLOC` reply's `params[]` window — the defect no status could show.**
//!
//! Subject: [`kayfabe_gsp::RpcCommand::reply_alloc`] and its dispatch in `GspFsm::answer`.
//!
//! # The mechanism, re-derived from the driver rather than inherited from a comment
//!
//! `rpcWriteCommonHeader` is given plain `sizeof(rpc_gsp_rm_alloc_v03_00)` for an alloc
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11196-11199`, `rpc_common.c:183`), so the
//! declared `rpc.length` stops exactly where the flexible `params[]` begins. Sizing the
//! reply by that length — which is what [`kayfabe_gsp::RpcCommand::reply`] correctly does
//! for every *other* function — leaves everything past it zero, because
//! `encode_message` builds `vec![0u8; total]`.
//!
//! The guest then reads that window back **unconditionally**, from a fixed offset, for
//! `rmapiGetClassAllocParamSize(hClass)` bytes — a **local** seeded from the class, never
//! from anything we send (`ogkm-580: rpc.c:11178`, `:11238-11241`; identical at
//! `ogkm-610: :11043-11047`). Receive copies whole elements
//! (`message_queue_cpu.c:648-650`) and `rpc.length` is only a checksum extent
//! (`:680-682`), so **nothing on the guest side bounds that read by our reply**.
//!
//! ⇒ a zero-filled window is not silence, it is `memset(caller's params, 0)` performed by
//! the guest kernel on a buffer that for every userspace alloc lives on libcuda's stack.
//!
//! # ★★ Why this file exists at all: the symptom has no status
//!
//! The defect's signature is a **SIGSEGV inside libcuda**. It produces no `NV_STATUS`, no
//! refusal, no unserviced-ledger row and no id-diff — every instrument this port owns sits
//! on planes where it is invisible. A boot is the only other oracle, and a boot cannot say
//! *which* bytes were wrong. So the property is pinned here, in the unit the bytes are
//! decided in.
//!
//! # ⊘ What this file does NOT claim
//!
//! - **Not a boot.** `only_live_boots_are_proof`.
//!
//! # ⊘⊘ And it does NOT claim the `[OUT]` fields are an open gap — that was REFUTED
//!
//! This file shipped saying *"echoing is a RESTORE, not a FILL"* and pointing at
//! `NV_GR_ALLOCATION_PARAMETERS.caps` (+12) as an unwritten `[OUT]` field. The audit that
//! followed found the opposite in every direction that matters: **nothing in either driver
//! tree writes `caps`** — `kernel_graphics_object.c` has zero references to `pAllocParams`
//! at `ogkm-580` and `ogkm-610` alike — and the four host captures under
//! `traces/real_ga106/` return `pAllocParms + 0x58` there, a **userspace stack pointer**
//! that tracks ASLR across boxes. ⇒ For `0xc7c0` the echo is the **correct final
//! behaviour**, and synthesising a value would write over a live stack slot.
//! [`the_out_window_is_libcudas_own_bytes_and_must_stay_so`] pins that, and is what the
//! old `echo_is_a_restore_and_not_a_fill` became.

use kayfabe_abi::GuestOs;
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, table_for};
use kayfabe_chips::Ga10xArch;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_gsp::{RpcCommand, RpcFunction};
use kayfabe_isolate::StillbornIsolates;
use kayfabe_rmrpc::GraphPolicy;
use kayfabe_tests::gspworld::{GspWorld, GuestMsg, MODEL_A, P580, REAL_QUEUE_SIZE};
use kayfabe_tests::rpcwire::{self as w, fn_id};

// =====================================================================================
// Harness
// =====================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// `rpc_gsp_rm_alloc_v03_00`'s fixed header, in body coordinates.
///
/// ★ Written as a literal on purpose. `kayfabe_abi::view::RpcAllocReq::HEADER` is the
/// constant the *implementation* uses, so asserting against it would assert a mirror
/// against itself. 32 is read off the struct's member list
/// (`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1491-1502`: seven `NvU32`s minus
/// the four-word `reserved` accounting — `hClient, hParent, hObject, hClass, status,
/// paramsSize, flags` then `params[]`).
const ALLOC_HEADER: usize = 32;

/// The element offset an alloc's `params[]` lands at: 48-byte element header + 32-byte
/// envelope + 32-byte alloc header. The C artifact reads exactly this
/// (`C: src/qemu/nvkvm_gpu_emul.c:6775-6781`, `params = cmd + 112`).
const ELEMENT_PARAMS_AT: usize = 112;

/// A `Gpu` built the way the **port** builds it: the shipped [`Ga10xArch`], a non-spawning
/// isolate factory, a declared guest-physical window.
fn port_gpu() -> Gpu {
    Gpu::new(
        Box::new(Ga10xArch::new()),
        Box::new(StillbornIsolates::new(
            "alloc_params_echo: no forwarding plane",
        )),
        GpaSpace::new(0x10_0000_0000..0x20_0000_0000, 0x1_0000_0000),
    )
    .expect("the port's object model realizes")
}

/// A params blob no zero-fill can be mistaken for, and no header can be mistaken for.
///
/// ⚠ Every byte non-zero and every byte distinct from its neighbours: a test whose
/// expected params were mostly zero would pass against the very bug it is written for.
fn marker_params(n: usize) -> Vec<u8> {
    (0..n).map(|i| (0xA5u8).wrapping_add(i as u8)).collect()
}

/// One `GSP_RM_ALLOC` request, built the way the guest builds it: a declared body of the
/// fixed header **only**, with `params[]` past the declared length in `delivered`.
fn alloc_cmd(class: u32, params: &[u8], declared_params_size: u32) -> RpcCommand {
    let mut header = vec![0u8; ALLOC_HEADER];
    header[0..4].copy_from_slice(&0xc1d0_05fdu32.to_le_bytes()); // hClient
    header[4..8].copy_from_slice(&0x5c00_0002u32.to_le_bytes()); // hParent
    header[8..12].copy_from_slice(&0x5c00_0010u32.to_le_bytes()); // hObject
    header[12..16].copy_from_slice(&class.to_le_bytes()); // hClass
    // +16 status, [OUT], sent as zero.
    header[20..24].copy_from_slice(&declared_params_size.to_le_bytes()); // paramsSize
    header[24..28].copy_from_slice(&w::RMAPI_RPC_FLAGS_NONE.to_le_bytes()); // flags

    let mut delivered = header.clone();
    delivered.extend_from_slice(params);

    RpcCommand {
        function: RpcFunction::RmAlloc,
        code: fn_id::GSP_RM_ALLOC,
        sequence: 0x2001,
        // ★ The declared body is the header ALONE. That is the whole defect's premise, and
        // it is a property of the guest's sender, not a choice this test makes.
        payload: header,
        elements: 1,
        delivered,
    }
}

/// The reply-payload ceiling `GspFsm::answer` computes: `element_size_max` minus the
/// element header minus the envelope.
fn payload_max() -> usize {
    (P580.element_size_max() as usize)
        .saturating_sub(P580.hdr())
        .saturating_sub(w::ENVELOPE)
}

// =====================================================================================
// 1. The unit — and its own negative control, in the same assertion pair
// =====================================================================================

/// ★★★ **The before/after pair.** The general clamp is what shipped, `reply_alloc` is the
/// fix, and both are run over the identical command so the difference is *read* rather
/// than described.
#[test]
fn the_general_clamp_zeroes_the_params_window_and_reply_alloc_restores_it() {
    let params = marker_params(48);
    let cmd = alloc_cmd(w::KEPLER_CHANNEL_GROUP_A, &params, params.len() as u32);
    let body = cmd.payload.clone();

    // ── BEFORE: `RpcCommand::reply`, unchanged and still correct for every other
    // function. Its payload stops at the declared length, so the params window the guest
    // is about to read `rmapiGetClassAllocParamSize(0xa06c)` bytes out of is ZERO.
    let before = cmd.reply(0, &body);
    assert_eq!(
        before.payload.len(),
        ALLOC_HEADER,
        "the general clamp sizes to the DECLARED length, which excludes params[]",
    );

    // ── AFTER.
    let after = cmd.reply_alloc(0, &body, abi(), payload_max());
    assert_eq!(
        after.payload.len(),
        ALLOC_HEADER + params.len(),
        "the alloc reply carries the fixed header AND the params window",
    );
    assert_eq!(
        &after.payload[..ALLOC_HEADER],
        &body[..],
        "the header is the one the port authored, byte for byte",
    );
    assert_eq!(
        &after.payload[ALLOC_HEADER..],
        &params[..],
        "…and the params are the REQUEST'S OWN bytes, verbatim",
    );

    // ⊘ Nothing else moved: same function, same sequence, same status.
    assert_eq!(
        (after.function, after.sequence, after.rpc_result),
        (before.function, before.sequence, before.rpc_result),
    );
}

/// ★★ **A caller-side inversion must be INERT, not dangerous.** This port has already paid
/// for an inverted argument that every unit test of the callee was blind to
/// (`c2c_absent(!self.chip.has_c2c)`), so `reply_alloc` refuses to widen anything that is
/// not an alloc — it delegates to the general clamp instead.
#[test]
fn reply_alloc_on_a_non_alloc_is_exactly_the_general_clamp() {
    let params = marker_params(64);
    let mut cmd = alloc_cmd(w::KEPLER_CHANNEL_GROUP_A, &params, params.len() as u32);
    cmd.function = RpcFunction::RmControl;
    cmd.code = fn_id::GSP_RM_CONTROL;
    let body = cmd.payload.clone();

    assert_eq!(
        cmd.reply_alloc(0, &body, abi(), payload_max()),
        cmd.reply(0, &body),
        "misdirected at a control, `reply_alloc` widens nothing",
    );

    // And the same for `Free`, the other verb `ObjectPolicy` claims — the one most likely
    // to be swept in by a future `match` arm.
    cmd.function = RpcFunction::Free;
    cmd.code = fn_id::FREE;
    assert_eq!(
        cmd.reply_alloc(0, &body, abi(), payload_max()),
        cmd.reply(0, &body),
        "misdirected at FREE, `reply_alloc` widens nothing",
    );
}

/// ★ A **refusal** keeps the named-refusal shape: an empty body, zero-filled to the
/// declared length. The guest short-circuits on the envelope's `rpc_result` ahead of the
/// copy-out (`ogkm-580: rpc.c:1994-2005`), so there is nothing for a params echo to do —
/// and echoing a guest's own bytes under a failing status is exactly the
/// `memcpy(resp, cmd, 4096)` this port refuses to reproduce.
#[test]
fn a_refused_alloc_still_gets_the_named_refusal_shape() {
    let params = marker_params(48);
    let cmd = alloc_cmd(w::KEPLER_CHANNEL_GROUP_A, &params, params.len() as u32);

    let out = cmd.reply_alloc(kayfabe_abi::NV_ERR_NOT_SUPPORTED, &[], abi(), payload_max());
    assert_eq!(out.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert_eq!(out.payload, vec![0u8; ALLOC_HEADER]);
    assert_eq!(out, cmd.reply(kayfabe_abi::NV_ERR_NOT_SUPPORTED, &[]));
}

/// ★★★ **The three clamps, each made to bite ALONE.**
///
/// `n = min(paramsSize, arrived − params_at, payload_max − params_at)`. A test that only
/// exercised the common case would leave two of the three unmeasured, and a clamp that
/// never binds is a clamp nobody has read.
#[test]
fn each_clamp_binds_on_its_own() {
    let params = marker_params(64);

    // (a) `paramsSize` binds: the guest declared LESS than it delivered. We echo what it
    //     declared — its own assertion about its own message.
    let cmd = alloc_cmd(w::AMPERE_COMPUTE_B, &params, 16);
    let out = cmd.reply_alloc(0, &cmd.payload.clone(), abi(), payload_max());
    assert_eq!(out.payload.len(), ALLOC_HEADER + 16);
    assert_eq!(&out.payload[ALLOC_HEADER..], &params[..16]);

    // (b) `arrived` binds: the guest declared MORE than it delivered. ⚠ This is the
    //     hostile direction — `paramsSize` is a number the guest picked, and trusting it
    //     past the run would read host memory into a guest-visible reply.
    let cmd = alloc_cmd(w::AMPERE_COMPUTE_B, &params, 4096);
    let out = cmd.reply_alloc(0, &cmd.payload.clone(), abi(), payload_max());
    assert_eq!(
        out.payload.len(),
        ALLOC_HEADER + params.len(),
        "a `paramsSize` past the delivered run is clamped to the run",
    );
    assert_eq!(&out.payload[ALLOC_HEADER..], &params[..]);

    // (c) `payload_max` binds: the transport ceiling, below both of the above.
    let cmd = alloc_cmd(w::AMPERE_COMPUTE_B, &params, params.len() as u32);
    let out = cmd.reply_alloc(0, &cmd.payload.clone(), abi(), ALLOC_HEADER + 8);
    assert_eq!(out.payload.len(), ALLOC_HEADER + 8);
    assert_eq!(&out.payload[ALLOC_HEADER..], &params[..8]);
}

/// ⊘ A body that is not exactly the fixed header cannot be followed by params — the echo
/// would land at the wrong offset. `reply_alloc` takes the general clamp instead, so a
/// future policy that starts authoring longer alloc bodies gets a *narrow* reply and a red
/// test, never a silently misaligned params window.
#[test]
fn a_body_that_is_not_the_fixed_header_takes_the_general_clamp() {
    let params = marker_params(32);
    let cmd = alloc_cmd(w::AMPERE_DMA_COPY_B, &params, params.len() as u32);

    let long = vec![0x5au8; ALLOC_HEADER + 8];
    assert_eq!(
        cmd.reply_alloc(0, &long, abi(), payload_max()),
        cmd.reply(0, &long),
    );

    let short = vec![0x5au8; ALLOC_HEADER - 1];
    assert_eq!(
        cmd.reply_alloc(0, &short, abi(), payload_max()),
        cmd.reply(0, &short),
    );
}

/// ★★★ **The `0xc7c0` window is libcuda's OWN BYTES, and preserving them is the whole
/// point — ⊘ this is not a gap to be closed by "filling the OUT fields".**
///
/// The refutation, in the order it was established:
/// 1. `NV_GR_ALLOCATION_PARAMETERS` appears in the whole 580 tree only in
///    `resource_list.h` and two read-only vGPU-serialisation sites, and
///    `kernel_graphics_object.c` has **zero** references to `pAllocParams` at either
///    `ogkm-580` or `ogkm-610`. **Nothing writes `caps`.**
/// 2. The four host captures under `traces/real_ga106/` return `pAllocParms + 0x58` at
///    bytes 8..15 — a **userspace stack pointer** that tracks ASLR across boxes and
///    driver builds. `serverAllocApiCopyIn` copied it in; it came straight back out.
///
/// ⇒ Bytes 8..15 must survive the round trip **unaltered**. That is what this asserts,
/// with a value shaped like the pointer the captures carry, so a future change that
/// "authors" the window is red here rather than corrupting a live stack slot in a guest.
#[test]
fn the_out_window_is_libcudas_own_bytes_and_must_stay_so() {
    // A stack-pointer-shaped value at +8..16, where the captures find `pAllocParms+0x58`.
    let mut params = vec![0u8; 32];
    params[8..16].copy_from_slice(&0x0000_7ffe_dead_be58u64.to_le_bytes());
    let cmd = alloc_cmd(w::AMPERE_COMPUTE_B, &params, params.len() as u32);

    let out = cmd.reply_alloc(0, &cmd.payload.clone(), abi(), payload_max());
    assert_eq!(
        &out.payload[ALLOC_HEADER + 8..ALLOC_HEADER + 16],
        &0x0000_7ffe_dead_be58u64.to_le_bytes(),
        "★ the guest's own stack pointer survives the round trip. ⊘ Do NOT 'fix' this by \
         synthesising caps/size — that writes over a live userspace stack slot.",
    );
}

// =====================================================================================
// 2. Through the real transport — because the DISPATCH is caller-side
// =====================================================================================

/// ★★★ **The dispatch is an ARGUMENT at a call site**, which is the shape this project has
/// measured to be invisible to every test of the callee. So the property is also read off
/// the bytes a re-implemented guest driver pulls out of a real message ring: element
/// offset 112, the same fixed offset `rpcRmApiAlloc_GSP` copies from.
#[test]
fn a_scripted_alloc_gets_its_own_params_back_through_the_ring() {
    let mut gpu = port_gpu();
    let mut world = GspWorld::new_sized(P580, MODEL_A, REAL_QUEUE_SIZE);
    let mut policy = GraphPolicy::new(P580.table(), GuestOs::Linux, &mut gpu);

    world.boot_with(&mut policy);
    let init = world.link_and_drain();
    assert_eq!(init.len(), 1, "the bind posts exactly GSP_INIT_DONE");

    // A client root, sent the way `rpcRmApiAlloc_GSP` sends one: `rpc.length` covers the
    // 32-byte fixed header ONLY, with `params[]` carried past it in the element run.
    //
    // ★★★ This is the whole reason `Guest::send_declaring` had to exist. Every alloc
    // fixture in this tree used `Guest::send`, which declares the *whole* payload — under
    // which the general clamp already echoes the params and the defect is UNREACHABLE.
    // The instrument, not the code, is why no test had ever seen it.
    let root = w::client_root_alloc_body(w::NV01_ROOT, 0xc1d0_05fd, 4242);
    world
        .guest
        .send_declaring(
            &mut world.ram,
            fn_id::GSP_RM_ALLOC,
            0x3000,
            &root,
            ALLOC_HEADER,
        )
        .expect("the ring has room");
    world
        .doorbell_with(&mut policy)
        .expect("the doorbell services the ring");
    let replies: Vec<GuestMsg> = world.guest.recv(&mut world.ram).expect("a clean stream");

    assert_eq!(replies.len(), 1);
    let reply = &replies[0];
    assert_eq!(
        (reply.function, reply.sequence, reply.rpc_result),
        (fn_id::GSP_RM_ALLOC, 0x3000, 0),
        "the client root is served",
    );

    // ★ The params the guest sent, read back out of the reply at the SAME offset the
    // guest's own `portMemCopy` reads from.
    let sent_params = &root[ALLOC_HEADER..];
    assert!(
        reply.payload.len() >= ALLOC_HEADER + sent_params.len(),
        "the reply body covers the params window: got {} want >= {}",
        reply.payload.len(),
        ALLOC_HEADER + sent_params.len(),
    );
    assert_eq!(
        &reply.payload[ALLOC_HEADER..ALLOC_HEADER + sent_params.len()],
        sent_params,
        "★★★ the guest reads its OWN params back, not a zero-fill of its caller's stack",
    );
    // ⊘ And the non-vacuity guard: a params window that was all zeros to begin with would
    // pass the assertion above against the very bug it is written for.
    assert!(
        sent_params.iter().any(|b| *b != 0),
        "the fixture's params are non-zero, so the assertion above can fail",
    );

    // The C's fixed element offset, stated once so the two coordinate systems are pinned
    // against each other rather than assumed to agree.
    assert_eq!(
        P580.hdr() + w::ENVELOPE + ALLOC_HEADER,
        ELEMENT_PARAMS_AT,
        "48-byte element header + 32-byte envelope + 32-byte alloc header = 112",
    );
}
