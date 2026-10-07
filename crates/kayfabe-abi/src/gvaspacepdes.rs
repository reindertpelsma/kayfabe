//! `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
//! (`0x20800a9f`) — 184 bytes, every field `[in]`, and ★★★ **the first control this port
//! answers in which the guest is TELLING US where its page directories live.**
//!
//! # ★★★ What is actually happening here
//!
//! Every other control this port serves is a question. This one is a **publication**. RM has
//! just built the client-RM half of a *split* virtual address space — `bClientRm`, because
//! `pGpu->bSplitVasManagementServerClientRm` defaults to true for any GSP client
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_registry.c:171-182`) — and it is handing the
//! server (us, since we are the GSP) the physical addresses of the page-directory levels it
//! reserved, so that the server's own walker can share them.
//!
//! ```c
//! gvaspaceReserveSplitVaSpace(pGVAS, pGpu)                      // gpu_vaspace.c:395
//!   → _gvaspaceReserveVaForClientRm(pGVAS, pGpu)                //             :315
//!       → mmuWalkReserveEntries(…, 0x1_0000_0000, 0x1_1FFF_FFFF, NV_TRUE)   // :368
//!       → gvaspaceCopyServerRmReservedPdesToServerRm(pGVAS, pGpu)           // :378
//!           → pRmApi->Control(…, 0x20800a9f, &globalCopyParams, 184)        // :4151
//! ```
//!
//! The range is fixed and named: `SPLIT_VAS_SERVER_RM_MANAGED_VA_START` = `0x1_0000_0000`
//! and a 512 MiB span (`ogkm-580: src/nvidia/generated/g_gpu_vaspace_nvoc.h:99-100`), at
//! `pageSize = NVBIT64(21)` — PD0 coverage, `GMMU_PD0_VADDR_BIT_LO = 21`
//! (`gpu_vaspace.c:64`).
//!
//! ★★ **This is a page-table publication event, and this port has a doctrine about those.**
//! `docs/design/mode2_address_table.md`: the address table is forward-populated from
//! bind-time bindings and from witnessed page-table writes, never reverse-resolved. These
//! four levels are a *binding*, arriving by the sanctioned transport. ⊘ Nothing is recorded
//! from them **yet** — the GVAS walker is not built and inventing a half-populated table
//! from a control nothing consumes would be worse than not having one. What this rung does
//! is make the event **decodable and refusable**, which is the precondition for recording it.
//!
//! # ★★★ Why answering `NV_OK` is a decision and not a fall-through
//!
//! `[inferred]` from `ogkm-580`, and the two facts that carry it:
//!
//! 1. **There are no `[out]` fields.** Every member of
//!    `NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS` is documented `[in]`
//!    (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:272-332`).
//! 2. **The caller never reads its params again.** `globalCopyParams` is a stack local at
//!    `gpu_vaspace.c:4142`, filled by `_gvaspacePopulatePDEentries` at `:4144`, passed at
//!    `:4152`, and the function returns at `:4156`. ⚠ That second fact is the one that
//!    matters, and it is the one the transport trap makes people skip: `rpc.c:11088` does
//!    `portMemCopy(pParamStructPtr, paramsSize, rpc_params->params, paramsSize)` — the reply
//!    is memcpy'd **over the caller's own struct** — so *"has `[out]` fields"* is the wrong
//!    question and *"does the caller read its own params afterwards"* is the right one. Here
//!    it does not, so the body cannot matter. It is re-encoded faithfully regardless.
//!
//! ⊘ **What `NV_OK` does NOT mean.** It does not mean the server shares those page
//! directories, because there is no server-side walker to share them with. It means *"the
//! publication was well-formed and was accepted"*. The day a host GPU is behind this, a
//! `NV_OK` here without an actual page-table publication becomes a lie of the kind
//! `kayfabe_abi::l2evict`'s header enumerates, and it must be re-decided there rather than
//! inherited.
//!
//! # ★★ Refusing it is survivable, which is why this is not the rung's fatal fix
//!
//! `NV_ASSERT_OK_OR_RETURN` at `gpu_vaspace.c:4148` sends `0x56` back up through
//! `gvaspaceConstruct_` (`:611`) and `_kgmmuCreateGlobalVASpace` (`kern_gmmu.c:245`) into
//! `kgmmuStatePostLoad_IMPL` — and then into `gpuStatePostLoad`, which maps
//! `NV_ERR_NOT_SUPPORTED` to `NV_OK` at `gpu.c:3438` like every other post-`gpuPreInit`
//! loop. ⇒ `[measured]` run `gmmu1` at `12b001f`: three assertion lines, and the boot
//! carried on to fail twenty lines later on something else entirely.
//!
//! ★ So this is served for what refusing *leaves behind*, not for what it returns:
//! `pGpuGrp->pGlobalVASpace` is assigned **before** `vaspaceConstruct_` runs
//! (`ogkm-580: src/nvidia/src/kernel/mem_mgr/virt_mem_mgr.c:126` vs `:134`), so a failed
//! construct leaves the GPU group without the device VA space that every later
//! `vaspaceGetByHandleOrDeviceDefault` expects to find. That is a `RefusalFailsOpen`, and
//! it is classified as one.

/// `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
/// (`ogkm-580: ctrl/ctrl2080/ctrl2080internal.h:1902`).
pub const NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER: u32 =
    0x2080_0a9f;

/// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:268`) — ★★★ **the same
/// publication, the same 184 bytes, and a completely different caller.**
///
/// # ★★★ Two ids, one struct, and the second one is the one a real boot hits
///
/// `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL` has two arms
/// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:4039-4137`), chosen on whether
/// there is a resserv call context:
///
/// - `pContext == NULL` — the GPU-group *global* VA space, built by RM for itself. Sends
///   the `NV2080` wrapper [`NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`].
/// - `pContext != NULL` — a VA space being constructed **under a client's device**, which
///   RPCs `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` directly through
///   `_gvaspaceCopyServerRmReservedPdesToServerRm` (`gpu_vaspace.c:5160-5190`).
///
/// ⚠ The second arm is not an alternative to the first — it is what runs for **every
/// device default VA space**, so serving only the `NV2080` id leaves the guest unable to
/// construct any VAS at all.
///
/// ## `[measured]` — this is the failure, not a theory about one
///
/// Run `stateload2` at `7819839` (`/workspace/bench/run_stateload2_dmesg.log:12-30`) shows
/// the whole chain, in one boot, twice over:
///
/// ```text
/// gpu_vaspace.c:5187   NV_ASSERT(NV_OK == status)          ← the RPC we refused
/// gpu_vaspace.c:4129   NV_ASSERT_OR_GOTO(…, done)
/// gpu_vaspace.c:611    NV_ASSERT_OR_GOTO(…, catch)         ← gvaspaceConstruct_
/// device_share.c:260   NV_ASSERT(0)                        ← vmmCreateVaspace failed
/// virtual_mem.c:133    vaspaceGetByHandleOrDeviceDefault → 0x56
/// mem_utils_gm107.c:322  NV50_MEMORY_VIRTUAL alloc failed
/// …:1301 / :857 → ce_utils.c:286 → mem_scrub.c:181 → mem_mgr.c:487 → kernel_fifo.c:3129
/// ```
///
/// ⇒ refusing this one control amputates **the device VA space, the CE utility channel and
/// the framebuffer scrubber**. ⊘ It is not what stopped that boot — `kernel_fifo.c:3129`'s
/// failure is swallowed and the verdict was `NV_ERR_IRQ_NOT_FIRING` four seconds later —
/// which is exactly why it had to be read off the log rather than inferred from the
/// verdict.
///
/// # The transport trap, asked the right way round
///
/// `rpc.c:11088` memcpy's the reply **over the caller's own struct**, so the question is
/// never *"does this control have `[OUT]` fields"* but *"does the caller read its params
/// afterwards"*. Here: `pdeCopyParams` is a stack local at `gpu_vaspace.c:4060`, filled by
/// `_gvaspacePopulatePDEentries` at `:4118`, passed at `:4127`, and from `done:` the
/// function only issues `NV_RM_RPC_FREE` and returns (`:4130-4137`). It never reads it
/// again. The re-encode is still faithful, for the reason
/// [`encode_server_reserved_pdes`] gives.
pub const NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES: u32 = 0x90f1_0106;

/// `GMMU_FMT_MAX_LEVELS` (`ogkm-580: ctrl/ctrl90f1.h:37`) — the `levels[]` bound, and the
/// only thing that makes `numLevelsToCopy` checkable.
pub const GMMU_FMT_MAX_LEVELS: usize = 6;

/// Size of one `levels[]` entry: `NvU64 physAddress`, `NvU64 size`, `NvU32 aperture`,
/// `NvU8 pageShift`, three bytes of tail padding to the 8-byte alignment the `NvU64`s force.
pub const LEVEL_SIZE: usize = 24;

/// `sizeof(NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS)`, and the identical size of
/// the `NV2080` wrapper that contains exactly this one member
/// (`ctrl2080internal.h:1906-1908`).
pub const COPY_SERVER_RESERVED_PDES_PARAMS_SIZE: usize = 0x28 + GMMU_FMT_MAX_LEVELS * LEVEL_SIZE;

// ★★★ **`GMMU_APERTURE`, transcribed from the enum's own declaration order** — `ogkm-580:
// src/nvidia/inc/libraries/mmu/gmmu_fmt.h:280-325`, an unnumbered C enum, so the ORDER is
// the encoding.
//
// ⚠⚠ **Every one of these four was wrong when they were first written, and a boot is what
// said so.** `[measured 2026-08-08, boot run_p35_a34025b]`: the walling channel's own
// publication carries `aperture 1` on all four levels, our device printed
// `root=0x2efa9c000/ap1/sh47`, and the resolver refused it `ROOTAP1` — *"aperture 1 is not
// this device's framebuffer"* — because the values had been assumed from the **PDE FIELD**
// encoding (`kern_gmmu_fmt_gm10x.c:165-182`, `0=INVALID 1=VIDEO 2=SYS_COH 3=SYS_NONCOH`)
// rather than read from the **enum** this control's `levels[].aperture` field actually
// carries. The two agree on `INVALID` and `VIDEO` and disagree on everything else
// (`two_encodings_agreeing_on_the_first_values`).
//
// ★ Note `SYS_NONCOH` precedes `SYS_COH`, which is the reverse of every other list in this
// port and is exactly the sort of ordering nobody re-checks. `gpu_vaspace.c:3798-3808` is
// the corroborating half: the *sender* fills the same field from `ADDR_FBMEM →
// GMMU_APERTURE_VIDEO` / `ADDR_SYSMEM → GMMU_APERTURE_SYS_{COH,NONCOH}`, and
// `:4291-4296` is the receiver switching it back.
//
// ★★ **They live HERE, in the ABI crate, and there is exactly one declaration.** They were
// written in `kayfabe_device::ceresolve` first, which re-exports them; a second
// transcription in the bridge — the crate that now turns `levels[0]` into an
// `RmEvent::SetPageDir` — would be this enum's own recorded bug, committed twice.

/// `GMMU_APERTURE_INVALID` — ⊘ **a real value, not a blank**: *"only supported for GPU PDEs
/// to distinguish invalid sub-levels"*. A level that publishes it has published *"there is
/// no sub-level here"*, which is not the same statement as an aperture.
pub const GMMU_APERTURE_INVALID: u32 = 0;
/// `GMMU_APERTURE_VIDEO` — the receiver's own fork value
/// (`ogkm-580: gpu_vaspace.c:4291-4292` switches `VIDEO → ADDR_FBMEM`).
pub const GMMU_APERTURE_VIDEO: u32 = 1;
/// `GMMU_APERTURE_PEER`.
pub const GMMU_APERTURE_PEER: u32 = 2;
/// `GMMU_APERTURE_SYS_NONCOH`. ⚠ **Three, and it comes BEFORE coherent.**
pub const GMMU_APERTURE_SYS_NONCOH: u32 = 3;
/// `GMMU_APERTURE_SYS_COH`.
pub const GMMU_APERTURE_SYS_COH: u32 = 4;

/// Decode a `GMMU_APERTURE_*` value. `None` is *"a value the header does not define"*,
/// which the receiver itself asserts on (`ogkm-580: gpu_vaspace.c:4503-4511`).
#[must_use]
pub fn decode_aperture(raw: u32) -> Option<kayfabe_arch::Aperture> {
    match raw {
        GMMU_APERTURE_VIDEO => Some(kayfabe_arch::Aperture::Vidmem),
        GMMU_APERTURE_PEER => Some(kayfabe_arch::Aperture::Peer),
        GMMU_APERTURE_SYS_COH => Some(kayfabe_arch::Aperture::SysmemCoherent),
        GMMU_APERTURE_SYS_NONCOH => Some(kayfabe_arch::Aperture::SysmemNonCoherent),
        // ⊘ `GMMU_APERTURE_INVALID` lands here with everything else the enum does not
        // define, and deliberately: *"this sub-level is absent"* and *"a value we do not
        // recognise"* are both **not an aperture**, and both must refuse. The raw word
        // travels beside the decode so a report can still distinguish them.
        _ => None,
    }
}

const O_H_SUBDEVICE: usize = 0x00;
const O_SUBDEVICE_ID: usize = 0x04;
const O_PAGE_SIZE: usize = 0x08;
const O_VIRT_ADDR_LO: usize = 0x10;
const O_VIRT_ADDR_HI: usize = 0x18;
const O_NUM_LEVELS: usize = 0x20;
const O_LEVELS: usize = 0x28;
const _: () = assert!(COPY_SERVER_RESERVED_PDES_PARAMS_SIZE == 184);

/// One published page-directory level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PdeLevel {
    /// Physical address of this level instance. ⚠ A **guest** physical address, in the
    /// guest's own frame of reference — nothing here translates it, and nothing may treat it
    /// as a host address.
    pub phys_address: u64,
    /// Bytes allocated for this level instance.
    pub size: u64,
    /// `GMMU_APERTURE_*` — which memory this level lives in.
    pub aperture: u32,
    /// Page shift of the level. `[measured]` on GA106 the four levels are 47, 38, 29, 21.
    pub page_shift: u8,
}

/// The decoded publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerReservedPdes {
    /// `hSubDevice` — zero means *"use `subDeviceId`"* (`ctrl90f1.h:274-277`).
    pub h_subdevice: u32,
    /// `subDeviceId`.
    pub subdevice_id: u32,
    /// VA coverage of the level being reserved.
    pub page_size: u64,
    /// First GPU VA of the reserved range; must be `page_size`-aligned.
    pub virt_addr_lo: u64,
    /// Last GPU VA of the reserved range; `+1` must be `page_size`-aligned.
    pub virt_addr_hi: u64,
    /// How many of [`Self::levels`] are meaningful.
    pub num_levels: u32,
    /// The published levels. Entries at or past [`Self::num_levels`] are decoded but carry
    /// no meaning; they are kept so the re-encode is faithful.
    pub levels: [PdeLevel; GMMU_FMT_MAX_LEVELS],
}

impl ServerReservedPdes {
    /// ★★★ **`levels[0]` — the ROOT page directory of the VA space this publication names.**
    ///
    /// One function so *"index 0 is the root"* is stated once rather than at each consumer.
    /// The derivation, both halves: `_gvaspacePopulatePDEentries` starts at
    /// `pGpuState->pFmt->pRoot` and descends via `mmuFmtGetNextLevel` at the **bottom** of
    /// its loop, filling `levels[i]` top-down (`ogkm-580: gpu_vaspace.c:3974-4031`); the
    /// receiver consumes it **bottom-up** (`for (i = numLevelsToCopy - 1; i >= 0; i--)`,
    /// `:4492`) — root consumed last, so root is index 0.
    ///
    /// ⊘ Always meaningful: [`decode_server_reserved_pdes`] refuses `numLevelsToCopy == 0`
    /// ([`ServerReservedPdesError::LevelCountOutOfRange`]), so a decoded publication has at
    /// least this one level and this is never an out-of-range read dressed as a default.
    #[must_use]
    pub fn root(&self) -> PdeLevel {
        self.levels[0]
    }
}

/// Why a publication was refused.
///
/// ⊘ Each variant is a property the header itself states, so a refusal here is *"the guest
/// contradicted its own ABI"* rather than *"we did not like it"*. That distinction is what
/// keeps `NV_OK` from being a fall-through: there exist 184-byte payloads this port says no
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerReservedPdesError {
    /// Not [`COPY_SERVER_RESERVED_PDES_PARAMS_SIZE`] bytes.
    WrongSize {
        /// Byte length actually supplied.
        got: usize,
    },
    /// `numLevelsToCopy > GMMU_FMT_MAX_LEVELS`, or zero — a publication of no levels is not
    /// a publication.
    LevelCountOutOfRange {
        /// The `numLevelsToCopy` the guest wrote.
        got: u32,
    },
    /// `pageSize` is zero or not a power of two. It is a *"VA coverage"* (`ctrl90f1.h:284`),
    /// so a non-power-of-two makes the two alignment rules below unstatable.
    PageSizeNotPowerOfTwo {
        /// The `pageSize` the guest wrote.
        got: u64,
    },
    /// `virtAddrLo` is not `pageSize`-aligned — the header requires it (`ctrl90f1.h:290`).
    VirtAddrLoMisaligned {
        /// The `virtAddrLo` the guest wrote.
        lo: u64,
        /// The `pageSize` it is required to be aligned to.
        page_size: u64,
    },
    /// `virtAddrHi + 1` is not `pageSize`-aligned (`ctrl90f1.h:296`), or `virtAddrHi` is
    /// `u64::MAX` so `+1` does not exist.
    VirtAddrHiMisaligned {
        /// The `virtAddrHi` the guest wrote.
        hi: u64,
        /// The `pageSize` that `hi + 1` is required to be aligned to.
        page_size: u64,
    },
    /// `virtAddrHi < virtAddrLo` — an empty or inverted range.
    RangeInverted {
        /// The `virtAddrLo` the guest wrote.
        lo: u64,
        /// The `virtAddrHi` the guest wrote.
        hi: u64,
    },
    /// A meaningful level has `size == 0`. A page-directory level of zero bytes is a
    /// publication of nothing at an address.
    ZeroLevelSize {
        /// Index into `levels[]` of the offending entry.
        level: u32,
    },
}

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn rd64(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Decode and **validate** one publication.
///
/// # Errors
/// [`ServerReservedPdesError`] — see that type; every variant is a rule from `ctrl90f1.h`.
pub fn decode_server_reserved_pdes(
    buf: &[u8],
) -> Result<ServerReservedPdes, ServerReservedPdesError> {
    if buf.len() != COPY_SERVER_RESERVED_PDES_PARAMS_SIZE {
        return Err(ServerReservedPdesError::WrongSize { got: buf.len() });
    }
    let page_size = rd64(buf, O_PAGE_SIZE);
    if page_size == 0 || !page_size.is_power_of_two() {
        return Err(ServerReservedPdesError::PageSizeNotPowerOfTwo { got: page_size });
    }
    let virt_addr_lo = rd64(buf, O_VIRT_ADDR_LO);
    let virt_addr_hi = rd64(buf, O_VIRT_ADDR_HI);
    if virt_addr_hi < virt_addr_lo {
        return Err(ServerReservedPdesError::RangeInverted {
            lo: virt_addr_lo,
            hi: virt_addr_hi,
        });
    }
    if !virt_addr_lo.is_multiple_of(page_size) {
        return Err(ServerReservedPdesError::VirtAddrLoMisaligned {
            lo: virt_addr_lo,
            page_size,
        });
    }
    // ⚠ `hi + 1`, not `hi`: `virtAddrHi` is the LAST address in the range, so it is the
    // exclusive end that must be aligned. Reading `hi % page_size == 0` would reject every
    // legal publication and accept none — and it is the natural misreading.
    let hi_end =
        virt_addr_hi
            .checked_add(1)
            .ok_or(ServerReservedPdesError::VirtAddrHiMisaligned {
                hi: virt_addr_hi,
                page_size,
            })?;
    if !hi_end.is_multiple_of(page_size) {
        return Err(ServerReservedPdesError::VirtAddrHiMisaligned {
            hi: virt_addr_hi,
            page_size,
        });
    }
    let num_levels = rd32(buf, O_NUM_LEVELS);
    if num_levels == 0 || num_levels as usize > GMMU_FMT_MAX_LEVELS {
        return Err(ServerReservedPdesError::LevelCountOutOfRange { got: num_levels });
    }
    let mut levels = [PdeLevel::default(); GMMU_FMT_MAX_LEVELS];
    for (i, lv) in levels.iter_mut().enumerate() {
        let at = O_LEVELS + i * LEVEL_SIZE;
        *lv = PdeLevel {
            phys_address: rd64(buf, at),
            size: rd64(buf, at + 8),
            aperture: rd32(buf, at + 16),
            page_shift: buf[at + 20],
        };
        if (i as u32) < num_levels && lv.size == 0 {
            return Err(ServerReservedPdesError::ZeroLevelSize { level: i as u32 });
        }
    }
    Ok(ServerReservedPdes {
        h_subdevice: rd32(buf, O_H_SUBDEVICE),
        subdevice_id: rd32(buf, O_SUBDEVICE_ID),
        page_size,
        virt_addr_lo,
        virt_addr_hi,
        num_levels,
        levels,
    })
}

/// Re-encode a publication into the 184-byte reply body.
///
/// ⊘ **A re-encode, not an echo**, on purpose. An echo would answer `NV_OK` for any 184
/// bytes whatsoever and would make the decode above dead code that no test could notice was
/// dead. Round-tripping means the reply is a function of the *fields this port understood*,
/// so a field it failed to decode would show up as a byte difference —
/// `tests/gvaspace_pdes.rs` is where that is checked against the oracle's own captured
/// publication.
#[must_use]
pub fn encode_server_reserved_pdes(p: &ServerReservedPdes) -> Vec<u8> {
    let mut out = vec![0u8; COPY_SERVER_RESERVED_PDES_PARAMS_SIZE];
    out[O_H_SUBDEVICE..O_H_SUBDEVICE + 4].copy_from_slice(&p.h_subdevice.to_le_bytes());
    out[O_SUBDEVICE_ID..O_SUBDEVICE_ID + 4].copy_from_slice(&p.subdevice_id.to_le_bytes());
    out[O_PAGE_SIZE..O_PAGE_SIZE + 8].copy_from_slice(&p.page_size.to_le_bytes());
    out[O_VIRT_ADDR_LO..O_VIRT_ADDR_LO + 8].copy_from_slice(&p.virt_addr_lo.to_le_bytes());
    out[O_VIRT_ADDR_HI..O_VIRT_ADDR_HI + 8].copy_from_slice(&p.virt_addr_hi.to_le_bytes());
    out[O_NUM_LEVELS..O_NUM_LEVELS + 4].copy_from_slice(&p.num_levels.to_le_bytes());
    for (i, lv) in p.levels.iter().enumerate() {
        let at = O_LEVELS + i * LEVEL_SIZE;
        out[at..at + 8].copy_from_slice(&lv.phys_address.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&lv.size.to_le_bytes());
        out[at + 16..at + 20].copy_from_slice(&lv.aperture.to_le_bytes());
        out[at + 20] = lv.page_shift;
    }
    out
}
