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
/// `[src]` **checked at both tags, byte-identical, same line numbers** — no version seam
/// here. ★ And the struct is **not** in `rpc_headers.h`, which carries only
/// `NV_VGPU_MSG_SIGNATURE_VALID` (`:61`) and the version DRFs (`:56-59`); it is generated:
/// `ogkm-610: src/nvidia/generated/g_rpc-message-header.h:41-52`,
/// `ogkm-580: g_rpc-message-header.h:41-52` — `header_version@0`, `signature@4`,
/// `length@8`, `function@12`, `rpc_result@16`, `rpc_result_private@20`, `sequence@24`,
/// `u@28` (a 4-byte union, `spare`/`cpuRmGfid`, `:33-37` at both). **32**, not the 36 the
/// C artifact's stale constant says.
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

/// `pRpc->maxRpcSize` — `RM_PAGE_SIZE` (`ogkm-610: rpc.c:1002`, `ogkm-580: rpc.c:1000`;
/// same statement, the line drifted).
///
/// The number [`fragment`] splits on. Tests mostly pass something far smaller, because
/// the split arithmetic is what is under test and 4096-byte fixtures hide off-by-ones in
/// pages of zeros.
pub const MAX_RPC_SIZE: usize = 4096;

/// Split one whole RPC message into the fragment run `_issueRpcLarge` would post —
/// **transcribed from the driver's own loop**, not from our reassembler.
///
/// `[src]` `_issueRpcLarge`, **checked at both tags and semantically identical** — only
/// the spelling of the header pointer differs (610 `pVgpuRpcHeader` / `rpcGetVgpuMessageData(pRpc)`
/// vs 580's `vgpu_rpc_message_header_v` / `rpc_message` macros), so there is no version
/// seam in this loop. `ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:2058-2147`,
/// `ogkm-580: rpc.c:2038-2126`; line by line, **610 number first, 580 second**:
///
/// ```text
/// entryLength = NV_MIN(bufSize, pRpc->maxRpcSize);          // :2082 / :2061
/// hdr->length = entryLength;                                // :2088 / :2067  <- the HEAD
/// rpcSendMessage(...)                                       // :2090 / :2069  sequence++
/// entryLength = pRpc->maxRpcSize - sizeof(rpc_message_header_v); // :2108 / :2087
/// while (remainingSize != 0) {
///     if (entryLength > remainingSize) entryLength = remainingSize; // :2111-2112 / :2090-2091
///     portMemCopy(<message data>, entryLength, pBuf8, entryLength); // :2120 / :2099
///     hdr->length   = entryLength + sizeof(rpc_message_header_v);   // :2123 / :2102
///     hdr->function = NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD;     // :2124 / :2103
///     rpcSendMessage(...)                                   // :2126 / :2105  sequence++
/// }
/// ```
///
/// ★ Two details a paraphrase loses, and both are asserted by the tests that use this:
///
/// - `bufSize` counts the **envelope**, so the head carries `max_rpc_size - 32` bytes of
///   body while each continuation carries `max_rpc_size - 32` bytes of the *original
///   buffer* — the fragments are slices of `[envelope ++ body]`, not of `body`;
/// - the sequence increments **per fragment**
///   (`NV_ASSERT(lastSequence == firstSequence + recordCount)`,
///   `ogkm-610: rpc.c:2147` / `ogkm-580: rpc.c:2126`), which is what
///   the receive side polls on.
///
/// A `body` that already fits in one message comes back as a single-element `Vec` with no
/// continuation at all — the *"a head with no continuations still translates"* arm of
/// `gsp_core_bridge.md` §5.2's B6 row.
///
/// # Panics
/// If `max_rpc_size` is not larger than [`ENVELOPE`], which would make the continuation
/// stride zero and the loop non-terminating. The driver has the same precondition, as
/// `NV_ASSERT_OR_RETURN(fixed_param_size <= pRpc->maxRpcSize)` (`ogkm-610: rpc.c:10562`,
/// `ogkm-580: rpc.c:10758` — identical statement, moved).
#[must_use]
pub fn fragment(
    function: u32,
    first_sequence: u32,
    body: &[u8],
    max_rpc_size: usize,
) -> Vec<Vec<u8>> {
    assert!(max_rpc_size > ENVELOPE, "the stride would be zero");
    let whole = message(function, first_sequence, body);
    let head_len = whole.len().min(max_rpc_size);
    // The head is the first `entryLength` bytes of the buffer, relabelled with its own
    // (shorter) length — which `message` writes from the body slice it is given.
    let mut out = vec![message(
        function,
        first_sequence,
        &whole[ENVELOPE..head_len],
    )];
    let stride = max_rpc_size - ENVELOPE;
    let mut at = head_len;
    while at < whole.len() {
        let take = stride.min(whole.len() - at);
        out.push(message(
            CONTINUATION_RECORD,
            first_sequence + out.len() as u32,
            &whole[at..at + take],
        ));
        at += take;
    }
    out
}

/// `NV_VGPU_MSG_FUNCTION_CONTINUATION_RECORD` = 71, for [`fragment`]'s use before
/// [`fn_id`] is in scope. Same number, transcribed once more from
/// `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:81` and
/// `ogkm-580: rpc_global_enums.h:81` — same line at both, no seam.
const CONTINUATION_RECORD: u32 = 71;

/// `rpc_gsp_rm_alloc_v03_00` — the `GSP_RM_ALLOC` body.
///
/// `[src]` `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1408-1419` and
/// `ogkm-580: g_rpc-structures.h:1491-1502` — member for member identical, only the line
/// moved. Transcribed field by field:
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
/// `[src]` `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:83-88`, `ogkm-580: rpc.h:83-88`
/// (**identical at both, same lines**) — the FWCLIENT macro calls
/// `AllocWithHandle(pRmApi, hclient, NV01_NULL_OBJECT, NV01_NULL_OBJECT, NV01_ROOT, …)`,
/// and `rpcRmApiAlloc_GSP` copies all three through verbatim
/// (`ogkm-610: rpc.c:11007-11009`, `ogkm-580: rpc.c:11201-11203`).
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
/// are the only two RM's own writer sets **that this builder can honestly reproduce**
/// (`ogkm-610: rpc.h:55,70,75`, `ogkm-580: rpc.h:55,70,75` — identical at both, same
/// lines). ★ Not the only two it writes: `:76` also stores `pOsPidInfo` from
/// `pClient->pOsPidInfo`, a **kernel pointer** with no wire meaning, on the non-kernel
/// arm. The tail (`processName[100]`, `pOsPidInfo`) has **no second oracle** — neither
/// nvproxy nor the C artifact models this struct at all — so this builder emits the
/// prefix followed by
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

/// Write a little-endian `u64` at `off`.
fn put64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// `NV0080_ALLOC_PARAMETERS` — the Device class's params, all 56 bytes.
///
/// `[src]` `ogkm-610: src/common/sdk/nvidia/inc/class/cl0080.h:54-64`,
/// `ogkm-580: cl0080.h:54-64` — **identical at both, same lines**. (The old `:47-56`
/// pointed at the doc-comment above the typedef, at both tags.) Transcribed field by
/// field with the `NV_DECLARE_ALIGNED(…, 8)` padding written out:
///
/// ```text
/// NvU32    deviceId;          // +0   ★ the multi-GPU routing fact
/// NvHandle hClientShare;      // +4
/// NvHandle hTargetClient;     // +8
/// NvHandle hTargetDevice;     // +12
/// NvV32    flags;             // +16
///                             // +20  4 bytes of padding to the 8-aligned u64
/// NvU64    vaSpaceSize;       // +24
/// NvU64    vaStartInternal;   // +32
/// NvU64    vaLimitInternal;   // +40
/// NvV32    vaMode;            // +48
///                             // +52  4 bytes of tail padding => sizeof == 56
/// ```
///
/// The non-`deviceId` fields are settable so a test can prove that moving one of them
/// does **not** move the fact the bridge reads.
#[must_use]
pub fn device_params(device_id: u32, h_client_share: u32, va_mode: u32) -> Vec<u8> {
    let mut p = vec![0u8; 56];
    put32(&mut p, 0, device_id);
    put32(&mut p, 4, h_client_share);
    // +8 hTargetClient, +12 hTargetDevice, +16 flags, +20 pad
    put64(&mut p, 24, 0);
    put64(&mut p, 32, 0);
    put64(&mut p, 40, 0);
    put32(&mut p, 48, va_mode);
    p
}

/// `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS` — the TSG's params, 20 bytes.
///
/// `[src]` `ogkm-610: src/common/sdk/nvidia/inc/nvos.h:2899-2907`, and — the point of
/// writing it out twice — `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2903-2911` is
/// character for character the same list:
///
/// ```text
/// NvHandle hObjectError;                  // +0
/// NvHandle hObjectEccError;               // +4
/// NvHandle hVASpace;                      // +8   ★ the fact
/// NvU32    engineType;                    // +12  declared, and dropped
/// NvBool   bIsCallingContextVgpuPlugin;   // +16  (NvU8)
///                                         // +17  3 bytes of tail padding => 20
/// ```
#[must_use]
pub fn tsg_params(h_vaspace: u32, engine_type: u32) -> Vec<u8> {
    let mut p = vec![0u8; 20];
    put32(&mut p, 0, 0); // hObjectError
    put32(&mut p, 4, 0); // hObjectEccError
    put32(&mut p, 8, h_vaspace);
    put32(&mut p, 12, engine_type);
    p[16] = 0; // bIsCallingContextVgpuPlugin
    p
}

/// `NV_CTXSHARE_ALLOCATION_PARAMETERS` — 12 bytes, and note `hVASpace` is **first** here
/// and **third** in the TSG. Getting the two the same way round is the bug this layout
/// invites.
///
/// `[src]` `ogkm-610: nvos.h:3223-3228`, `ogkm-580: nvos.h:3232-3237` (identical):
///
/// ```text
/// NvHandle hVASpace;   // +0   ★ the fact
/// NvU32    flags;      // +4
/// NvU32    subctxId;   // +8
/// ```
#[must_use]
pub fn ctxshare_params(h_vaspace: u32, flags: u32, subctx_id: u32) -> Vec<u8> {
    let mut p = vec![0u8; 12];
    put32(&mut p, 0, h_vaspace);
    put32(&mut p, 4, flags);
    put32(&mut p, 8, subctx_id);
    p
}

/// `NV_CHANNEL_ALLOC_PARAMS` — a channel's params, emitted at the **580** length.
///
/// `[src]` `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342`, which is
/// the tree matching this project's bench driver. The typedef opens at `:296` in **both**
/// trees; only the close differs (`ogkm-610: alloc_channel.h:296-347`), because 610 has
/// one extra member — see the `+32` note below:
///
/// ```text
/// NvHandle hObjectError;      // +0
/// NvHandle hObjectBuffer;     // +4
///                             // +8   (NvU64 gpFifoOffset, 8-aligned — already is)
/// NvU64    gpFifoOffset;      // +8
/// NvU32    gpFifoEntries;     // +16
/// NvU32    flags;             // +20  ★ the opaque USERD/flags word
/// NvHandle hContextShare;     // +24  ★
/// NvHandle hVASpace;          // +28  ★
/// NvHandle hUserdMemory[8];   // +32  ── 610 INSERTS `hHandleVASpace` HERE ──
/// ```
///
/// ★★ **The seam, stated exactly.** `flags@20 / hContextShare@24 / hVASpace@28` are
/// identical at both tags (`ogkm-580: alloc_channel.h:303-307`,
/// `ogkm-610: alloc_channel.h:303-309`). At +32, 580 begins `hUserdMemory[NV_MAX_SUBDEVICES]`
/// (`ogkm-580: :310`) while 610 inserts a *new* `hHandleVASpace` (`ogkm-610: :312`) and
/// pushes `hUserdMemory` to +36 (`ogkm-610: :315`). Everything past +32 therefore differs
/// by four bytes of shift, which is why this builder stops there.
///
/// ★★ The builder stops caring at +32 on purpose: everything past it is exactly the
/// region the two vendored trees disagree about, so a fixture that asserted anything
/// there would be asserting one tree's opinion. `total` lets a test choose how long the
/// message claims to be — a real guest sends `sizeof`, which differs *between the two
/// trees* and is therefore not something this file is entitled to state.
#[must_use]
pub fn channel_params_sized(flags: u32, h_ctx_share: u32, h_vaspace: u32, total: usize) -> Vec<u8> {
    let mut p = vec![0u8; total.max(32)];
    put32(&mut p, 20, flags);
    put32(&mut p, 24, h_ctx_share);
    put32(&mut p, 28, h_vaspace);
    p.truncate(total);
    p
}

/// [`channel_params_sized`] at the agreed prefix — the shortest params a channel can
/// declare and still be decodable.
#[must_use]
pub fn channel_params(flags: u32, h_ctx_share: u32, h_vaspace: u32) -> Vec<u8> {
    channel_params_sized(flags, h_ctx_share, h_vaspace, 32)
}

/// `rpc_gsp_rm_control_v03_00` — the `GSP_RM_CONTROL` body.
///
/// `[src]` `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1423-1435` and
/// `ogkm-580: g_rpc-structures.h:1506-1518` — transcribed by hand from **both**, because
/// this file's whole job is to be a second pair of eyes:
///
/// ```text
/// NvHandle hClient;            // +0
/// NvHandle hObject;            // +4
/// NvU32    cmd;                // +8
/// NvU32    status;             // +12  [OUT] — see below, this zero is load-bearing
/// NvU32    paramsSize;         // +16
/// NvU32    rmapiRpcFlags;      // +20
/// NvU32    rmctrlFlags;        // +24
/// NvU32    rmctrlAccessRight;  // +28
/// NvU64    reserved0;          // +32  NV_ALIGN_BYTES(8)
/// NvU8     params[];           // +40  ← not 36
/// ```
///
/// ★ `status` @ +12 is written as zero and that is a *protocol* fact, not tidiness:
/// `rpcWriteCommonHeader` `portMemSet`s the whole message buffer before the sender fills
/// it (`ogkm-610: src/nvidia/src/kernel/rmapi/rpc_common.c:149-152`,
/// `ogkm-580: rpc_common.c:149-152` — identical at both, same lines), and the guest reads
/// **this field out of the reply** to get the control handler's own status
/// (`ogkm-610: rpc.c:10868-10875`, `ogkm-580: rpc.c:11063-11070`). An accepted control is
/// acked with the request body
/// preserved, so the zero the guest sent is the `NV_OK` it reads back.
#[must_use]
pub fn control_body(
    h_client: u32,
    h_object: u32,
    cmd: u32,
    params_size: u32,
    rmapi_rpc_flags: u32,
    params: &[u8],
) -> Vec<u8> {
    let mut b = vec![0u8; 40 + params.len()];
    put32(&mut b, 0, h_client);
    put32(&mut b, 4, h_object);
    put32(&mut b, 8, cmd);
    put32(&mut b, 12, 0); // status: [OUT], the guest sends zero
    put32(&mut b, 16, params_size);
    put32(&mut b, 20, rmapi_rpc_flags);
    put32(&mut b, 24, 0); // rmctrlFlags — `rpcRmApiControl_GSP` sends 0
    put32(&mut b, 28, 0); // rmctrlAccessRight — likewise
    // +32 reserved0 (NvU64)
    b[40..].copy_from_slice(params);
    b
}

/// `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS` — 32 bytes.
///
/// `[src]` `ogkm-610: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:802-810` and
/// `ogkm-580: ctrl0080dma.h:832-840`, which are the same seven members in the same order —
/// so the tail is **not** a 610-only shape, whatever the generated module's caveat says:
///
/// ```text
/// NvU64    physAddress;   // +0   ★ the PDB
/// NvU32    numEntries;    // +8
/// NvU32    flags;         // +12  [1:0] aperture, [2:2] PRESERVE_PDES, …
/// NvHandle hVASpace;      // +16  ★ 0 => the client/device pair's IMPLICIT VAS
/// NvU32    chId;          // +20
/// NvU32    subDeviceId;   // +24
/// NvU32    pasid;         // +28
/// ```
///
/// The three tail fields are settable so a test can prove that moving one of them does
/// **not** move a fact the bridge reads.
#[must_use]
pub fn set_page_dir_params(
    phys_address: u64,
    num_entries: u32,
    flags: u32,
    h_vaspace: u32,
    ch_id: u32,
    sub_device_id: u32,
    pasid: u32,
) -> Vec<u8> {
    let mut p = vec![0u8; 32];
    put64(&mut p, 0, phys_address);
    put32(&mut p, 8, num_entries);
    put32(&mut p, 12, flags);
    put32(&mut p, 16, h_vaspace);
    put32(&mut p, 20, ch_id);
    put32(&mut p, 24, sub_device_id);
    put32(&mut p, 28, pasid);
    p
}

/// `rpc_free_v03_00` — which *is* `NVOS00_PARAMETERS_v03_00`
/// (`ogkm-610: g_rpc-structures.h:162-167`, `ogkm-580: g_rpc-structures.h:160-165`), i.e.
/// no wrapper and no header of its own:
///
/// ```text
/// NvHandle hRoot;          // +0
/// NvHandle hObjectParent;  // +4
/// NvHandle hObjectOld;     // +8
/// NvV32    status;         // +12  [OUT]
/// ```
///
/// `[src]` `ogkm-610: src/common/sdk/nvidia/inc/nvos.h:164-167` /
/// `ogkm-580: nvos.h:162-165` for the field order (same four members, moved);
/// `ogkm-610: rpc.c:11147-11149` / `ogkm-580: rpc.c:11339-11341` for what the driver puts
/// in each one.
#[must_use]
pub fn free_body(h_root: u32, h_object_parent: u32, h_object_old: u32) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    put32(&mut b, 0, h_root);
    put32(&mut b, 4, h_object_parent);
    put32(&mut b, 8, h_object_old);
    put32(&mut b, 12, 0); // status: [OUT]
    b
}

/// `rpc_dup_object_v03_00` — which, like the free body, *is* the bare SDK struct
/// `NVOS55_PARAMETERS_v03_00` with no wrapper of its own
/// (`ogkm-610: g_rpc-structures.h:200-205`, `ogkm-580: g_rpc-structures.h:198-203`):
///
/// ```text
/// NvHandle hClient;     // +0   ★ the DESTINATION client — the message's own namespace
/// NvHandle hParent;     // +4   the new alias's parent (a REAL handle on this path)
/// NvHandle hObject;     // +8   ★ the new alias handle
/// NvHandle hClientSrc;  // +12  ★ the SOURCE namespace — the cross-client edge
/// NvHandle hObjectSrc;  // +16  ★ the source object
/// NvU32    flags;       // +20  NV04_DUP_HANDLE_FLAGS_*
/// NvU32    status;      // +24  [OUT]
///                       //      sizeof == 28
/// ```
///
/// `[src]` field order and widths from `ogkm-610: g_sdk-structures.h:368-377` and
/// `ogkm-580: g_sdk-structures.h:366-375` — transcribed from **both**, and they are
/// identical member for member; `ogkm-610: rpc.c:11098-11103` /
/// `ogkm-580: rpc.c:11291-11296` for what `rpcRmApiDupObject_GSP` puts in each one.
///
/// ★ Note `hParent` is **not** the always-zero the `FREE` path carries: the caller passes
/// `pDstParentRef->hResource`, a live handle in the destination namespace
/// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/mem.c:1116`). So a builder that emitted zero
/// there would make the bridge's *drop* of that field untestable.
#[must_use]
pub fn dup_body(
    h_client: u32,
    h_parent: u32,
    h_object: u32,
    h_client_src: u32,
    h_object_src: u32,
    flags: u32,
) -> Vec<u8> {
    let mut b = vec![0u8; 28];
    put32(&mut b, 0, h_client);
    put32(&mut b, 4, h_parent);
    put32(&mut b, 8, h_object);
    put32(&mut b, 12, h_client_src);
    put32(&mut b, 16, h_object_src);
    put32(&mut b, 20, flags);
    put32(&mut b, 24, 0); // status: [OUT]
    b
}

/// `NV04_DUP_HANDLE_FLAGS_NONE` (`ogkm-610: nvos.h:2276`, `ogkm-580: nvos.h:2275`) — what
/// the memory dup path sends verbatim (`ogkm-580: mem.c:1116-1120` passes a literal `0`).
pub const NV04_DUP_HANDLE_FLAGS_NONE: u32 = 0;

/// `NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE` (`ogkm-610: nvos.h:2277`,
/// `ogkm-580: nvos.h:2276`) — *"prevents an RM kernel client from duping
/// unconditionally"*.
///
/// ★ A **privilege** assertion about the duping client, which the core models as
/// `ClientKind`, declared once at the client root. `RmEvent::Dup` has nowhere to put it
/// and the bridge drops it; this constant exists so a test can prove the drop rather than
/// assume it.
pub const NV04_DUP_HANDLE_FLAGS_REJECT_KERNEL_DUP_PRIVILEGE: u32 = 1;

/// The body `rpcRmApiFree_GSP` actually sends: `hObjectParent` is always
/// `NV01_NULL_OBJECT` on this path (`ogkm-610: rpc.c:11148`, `ogkm-580: rpc.c:11340`).
#[must_use]
pub fn driver_free_body(h_client: u32, h_object: u32) -> Vec<u8> {
    free_body(h_client, NV01_NULL_OBJECT, h_object)
}

/// `NV01_NULL_OBJECT` (`ogkm-610: src/common/sdk/nvidia/inc/class/cl0000.h:37`,
/// `ogkm-580: class/cl0000.h:37` — same line at both).
///
/// ★ It is **not** in `nvlimits.h`, which this comment used to name: that header defines
/// only `NV_MAX_DEVICES` (`:37` at both). The only other spelling is the generated alias
/// `ogkm-610: src/nvidia/generated/g_allclasses.h:275` / `ogkm-580: g_allclasses.h:262`.
pub const NV01_NULL_OBJECT: u32 = 0;

/// `RMAPI_RPC_FLAGS_NONE` (`ogkm-610: src/nvidia/inc/kernel/rmapi/rmapi.h:161`,
/// `ogkm-580: rmapi.h:161` — same line at both; the whole trio `161/162/163` is
/// unmoved).
pub const RMAPI_RPC_FLAGS_NONE: u32 = 0;

/// `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)` (`ogkm-610: rmapi.h:163`,
/// `ogkm-580: rmapi.h:163`) — transcribed here a
/// SECOND time, deliberately, so a test that builds a serialized alloc does not take the
/// bit from the same constant the predicate under test reads.
pub const RMAPI_RPC_FLAGS_SERIALIZED: u32 = 2;

/// `RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR` = `NVBIT(0)` (`ogkm-610: rmapi.h:162`,
/// `ogkm-580: rmapi.h:162`).
///
/// ★ The **neighbour** of the serialized bit, and the reason the serialization question is
/// a bit test rather than `!= 0`: `rpcRmApiControl_GSP` sets this one whenever the control
/// carries `RMCTRL_FLAGS_COPYOUT_ON_ERROR` (`ogkm-610: rpc.c:10803`,
/// `ogkm-580: rpc.c:10998`), entirely
/// independently. A control with only this bit set is perfectly ordinary and must
/// translate.
pub const RMAPI_RPC_FLAGS_COPYOUT_ON_ERROR: u32 = 1;

/// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`ogkm-610: ctrl0080dma.h:798`,
/// `ogkm-580: ctrl0080dma.h:828`) — the one control this port models.
pub const NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY: u32 = 0x0080_1813;

/// `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` (`ogkm-580: ctrl0080dma.h:878`) — the
/// symmetric teardown, one **less** than the modelled command. Adjacent on purpose: a
/// table that got the constant off by one would still look plausible.
pub const NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY: u32 = 0x0080_1814;

/// `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`ogkm-610: ctrl2080gpu.h:947`,
/// `ogkm-580: ctrl2080gpu.h:976`) — the address-plane control.
pub const NV2080_CTRL_CMD_GPU_PROMOTE_CTX: u32 = 0x2080_012b;

/// `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES` (`ogkm-610: ctrl2080gpu.h:917`,
/// `ogkm-580: ctrl2080gpu.h:946`) — **16**, transcribed by hand here so a decoder that
/// hard-codes a different bound disagrees with an independent reading rather than with
/// itself.
pub const PROMOTE_MAX_ENTRIES: usize = 16;

/// `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY)`.
pub const PROMOTE_ENTRY_SIZE: usize = 32;

/// `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)` — 48 + 16 × 32.
pub const PROMOTE_PARAMS_SIZE: usize = 48 + PROMOTE_MAX_ENTRIES * PROMOTE_ENTRY_SIZE;

/// One `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY`, as a caller writes it.
///
/// `[src]` `ogkm-610: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:893-901`,
/// `ogkm-580: ctrl2080gpu.h:922-930` — the same seven members at both:
///
/// ```text
/// NV_DECLARE_ALIGNED(NvU64 gpuPhysAddr, 8);  // +0
/// NV_DECLARE_ALIGNED(NvU64 gpuVirtAddr, 8);  // +8
/// NV_DECLARE_ALIGNED(NvU64 size, 8);         // +16
/// NvU32 physAttr;                            // +24  [1:0] APERTURE
/// NvU16 bufferId;                            // +28  ★ TWO bytes
/// NvU8  bInitialize;                         // +30
/// NvU8  bNonmapped;                          // +31
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct PromoteEntryWire {
    /// `gpuPhysAddr` @ +0.
    pub gpu_phys_addr: u64,
    /// `gpuVirtAddr` @ +8.
    pub gpu_virt_addr: u64,
    /// `size` @ +16.
    pub size: u64,
    /// `physAttr` @ +24.
    pub phys_attr: u32,
    /// `bufferId` @ +28 — 16 bits.
    pub buffer_id: u16,
    /// `bInitialize` @ +30.
    pub b_initialize: u8,
    /// `bNonmapped` @ +31.
    pub b_nonmapped: u8,
}

/// `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` — 560 bytes.
///
/// `[src]` `ogkm-610: ctrl2080gpu.h:959-971`, `ogkm-580: ctrl2080gpu.h:988-1000` — the
/// same ten members at both:
///
/// ```text
/// NvU32    engineType;    // +0
/// NvHandle hClient;       // +4   (the LEGACY path's client, for hVirtMemory)
/// NvU32    ChID;          // +8   (deprecated)
/// NvHandle hChanClient;   // +12  ★ the namespace hObject lives in
/// NvHandle hObject;       // +16  ★ a channel OR a channel group
/// NvHandle hVirtMemory;   // +20  ┐
/// NvU64    virtAddress;   // +24  ├ the LEGACY path
/// NvU64    size;          // +32  ┘
/// NvU32    entryCount;    // +40
/// //                         +44  (padding; promoteEntry is 8-aligned)
/// NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY promoteEntry[16];  // +48
/// ```
///
/// `entry_count` is written **verbatim**, so a test can declare a count that disagrees
/// with `entries.len()` — which is the whole of C defect D1.
#[must_use]
pub fn promote_ctx_params(
    engine_type: u32,
    h_chan_client: u32,
    h_object: u32,
    entry_count: u32,
    entries: &[PromoteEntryWire],
) -> Vec<u8> {
    let mut p = vec![0u8; PROMOTE_PARAMS_SIZE];
    put32(&mut p, 0, engine_type);
    put32(&mut p, 12, h_chan_client);
    put32(&mut p, 16, h_object);
    put32(&mut p, 40, entry_count);
    for (i, e) in entries.iter().enumerate() {
        let at = 48 + i * PROMOTE_ENTRY_SIZE;
        put64(&mut p, at, e.gpu_phys_addr);
        put64(&mut p, at + 8, e.gpu_virt_addr);
        put64(&mut p, at + 16, e.size);
        put32(&mut p, at + 24, e.phys_attr);
        p[at + 28..at + 30].copy_from_slice(&e.buffer_id.to_le_bytes());
        p[at + 30] = e.b_initialize;
        p[at + 31] = e.b_nonmapped;
    }
    p
}

/// The **legacy** `PROMOTE_CTX` shape — `hVirtMemory` / `(virtAddress, size)` set, no
/// entries. Neither real producer emits it; the decoder refuses it by name rather than
/// guessing a reading for it.
#[must_use]
pub fn promote_ctx_legacy_params(
    h_chan_client: u32,
    h_object: u32,
    h_virt_memory: u32,
    virt_address: u64,
    size: u64,
) -> Vec<u8> {
    let mut p = vec![0u8; PROMOTE_PARAMS_SIZE];
    put32(&mut p, 12, h_chan_client);
    put32(&mut p, 16, h_object);
    put32(&mut p, 20, h_virt_memory);
    put64(&mut p, 24, virt_address);
    put64(&mut p, 32, size);
    p
}

/// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`ogkm-580: ctrl90f1.h:268`,
/// `ogkm-610: ctrl90f1.h:268` — same line at both; the old `:272` was the *params*
/// typedef, not the command id).
///
/// ★★ The control that carries the root page directory of every **ordinary** RM-managed
/// VASpace on a GSP client, at construct time, as `levels[0].physAddress`. This port does
/// not decode it and refuses it by name — which is the whole point of having a name.
pub const NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES: u32 = 0x90f1_0106;

/// `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
/// (`ogkm-580: ctrl2080internal.h:1902`, `ogkm-610: ctrl2080internal.h:1905`) — the same
/// payload for the GPU-group global VAS,
/// on the `!IS_VIRTUAL` (i.e. bare-metal) arm.
pub const NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER: u32 =
    0x2080_0a9f;

/// `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_VIDMEM`
/// (`ogkm-610: ctrl0080dma.h:813`, `ogkm-580: ctrl0080dma.h:843`) — `flags[1:0] == 0`,
/// the root is in framebuffer.
pub const PDB_APERTURE_VIDMEM: u32 = 0;

/// `…_APERTURE_SYSMEM_COH` (`ogkm-610: ctrl0080dma.h:814`, `ogkm-580: ctrl0080dma.h:844`)
/// — the root is in guest RAM.
pub const PDB_APERTURE_SYSMEM_COH: u32 = 1;

/// `…_APERTURE_SYSMEM_NONCOH` (`ogkm-610: ctrl0080dma.h:815`,
/// `ogkm-580: ctrl0080dma.h:845`).
pub const PDB_APERTURE_SYSMEM_NONCOH: u32 = 2;

/// `NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_ALL_CHANNELS_TRUE` shifted into `[3:3]`
/// (`ogkm-610: ctrl0080dma.h:819-821`, `ogkm-580: ctrl0080dma.h:849-851` — the field is
/// `_ALL_CHANNELS 3:3` with `_FALSE`/`_TRUE` under it; the old `:820-822` ran one line
/// late and swallowed `_IGNORE_CHANNEL_BUSY`). UVM always sets it
/// (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:8857-8862`), so a realistic
/// fixture carries it — and it also proves the aperture decode masks rather than compares.
pub const PDB_FLAGS_ALL_CHANNELS: u32 = 1 << 3;

/// `NV01_ROOT` — the client-root class
/// (`ogkm-610: src/common/sdk/nvidia/inc/class/cl0000.h:42`, `ogkm-580: cl0000.h:42`).
/// Transcribed independently of `kayfabe_abi::generated::classes`, same reasoning.
pub const NV01_ROOT: u32 = 0x0;

/// `NV01_ROOT_CLIENT` — the modern spelling of the same resource kind
/// (`ogkm-610: src/nvidia/generated/g_allclasses.h:289`, `ogkm-580: g_allclasses.h:276`).
pub const NV01_ROOT_CLIENT: u32 = 0x41;

/// `NV01_DEVICE_0` — the Device class (`ogkm-610: cl0080.h:36`, `ogkm-580: cl0080.h:36`).
pub const NV01_DEVICE_0: u32 = 0x80;

/// `FERMI_VASPACE_A` — the VASpace class (`ogkm-610: cl90f1.h:33`,
/// `ogkm-580: cl90f1.h:33`).
pub const FERMI_VASPACE_A: u32 = 0x90f1;

/// `KEPLER_CHANNEL_GROUP_A` — the TSG class (`ogkm-610: cla06c.h:33`,
/// `ogkm-580: cla06c.h:33`).
pub const KEPLER_CHANNEL_GROUP_A: u32 = 0xa06c;

/// `FERMI_CONTEXT_SHARE_A` — the subcontext class (`ogkm-610: cl9067.h:33`,
/// `ogkm-580: cl9067.h:33`).
pub const FERMI_CONTEXT_SHARE_A: u32 = 0x9067;

/// `AMPERE_CHANNEL_GPFIFO_A` — the GA10x GPFIFO channel class
/// (`ogkm-610: kernel-open/nvidia-uvm/clc56f.h:43`, `ogkm-580: clc56f.h:43`).
///
/// ★ There is only one. A CUDA process's GR channel and its CE channel are both this
/// `hClass`; `NV_CHANNEL_ALLOC_PARAMS.engineType` is the difference, and it lives past
/// the prefix the bridge reads and has nowhere to go in `RmEvent` anyway.
pub const AMPERE_CHANNEL_GPFIFO_A: u32 = 0xc56f;

/// `AMPERE_COMPUTE_B` — the compute engine object (`ogkm-610: clc7c0.h:32`,
/// `ogkm-580: clc7c0.h:32`).
pub const AMPERE_COMPUTE_B: u32 = 0xc7c0;

/// `AMPERE_DMA_COPY_B` — the copy-engine object
/// (`ogkm-610: kernel-open/nvidia-uvm/clc7b5.h:33`, `ogkm-580: clc7b5.h:33`).
pub const AMPERE_DMA_COPY_B: u32 = 0xc7b5;

/// `AMPERE_B` — the GA10x **3D** engine object (`ogkm-610: clc797.h:32`,
/// `ogkm-580: clc797.h:32`).
///
/// ⊘ Not a CUDA process's object. The one allocator on the boot path is RM's own
/// `kgraphicsCreateGoldenImageChannel_IMPL`, which allocates it on the golden-image
/// channel with `NULL, 0` params and then frees the whole tree
/// (`ogkm-580: kernel_graphics.c:2519`).
pub const AMPERE_B: u32 = 0xc797;

/// `NV01_MEMORY_SYSTEM` (`ogkm-610: src/common/sdk/nvidia/inc/class/cl003e.h:33`,
/// `ogkm-580: cl003e.h:33`) — a real
/// class that this port **does not map**, used wherever a test needs one.
///
/// ★ It is the honest choice rather than an invented id, and the reason is a finding
/// rather than convenience: `gsp_core_bridge.md` §6 lists *Memory (`mem_phys`)* as part
/// of stage B3, and it is not buildable. `NV_MEMORY_ALLOCATION_PARAMS.offset`/`address`
/// are `[OUT]` in the guest→GSP direction — RM picks the address and reports it back — so
/// the request carries no physical backing to decode; and the only consumer of
/// `AllocFacts::mem_phys` is the RPC-mapping sync, driven by `RmEvent::MapMemoryDma`,
/// which §2.7 shows never reaches this wire at all. So a memory alloc is refused by name,
/// and that refusal is the record of both facts.
pub const NV01_MEMORY_SYSTEM: u32 = 0x3e;

/// `KERNEL_PID` — the `processID` a kernel-privileged client declares.
///
/// `[src]` the **use** site is `ogkm-610: rpc.h:67-71` / `ogkm-580: rpc.h:67-71`
/// (identical, same lines); the **value** `0xFFFFFFFF` is defined elsewhere and does
/// move: `ogkm-610: src/nvidia/generated/g_kernel_fifo_nvoc.h:103`,
/// `ogkm-580: g_kernel_fifo_nvoc.h:113`.
pub const KERNEL_PID: u32 = 0xFFFF_FFFF;

// =================================================================================
// ★★ `RpcScript` — a whole guest's RPC stream, as BYTES
// =================================================================================

/// One message of a script: the wire function id and the body behind the envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// `rpc.function`.
    pub function: u32,
    /// The RPC body — everything after the 32-byte envelope.
    pub body: Vec<u8>,
}

/// ★★ An ordered stream of RPC messages, built from `ogkm`'s struct definitions and
/// nothing else — the **bytes** half of `gsp_core_bridge.md` §5.1's strongest oracle:
///
/// > `Gpu::apply` fed from `RpcScript(X)` produces the **same** `project::Boundaries` as
/// > `Gpu::apply` fed from `Scenario(X)`.
///
/// `Scenario` is not replaced by this; it becomes the **reference implementation**. The
/// two are written independently — one in `RmEvent`s by hand, one in NVIDIA's wire
/// layout by hand — and the projection is what must not be able to tell them apart. That
/// is why this type lives in *this* file, which imports nothing: a script that took its
/// offsets from the decoder under test would be the round-trip-with-itself trap §5.1
/// exists to name.
///
/// ★ It carries **no sequence numbers**. The transport owns those (`Guest::send` assigns
/// its own `tx_seq` per element, and the envelope's `rpc.sequence` is the guest's
/// transaction id), and a script that baked them in would be asserting a transport fact
/// from the object model's side. [`RpcScript::messages`] numbers them positionally only
/// for the tests that bypass the ring entirely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RpcScript {
    steps: Vec<Step>,
}

impl RpcScript {
    /// An empty script.
    #[must_use]
    pub fn new() -> RpcScript {
        RpcScript::default()
    }

    /// A conforming client-root `GSP_RM_ALLOC` — see [`client_root_alloc_body`].
    pub fn client_root(&mut self, h_class: u32, h_client: u32, process_id: u32) -> &mut RpcScript {
        self.raw(
            fn_id::GSP_RM_ALLOC,
            client_root_alloc_body(h_class, h_client, process_id),
        )
    }

    /// A `FREE` shaped the way `rpcRmApiFree_GSP` shapes one — see [`driver_free_body`].
    pub fn free(&mut self, h_client: u32, h_object: u32) -> &mut RpcScript {
        self.raw(fn_id::FREE, driver_free_body(h_client, h_object))
    }

    /// A `DUP_OBJECT` aliasing `(src_client, src_handle)` into `dst_client`'s namespace as
    /// `dst_handle`, parented at `dst_parent`, with `NV04_DUP_HANDLE_FLAGS_NONE` — the
    /// shape the memory dup path sends (`ogkm-580: mem.c:1116-1119`). A test that wants a
    /// different `flags` builds the body with [`dup_body`] and posts it through
    /// [`Self::raw`].
    pub fn dup(
        &mut self,
        dst_client: u32,
        dst_parent: u32,
        dst_handle: u32,
        src_client: u32,
        src_handle: u32,
    ) -> &mut RpcScript {
        self.raw(
            fn_id::DUP_OBJECT,
            dup_body(
                dst_client,
                dst_parent,
                dst_handle,
                src_client,
                src_handle,
                NV04_DUP_HANDLE_FLAGS_NONE,
            ),
        )
    }

    /// A `GSP_RM_ALLOC` of any class, with `paramsSize` taken from the params actually
    /// carried — i.e. a **conforming** guest. A test that wants the guest to lie about
    /// its own size builds the body with [`alloc_body`] and posts it through [`Self::raw`].
    pub fn alloc(
        &mut self,
        h_client: u32,
        h_parent: u32,
        h_object: u32,
        h_class: u32,
        params: &[u8],
    ) -> &mut RpcScript {
        self.raw(
            fn_id::GSP_RM_ALLOC,
            alloc_body(
                h_client,
                h_parent,
                h_object,
                h_class,
                params.len() as u32,
                RMAPI_RPC_FLAGS_NONE,
                params,
            ),
        )
    }

    /// A `NV01_DEVICE_0` alloc declaring physical-GPU index `device_id`.
    pub fn device(
        &mut self,
        h_client: u32,
        h_parent: u32,
        h_object: u32,
        device_id: u32,
    ) -> &mut RpcScript {
        self.alloc(
            h_client,
            h_parent,
            h_object,
            NV01_DEVICE_0,
            &device_params(device_id, 0, 0),
        )
    }

    /// A `FERMI_VASPACE_A` alloc.
    ///
    /// `NV_VASPACE_ALLOCATION_PARAMETERS` is 56 bytes of geometry (`index`, `flags`,
    /// `vaSize`, `vaStartInternal`, `vaLimitInternal`, `bigPageSize`, `vaBase`, `pasid` —
    /// `ogkm-610: nvos.h:3146-3156`, `ogkm-580: nvos.h:3154-3164` — identical member for
    /// member) and **not one field of it** is something the object model
    /// models, so the script sends a plausibly-sized params the bridge never reads. The
    /// VAS's data-plane identity arrives later, on `SET_PAGE_DIRECTORY`.
    pub fn vaspace(&mut self, h_client: u32, h_parent: u32, h_object: u32) -> &mut RpcScript {
        self.alloc(h_client, h_parent, h_object, FERMI_VASPACE_A, &[0u8; 56])
    }

    /// A `KEPLER_CHANNEL_GROUP_A` (TSG) alloc declaring `h_vaspace`.
    pub fn tsg(
        &mut self,
        h_client: u32,
        h_parent: u32,
        h_object: u32,
        h_vaspace: u32,
    ) -> &mut RpcScript {
        self.alloc(
            h_client,
            h_parent,
            h_object,
            KEPLER_CHANNEL_GROUP_A,
            &tsg_params(h_vaspace, 0),
        )
    }

    /// A `FERMI_CONTEXT_SHARE_A` alloc declaring `h_vaspace`.
    pub fn ctxshare(
        &mut self,
        h_client: u32,
        h_parent: u32,
        h_object: u32,
        h_vaspace: u32,
    ) -> &mut RpcScript {
        self.alloc(
            h_client,
            h_parent,
            h_object,
            FERMI_CONTEXT_SHARE_A,
            &ctxshare_params(h_vaspace, 0, 0),
        )
    }

    /// An `AMPERE_CHANNEL_GPFIFO_A` alloc — the only channel class there is.
    ///
    /// `flags` is the opaque `NVOS04_FLAGS_*` word; nothing in the bridge interprets it,
    /// and the arch is what turns it into a `VChid`.
    #[allow(clippy::too_many_arguments)]
    pub fn channel(
        &mut self,
        h_client: u32,
        h_parent: u32,
        h_object: u32,
        flags: u32,
        h_ctx_share: u32,
        h_vaspace: u32,
    ) -> &mut RpcScript {
        self.alloc(
            h_client,
            h_parent,
            h_object,
            AMPERE_CHANNEL_GPFIFO_A,
            &channel_params(flags, h_ctx_share, h_vaspace),
        )
    }

    /// An engine object (`AMPERE_COMPUTE_B` / `AMPERE_DMA_COPY_B`) on a channel.
    ///
    /// Its params declare nothing; the protocol content is the edge, and the edge is what
    /// tells the core which engine the parent channel belongs to.
    pub fn engine_object(
        &mut self,
        h_client: u32,
        h_channel: u32,
        h_object: u32,
        h_class: u32,
    ) -> &mut RpcScript {
        self.alloc(h_client, h_channel, h_object, h_class, &[0u8; 8])
    }

    /// A `GSP_RM_CONTROL` with `paramsSize` taken from the params actually carried — i.e.
    /// a **conforming** guest, and `rmapiRpcFlags = NONE`. A test that wants the guest to
    /// lie builds the body with [`control_body`] and posts it through [`Self::raw`].
    pub fn control(
        &mut self,
        h_client: u32,
        h_object: u32,
        cmd: u32,
        params: &[u8],
    ) -> &mut RpcScript {
        self.raw(
            fn_id::GSP_RM_CONTROL,
            control_body(
                h_client,
                h_object,
                cmd,
                params.len() as u32,
                RMAPI_RPC_FLAGS_NONE,
                params,
            ),
        )
    }

    /// A `SET_PAGE_DIRECTORY` binding `h_vaspace` to `pdb`, issued against the Device.
    ///
    /// ★ Note `h_object` is the **Device** handle, not the VASpace: `NV_RM_RPC_CONTROL` is
    /// called with `hDevice` and the VAS is named by a params field
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:508-518`). The two travel in
    /// different places on purpose, and the bridge must not confuse them.
    pub fn set_page_dir(
        &mut self,
        h_client: u32,
        h_device: u32,
        h_vaspace: u32,
        pdb: u64,
        flags: u32,
    ) -> &mut RpcScript {
        self.control(
            h_client,
            h_device,
            NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
            &set_page_dir_params(pdb, 512, flags, h_vaspace, 0, 1, 0),
        )
    }

    /// Any function and any body, including a hostile one.
    pub fn raw(&mut self, function: u32, body: Vec<u8>) -> &mut RpcScript {
        self.steps.push(Step { function, body });
        self
    }

    /// The script's steps, for a caller that posts them through a real command ring.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The script as whole RPC **messages** (envelope + body), numbered positionally.
    ///
    /// For the tests that translate directly, with no ring and no FSM.
    #[must_use]
    pub fn messages(&self) -> Vec<Vec<u8>> {
        self.steps
            .iter()
            .enumerate()
            .map(|(i, s)| message(s.function, i as u32, &s.body))
            .collect()
    }
}

/// The wire ids this module's tests use, from the driver's X-macro table
/// (`ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h:11, 20, 31, 57, 75, 81, 82,
/// 83, 86, 113, 254, 256`). ★ The ten **X-macro** rows are on the same lines at
/// `ogkm-580`; the two **E-macro** event rows are not — `GSP_INIT_DONE` is
/// `ogkm-580: rpc_global_enums.h:253` and `POST_EVENT` is `:255`, because 610 inserts an
/// `#endif` at `:252` that 580 does not have. The *ids* are identical at both.
/// Transcribed here rather than taken from `gspworld::FUNCTIONS`, for the same
/// independence reason as everything else in this file.
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
