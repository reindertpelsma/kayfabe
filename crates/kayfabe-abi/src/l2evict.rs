//! The `GSP_RM_CONTROL` request and reply body for
//! `NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT` (`0x20800a6c`) — four bytes, and
//! ★★★ **the first control this port serves that asks the device to DO something rather
//! than to describe itself.**
//!
//! # ★★★ The difference, stated before the answer
//!
//! Every control served up to here answers with *data*: a table, a topology, a size, an
//! identity. Its correctness question is *"is this the right number?"*. This one has no
//! number in it. `NV2080_CTRL_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT_PARAMS` is one `NvU32`
//! of `[IN]` flags and nothing else (`ogkm-580:
//! src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:2137-2158`), and its
//! correctness question is *"did the postcondition become true?"*.
//!
//! ⊘ So the ordinary reasons to serve a control do not transfer, and the reason this one is
//! served has to be built from scratch. It is built twice below, from two independent
//! sources, and the two agree.
//!
//! # ★★★ Argument 1 — the postcondition is satisfied, and it is satisfied STRUCTURALLY
//!
//! The postcondition the caller is buying is written down at the only site that consumes
//! it. `kbusVerifyBar2_GM107` issues this control three times, and each time the very next
//! thing it does is **re-read the address it just wrote** and demand `SAMPLEDATA`:
//!
//! ```c
//! // Evict L2 to ensure that the next read doesn't hit L2 and mistakenly
//! // assume that the BAR0 window to vidmem works
//! status = kmemsysSendL2InvalidateEvict(pGpu, pKernelMemorySystem, flagsClean);
//! if (NV_OK != status) { NV_PRINTF(LEVEL_ERROR, "L2 evict failed\n"); goto …_failed; }
//!
//! if (GPU_REG_RD32(pGpu, DRF_BASE(NV_PRAMIN) + …) != SAMPLEDATA) { … }
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:4106-4118`, and
//! again at `:4175` and `:4224`.) The operation asked for is *"make the next read miss the
//! cache"*, and the only observable it has is that next read.
//!
//! ★★ On this device that read cannot be served from a stale copy, because **there is no
//! copy**. This port's framebuffer plane — `kayfabe_device::fbwin` — is not a cache over a
//! backing store; it *is* the backing store, and both the read path and the write path
//! resolve an address through the same single function
//! (`Bar0Window::fb_addr`, and `fbwin`'s own docs §2 on why there is exactly one). A
//! `PRAMIN` write is trapped,
//! resolved, and committed to the store before the vmexit returns; the next `PRAMIN` read
//! is trapped, resolved through the same arithmetic, and answered from that store. There is
//! no layer between them that could hold a different value, so *"a read after this returns
//! sees the last write"* is not something this port arranges on request — it is the only
//! thing it can do.
//!
//! ⇒ `NV_OK` here is not *"we did nothing and said yes"*. It is *"the state you asked for
//! already holds, and it holds for a structural reason rather than because we performed an
//! action"*. ⊘ That distinction is the whole licence, and it is why this must not be
//! generalised: the moment a device has a second place a byte can live, the same `NV_OK`
//! becomes a lie.
//!
//! # ★★★ Argument 2 — a real GA106's own GSP answers `NV_OK`, `[measured]`
//!
//! The C artifact's init-control table is a **capture from real hardware** — *"GA106 init
//! `GSP_RM_CONTROL` responses (real, captured from host)"*
//! (`C: src/qemu/mode2_initctrl_ga106.h:1-2`). Its row is:
//!
//! ```c
//! {0x20800a6cu, 0x0u, 4u, 0u, ctl_20800a6c},   /* cmd, status, psize, dlen, data */
//! static const unsigned char ctl_20800a6c[] = { };
//! ```
//!
//! (`C: mode2_initctrl_ga106.h:6245` and `:3346`.) `status = 0x0` is `NV_OK`. So the answer
//! this port is about to give is the answer NVIDIA's own firmware gives on the part this
//! port emulates. ⊘ It is corroboration and not proof — the C's capture is a different boot
//! of a different machine — but it removes the one reading that would have been fatal,
//! namely that a real GSP refuses this and RM's `NV_OK` path is the unusual one.
//!
//! # ★★★ What would make this FALSE — three named futures, not a disclaimer
//!
//! 1. **Real host-GPU forwarding.** The instant the bytes behind a guest framebuffer
//!    address live in a *host* GPU's VIDMEM, they live behind a real L2, and the
//!    postcondition stops holding by construction. ⚠ And the fix is not *"forward the
//!    control"*: `docs/design/mode2_forwarding_model.md` forbids replaying a privileged
//!    GSP-internal control, and this port's host side is an unprivileged process that could
//!    not issue it anyway. What must change is that this row is **re-decided** — either an
//!    unprivileged host operation with the same observable end state is named and performed,
//!    or the control goes back to being refused. Inheriting today's `NV_OK` into that world
//!    is the specific mistake this paragraph exists to make un-quiet.
//! 2. **A cache of this port's own.** If the framebuffer plane ever gains a write-back
//!    layer — a host-side staging buffer for `PRAMIN` writes, a lazily-flushed residency
//!    map, or a memslot-backed page the guest writes *without* a trap — then a stale copy
//!    becomes observable and Argument 1 collapses. ⊘ `kayfabe_device::fbwin`'s store being
//!    the **authority** rather than a cache is therefore load-bearing for this control, not
//!    merely an implementation choice.
//! 3. **A second writer.** Argument 1 says the guest's own trapped write is the only way a
//!    framebuffer byte changes. A real copy engine, a host GPU DMA, or any agent writing
//!    the store behind the guest's back would give the guest something to be stale about.
//!
//! # ★★★ Empty body versus re-encode — and here the two answers AGREE
//!
//! `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` taught this port that *"the params have no
//! `[OUT]` fields"* does **not** license an empty reply, because the transport copies the
//! reply's params back over the caller's own struct
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`):
//!
//! ```c
//! if (paramsSize != 0)
//! {
//!     portMemCopy(pParamStructPtr, paramsSize, rpc_params->params, paramsSize);
//! }
//! ```
//!
//! ⇒ the question that must be asked of **every** such control is not *"are there `[OUT]`
//! fields?"* but *"does the caller read its own params after the call returns?"*. For this
//! control the answer is **no**, and it is visible in four lines:
//!
//! ```c
//! NV_STATUS kmemsysSendL2InvalidateEvict_IMPL(OBJGPU *pGpu,
//!         KernelMemorySystem *pKernelMemorySystem, NvU32 flags) {
//!     RM_API *pRmApi = GPU_GET_PHYSICAL_RMAPI(pGpu);
//!     NV2080_CTRL_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT_PARAMS params = {0};
//!     params.flags = flags;
//!     return pRmApi->Control(pRmApi, pGpu->hInternalClient, pGpu->hInternalSubdevice,
//!                            NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT,
//!                            &params, sizeof(params));
//! }
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys.c:1079-1093`.) `params` is a
//! **stack local**; the function `return`s the transport's status directly and `params` dies
//! there. Every one of the three call sites in `kbusVerifyBar2_GM107` passes `flagsClean` by
//! value and keeps no struct of its own. The copyout therefore lands in memory nothing
//! reads.
//!
//! ★★ And the oracle says the same thing from the other side. The captured row is
//! `psize = 4, dlen = 0` with an **empty** `ctl_20800a6c[]`, and the C's own decoder
//! documents `dlen` as a trailing-zero-trimmed length — *"Any captured tail beyond
//! `cr->dlen` is zero"* (`C: src/qemu/nvkvm_gpu_emul.c:3422-3425`). ⇒ a real GA106 GSP
//! returned **four zero bytes**, not the `flags` it was sent. Contrast `0x20800301`, whose
//! captured row is `psize = 20, dlen = 16` — the same capture *does* record echoed `[IN]`
//! fields when the firmware sends them back.
//!
//! ⇒ [`encode_l2_invalidate_evict`] returns four zeros. Not because *"nobody reads it"*
//! alone, and not because *"zero is the default"* — because the caller is measured not to
//! read it **and** the hardware is measured to answer that way, and the two agree.
//!
//! # ★★ The flags this port may say yes to, and the one it may not
//!
//! [`L2_INVALIDATE_EVICT_FLAGS_KNOWN`] is every bit the 580 SDK names. Each of the six is an
//! instruction whose postcondition Argument 1 satisfies vacuously — an invalidate has no
//! lines to drop, a clean has no dirty lines to write back, a wait-for-FB-pull has no pull
//! outstanding, and `FIRST`/`LAST` sequence a multi-part operation that is not multi-part
//! here.
//!
//! ⊘ **A bit outside that set is refused**, and that is this port's own policy rather than
//! a rule RM has — the same shape as
//! [`SILENT_NOTIFIERS`](crate::eventnotify::SILENT_NOTIFIERS). The argument above is
//! per-operation: it enumerates six things and says why each is already true. A seventh
//! operation, named by a bit this port cannot read, has no such argument, and answering
//! `NV_OK` to it would be claiming to have performed something we cannot name. Refusing is
//! the safe direction and it is loud.
//!
//! ⚠ `[measured]` what the guest actually sends: `kbusVerifyBar2_GM107:4018-4023` builds
//! `flagsClean = FLAGS_ALL | FLAGS_CLEAN`, plus `FLAGS_WAIT_FB_PULL` when
//! `kmemsysIsL2CleanFbPull` — and GA106 **is** in the chip list that sets `bL2CleanFbPull`
//! to `NV_TRUE` (`ogkm-580: src/nvidia/generated/g_kern_mem_sys_nvoc.c:256-262`). So the
//! expected wire value is [`FLAGS_CLEAN_VERIFY_BAR2`] = `0x31`. ⊘ Reading only
//! `kern_mem_sys.c:44-53`, where the sole assignment is `bL2CleanFbPull = NV_FALSE` under a
//! registry override, gives `0x11` and is wrong; the NVOC HAL field is the initialiser.
//!
//! # ⚠ `[measured]` where it appears in the oracle's own boot
//!
//! # ★★★ `[measured]` — boot `l2evict1`, rev `9551dd1`, 2026-08-01
//!
//! A stock 580.159.04 guest accepted the answer:
//! `docs/design/boot_measured_2026_08_01.md` §27. `kbusVerifyBar2_GM107: L2 evict failed` is
//! **gone**, `0x20800a6c` leaves the device's unserviced list, and the same function now
//! fails ninety lines later at `:4200` — the MMU test's read-back — which is past **both**
//! the first evict (`:4110`) and the second (`:4175`).
//!
//! ⊘ Three things that boot did **not** establish, and they matter here specifically:
//! the third call at `:4224` was never made; the `0x31` prediction below is unconfirmed
//! because a `0x11` would have decoded identically; and the boot went **backwards one phase**
//! (`NV_ERR_MEMORY_ERROR` is not in `gpuStateInit_IMPL`'s absorb map where
//! `NV_ERR_NOT_SUPPORTED` was), so every later-phase result in the previous log is simply
//! absent rather than regressed. §28 carries both halves.
//!
//! # ⚠ `[measured]` where it appears in the oracle's own boot
//!
//! `cap1b` carries it at `rpc.sequence` **30, 31 and 32** (txn 1006-1008), `paylen 44` = 40
//! header + **4** params — an independent confirmation of the struct size below — all three
//! inside the replay's closure limit of 1028. `cargo run -p kayfabe-crec --example
//! cap1b_report`. Three occurrences is itself the signature of `kbusVerifyBar2_GM107`, which
//! is the only function in the driver that issues it three times.

/// `NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:2145`).
pub const NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT: u32 = 0x2080_0a6c;

/// Byte offset of `flags` — `[IN]`, and the struct's only member.
pub const FLAGS_OFF: usize = 0;

/// `sizeof(NV2080_CTRL_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT_PARAMS)` — one `NvU32`.
///
/// ★ Confirmed independently of the header read: `cap1b`'s `rpc.sequence` 30 carries
/// `paylen 44`, and this port's control header is 40 bytes.
pub const L2_INVALIDATE_EVICT_PARAMS_SIZE: usize = 4;

/// `…_FLAGS_ALL` — invalidate/evict the whole cache rather than a range
/// (`ogkm-580: ctrl2080internal.h:2153`).
pub const FLAGS_ALL: u32 = 0x0000_0001;

/// `…_FLAGS_FIRST` — this is the first of a multi-part operation
/// (`ogkm-580: ctrl2080internal.h:2154`).
pub const FLAGS_FIRST: u32 = 0x0000_0002;

/// `…_FLAGS_LAST` — this is the last of a multi-part operation
/// (`ogkm-580: ctrl2080internal.h:2155`).
pub const FLAGS_LAST: u32 = 0x0000_0004;

/// `…_FLAGS_NORMAL` — invalidate without writing dirty lines back
/// (`ogkm-580: ctrl2080internal.h:2156`).
pub const FLAGS_NORMAL: u32 = 0x0000_0008;

/// `…_FLAGS_CLEAN` — write dirty lines back to framebuffer before dropping them
/// (`ogkm-580: ctrl2080internal.h:2157`).
pub const FLAGS_CLEAN: u32 = 0x0000_0010;

/// `…_FLAGS_WAIT_FB_PULL` — do not return until the framebuffer pull has drained
/// (`ogkm-580: ctrl2080internal.h:2158`).
pub const FLAGS_WAIT_FB_PULL: u32 = 0x0000_0020;

/// ★★ **Every flag bit the 580 SDK names**, and therefore every operation this port has an
/// argument for.
///
/// ⊘ Derived by `|` from the six constants rather than written as `0x3f`, so a bit added to
/// the SDK cannot be silently covered by a literal that nobody re-derived — and so the
/// relationship between *"this bit is named"* and *"this bit is accepted"* is the one thing
/// stated, in one place.
pub const L2_INVALIDATE_EVICT_FLAGS_KNOWN: u32 =
    FLAGS_ALL | FLAGS_FIRST | FLAGS_LAST | FLAGS_NORMAL | FLAGS_CLEAN | FLAGS_WAIT_FB_PULL;

/// ★ The exact value `kbusVerifyBar2_GM107` sends on a GA106 — `ALL | CLEAN | WAIT_FB_PULL`.
///
/// `flagsClean` is built at `ogkm-580: kern_bus_gm107.c:4018-4023`, and `WAIT_FB_PULL` is
/// included because `bL2CleanFbPull` is `NV_TRUE` for GA106 in the NVOC HAL initialiser
/// (`ogkm-580: src/nvidia/generated/g_kern_mem_sys_nvoc.c:256-262`). ⊘ Not a value this
/// module branches on — a prediction a boot can falsify, and the one the tests pin.
pub const FLAGS_CLEAN_VERIFY_BAR2: u32 = FLAGS_ALL | FLAGS_CLEAN | FLAGS_WAIT_FB_PULL;

/// A decoded `NV2080_CTRL_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT_PARAMS`.
///
/// ⊘ One `[IN]` field, and this port answers with none. The struct exists so that *which
/// operation was asked for* is a value the serve decision can be made from, rather than
/// bytes nobody looked at — see the module docs on why an unnamed bit is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L2InvalidateEvict {
    /// The `…_FLAGS_*` bits the guest asked for. Every set bit is in
    /// [`L2_INVALIDATE_EVICT_FLAGS_KNOWN`], because [`decode_l2_invalidate_evict`] refuses
    /// otherwise.
    pub flags: u32,
}

impl L2InvalidateEvict {
    /// Whether the request asks for dirty lines to be written back before being dropped.
    ///
    /// ★ Exposed so the vacuity argument is *checkable* rather than only narrated: on this
    /// device a clean and a plain invalidate leave the same framebuffer, and a test can say
    /// so by constructing both.
    #[must_use]
    pub fn writes_back(self) -> bool {
        self.flags & FLAGS_CLEAN != 0
    }

    /// Whether the request asks the device to block until the framebuffer pull has drained.
    #[must_use]
    pub fn waits_for_fb_pull(self) -> bool {
        self.flags & FLAGS_WAIT_FB_PULL != 0
    }
}

/// Why an L2 invalidate/evict could not be decoded, or may not be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L2EvictError {
    /// The guest's params are shorter than the struct it declared.
    ShortParams {
        /// How many bytes arrived.
        got: usize,
    },
    /// ★★★ A bit outside [`L2_INVALIDATE_EVICT_FLAGS_KNOWN`] — an operation this port
    /// cannot name and therefore cannot claim to have performed. See the module docs: the
    /// licence to answer `NV_OK` is enumerated per operation, so it does not extend to one
    /// that is not enumerated.
    UnknownFlags {
        /// Everything the guest asked for.
        flags: u32,
        /// Just the bits this port could not name.
        unknown: u32,
    },
}

impl core::fmt::Display for L2EvictError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { got } => write!(
                f,
                "L2 invalidate/evict params are {got} bytes; the struct is \
                 {L2_INVALIDATE_EVICT_PARAMS_SIZE}"
            ),
            Self::UnknownFlags { flags, unknown } => write!(
                f,
                "L2 invalidate/evict flags {flags:#x} set {unknown:#x}, which the 580 SDK \
                 does not name; this port answers NV_OK only for operations it has an \
                 argument for, and it has none for a bit it cannot read"
            ),
        }
    }
}

impl core::error::Error for L2EvictError {}

/// Decode `NV2080_CTRL_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT_PARAMS`.
///
/// ⊘ The unknown-bit check is *here* rather than at the serve site, unlike
/// [`crate::eventnotify`]'s `SILENT_NOTIFIERS`: an unnamed bit is not a policy question but
/// a decode failure — the struct's only field could not be read, so there is no request to
/// describe.
///
/// # Errors
///
/// [`L2EvictError::ShortParams`], [`L2EvictError::UnknownFlags`].
pub fn decode_l2_invalidate_evict(params: &[u8]) -> Result<L2InvalidateEvict, L2EvictError> {
    if params.len() < L2_INVALIDATE_EVICT_PARAMS_SIZE {
        return Err(L2EvictError::ShortParams { got: params.len() });
    }
    let flags = u32::from_le_bytes([
        params[FLAGS_OFF],
        params[FLAGS_OFF + 1],
        params[FLAGS_OFF + 2],
        params[FLAGS_OFF + 3],
    ]);
    let unknown = flags & !L2_INVALIDATE_EVICT_FLAGS_KNOWN;
    if unknown != 0 {
        return Err(L2EvictError::UnknownFlags { flags, unknown });
    }
    Ok(L2InvalidateEvict { flags })
}

/// The reply body — ★★★ **four zero bytes**, and the argument for that is measured on both
/// sides.
///
/// ⊘ **Not** an echo of `req.flags`, and the distinction matters because the sibling control
/// [`crate::eventnotify`] *does* have to echo. The rule that separates them is *"does the
/// caller read its own params after the call returns?"*, and here it does not:
/// `kmemsysSendL2InvalidateEvict_IMPL` declares its params as a stack local and `return`s
/// the transport's status directly (`ogkm-580: kern_mem_sys.c:1079-1093`), so
/// `rpcRmApiControl_GSP`'s copyout (`rpc.c:11085-11090`) lands in memory nothing reads.
///
/// ★ And a real GA106's GSP is `[measured]` to answer the same way: the captured row is
/// `{0x20800a6cu, 0x0u, 4u, 0u, ctl_20800a6c}` with an empty array, and the C's decoder
/// treats `dlen` as trailing-zero-trimmed (`C: mode2_initctrl_ga106.h:6245, :3346`;
/// `C: src/qemu/nvkvm_gpu_emul.c:3422-3425`). Four zeros, not the flags.
///
/// ⚠ Takes the decoded request anyway. The body does not depend on it — but a signature
/// that could not see the request would let a future edit forget that the *decision* to
/// answer at all does, and this control's whole licence lives in that decision.
#[must_use]
pub fn encode_l2_invalidate_evict(_req: &L2InvalidateEvict) -> Vec<u8> {
    vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_known_flag_set_is_the_six_the_sdk_names() {
        // ⊘ Pinned as a literal *here* and derived by `|` at the definition, so the two
        // disagree loudly if a constant's value is ever mistyped. One of the two has to be
        // written twice for that to be possible at all.
        assert_eq!(L2_INVALIDATE_EVICT_FLAGS_KNOWN, 0x3f);
        for bit in [
            FLAGS_ALL,
            FLAGS_FIRST,
            FLAGS_LAST,
            FLAGS_NORMAL,
            FLAGS_CLEAN,
            FLAGS_WAIT_FB_PULL,
        ] {
            assert_eq!(bit.count_ones(), 1, "{bit:#x} is not a single bit");
            assert_eq!(bit & L2_INVALIDATE_EVICT_FLAGS_KNOWN, bit);
        }
    }

    #[test]
    fn the_value_kbusverifybar2_sends_on_a_ga106_decodes() {
        // ★ The prediction a boot can falsify: `flagsClean` on GA106 is ALL|CLEAN|WAIT_FB_PULL
        // because `bL2CleanFbPull` is NV_TRUE for this chip in the NVOC HAL initialiser.
        assert_eq!(FLAGS_CLEAN_VERIFY_BAR2, 0x31);
        let req = decode_l2_invalidate_evict(&FLAGS_CLEAN_VERIFY_BAR2.to_le_bytes())
            .expect("the guest's own value must decode");
        assert_eq!(req.flags, 0x31);
        assert!(req.writes_back());
        assert!(req.waits_for_fb_pull());
    }

    #[test]
    fn a_bit_the_sdk_does_not_name_is_refused() {
        // ★★★ The gate the module's licence rests on: the `NV_OK` argument is enumerated
        // per operation, so an operation that is not enumerated gets no answer.
        let err = decode_l2_invalidate_evict(&0x0000_0041u32.to_le_bytes())
            .expect_err("bit 6 is named by no 580 SDK constant");
        assert_eq!(
            err,
            L2EvictError::UnknownFlags {
                flags: 0x41,
                unknown: 0x40
            }
        );
        // ⊘ And the top bit too, which is the one a sign-extension bug would set.
        assert!(matches!(
            decode_l2_invalidate_evict(&0x8000_0001u32.to_le_bytes()),
            Err(L2EvictError::UnknownFlags {
                unknown: 0x8000_0000,
                ..
            })
        ));
    }

    #[test]
    fn every_subset_of_the_named_bits_decodes_including_the_empty_one() {
        // ⊘ Quantified over the whole 64-element subset lattice rather than over the two
        // values the driver happens to send. The licence is per-bit, so the gate is too —
        // and `flags == 0` is accepted rather than refused, because refusing it would be
        // inventing a rule RM does not have for a request that asks for nothing.
        for flags in 0..=L2_INVALIDATE_EVICT_FLAGS_KNOWN {
            if flags & !L2_INVALIDATE_EVICT_FLAGS_KNOWN != 0 {
                continue;
            }
            let req = decode_l2_invalidate_evict(&flags.to_le_bytes())
                .unwrap_or_else(|e| panic!("flags {flags:#x} is a named subset but {e}"));
            assert_eq!(req.flags, flags);
        }
    }

    #[test]
    fn short_params_are_refused_rather_than_padded() {
        for got in 0..L2_INVALIDATE_EVICT_PARAMS_SIZE {
            assert_eq!(
                decode_l2_invalidate_evict(&vec![0xffu8; got]),
                Err(L2EvictError::ShortParams { got })
            );
        }
    }

    #[test]
    fn the_reply_is_four_zeros_and_carries_no_byte_of_the_request() {
        // ★★★ The measured shape: `psize = 4, dlen = 0` on a real GA106
        // (C: mode2_initctrl_ga106.h:6245). A reply that echoed `flags` would be a
        // defensible design and it is NOT the one the hardware uses.
        for flags in [0x00, 0x01, 0x11, FLAGS_CLEAN_VERIFY_BAR2, 0x3f] {
            let body = encode_l2_invalidate_evict(&L2InvalidateEvict { flags });
            assert_eq!(body.len(), L2_INVALIDATE_EVICT_PARAMS_SIZE);
            assert_eq!(
                body,
                vec![0u8; L2_INVALIDATE_EVICT_PARAMS_SIZE],
                "{flags:#x}"
            );
        }
    }

    #[test]
    fn the_control_id_is_the_one_the_capture_carries() {
        assert_eq!(
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT,
            0x2080_0a6c
        );
        // ⊘ Bit 15 clear: this control is outside the GSS-legacy mask, so the guest's reply
        // cache cannot make our answer permanent (`ogkm-580: rpc.c:11096-11103`). Stated
        // here as well as at the serve site because it is a property of the *id*.
        assert_eq!(
            NV2080_CTRL_CMD_INTERNAL_MEMSYS_L2_INVALIDATE_EVICT & 0x8000,
            0
        );
    }
}
