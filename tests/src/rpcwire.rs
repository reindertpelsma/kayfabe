//! GSP-RPC **wire bytes**, built from NVIDIA's own struct definitions — the independent
//! half of the bridge's oracle.
//!
//! # ★★ Why this file imports nothing
//!
//! `docs/design/gsp_core_bridge.md` §5.1 states the rule this file exists to obey:
//!
//! > **An oracle may not derive the value under test from the artifact under test.**
//!
//! It was written because `gspworld::Guest::recv` — an oracle that *is* independent about
//! the checksum and the acceptance predicate — reads `rpc_length` **out of the element
//! under test** and derives everything from it, so `encode_message`'s `elem_count` write
//! is unobserved by its own oracle and no test could fail before the fix.
//!
//! The obvious trap here is the same shape one level up: *build the RPC bytes with a
//! helper, decode them with the bridge, assert the round trip*. That tests nothing but the
//! helper's agreement with itself.
//!
//! So this file `use`s **nothing** — not `kayfabe_abi`, not `kayfabe_rmrpc`, not
//! `kayfabe_gsp`. Every offset below is a literal, written beside the `ogkm` line it came
//! from, transcribed by hand from the header rather than copied from the decoder. If the
//! decoder's offsets and these ever disagree, the tests fail and one of two humans was
//! wrong — which is the only kind of agreement worth having here.
//!
//! ★ The tests pair each builder with a **hand-written hex array** of the same message
//! (`tests/tests/rmrpc_bridge.rs`), so there is a third transcription with no shared code
//! path at all.

/// `sizeof(rpc_message_header_v03_00)` — the envelope every RPC body sits behind.
///
/// `[src]` `ogkm: src/nvidia/inc/kernel/vgpu/rpc_headers.h`: `header_version@0`,
/// `signature@4`, `length@8`, `function@12`, `rpc_result@16`, `rpc_result_private@20`,
/// `sequence@24`, `u@28`. **32**, not the 36 the C artifact's stale constant says.
pub const ENVELOPE: usize = 32;

/// `NV_VGPU_MSG_SIGNATURE_VALID` — ASCII `"VRPC"`, little-endian.
pub const SIGNATURE: u32 = 0x4350_5256;

/// `header_version` — MAJOR 3 / MINOR 0.
pub const HEADER_VERSION: u32 = 0x0300_0000;

/// Write a little-endian `u32` at `off`.
fn put32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// One whole RPC message: the 32-byte envelope, then `body`.
///
/// `length` is total-including-header, which is the field a receiver slices the payload
/// with, so it is written from `body.len()` here on purpose — a test that wants to lie
/// about it uses [`with_length`].
#[must_use]
pub fn message(function: u32, sequence: u32, body: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; ENVELOPE + body.len()];
    put32(&mut m, 0, HEADER_VERSION);
    put32(&mut m, 4, SIGNATURE);
    put32(&mut m, 8, (ENVELOPE + body.len()) as u32);
    put32(&mut m, 12, function);
    // +16 rpc_result, +20 rpc_result_private, +24 sequence, +28 u
    put32(&mut m, 24, sequence);
    m[ENVELOPE..].copy_from_slice(body);
    m
}

/// Overwrite a message's declared `length` — the guest-written field an attacker sets.
#[must_use]
pub fn with_length(mut msg: Vec<u8>, length: u32) -> Vec<u8> {
    put32(&mut msg, 8, length);
    msg
}

/// `rpc_gsp_rm_alloc_v03_00` — the `GSP_RM_ALLOC` body.
///
/// `[src]` `ogkm: src/nvidia/generated/g_rpc-structures.h:1408-1419`, transcribed field by
/// field:
///
/// ```text
/// NvHandle hClient;      // +0
/// NvHandle hParent;      // +4
/// NvHandle hObject;      // +8
/// NvU32    hClass;       // +12
/// NvU32    status;       // +16   [OUT]
/// NvU32    paramsSize;   // +20
/// NvU32    flags;        // +24
/// NvU8     reserved[4];  // +28
/// NvU8     params[];     // +32
/// ```
///
/// `params_size` is passed separately from `params` so a test can declare a size the
/// payload does not honour — which is exactly what a hostile guest does.
#[must_use]
pub fn alloc_body(
    h_client: u32,
    h_parent: u32,
    h_object: u32,
    h_class: u32,
    params_size: u32,
    flags: u32,
    params: &[u8],
) -> Vec<u8> {
    let mut b = vec![0u8; 32 + params.len()];
    put32(&mut b, 0, h_client);
    put32(&mut b, 4, h_parent);
    put32(&mut b, 8, h_object);
    put32(&mut b, 12, h_class);
    put32(&mut b, 16, 0); // status: [OUT], the guest sends zero
    put32(&mut b, 20, params_size);
    put32(&mut b, 24, flags);
    put32(&mut b, 28, 0); // reserved[4]
    b[32..].copy_from_slice(params);
    b
}

/// The body a **conforming guest** sends for a client root: `hParent` and `hObject` are
/// both `NV01_NULL_OBJECT`, and `paramsSize` matches the params it carries.
///
/// `[src]` `ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:83-88` — the FWCLIENT macro calls
/// `AllocWithHandle(pRmApi, hclient, NV01_NULL_OBJECT, NV01_NULL_OBJECT, NV01_ROOT, …)`,
/// and `rpcRmApiAlloc_GSP` copies all three through verbatim (`ogkm: rpc.c:11007-11009`).
/// The `0/0` is the whole reason the bridge normalises.
#[must_use]
pub fn client_root_alloc_body(h_class: u32, h_client: u32, process_id: u32) -> Vec<u8> {
    let params = client_root_params(h_client, process_id);
    alloc_body(
        h_client,
        NV01_NULL_OBJECT,
        NV01_NULL_OBJECT,
        h_class,
        params.len() as u32,
        RMAPI_RPC_FLAGS_NONE,
        &params,
    )
}

/// `NV0000_ALLOC_PARAMETERS`, as far as anyone can honestly claim to know it.
///
/// `[src]` `hClient@0` and `processID@4` are the first two members in every ogkm tree and
/// are the only two RM's own writer sets (`ogkm: rpc.h:55,70,75`). The tail
/// (`processName[100]`, `pOsPidInfo`) has **no second oracle** — neither nvproxy nor the C
/// artifact models this struct at all — so this builder emits the prefix followed by
/// zeroed filler and lets the caller say how long the whole thing is. A guest sends
/// `sizeof`; we decode 8 bytes and never look at the rest.
#[must_use]
pub fn client_root_params_sized(h_client: u32, process_id: u32, total: usize) -> Vec<u8> {
    let mut p = vec![0u8; total.max(8)];
    put32(&mut p, 0, h_client);
    put32(&mut p, 4, process_id);
    p.truncate(total);
    p
}

/// [`client_root_params_sized`] at the 8-byte prefix — the smallest params a client root
/// can legally declare and still be decodable.
#[must_use]
pub fn client_root_params(h_client: u32, process_id: u32) -> Vec<u8> {
    client_root_params_sized(h_client, process_id, 8)
}

/// `rpc_free_v03_00` — which *is* `NVOS00_PARAMETERS_v03_00`
/// (`ogkm: g_rpc-structures.h:162-167`), i.e. no wrapper and no header of its own:
///
/// ```text
/// NvHandle hRoot;          // +0
/// NvHandle hObjectParent;  // +4
/// NvHandle hObjectOld;     // +8
/// NvV32    status;         // +12  [OUT]
/// ```
///
/// `[src]` `ogkm: src/common/sdk/nvidia/inc/nvos.h:164-167` for the field order,
/// `ogkm: rpc.c:11147-11149` for what the driver puts in each one.
#[must_use]
pub fn free_body(h_root: u32, h_object_parent: u32, h_object_old: u32) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    put32(&mut b, 0, h_root);
    put32(&mut b, 4, h_object_parent);
    put32(&mut b, 8, h_object_old);
    put32(&mut b, 12, 0); // status: [OUT]
    b
}

/// The body `rpcRmApiFree_GSP` actually sends: `hObjectParent` is always
/// `NV01_NULL_OBJECT` on this path (`ogkm: rpc.c:11148`).
#[must_use]
pub fn driver_free_body(h_client: u32, h_object: u32) -> Vec<u8> {
    free_body(h_client, NV01_NULL_OBJECT, h_object)
}

/// `NV01_NULL_OBJECT` (`ogkm: src/common/sdk/nvidia/inc/nvlimits.h` / `cl0000.h`).
pub const NV01_NULL_OBJECT: u32 = 0;

/// `RMAPI_RPC_FLAGS_NONE` (`ogkm: src/nvidia/inc/kernel/rmapi/rmapi.h:161`).
pub const RMAPI_RPC_FLAGS_NONE: u32 = 0;

/// `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)` (`ogkm: rmapi.h:163`) — transcribed here a
/// SECOND time, deliberately, so a test that builds a serialized alloc does not take the
/// bit from the same constant the predicate under test reads.
pub const RMAPI_RPC_FLAGS_SERIALIZED: u32 = 2;

/// `NV01_ROOT` — the client-root class (`ogkm: src/common/sdk/nvidia/inc/class/cl0000.h`).
/// Transcribed independently of `kayfabe_abi::generated::classes`, same reasoning.
pub const NV01_ROOT: u32 = 0x0;

/// `NV01_ROOT_CLIENT` — the modern spelling of the same resource kind
/// (`ogkm: src/nvidia/generated/g_allclasses.h:289`).
pub const NV01_ROOT_CLIENT: u32 = 0x41;

/// `NV01_DEVICE_0` — a non-root class, for the "this is not a client root" arms.
pub const NV01_DEVICE_0: u32 = 0x80;

/// `KERNEL_PID` — the `processID` a kernel-privileged client declares
/// (`ogkm: rpc.h:67-71`).
pub const KERNEL_PID: u32 = 0xFFFF_FFFF;

/// The wire ids this module's tests use, from the driver's X-macro table
/// (`ogkm: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82, 83,
/// 86, 113, 254, 256`). Transcribed here rather than taken from `gspworld::FUNCTIONS`,
/// for the same independence reason as everything else in this file.
pub mod fn_id {
    /// `SET_GUEST_SYSTEM_INFO`.
    pub const SET_GUEST_SYSTEM_INFO: u32 = 1;
    /// `FREE`.
    pub const FREE: u32 = 10;
    /// `DUP_OBJECT`.
    pub const DUP_OBJECT: u32 = 21;
    /// `UNLOADING_GUEST_DRIVER`.
    pub const UNLOADING_GUEST_DRIVER: u32 = 47;
    /// `GET_GSP_STATIC_INFO`.
    pub const GET_GSP_STATIC_INFO: u32 = 65;
    /// `CONTINUATION_RECORD`.
    pub const CONTINUATION_RECORD: u32 = 71;
    /// `GSP_SET_SYSTEM_INFO`.
    pub const GSP_SET_SYSTEM_INFO: u32 = 72;
    /// `SET_REGISTRY`.
    pub const SET_REGISTRY: u32 = 73;
    /// `GSP_RM_CONTROL`.
    pub const GSP_RM_CONTROL: u32 = 76;
    /// `GSP_RM_ALLOC`.
    pub const GSP_RM_ALLOC: u32 = 103;
    /// `GSP_INIT_DONE` — an EVENT id: ours to send, never to receive.
    pub const GSP_INIT_DONE: u32 = 0x1001;
    /// `POST_EVENT` — likewise.
    pub const POST_EVENT: u32 = 0x1003;
}
