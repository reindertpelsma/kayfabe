//! `NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` (`0xa06c010a`) — the guest
//! telling us **where it put the CE fault method buffers** it allocated because
//! [`crate::fmbsize`] told it how big they are.
//!
//! # ★★★ Why this is the rung
//!
//! `[measured 2026-08-09, boot `ce1442` at `8ea44dc`]` — the first boot in which
//! `queryCopyEngines` does not appear. `cup2`'s `cuInit` now dies here instead:
//!
//! ```text
//! [   64.221235] NVRM: kchangrpapiConstruct_IMPL: Control call to update method buffer
//!                      memdesc failed
//! ```
//!
//! and `traces/guest_boots/ce1442_8ea44dc_census.log:42` carries
//! `unserviced fn 76 cmd 0xa06c010a` — the same call, named from the other side. The
//! `NV_PRINTF` is six lines below the control and is followed by a **hard `goto failed`**
//! (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:492-505`), so the
//! whole `KernelChannelGroupApi` construct fails and the caller's TSG never exists.
//!
//! ★ **Read the caller, not the id** (§14.42's lesson): the call site is the **only** one
//! in the open tree (`grep` gives `kernel_channel_group_api.c:494` and nothing else), it is
//! issued **once** per channel-group alloc, and the fifteen lines after it that could still
//! fail (`kfifoIsZombieSubctxWarEnabled`, `listInit`, `listAppendValue`) reach no emulated
//! GSP at all. So unlike `queryCopyEngines` this rung really is one id.
//!
//! ⊘ The `rpcCtrlInternalPromoteFaultMethodBuffers_v1E_07` route (`ogkm-580: rpc.c:7171+`,
//! `NV_VGPU_MSG_FUNCTION_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS`) is the legacy paravirt
//! decoy, the exact twin of the `_v1A_20` decoy `docs/design/gpu_promote_ctx.md` §1.3 warns
//! about. A GSP client never takes it — the boot ledger measured **`fn 76`**,
//! `GSP_RM_CONTROL`. Do not implement the dedicated function number.
//!
//! # ★★★ KERNEL_PRIVILEGED — and there is nonetheless **nothing to derive**
//!
//! Flags are `0x14240` (`ogkm-580: src/nvidia/generated/g_kernel_channel_group_api_nvoc.c:
//! 326-341`) = `GSP_PLUGIN_FOR_VGPU_GSP(0x10000) | CPU_PLUGIN_FOR_SRIOV(0x4000) |
//! ROUTE_TO_VGPU_HOST(0x200) | ROUTE_TO_PHYSICAL(0x40)`, carrying **neither**
//! `PRIVILEGED(0x4)` **nor** `NON_PRIVILEGED(0x8)`
//! (`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:170-308`) — `KERNEL_PRIVILEGED`, the
//! same class as `0x20802a07` and `0x20802a0f`, therefore **unreachable from usermode** and
//! **not measurable** by `rmladder`. `ROUTE_TO_PHYSICAL` also compiles
//! `kchangrpapiCtrlCmdInternalPromoteFaultMethodBuffers_IMPL`'s pointer out of CPU-RM: the
//! symbol is *declared* in `g_kernel_channel_group_api_nvoc.h` and **defined nowhere in the
//! open tree**, so the body lives only in the firmware we are faking.
//!
//! ★ That is the shape §14.42 flagged as *"exactly where an invented number propagates"* —
//! except that here the set of numbers this port must state is **empty**. Every one of the
//! three fields is `[input]` in the SDK's own prose (`ogkm-580: ctrl/ctrla06c.h:329-352`:
//! *"methodBufferMemdesc [input] … bar2Addr [input] … numValidEntries [input]"*), and the
//! caller confirms it from the other end — it fills all three, passes them, checks only
//! `rmStatus`, and never reads `params` again. **There is no `[OUT]` half.** So the honest
//! answer is an acknowledgement, and it invents nothing.
//!
//! ⊘⊘ **The honesty question, RE-ASKED and not inherited.** §14.42 measured that the
//! pure-`[IN]` identity-echo argument does **not** transfer between ids — `0x20802a02` had a
//! real `[OUT]` half that a verbatim echo would have filled with the guest's own
//! uninitialised buffer. It is re-asked here and the answer is different for a *stated*
//! reason: the SDK marks every field `[input]`, the sole caller writes every field, and the
//! `Possible status values` block names only `NV_OK` and `NV_ERR_INVALID_ARGUMENT` — no
//! output contract at all.
//!
//! # The reply body: the request's own bytes
//!
//! `paramsSize` is 88, non-zero, so `rpcRmApiControl_GSP`'s copyout
//! (`ogkm-580: rpc.c:11085-11090`) lands on the caller's struct. That struct is
//! `NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS params = {0}` — a stack local
//! in `kchangrpapiConstruct_IMPL` that is dead after the call — so zeros and an echo are
//! equally unobservable *today*. The echo is chosen because it is the only body that is
//! provably not a fiction: it repeats facts the guest supplied and states none of our own.
//! That is also the documented port of C defect **D7**, where a captured foreign-boot blob
//! was replayed into a caller's buffer under `NV_OK` (`docs/design/gpu_promote_ctx.md` §3
//! D7) — *"a Case-2 ACK writes back nothing"*.
//!
//! ⊘ Unlike [`crate::l2evict`], no divergence from silicon is being recorded here, because
//! no silicon reading exists **or can exist**: the id is `KERNEL_PRIVILEGED`, and
//! `traces/real_ga106/` cannot contain it. Stating that is the point — an unmeasurable id
//! must not borrow a neighbour's measurement.
//!
//! # What the guest is actually telling us
//!
//! `kchangrpAllocFaultMethodBuffers_GV100`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_group_gv100.c:
//! 74-143`) allocates **one buffer per runqueue** of exactly
//! `gpuGetCeFaultMethodBufferSize` bytes — the number [`crate::fmbsize`] answers, `20480` on
//! this part — in `ADDR_SYSMEM` with `NV_MEMORY_CACHED`, and sets `bar2Addr = 0` on every
//! path that is not full-SRIOV. The control then reports, per runqueue,
//! `memdescGetPhysAddr(pSrcMemDesc, AT_GPU, 0)`, that size, `alignment = 1`, the aperture
//! and the cache attribute (`kernel_channel_group_api.c:459-486`).
//!
//! ⇒ Every value on the wire is a **fact about guest memory**, which is why this port may
//! record it. `[`FaultMethodBuffers`]` keeps them decoded rather than as bytes so a future
//! fault plane has something named to consume; nothing consumes them today and the module
//! says so rather than pretending otherwise.
//!
//! # ★★ Byte-identical at both tags — do **not** build a version seam
//!
//! Pinned by compiling the vendored declarations with `offsetof`/`sizeof` under the real
//! `NV_DECLARE_ALIGNED` semantics (the discipline `docs/design/gpu_promote_ctx.md` §1.2
//! sets), at **both** `ogkm-580.159.04` and `ogkm` (610.43.02):
//!
//! ```text
//! PARAMS sizeof=88 align=8   methodBufferMemdesc +0   bar2Addr +64   numValidEntries +80
//! MEMDESC sizeof=32 align=8  base +0  size +8  alignment +16  addressSpace +24
//!                            cpuCacheAttrib +28
//! MAX_RUNQUEUES=2            CMD=0xa06c010a
//! ```
//!
//! Identical at both tags, field for field. Per `gsp_core_bridge.md`'s Axis-A rule a
//! non-split fact must **not** become a seam: adding a version fork for this struct would be
//! inventing one the trees say does not exist.
//!
//! ⊘ Not FINN-serialized either — absent from `g_finn_rm_api.h` at both tags — so
//! `rpcRmApiControl_GSP` takes the flat `portMemCopy` branch. And 40 (control header) + 88 =
//! 128 bytes against a message-buffer remainder near 3976, so it is always a **single
//! record**: the reassembler is not on this path.

/// `NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06c.h:354`).
pub const NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS: u32 = 0xa06c_010a;

/// `NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_MAX_RUNQUEUES`, which resolves to
/// `NVC36F_CTRL_CMD_GPFIFO_FAULT_METHOD_BUFFER_MAX_RUNQUEUES = 0x2`
/// (`ogkm-580: ctrl/ctrlc36f.h:102`).
///
/// ★ This is the bound `numValidEntries` is refused against. It is **not** clamped to:
/// C defect **D1** clamped a guest count to a number nobody had checked and read past the
/// struct (`docs/design/gpu_promote_ctx.md` §3 D1).
pub const MAX_RUNQUEUES: usize = 2;

/// `sizeof(NV2080_CTRL_INTERNAL_MEMDESC_INFO)` — three `NvU64` and two `NvU32`
/// (`ogkm-580: ctrl/ctrl2080/ctrl2080internal.h:388-394`). Compiler-pinned, both tags.
pub const MEMDESC_INFO_SIZE: usize = 32;

/// `sizeof(NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS)`. Compiler-pinned at
/// both tags; **checked exactly**, never as a lower bound (`gsp_core_bridge.md` §4.3).
pub const PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE: usize = 88;

/// Byte offset of `methodBufferMemdesc[0]`.
pub const METHOD_BUFFER_MEMDESC_OFF: usize = 0;

/// Byte offset of `bar2Addr[0]`.
pub const BAR2_ADDR_OFF: usize = 64;

/// Byte offset of `numValidEntries`.
pub const NUM_VALID_ENTRIES_OFF: usize = 80;

/// `ADDR_SYSMEM` — system memory (`ogkm-580: g_mem_desc_nvoc.h:115`). What
/// `kchangrpAllocFaultMethodBuffers_GV100` uses on every path this port can be on.
pub const ADDR_SYSMEM: u32 = 1;

/// `ADDR_FBMEM` — framebuffer memory (`ogkm-580: g_mem_desc_nvoc.h:116`). Reachable through
/// the `retryInFB` fallback and through an `instLocOverride`, so it is legal here.
pub const ADDR_FBMEM: u32 = 2;

/// One runqueue's method buffer, as the guest described it.
///
/// ⊘ A **destroy** record is legal and is not a malformed one: the SDK says *"If the size of
/// the memory region is zero, the descriptor will be destroyed"* (`ogkm-580:
/// ctrl/ctrla06c.h:337-339`). It is [`MethodBuffer::is_destroy`], not an error — the same
/// distinction `gpu_promote_ctx.md` §2.3 draws between *absence of a fact* and a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodBuffer {
    /// `base` — `memdescGetPhysAddr(pSrcMemDesc, AT_GPU, 0)` as the guest computed it.
    ///
    /// ⊘ A **guest** address in the guest's own aperture. Nothing here resolves it, and
    /// nothing may: `no_real_phys_only_gpga_or_gpa` — this port stores no host-physical
    /// address.
    pub base: u64,
    /// `size` — zero means *destroy this descriptor*, not *malformed*.
    pub size: u64,
    /// `alignment` — the caller hard-codes `1` (`kernel_channel_group_api.c:468`).
    pub alignment: u64,
    /// `addressSpace` — an `NV_ADDRESS_SPACE`; [`ADDR_SYSMEM`] or [`ADDR_FBMEM`] here.
    pub address_space: u32,
    /// `cpuCacheAttrib` — `memdescGetCpuCacheAttrib`, `NV_MEMORY_CACHED` on this path.
    ///
    /// ⊘ Carried verbatim and not interpreted. This port maps nothing for the guest here,
    /// so there is no cache policy for it to be right or wrong about, and inventing a
    /// meaning for it would be the fiction this module exists not to tell.
    pub cpu_cache_attrib: u32,
    /// `bar2Addr` for this runqueue — `0` on every non-full-SRIOV path
    /// (`kernel_channel_group_gv100.c:142`), which is the only path this port is on.
    pub bar2_addr: u64,
}

impl MethodBuffer {
    /// Whether this record asks for the descriptor to be **destroyed** rather than promoted.
    #[must_use]
    pub fn is_destroy(self) -> bool {
        self.size == 0
    }
}

/// A decoded `NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS`.
///
/// ⊘ Only the first `num_valid_entries` records are carried. The trailing slots of the
/// fixed `[2]` array are the guest's uninitialised stack as far as this port is concerned
/// (`params = {0}` in the one caller, but that is the caller's courtesy, not the ABI's
/// promise), and reading them would be manufacturing facts out of padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultMethodBuffers {
    /// The `num_valid_entries` records the guest declared, in runqueue order.
    pub buffers: Vec<MethodBuffer>,
}

impl FaultMethodBuffers {
    /// How many runqueues the guest described.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// Whether the guest described no runqueue at all.
    ///
    /// ⊘ Legal: `kfifoGetNumRunqueues_HAL` decides the count and a zero is a guest that
    /// promoted nothing. It is not an error and it is not refused.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

/// Why a fault-method-buffer promotion could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMethodBufferError {
    /// The guest's params are shorter than the struct it declared.
    ShortParams {
        /// How many bytes arrived.
        got: usize,
    },
    /// ★★★ `numValidEntries` above [`MAX_RUNQUEUES`] — refused **by name**, never clamped.
    ///
    /// This is C defect **D1** in its own habitat: that handler clamped a guest count to a
    /// number nobody had checked and then read 1536 bytes past the struct. The array here
    /// is two elements; a guest declaring three has described records that are not in the
    /// message, and there is no honest reading of them.
    TooManyRunqueues {
        /// What the guest declared.
        declared: u32,
    },
    /// An `addressSpace` this port cannot name.
    ///
    /// ⊘ Refused rather than folded into sysmem — the same ruling `gpu_promote_ctx.md` §1.4
    /// makes for `physAttr[1:0] == 3`. A buffer whose aperture we cannot name is a buffer
    /// whose location we do not know, and acknowledging it would claim otherwise.
    UnknownAddressSpace {
        /// Which runqueue.
        runqueue: usize,
        /// The value the guest sent.
        address_space: u32,
    },
    /// A record that declares a size but no address.
    ///
    /// ⊘ Distinct from [`MethodBuffer::is_destroy`], which is `size == 0` and legal. A
    /// non-zero size at base `0` is the *manufactured address* case MISS = FAULT forbids
    /// (`mode2_address_table_of_truth.md`): physical zero is not a location the guest can
    /// have allocated 20 KiB at, so it is an unread field, not a fact.
    SizedAtAddressZero {
        /// Which runqueue.
        runqueue: usize,
        /// The size the guest declared at address zero.
        size: u64,
    },
}

impl core::fmt::Display for FaultMethodBufferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { got } => write!(
                f,
                "fault method buffer promotion params are {got} bytes; the struct is \
                 {PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE}"
            ),
            Self::TooManyRunqueues { declared } => write!(
                f,
                "fault method buffer promotion declares {declared} runqueues; the array is \
                 {MAX_RUNQUEUES} elements, and a count is refused by name rather than \
                 clamped"
            ),
            Self::UnknownAddressSpace {
                runqueue,
                address_space,
            } => write!(
                f,
                "fault method buffer for runqueue {runqueue} declares address space \
                 {address_space:#x}, which is neither ADDR_SYSMEM ({ADDR_SYSMEM}) nor \
                 ADDR_FBMEM ({ADDR_FBMEM})"
            ),
            Self::SizedAtAddressZero { runqueue, size } => write!(
                f,
                "fault method buffer for runqueue {runqueue} declares {size} bytes at \
                 address 0; size 0 is the SDK's destroy form, but a sized buffer at address \
                 zero is an unread field rather than a location"
            ),
        }
    }
}

impl core::error::Error for FaultMethodBufferError {}

fn u64_at(params: &[u8], off: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&params[off..off + 8]);
    u64::from_le_bytes(b)
}

fn u32_at(params: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&params[off..off + 4]);
    u32::from_le_bytes(b)
}

/// Decode `NVA06C_CTRL_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS_PARAMS`.
///
/// The order of the checks is the order in which a reading of the struct becomes possible:
/// length first (nothing is readable without it), then `numValidEntries` (which decides how
/// much of the array is a claim at all), then per-record validation.
///
/// # Errors
///
/// [`FaultMethodBufferError`], by variant.
pub fn decode_promote_fault_method_buffers(
    params: &[u8],
) -> Result<FaultMethodBuffers, FaultMethodBufferError> {
    if params.len() < PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE {
        return Err(FaultMethodBufferError::ShortParams { got: params.len() });
    }
    let declared = u32_at(params, NUM_VALID_ENTRIES_OFF);
    if declared as usize > MAX_RUNQUEUES {
        return Err(FaultMethodBufferError::TooManyRunqueues { declared });
    }
    let mut buffers = Vec::with_capacity(declared as usize);
    for runqueue in 0..declared as usize {
        let at = METHOD_BUFFER_MEMDESC_OFF + runqueue * MEMDESC_INFO_SIZE;
        let buf = MethodBuffer {
            base: u64_at(params, at),
            size: u64_at(params, at + 8),
            alignment: u64_at(params, at + 16),
            address_space: u32_at(params, at + 24),
            cpu_cache_attrib: u32_at(params, at + 28),
            bar2_addr: u64_at(params, BAR2_ADDR_OFF + runqueue * 8),
        };
        if buf.address_space != ADDR_SYSMEM && buf.address_space != ADDR_FBMEM {
            return Err(FaultMethodBufferError::UnknownAddressSpace {
                runqueue,
                address_space: buf.address_space,
            });
        }
        if buf.base == 0 && !buf.is_destroy() {
            return Err(FaultMethodBufferError::SizedAtAddressZero {
                runqueue,
                size: buf.size,
            });
        }
        buffers.push(buf);
    }
    Ok(FaultMethodBuffers { buffers })
}

/// The reply body — ★ **the request's own bytes, unchanged**.
///
/// Every field is `[input]`, so there is no `[OUT]` half to write and no number for this
/// port to state. Re-encoding from the decoded view rather than copying the caller's slice
/// is deliberate: it means the reply can only contain records the decoder **accepted**, so a
/// field the validation rejected can never reach the guest by having been memcpy'd around
/// the check.
///
/// ⊘ The trailing array slots beyond `numValidEntries` come back as zeros rather than as
/// whatever the guest sent. That is not a claim about them — it is the refusal to repeat
/// bytes this port did not read, and the one caller zeroes them itself
/// (`params = {0}`, `kernel_channel_group_api.c:456`).
#[must_use]
pub fn encode_promote_fault_method_buffers(req: &FaultMethodBuffers) -> Vec<u8> {
    let mut out = vec![0u8; PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE];
    for (runqueue, buf) in req.buffers.iter().enumerate() {
        let at = METHOD_BUFFER_MEMDESC_OFF + runqueue * MEMDESC_INFO_SIZE;
        out[at..at + 8].copy_from_slice(&buf.base.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&buf.size.to_le_bytes());
        out[at + 16..at + 24].copy_from_slice(&buf.alignment.to_le_bytes());
        out[at + 24..at + 28].copy_from_slice(&buf.address_space.to_le_bytes());
        out[at + 28..at + 32].copy_from_slice(&buf.cpu_cache_attrib.to_le_bytes());
        let b2 = BAR2_ADDR_OFF + runqueue * 8;
        out[b2..b2 + 8].copy_from_slice(&buf.bar2_addr.to_le_bytes());
    }
    let n = u32::try_from(req.buffers.len()).unwrap_or(u32::MAX);
    out[NUM_VALID_ENTRIES_OFF..NUM_VALID_ENTRIES_OFF + 4].copy_from_slice(&n.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An independently-written builder — the third transcription the house discipline asks
    /// for (`tests/tests/rmrpc_bridge.rs:7-24`): written from the offsets in the header
    /// read, not from the decoder and not from the encoder.
    fn build(entries: &[(u64, u64, u64, u32, u32, u64)], declared: u32) -> Vec<u8> {
        let mut p = vec![0u8; 88];
        for (i, (base, size, align, aspace, cache, bar2)) in entries.iter().enumerate() {
            let at = i * 32;
            p[at..at + 8].copy_from_slice(&base.to_le_bytes());
            p[at + 8..at + 16].copy_from_slice(&size.to_le_bytes());
            p[at + 16..at + 24].copy_from_slice(&align.to_le_bytes());
            p[at + 24..at + 28].copy_from_slice(&aspace.to_le_bytes());
            p[at + 28..at + 32].copy_from_slice(&cache.to_le_bytes());
            p[64 + i * 8..64 + i * 8 + 8].copy_from_slice(&bar2.to_le_bytes());
        }
        p[80..84].copy_from_slice(&declared.to_le_bytes());
        p
    }

    /// The shape a GA106 guest actually sends: two runqueues, `20480` bytes each — the size
    /// [`crate::fmbsize`] answers — in `ADDR_SYSMEM`, alignment `1`, `bar2Addr = 0`.
    fn realistic() -> Vec<u8> {
        build(
            &[
                (0x1_0000_0000, 20480, 1, ADDR_SYSMEM, 1, 0),
                (0x1_0000_8000, 20480, 1, ADDR_SYSMEM, 1, 0),
            ],
            2,
        )
    }

    #[test]
    fn the_id_and_the_bounds_are_the_headers() {
        assert_eq!(
            NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS,
            0xa06c_010a
        );
        assert_eq!(MAX_RUNQUEUES, 2);
        assert_eq!(MEMDESC_INFO_SIZE, 32);
        // ⊘ Written as the product of the compiler-pinned parts as well as the literal, so
        // a mistyped offset and a mistyped size cannot agree with each other.
        assert_eq!(PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE, 88);
        assert_eq!(BAR2_ADDR_OFF, MEMDESC_INFO_SIZE * MAX_RUNQUEUES);
        assert_eq!(NUM_VALID_ENTRIES_OFF, BAR2_ADDR_OFF + 8 * MAX_RUNQUEUES);
    }

    #[test]
    fn a_realistic_ga106_promotion_decodes_field_for_field() {
        let got = decode_promote_fault_method_buffers(&realistic()).expect("legal");
        assert_eq!(got.len(), 2);
        assert!(!got.is_empty());
        assert_eq!(
            got.buffers[0],
            MethodBuffer {
                base: 0x1_0000_0000,
                size: 20480,
                alignment: 1,
                address_space: ADDR_SYSMEM,
                cpu_cache_attrib: 1,
                bar2_addr: 0,
            }
        );
        assert_eq!(got.buffers[1].base, 0x1_0000_8000);
        assert!(!got.buffers[0].is_destroy());
    }

    /// ★ The `bar2Addr` array is a **separate** array at +64, not a member of the memdesc
    /// records — a decoder that folded it into the 32-byte stride would read the wrong
    /// runqueue's value, and this is the assertion that says so.
    #[test]
    fn bar2_addr_is_indexed_out_of_its_own_array() {
        let p = build(
            &[
                (0x2000, 4096, 1, ADDR_SYSMEM, 0, 0xaaaa),
                (0x3000, 4096, 1, ADDR_SYSMEM, 0, 0xbbbb),
            ],
            2,
        );
        let got = decode_promote_fault_method_buffers(&p).expect("legal");
        assert_eq!(got.buffers[0].bar2_addr, 0xaaaa);
        assert_eq!(got.buffers[1].bar2_addr, 0xbbbb);
    }

    /// The sweep, not a witness: every count `0..=3` across the bound.
    #[test]
    fn the_runqueue_count_is_swept_across_its_bound() {
        for declared in 0u32..=3 {
            let p = build(
                &[
                    (0x2000, 4096, 1, ADDR_SYSMEM, 0, 0),
                    (0x3000, 4096, 1, ADDR_SYSMEM, 0, 0),
                ],
                declared,
            );
            match decode_promote_fault_method_buffers(&p) {
                Ok(got) => {
                    assert!(declared <= 2, "{declared} accepted");
                    assert_eq!(got.len(), declared as usize);
                }
                Err(e) => {
                    assert_eq!(declared, 3);
                    assert_eq!(e, FaultMethodBufferError::TooManyRunqueues { declared });
                }
            }
        }
    }

    /// ★★★ D1's shape, refused rather than clamped: a guest declaring `u32::MAX` must not
    /// come back as two accepted records.
    #[test]
    fn a_wild_runqueue_count_is_refused_by_name_and_never_clamped() {
        let p = build(&[(0x2000, 4096, 1, ADDR_SYSMEM, 0, 0)], u32::MAX);
        assert_eq!(
            decode_promote_fault_method_buffers(&p),
            Err(FaultMethodBufferError::TooManyRunqueues { declared: u32::MAX })
        );
    }

    /// Zero runqueues is legal and is **not** an error — `kfifoGetNumRunqueues_HAL` decides
    /// the count, and a guest that promoted nothing has said something well-formed.
    #[test]
    fn zero_runqueues_is_legal_and_empty() {
        let got = decode_promote_fault_method_buffers(&build(&[], 0)).expect("legal");
        assert!(got.is_empty());
        assert_eq!(encode_promote_fault_method_buffers(&got), vec![0u8; 88]);
    }

    /// The SDK's destroy form: `size == 0`, which must decode rather than be refused.
    #[test]
    fn a_zero_size_record_is_the_destroy_form_not_a_malformed_one() {
        let p = build(&[(0, 0, 0, ADDR_SYSMEM, 0, 0)], 1);
        let got = decode_promote_fault_method_buffers(&p).expect("destroy is legal");
        assert!(got.buffers[0].is_destroy());
    }

    /// …and the near neighbour that is **not** legal: a size at address zero.
    #[test]
    fn a_sized_buffer_at_address_zero_is_refused() {
        let p = build(&[(0, 20480, 1, ADDR_SYSMEM, 0, 0)], 1);
        assert_eq!(
            decode_promote_fault_method_buffers(&p),
            Err(FaultMethodBufferError::SizedAtAddressZero {
                runqueue: 0,
                size: 20480
            })
        );
    }

    /// Every `NV_ADDRESS_SPACE` the driver names, swept — two accepted, the rest refused by
    /// name. ⊘ Quantified over the enum rather than witnessed on one bad value, so a decoder
    /// that accepted `ADDR_VIRTUAL` could not pass.
    #[test]
    fn the_address_space_is_swept_over_the_whole_enum() {
        for aspace in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 0xffff_ffff] {
            let p = build(&[(0x2000, 4096, 1, aspace, 0, 0)], 1);
            let got = decode_promote_fault_method_buffers(&p);
            if aspace == ADDR_SYSMEM || aspace == ADDR_FBMEM {
                assert!(got.is_ok(), "{aspace} should be accepted");
            } else {
                assert_eq!(
                    got,
                    Err(FaultMethodBufferError::UnknownAddressSpace {
                        runqueue: 0,
                        address_space: aspace
                    })
                );
            }
        }
    }

    /// The second runqueue is validated too — a decoder that checked only record `0` would
    /// pass every other test in this module.
    #[test]
    fn the_second_runqueue_is_validated_as_well_as_the_first() {
        let p = build(
            &[
                (0x2000, 4096, 1, ADDR_SYSMEM, 0, 0),
                (0x3000, 4096, 1, 4, 0, 0),
            ],
            2,
        );
        assert_eq!(
            decode_promote_fault_method_buffers(&p),
            Err(FaultMethodBufferError::UnknownAddressSpace {
                runqueue: 1,
                address_space: 4
            })
        );
    }

    /// The length sweep every decoder gets: refuse short at **every** length below the
    /// struct, accept at and above it.
    #[test]
    fn every_short_length_is_refused_and_the_exact_length_is_not() {
        let full = realistic();
        for n in 0..PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE {
            assert_eq!(
                decode_promote_fault_method_buffers(&full[..n]),
                Err(FaultMethodBufferError::ShortParams { got: n })
            );
        }
        assert!(decode_promote_fault_method_buffers(&full).is_ok());
        let mut long = full.clone();
        long.extend_from_slice(&[0xcd; 16]);
        assert_eq!(
            decode_promote_fault_method_buffers(&long),
            decode_promote_fault_method_buffers(&full)
        );
    }

    /// ★ The round trip: the reply is the request, byte for byte, for the records the
    /// decoder accepted.
    #[test]
    fn the_reply_is_the_requests_own_bytes() {
        let full = realistic();
        let got = decode_promote_fault_method_buffers(&full).expect("legal");
        assert_eq!(encode_promote_fault_method_buffers(&got), full);
        assert_eq!(
            encode_promote_fault_method_buffers(&got).len(),
            PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE
        );
    }

    /// ⊘ …and the one place it is deliberately **not** a memcpy: bytes past
    /// `numValidEntries` are not repeated back, because this port did not read them.
    #[test]
    fn slots_past_the_declared_count_are_not_echoed() {
        let mut p = build(&[(0x2000, 4096, 1, ADDR_SYSMEM, 0, 0)], 1);
        // Runqueue 1's slot carries garbage the guest never declared valid.
        p[32..64].copy_from_slice(&[0xcd; 32]);
        p[72..80].copy_from_slice(&[0xcd; 8]);
        let got = decode_promote_fault_method_buffers(&p).expect("one valid entry");
        assert_eq!(got.len(), 1);
        let reply = encode_promote_fault_method_buffers(&got);
        assert_eq!(&reply[32..64], &[0u8; 32]);
        assert_eq!(&reply[72..80], &[0u8; 8]);
        assert_ne!(reply, p);
    }
}
