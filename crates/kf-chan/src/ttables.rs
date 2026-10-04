// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ P1+P2 inc C (`docs/design/V3_P1P2_TSPACE.md` §3.2, §3.6) — **the per-tier method tables of the
//! T-mode rewriter.**
//!
//! ⊘ **Written by hand, never generated from the class headers.** The open headers are trimmed:
//! `clc9b5.h` defines no method at all, `clcab5.h` two, `clc8b5.h` no `LINE_COUNT`/`PITCH_IN`/
//! `SET_SRC_PHYS_MODE`, and `clc86f.h`/`clc96f.h`/`clca6f.h` list no `SEMAPHOREA-D`, `NOP` or
//! `NON_STALL_INTERRUPT` — yet stock CeUtils pushes `NV906F_SEMAPHOREA-D` on every family
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:731-746`) and UVM inherits CE
//! bodies by `parent_id` (`ogkm-580: kernel-open/nvidia-uvm/uvm_hal.c:143-180`: C7B5 → C8B5 → C9B5
//! → CAB5). A header-generated table would refuse CeUtils' own release on Hopper and Blackwell, and
//! pass vacuously where a header defines nothing.
//!
//! So each row is classified by SEMANTICS and names the class it is read from. Every method has one
//! of three dispositions: an ADDRESS register (state; the rewriter authors the address — §3.4), an
//! ADDRESS-FREE method (state, or authored with its decoded argument — §3.2), or REFUSED by name.
//! A method no row names is UNCLASSIFIED: the shadow counts it and T-mode refuses it.
//!
//! ⚠ The rows for the source-only tiers (Hopper, GA100, the Blackwell datacenter dies) are read from
//! source only; the CENSUS box step (§8) counts what each family's stock drivers actually push.

use crate::translated::Refusal;

/// ★ A rewriter tier — the CE class the guest bound, which fixes the CE method layout and the
/// `LAUNCH_DMA` field table. The host-method rows are common to every tier (every class from
/// `NVC46F` places `SEM_*` and `MEM_OP_*` at the same offsets); only the `SEM_ADDR_HI` width moves,
/// at Hopper ([`Tier::wide_sem_addr`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// `TURING_DMA_COPY_A` (`0xC5B5`) — `clc5b5.h`.
    C5b5,
    /// `AMPERE_DMA_COPY_A` (`0xC6B5`, GA100) — `clc6b5.h`.
    C6b5,
    /// `AMPERE_DMA_COPY_B` (`0xC7B5`, GA10x and Ada) — `clc7b5.h`.
    C7b5,
    /// `HOPPER_DMA_COPY_A` (`0xC8B5`) — `clc8b5.h`, the rows it omits inherited from `C7B5`.
    C8b5,
    /// `BLACKWELL_DMA_COPY_A` (`0xC9B5`) — inherits `C8B5` whole (`clc9b5.h` defines no method).
    C9b5,
    /// `BLACKWELL_DMA_COPY_B` (`0xCAB5`) — inherits `C8B5`, adds `REQ_ATTR` and `PREFETCH`.
    Cab5,
}

impl Tier {
    /// The tier of a bound CE class, or `None` (refused by name by the caller).
    #[must_use]
    pub const fn of_ce_class(class: u32) -> Option<Tier> {
        Some(match class {
            0xC5B5 => Tier::C5b5,
            0xC6B5 => Tier::C6b5,
            0xC7B5 => Tier::C7b5,
            0xC8B5 => Tier::C8b5,
            0xC9B5 => Tier::C9b5,
            0xCAB5 => Tier::Cab5,
            _ => return None,
        })
    }

    /// `OFFSET_{IN,OUT}_UPPER` / `SET_SEMAPHORE_A` upper-bits mask: `16:0` through `C7B5`, `24:0`
    /// from `C8B5` (`clc7b5.h`, `clc8b5.h`).
    #[must_use]
    pub const fn upper_mask(self) -> u32 {
        match self {
            Tier::C5b5 | Tier::C6b5 | Tier::C7b5 => 0x1_FFFF,
            Tier::C8b5 | Tier::C9b5 | Tier::Cab5 => 0x1FF_FFFF,
        }
    }

    /// The host `SEM_ADDR_HI` is `24:0` (57-bit) from Hopper's `NVC86F` on, `7:0` (40-bit) before
    /// (`clc46f.h`, `clc56f.h` vs `clc86f.h`, `clc96f.h`, `clca6f.h`). The legacy `SEMAPHOREA` is
    /// 40-bit on every tier.
    #[must_use]
    pub const fn wide_sem_addr(self) -> bool {
        matches!(self, Tier::C8b5 | Tier::C9b5 | Tier::Cab5)
    }

    /// `LAUNCH_DMA` bit 23 is `MEMORY_SCRUB_ENABLE` from `C8B5` on (it is `VPRMODE`'s high bit
    /// before).
    #[must_use]
    pub const fn has_fast_scrub(self) -> bool {
        matches!(self, Tier::C8b5 | Tier::C9b5 | Tier::Cab5)
    }

    /// The family's host class defines the GP control opcode `SET_PB_SEGMENT_EXTENDED_BASE` (4):
    /// Hopper's `NVC86F` and later (`ogkm-580: src/common/sdk/nvidia/inc/class/clc86f.h:184-189`;
    /// `clc56f.h:280-284` names opcodes 0-3 only). Keyed on the CE class of the same family: kayfabe
    /// presents the host family, so the host CE class and the host channel class move together.
    #[must_use]
    pub const fn has_pb_extended_base(self) -> bool {
        matches!(self, Tier::C8b5 | Tier::C9b5 | Tier::Cab5)
    }
}

/// A copy-engine method's disposition (subchannels 0-4, methods at or above `0x100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeMethod {
    /// `NOP` (`0x100`) — address-free, nothing to author.
    Nop,
    /// `SET_SEMAPHORE_A` — ADDRESS (upper bits).
    SemA,
    /// `SET_SEMAPHORE_B` — ADDRESS (lower bits).
    SemB,
    /// `SET_SEMAPHORE_PAYLOAD`.
    SemPayload,
    /// `SET_SEMAPHORE_PAYLOAD_UPPER` (`C7B5`+).
    SemPayloadUpper,
    /// `SET_SRC_PHYS_MODE` — state only (no T launch is physical).
    SrcPhysMode,
    /// `SET_DST_PHYS_MODE` — state only.
    DstPhysMode,
    /// `LAUNCH_DMA` — the trigger.
    Launch,
    /// `OFFSET_IN_UPPER` — ADDRESS.
    OffsetInUpper,
    /// `OFFSET_IN_LOWER` — ADDRESS.
    OffsetInLower,
    /// `OFFSET_OUT_UPPER` — ADDRESS.
    OffsetOutUpper,
    /// `OFFSET_OUT_LOWER` — ADDRESS.
    OffsetOutLower,
    /// `PITCH_IN` — footprint (re-emitted at the trigger).
    PitchIn,
    /// `PITCH_OUT` — footprint.
    PitchOut,
    /// `LINE_LENGTH_IN` — footprint.
    LineLength,
    /// `LINE_COUNT` — footprint.
    LineCount,
    /// `SET_REMAP_CONST_A` — footprint (with `REMAP_ENABLE`).
    RemapConstA,
    /// `SET_REMAP_CONST_B`.
    RemapConstB,
    /// `SET_REMAP_COMPONENTS`.
    RemapComponents,
    /// The block-linear geometry (`SET_{SRC,DST}_BLOCK_SIZE` … `{SRC,DST}_ORIGIN_X/Y`,
    /// `0x70C`-`0x750`): state a PITCH launch never reads; every block-linear virtual or physical
    /// launch is refused, so it is consumed, never emitted.
    BlockLinearState,
    /// `SET_MEMORY_SCRUB_PARAMETERS` (`0x6FC`, `C8B5`+) — address-free; read only by a fast scrub,
    /// which T-mode converts to a virtual fill, so consumed.
    ScrubParameters,
    /// `REQ_ATTR` (`0x754`, `CAB5`) — address-free; authored from its decoded `1:0` field.
    ReqAttr,
    /// Refused by name on every tier ([`crate::translated::REFUSED_METHODS`]).
    Refused(&'static str),
}

/// ★ The CE method table: `None` = unclassified.
#[must_use]
pub fn ce_method(tier: Tier, m: u32) -> Option<CeMethod> {
    if let Some(name) = crate::translated::refused_method(m, true) {
        return Some(CeMethod::Refused(name));
    }
    Some(match m {
        // Every tier (`clc5b5.h` … `clc7b5.h`; `C8B5`+ inherit the rows `clc8b5.h` omits).
        0x100 => CeMethod::Nop,
        0x240 => CeMethod::SemA,
        0x244 => CeMethod::SemB,
        0x248 => CeMethod::SemPayload,
        0x260 => CeMethod::SrcPhysMode,
        0x264 => CeMethod::DstPhysMode,
        0x300 => CeMethod::Launch,
        0x400 => CeMethod::OffsetInUpper,
        0x404 => CeMethod::OffsetInLower,
        0x408 => CeMethod::OffsetOutUpper,
        0x40C => CeMethod::OffsetOutLower,
        0x410 => CeMethod::PitchIn,
        0x414 => CeMethod::PitchOut,
        0x418 => CeMethod::LineLength,
        0x41C => CeMethod::LineCount,
        0x700 => CeMethod::RemapConstA,
        0x704 => CeMethod::RemapConstB,
        0x708 => CeMethod::RemapComponents,
        0x70C..=0x720 | 0x728..=0x73C | 0x744..=0x750 if m.is_multiple_of(4) => {
            CeMethod::BlockLinearState
        }
        // `C7B5` adds the payload's upper word (`clc7b5.h:53-54`); `C8B5`+ inherit it.
        0x24C if tier >= Tier::C7b5 => CeMethod::SemPayloadUpper,
        // `C8B5` (`clc8b5.h`): the scrub parameters; `C9B5`/`CAB5` inherit.
        0x6FC if tier >= Tier::C8b5 => CeMethod::ScrubParameters,
        // `CAB5` (`clcab5.h:29-41`).
        0x754 if tier == Tier::Cab5 => CeMethod::ReqAttr,
        _ => return None,
    })
}

/// A host (channel) method's disposition (below `0x100`, any subchannel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMethod {
    /// `SET_OBJECT` — authored from the decoded class.
    SetObject,
    /// `NOP` — address-free; its payload is data the engine never executes, consumed.
    Nop,
    /// `SEMAPHOREA` — ADDRESS (39:32).
    SemaphoreA,
    /// `SEMAPHOREB` — ADDRESS (31:2).
    SemaphoreB,
    /// `SEMAPHOREC` — payload.
    SemaphoreC,
    /// `SEMAPHORED` — the trigger.
    SemaphoreD,
    /// `NON_STALL_INTERRUPT` — address-free, authored.
    NonStallInterrupt,
    /// `FB_FLUSH` — address-free, authored.
    FbFlush,
    /// `MEM_OP_A`.
    MemOpA,
    /// `MEM_OP_B`.
    MemOpB,
    /// `MEM_OP_C`.
    MemOpC,
    /// `MEM_OP_D` — the trigger (MEMBAR / L2 / TLB invalidate split / served).
    MemOpD,
    /// `SET_REFERENCE` — address-free, authored.
    SetReference,
    /// `SEM_ADDR_LO` — ADDRESS.
    SemAddrLo,
    /// `SEM_ADDR_HI` — ADDRESS.
    SemAddrHi,
    /// `SEM_PAYLOAD_LO`.
    SemPayloadLo,
    /// `SEM_PAYLOAD_HI`.
    SemPayloadHi,
    /// `SEM_EXECUTE` — the trigger.
    SemExecute,
    /// `WFI` — address-free, authored.
    Wfi,
    /// `YIELD` — address-free, authored.
    Yield,
    /// Refused by name ([`crate::translated::REFUSED_METHODS`]).
    Refused(&'static str),
}

/// ★ The host method table (`clc46f.h`, `clc56f.h`; Hopper+ inherit the rows their trimmed headers
/// omit — CeUtils pushes `NV906F_SEMAPHOREA-D` on every family): `None` = unclassified.
#[must_use]
pub fn host_method(m: u32) -> Option<HostMethod> {
    if let Some(name) = crate::translated::refused_method(m, false) {
        return Some(HostMethod::Refused(name));
    }
    Some(match m {
        0x00 => HostMethod::SetObject,
        0x08 => HostMethod::Nop,
        0x10 => HostMethod::SemaphoreA,
        0x14 => HostMethod::SemaphoreB,
        0x18 => HostMethod::SemaphoreC,
        0x1C => HostMethod::SemaphoreD,
        0x20 => HostMethod::NonStallInterrupt,
        0x24 => HostMethod::FbFlush,
        0x28 => HostMethod::MemOpA,
        0x2C => HostMethod::MemOpB,
        0x30 => HostMethod::MemOpC,
        0x34 => HostMethod::MemOpD,
        0x50 => HostMethod::SetReference,
        0x5C => HostMethod::SemAddrLo,
        0x60 => HostMethod::SemAddrHi,
        0x64 => HostMethod::SemPayloadLo,
        0x68 => HostMethod::SemPayloadHi,
        0x6C => HostMethod::SemExecute,
        0x78 => HostMethod::Wfi,
        0x80 => HostMethod::Yield,
        _ => return None,
    })
}

/// `SET_REMAP_COMPONENTS`'s named fields (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:181-228`):
/// `DST_X/Y/Z/W` 2:0, 6:4, 10:8, 14:12; `COMPONENT_SIZE` 17:16; `NUM_SRC_COMPONENTS` 21:20;
/// `NUM_DST_COMPONENTS` 25:24. A word with any other bit set is refused, never re-emitted.
pub const REMAP_NAMED: u32 =
    0x7 | (0x7 << 4) | (0x7 << 8) | (0x7 << 12) | (0x3 << 16) | (0x3 << 20) | (0x3 << 24);

/// ★ A `LAUNCH_DMA` word, decoded and checked against the tier's field table (§3.2). Every field
/// stock sets is named; a non-zero field the table does not name is REFUSED, never cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Launch {
    /// `DATA_TRANSFER_TYPE` (1:0): 0 NONE, 1 PIPELINED, 2 NON_PIPELINED.
    pub transfer: u32,
    /// `FLUSH_ENABLE` (2).
    pub flush: bool,
    /// `FLUSH_TYPE` (25): GL.
    pub flush_gl: bool,
    /// `SEMAPHORE_TYPE` (4:3): 0, 1 (one-word / no timestamp) or 2 (four-word / with timestamp).
    pub sema: u32,
    /// `INTERRUPT_TYPE` (6:5): 0 NONE or 2 NON_BLOCKING.
    pub intr: u32,
    /// `SRC_MEMORY_LAYOUT` (7) = PITCH.
    pub src_pitch: bool,
    /// `DST_MEMORY_LAYOUT` (8) = PITCH.
    pub dst_pitch: bool,
    /// `MULTI_LINE_ENABLE` (9).
    pub multi_line: bool,
    /// `REMAP_ENABLE` (10).
    pub remap: bool,
    /// `SRC_TYPE` (12) as the guest wrote it (PHYSICAL) — never emitted.
    pub src_phys: bool,
    /// `DST_TYPE` (13) as the guest wrote it.
    pub dst_phys: bool,
    /// `SEMAPHORE_REDUCTION` (17:14).
    pub reduction: u32,
    /// `SEMAPHORE_REDUCTION_SIGN` (18).
    pub reduction_unsigned: bool,
    /// `SEMAPHORE_REDUCTION_ENABLE` (19).
    pub reduction_enable: bool,
    /// `DISABLE_PLC` (26).
    pub disable_plc: bool,
    /// `SEMAPHORE_PAYLOAD_SIZE` (27, `C7B5`+): TWO_WORD.
    pub payload_two_word: bool,
    /// `MEMORY_SCRUB_ENABLE` (23, `C8B5`+).
    pub scrub: bool,
}

/// The fields of `LAUNCH_DMA` every tier names: `(lo, width)`.
const F_TRANSFER: (u32, u32) = (0, 2);
const F_FLUSH: u32 = 2;
const F_SEMA: (u32, u32) = (3, 2);
const F_INTR: (u32, u32) = (5, 2);
const F_SRC_PITCH: u32 = 7;
const F_DST_PITCH: u32 = 8;
const F_MULTI_LINE: u32 = 9;
const F_REMAP: u32 = 10;
const F_SRC_TYPE: u32 = 12;
const F_DST_TYPE: u32 = 13;
const F_REDUCTION: (u32, u32) = (14, 4);
const F_REDUCTION_SIGN: u32 = 18;
const F_REDUCTION_ENABLE: u32 = 19;
const F_SCRUB: u32 = 23;
const F_FLUSH_TYPE: u32 = 25;
const F_DISABLE_PLC: u32 = 26;
const F_PAYLOAD_SIZE: u32 = 27;

const fn field(w: u32, (lo, width): (u32, u32)) -> u32 {
    (w >> lo) & ((1 << width) - 1)
}
const fn bit(w: u32, b: u32) -> bool {
    (w >> b) & 1 != 0
}

/// The bits a tier's field table names. Anything else set is refused
/// ([`Refusal::LaunchField`]): on `C5B5`/`C6B5`/`C7B5` that is `FORCE_RMWDISABLE` (11), `C5B5`'s
/// `SRC/DST_BYPASS_L2` (20/21), `VPRMODE` (23:22), the reserved bits 24 and 31:28, and — before
/// `C7B5` — bit 27; on `C8B5`+ `COPY_TYPE` (21:20) when not DEFAULT.
#[must_use]
pub const fn named_bits(tier: Tier) -> u32 {
    let common = 0x3 // transfer
        | (1 << F_FLUSH)
        | (0x3 << F_SEMA.0)
        | (0x3 << F_INTR.0)
        | (1 << F_SRC_PITCH)
        | (1 << F_DST_PITCH)
        | (1 << F_MULTI_LINE)
        | (1 << F_REMAP)
        | (1 << F_SRC_TYPE)
        | (1 << F_DST_TYPE)
        | (0xF << F_REDUCTION.0)
        | (1 << F_REDUCTION_SIGN)
        | (1 << F_REDUCTION_ENABLE)
        | (1 << F_FLUSH_TYPE)
        | (1 << F_DISABLE_PLC);
    match tier {
        Tier::C5b5 | Tier::C6b5 => common,
        Tier::C7b5 => common | (1 << F_PAYLOAD_SIZE),
        Tier::C8b5 | Tier::C9b5 | Tier::Cab5 => common | (1 << F_PAYLOAD_SIZE) | (1 << F_SCRUB),
    }
}

/// ★ Decode `w` against `tier`'s field table.
///
/// # Errors
/// [`Refusal::LaunchField`] for a bit the table does not name, or a named field holding a value
/// the table refuses: `SEMAPHORE_TYPE == 3` (conditional interrupt), `INTERRUPT_TYPE` BLOCKING (no
/// stock emitter) or 3, `DATA_TRANSFER_TYPE` 3 (`CAB5` `PREFETCH`, gated on the census).
pub fn decode_launch(tier: Tier, w: u32) -> Result<Launch, Refusal> {
    let unnamed = w & !named_bits(tier);
    if unnamed != 0 {
        return Err(Refusal::LaunchField {
            word: w,
            what: "a bit this tier's field table does not name",
        });
    }
    let l = Launch {
        transfer: field(w, F_TRANSFER),
        flush: bit(w, F_FLUSH),
        flush_gl: bit(w, F_FLUSH_TYPE),
        sema: field(w, F_SEMA),
        intr: field(w, F_INTR),
        src_pitch: bit(w, F_SRC_PITCH),
        dst_pitch: bit(w, F_DST_PITCH),
        multi_line: bit(w, F_MULTI_LINE),
        remap: bit(w, F_REMAP),
        src_phys: bit(w, F_SRC_TYPE),
        dst_phys: bit(w, F_DST_TYPE),
        reduction: field(w, F_REDUCTION),
        reduction_unsigned: bit(w, F_REDUCTION_SIGN),
        reduction_enable: bit(w, F_REDUCTION_ENABLE),
        disable_plc: bit(w, F_DISABLE_PLC),
        payload_two_word: bit(w, F_PAYLOAD_SIZE),
        scrub: bit(w, F_SCRUB),
    };
    let bad = if l.transfer == 3 {
        Some("DATA_TRANSFER_TYPE 3 (PREFETCH, census-gated)")
    } else if l.sema == 3 {
        Some("SEMAPHORE_TYPE 3 (conditional interrupt)")
    } else if l.intr == 1 {
        Some("INTERRUPT_TYPE BLOCKING")
    } else if l.intr == 3 {
        Some("INTERRUPT_TYPE 3")
    } else {
        None
    };
    match bad {
        Some(what) => Err(Refusal::LaunchField { word: w, what }),
        None => Ok(l),
    }
}

/// ★ The `LAUNCH_DMA` word T-mode emits for `l`: every field from the decoded value, `SRC_TYPE` and
/// `DST_TYPE` VIRTUAL, `MEMORY_SCRUB_ENABLE` clear (a scrub is converted to a fill), every bit the
/// table does not name zero. Only [`crate::tspace_unsafe`] emits it.
#[must_use]
pub const fn encode_launch(l: &Launch) -> u32 {
    (l.transfer & 0x3)
        | ((l.flush as u32) << F_FLUSH)
        | ((l.sema & 0x3) << F_SEMA.0)
        | ((l.intr & 0x3) << F_INTR.0)
        | ((l.src_pitch as u32) << F_SRC_PITCH)
        | ((l.dst_pitch as u32) << F_DST_PITCH)
        | ((l.multi_line as u32) << F_MULTI_LINE)
        | ((l.remap as u32) << F_REMAP)
        | ((l.reduction & 0xF) << F_REDUCTION.0)
        | ((l.reduction_unsigned as u32) << F_REDUCTION_SIGN)
        | ((l.reduction_enable as u32) << F_REDUCTION_ENABLE)
        | ((l.flush_gl as u32) << F_FLUSH_TYPE)
        | ((l.disable_plc as u32) << F_DISABLE_PLC)
        | ((l.payload_two_word as u32) << F_PAYLOAD_SIZE)
}

/// Host `SEMAPHORED` (`clc56f.h:83-107`), decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemaphoreD {
    /// `OPERATION` (4:0): ACQUIRE 1, RELEASE 2, ACQ_GEQ 4, ACQ_AND 8, REDUCTION 0x10.
    pub op: u32,
    /// `ACQUIRE_SWITCH` (12).
    pub acquire_switch: bool,
    /// `RELEASE_WFI` (20) = DIS.
    pub release_wfi_dis: bool,
    /// `RELEASE_SIZE` (24) = 4BYTE.
    pub size_4byte: bool,
    /// `REDUCTION` (30:27).
    pub reduction: u32,
    /// `FORMAT` (31) = UNSIGNED.
    pub unsigned: bool,
}

/// The bits `SEMAPHORED` names.
const SEMD_NAMED: u32 = 0x1F | (1 << 12) | (1 << 20) | (1 << 24) | (0xF << 27) | (1 << 31);

impl SemaphoreD {
    /// Decode, refusing an unnamed bit or an operation the class does not define.
    ///
    /// # Errors
    /// [`Refusal::HostSemOp`].
    pub fn decode(w: u32) -> Result<SemaphoreD, Refusal> {
        let op = w & 0x1F;
        if w & !SEMD_NAMED != 0 || !matches!(op, 1 | 2 | 4 | 8 | 0x10) {
            return Err(Refusal::HostSemOp {
                method: 0x1C,
                word: w,
            });
        }
        Ok(SemaphoreD {
            op,
            acquire_switch: bit(w, 12),
            release_wfi_dis: bit(w, 20),
            size_4byte: bit(w, 24),
            reduction: (w >> 27) & 0xF,
            unsigned: bit(w, 31),
        })
    }

    /// The word T-mode emits — authored from the decoded fields.
    #[must_use]
    pub const fn encode(&self) -> u32 {
        self.op
            | ((self.acquire_switch as u32) << 12)
            | ((self.release_wfi_dis as u32) << 20)
            | ((self.size_4byte as u32) << 24)
            | ((self.reduction & 0xF) << 27)
            | ((self.unsigned as u32) << 31)
    }

    /// The operation writes the semaphore (a release or a reduction).
    #[must_use]
    pub const fn writes(&self) -> bool {
        matches!(self.op, 2 | 0x10)
    }

    /// The bytes it touches: a 16-byte release (payload + timestamp) unless `RELEASE_SIZE` 4BYTE;
    /// an acquire or a reduction reads/updates 4 bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        if self.op == 2 && !self.size_4byte {
            16
        } else {
            4
        }
    }
}

/// Host `SEM_EXECUTE` (`clc56f.h:214-244`, the same fields in `clc86f.h`), decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemExecute {
    /// `OPERATION` (2:0): ACQUIRE 0, RELEASE 1, ACQ_STRICT_GEQ 2, ACQ_CIRC_GEQ 3, ACQ_AND 4,
    /// ACQ_NOR 5, REDUCTION 6.
    pub op: u32,
    /// `ACQUIRE_SWITCH_TSG` (12).
    pub switch_tsg: bool,
    /// `RELEASE_WFI` (20).
    pub release_wfi: bool,
    /// `PAYLOAD_SIZE` (24) = 64BIT.
    pub payload_64: bool,
    /// `RELEASE_TIMESTAMP` (25).
    pub timestamp: bool,
    /// `REDUCTION` (30:27).
    pub reduction: u32,
    /// `REDUCTION_FORMAT` (31).
    pub unsigned: bool,
}

const SEMX_NAMED: u32 =
    0x7 | (1 << 12) | (1 << 20) | (1 << 24) | (1 << 25) | (0xF << 27) | (1 << 31);

impl SemExecute {
    /// Decode, refusing an unnamed bit or operation 7.
    ///
    /// # Errors
    /// [`Refusal::HostSemOp`].
    pub fn decode(w: u32) -> Result<SemExecute, Refusal> {
        let op = w & 0x7;
        if w & !SEMX_NAMED != 0 || op == 7 {
            return Err(Refusal::HostSemOp {
                method: 0x6C,
                word: w,
            });
        }
        Ok(SemExecute {
            op,
            switch_tsg: bit(w, 12),
            release_wfi: bit(w, 20),
            payload_64: bit(w, 24),
            timestamp: bit(w, 25),
            reduction: (w >> 27) & 0xF,
            unsigned: bit(w, 31),
        })
    }

    /// The word T-mode emits — authored from the decoded fields.
    #[must_use]
    pub const fn encode(&self) -> u32 {
        self.op
            | ((self.switch_tsg as u32) << 12)
            | ((self.release_wfi as u32) << 20)
            | ((self.payload_64 as u32) << 24)
            | ((self.timestamp as u32) << 25)
            | ((self.reduction & 0xF) << 27)
            | ((self.unsigned as u32) << 31)
    }

    /// A release or a reduction.
    #[must_use]
    pub const fn writes(&self) -> bool {
        matches!(self.op, 1 | 6)
    }

    /// The bytes it touches: 16 for a timestamped release, else 8 with a 64-bit payload, else 4.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        if self.op == 1 && self.timestamp {
            16
        } else if self.payload_64 {
            8
        } else {
            4
        }
    }
}

#[cfg(test)]
mod hwref_check {
    use super::*;
    use kf_chip::hwref::expect::{class_range, class_val};

    fn range_bits(name: &str) -> u32 {
        let (hi, lo) = class_range(name);
        (((1u64 << (hi - lo + 1)) - 1) << lo) as u32
    }

    /// ★ The rows the tables NAME are the class headers' own — for every row a header states. The
    /// rows a trimmed header omits are inherited by construction (one match arm for every tier).
    #[test]
    fn the_tables_are_the_class_headers() {
        for (m, name, want) in [
            (0x100, "NVC7B5_NOP", CeMethod::Nop),
            (0x240, "NVC8B5_SET_SEMAPHORE_A", CeMethod::SemA),
            (0x244, "NVC8B5_SET_SEMAPHORE_B", CeMethod::SemB),
            (0x248, "NVC8B5_SET_SEMAPHORE_PAYLOAD", CeMethod::SemPayload),
            (0x264, "NVC8B5_SET_DST_PHYS_MODE", CeMethod::DstPhysMode),
            (0x300, "NVC8B5_LAUNCH_DMA", CeMethod::Launch),
            (0x408, "NVC8B5_OFFSET_OUT_UPPER", CeMethod::OffsetOutUpper),
            (0x418, "NVC8B5_LINE_LENGTH_IN", CeMethod::LineLength),
            (0x41C, "NVC7B5_LINE_COUNT", CeMethod::LineCount),
            (0x410, "NVC7B5_PITCH_IN", CeMethod::PitchIn),
            (0x700, "NVC8B5_SET_REMAP_CONST_A", CeMethod::RemapConstA),
            (
                0x708,
                "NVC8B5_SET_REMAP_COMPONENTS",
                CeMethod::RemapComponents,
            ),
            (
                0x6FC,
                "NVC8B5_SET_MEMORY_SCRUB_PARAMETERS",
                CeMethod::ScrubParameters,
            ),
            (
                0x70C,
                "NVC7B5_SET_DST_BLOCK_SIZE",
                CeMethod::BlockLinearState,
            ),
            (0x750, "NVC7B5_DST_ORIGIN_Y", CeMethod::BlockLinearState),
        ] {
            assert_eq!(u64::from(m), class_val(name), "{name}");
            assert_eq!(ce_method(Tier::C8b5, m), Some(want), "{name}");
        }
        assert_eq!(class_val("NVCAB5_REQ_ATTR"), 0x754);
        assert_eq!(ce_method(Tier::Cab5, 0x754), Some(CeMethod::ReqAttr));
        assert_eq!(ce_method(Tier::C8b5, 0x754), None, "REQ_ATTR is CAB5's");
        assert_eq!(class_val("NVC7B5_SET_SEMAPHORE_PAYLOAD_UPPER"), 0x24C);
        assert_eq!(
            ce_method(Tier::C6b5, 0x24C),
            None,
            "C6B5 has no payload upper"
        );
        for (m, name) in [
            (0x00, "NVC56F_SET_OBJECT"),
            (0x08, "NVC56F_NOP"),
            (0x10, "NVC56F_SEMAPHOREA"),
            (0x1C, "NVC56F_SEMAPHORED"),
            (0x20, "NVC56F_NON_STALL_INTERRUPT"),
            (0x24, "NVC56F_FB_FLUSH"),
            (0x50, "NVC56F_SET_REFERENCE"),
            (0x5C, "NVC86F_SEM_ADDR_LO"),
            (0x60, "NVC86F_SEM_ADDR_HI"),
            (0x6C, "NVC86F_SEM_EXECUTE"),
            (0x78, "NVC86F_WFI"),
            (0x80, "NVC56F_YIELD"),
        ] {
            assert_eq!(u64::from(m), class_val(name), "{name}");
            assert!(host_method(m).is_some(), "{name}");
        }
        // The legacy semaphore is 40-bit on every family; SEM_ADDR_HI widens at Hopper.
        assert_eq!(class_range("NVC56F_SEMAPHOREA_OFFSET_UPPER"), (7, 0));
        assert_eq!(class_range("NVC56F_SEM_ADDR_HI_OFFSET"), (7, 0));
        assert_eq!(class_range("NVC86F_SEM_ADDR_HI_OFFSET"), (24, 0));
        assert!(!Tier::C7b5.wide_sem_addr() && Tier::C8b5.wide_sem_addr());
    }

    /// ★ Review fix 2026-10-04: `REMAP_NAMED` is exactly the union of `SET_REMAP_COMPONENTS`'s
    /// fields in the class header.
    #[test]
    fn the_remap_fields_are_the_class_header() {
        let want = [
            "DST_X",
            "DST_Y",
            "DST_Z",
            "DST_W",
            "COMPONENT_SIZE",
            "NUM_SRC_COMPONENTS",
            "NUM_DST_COMPONENTS",
        ]
        .iter()
        .map(|f| range_bits(&format!("NVC7B5_SET_REMAP_COMPONENTS_{f}")))
        .fold(0, |a, b| a | b);
        assert_eq!(REMAP_NAMED, want);
    }

    /// ★ Every `LAUNCH_DMA` field the table names is the header's own range, on the class that
    /// defines it; the bits it refuses are exactly what is left.
    #[test]
    fn the_launch_field_table_is_the_class_headers() {
        let c7 = [
            "DATA_TRANSFER_TYPE",
            "FLUSH_ENABLE",
            "SEMAPHORE_TYPE",
            "INTERRUPT_TYPE",
            "SRC_MEMORY_LAYOUT",
            "DST_MEMORY_LAYOUT",
            "MULTI_LINE_ENABLE",
            "REMAP_ENABLE",
            "SRC_TYPE",
            "DST_TYPE",
            "SEMAPHORE_REDUCTION",
            "SEMAPHORE_REDUCTION_SIGN",
            "SEMAPHORE_REDUCTION_ENABLE",
            "FLUSH_TYPE",
            "DISABLE_PLC",
            "SEMAPHORE_PAYLOAD_SIZE",
        ];
        let want: u32 = c7
            .iter()
            .map(|f| range_bits(&format!("NVC7B5_LAUNCH_DMA_{f}")))
            .fold(0, |a, b| a | b);
        assert_eq!(named_bits(Tier::C7b5), want);
        assert_eq!(
            named_bits(Tier::C8b5),
            want | range_bits("NVC8B5_LAUNCH_DMA_MEMORY_SCRUB_ENABLE")
        );
        assert_eq!(
            named_bits(Tier::C5b5),
            want & !range_bits("NVC7B5_LAUNCH_DMA_SEMAPHORE_PAYLOAD_SIZE")
        );
        // What the table leaves out is refused: VPRMODE, FORCE_RMWDISABLE, COPY_TYPE, reserved.
        for (tier, name) in [
            (Tier::C7b5, "NVC7B5_LAUNCH_DMA_VPRMODE"),
            (Tier::C7b5, "NVC7B5_LAUNCH_DMA_FORCE_RMWDISABLE"),
            (Tier::C7b5, "NVC7B5_LAUNCH_DMA_RESERVED_START_OF_COPY"),
            (Tier::C7b5, "NVC7B5_LAUNCH_DMA_RESERVED_ERR_CODE"),
            (Tier::C8b5, "NVC8B5_LAUNCH_DMA_COPY_TYPE"),
            (Tier::C5b5, "NVC5B5_LAUNCH_DMA_SRC_BYPASS_L2"),
            (Tier::C5b5, "NVC5B5_LAUNCH_DMA_DST_BYPASS_L2"),
        ] {
            assert_eq!(named_bits(tier) & range_bits(name), 0, "{name}");
        }
    }
}
