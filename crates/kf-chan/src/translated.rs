//! ★★★★★ **The Translated channel's rewriter** — `THE_TRANSLATED_PLANE.md` §24.2.
//!
//! A kernel CE channel (RM's CeUtils scrub, UVM) runs on **our own** host channel, in a host VA
//! space that mirrors the guest's kernel space. Its pushbuffer is copied, not decoded into
//! intents, and exactly two things are rewritten:
//!
//! 1. **A CE `LAUNCH_DMA` with a PHYSICAL side** — the operand becomes a virtual address in the
//!    identity window (local FB) or the guest-RAM window (sysmem), the type bit flips to
//!    VIRTUAL, and the guest's own offsets are restored right after the launch so a later
//!    launch that reuses them still reads the guest's values. This is RM's own `fbAliasVA`
//!    rewrite (`ogkm-610 channel_utils.c:1055-1056`).
//! 2. **`MEM_OP_A..D`** — a `MEM_OP_D` TLB invalidate is a privileged host method naming a guest
//!    PDB address: it is DROPPED from the forwarded stream and becomes a [`Piece::Invalidate`]
//!    split point, where the worker walks the named root and reconciles before the rest runs.
//!    ★ P6b ruling (d): a `MEM_OP_D` **MEMBAR** is FORWARDED (A, B, C, D as the guest wrote them):
//!    it is an ordering method, not a privileged one — RM names the privileged host methods as
//!    exactly `TLB_INVALIDATE` and `ACCESS_COUNTER_CLR` (`ogkm-580 alloc_channel.h:207-214`,
//!    `NVOS04_FLAGS_CHANNEL_DENY_AUTH_LEVEL_PRIV`), so our unprivileged host channel may execute
//!    it, and dropping it would let the guest's later work (a semaphore release UVM orders behind
//!    it, `uvm_pascal_host.c:32-45` / `uvm_hal.c:938`) overtake writes it ordered. An invalidate
//!    that asked for a `SYSMEMBAR` (`MEM_OP_A` 11:11) keeps it the same way: a forwarded MEMBAR
//!    (`SYS_MEMBAR`) ahead of the split. `L2_*` maintenance is unprivileged and forwarded as
//!    written; `ACCESS_COUNTER_CLR` is served (no access counters exist on our device); anything
//!    else (Hopper's `MMU_OPERATION`, an unnamed operation) is refused by name — never consumed.
//!
//! Everything else — semaphores, virtual operands, host methods — is forwarded unchanged, except
//! what is refused by name WHEN [`CeState::strict`]: ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md`
//! §3.2, §3.7) — the methods of [`REFUSED_METHODS`] on every tier, and every `SubDeviceMask`
//! header. ⊘ Strict is OFF on the default path until box step 1 (review fix 2026-10-04): there each
//! is COUNTED ([`IncACounts`]) and handled exactly as before inc A. ⚠ S1-23 stays open until the
//! T-mode rewriter ([`crate::tmode`]) authors every word (§3): until then a guest-kernel virtual
//! operand or semaphore reaches the engine as written.
//!
//! ⊘ Pure: no GPU, no isolate, no table. The output is NORMALISED to one method per header,
//! which is what makes insertion before a launch trivial; it is equivalent method-for-method.

use crate::census::{Census, OpKind, SubKind};
use kf_abi::submit::{MethodForm, ce, method_header_decode, method_header_inc};

/// `NVC56F_MEM_OP_A..D` — `ogkm-580 clc56f.h:133-181`.
const MEM_OP_A: u32 = 0x28;
const MEM_OP_B: u32 = 0x2c;
const MEM_OP_C: u32 = 0x30;
const MEM_OP_D: u32 = 0x34;
/// `NVC56F_MEM_OP_D_OPERATION` 31:27 — `MEMBAR` (`clc56f.h:184`; the same value in every
/// family's channel class: `clc46f.h:179`, `clc86f.h:118`).
const OP_MEMBAR: u32 = 5;
/// `NVC56F_MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR` 11:11 (`clc56f.h`).
const MEM_OP_A_SYSMEMBAR_EN: u32 = 1 << 11;
/// `NVC56F_MEM_OP_C_MEMBAR_TYPE_SYS_MEMBAR` (2:0 = 0).
const MEMBAR_TYPE_SYS: u32 = 0;
/// `NVC56F_MEM_OP_D_OPERATION` 31:27 — `MMU_TLB_INVALIDATE` / `_TARGETED`.
const OP_TLB_INVALIDATE: u32 = 9;
const OP_TLB_INVALIDATE_TARGETED: u32 = 0xa;
/// `MEM_OP_D_OPERATION` L2 maintenance — `L2_PEERMEM_INVALIDATE` 0xd, `L2_SYSMEM_INVALIDATE` 0xe,
/// `L2_CLEAN_COMPTAGS` 0xf, `L2_FLUSH_DIRTY` 0x10, `L2_SYSMEM_NCOH_INVALIDATE` 0x11 (Blackwell,
/// `clc96f.h:73`), `L2_WAIT_FOR_SYS_PENDING_READS` 0x15 (`clc56f.h:187-193`, `clc86f.h:122-128`).
/// ⊘ Not privileged: `alloc_channel.h:207-214` names ONLY `TLB_INVALIDATE` and
/// `ACCESS_COUNTER_CLR` as privileged host methods ⇒ forwarded as the guest wrote them.
const OPS_L2: [u32; 6] = [0xd, 0xe, 0xf, 0x10, 0x11, 0x15];
/// `MEM_OP_D_OPERATION_ACCESS_COUNTER_CLR` 0x16 — privileged; our device exposes no access
/// counters (the notify buffer is advertised and never written), so there is nothing to clear.
const OP_ACCESS_COUNTER_CLR: u32 = 0x16;
/// `NVC7B5_LINE_COUNT` — `clc7b5.h`.
const LINE_COUNT: u32 = 0x41c;
/// `NVC7B5_OFFSET_IN_LOWER` / `OUT_LOWER`.
const OFFSET_IN_LOWER: u32 = 0x404;
const OFFSET_OUT_LOWER: u32 = 0x40c;
/// `NVC7B5_PITCH_IN` / `PITCH_OUT` — `clc7b5.h:169-172`.
const PITCH_IN: u32 = 0x410;
const PITCH_OUT: u32 = 0x414;
/// `NVC7B5_SET_REMAP_COMPONENTS_NUM_SRC_COMPONENTS` 21:20, size-minus-one (`clc7b5.h:219-223`).
const REMAP_NUM_SRC_SHIFT: u32 = 20;

/// `GP100_UVM_SW` — the software class UVM binds on a subchannel of its kernel channels
/// (`ogkm-580: clc076.h:33`, `uvm_pascal_host.c:317`). No engine executes it: on bare metal its
/// methods trap to RM. ⇒ It never reaches our host channel.
pub const GP100_UVM_SW: u32 = 0xc076;
/// `NVC076_NO_OPERATION` (`clc076.h:36`).
const SW_NO_OPERATION: u32 = 0x100;

/// `NVC7B5_SET_*_PHYS_MODE_TARGET` — `clc7b5.h:67-71`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `LOCAL_FB` (0) — a GPGA offset.
    LocalFb,
    /// `COHERENT_SYSMEM` (1) — a guest-physical address.
    CoherentSysmem,
    /// `NONCOHERENT_SYSMEM` (2).
    NonCoherentSysmem,
    /// `PEERMEM` (3) — meaningless for a single-GPU guest; refused.
    Peer,
}

impl Target {
    const fn from_bits(v: u32) -> Self {
        match v & ce::PHYS_MODE_TARGET_MASK {
            0 => Self::LocalFb,
            1 => Self::CoherentSysmem,
            2 => Self::NonCoherentSysmem,
            _ => Self::Peer,
        }
    }
}

/// Where a physical operand lives in the host VA space. Implemented by the worker from the two
/// window bases and the VMM's guest-RAM layout.
pub trait Window {
    /// The host VA of `[phys, phys+len)` in `target`, or `None` if it is not one contiguous
    /// range of a window (refused by name by the caller).
    fn translate(&self, target: Target, phys: u64, len: u64) -> Option<u64>;
}

/// The CE engine state a channel builds up across doorbells — the registers the rewriter needs
/// to see to translate a launch. One per channel, kept by the caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CeState {
    /// Subchannels a `SET_OBJECT` bound to a CE class, as a bit mask. ⊘ Recorded, NOT the routing
    /// rule: every hardware subchannel of a CE channel reaches the CE (see `one_write`).
    pub ce_subch: u8,
    /// Subchannels bound to `GP100_UVM_SW`, as a bit mask — consumed here, never forwarded.
    pub sw_subch: u8,
    /// `OFFSET_IN` as the guest wrote it.
    pub off_in: u64,
    /// `OFFSET_OUT` as the guest wrote it.
    pub off_out: u64,
    /// `LINE_LENGTH_IN`.
    pub line_len: u32,
    /// `LINE_COUNT`.
    pub line_count: u32,
    /// `SET_SRC_PHYS_MODE` target bits.
    pub src_mode: u32,
    /// `SET_DST_PHYS_MODE` target bits.
    pub dst_mode: u32,
    /// `PITCH_IN` — the source line stride in bytes when multi-line.
    pub pitch_in: u32,
    /// `PITCH_OUT` — the destination line stride in bytes when multi-line.
    pub pitch_out: u32,
    /// `SET_REMAP_COMPONENTS` — with `REMAP_ENABLE`, `LINE_LENGTH_IN` counts ELEMENTS of
    /// `component_size × num_components` bytes, not bytes.
    pub remap: u32,
    /// `MEM_OP_C` as last written — the low PDB bits and `PDB_ALL` of a following `MEM_OP_D`.
    pub mem_op_c: u32,
    /// ★ P6b: `MEM_OP_A` as last written (a forwarded MEMBAR re-emits it; a SYSMEMBAR request).
    pub mem_op_a: u32,
    /// ★ P6b: `MEM_OP_B` as last written.
    pub mem_op_b: u32,
    /// ★ 2026-09-26: the CE class last bound by `SET_OBJECT` — `LAUNCH_DMA` bit 23 is
    /// `MEMORY_SCRUB_ENABLE` only from `HOPPER_DMA_COPY_A` (`0xC8B5`) on (on `NVC7B5` it is `VPRMODE`).
    pub ce_class: u32,
    /// ★ 2026-09-26: `SET_REMAP_CONST_A` as the guest wrote it (restored after a converted scrub).
    pub const_a: u32,
    /// ★ v3-initrace (diagnostic record only): the semaphore state the guest's methods set, and
    /// the releases its launches asked for — see [`Releases`]. Nothing here changes a word the
    /// rewriter emits.
    pub sema: SemaRegs,
    /// The releases recorded since the ring last took them ([`crate::ring::TranslatedRing`]).
    pub releases: Releases,
    /// ★ v3-initrace: the data-moving launches since the last take, as the guest wrote them
    /// (before our rewrite) — the completion probe reads their bytes back.
    pub launches: PhysLaunches,
    /// ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §8, review fix 2026-10-04): REFUSE what
    /// inc A refuses by name — a [`REFUSED_METHODS`] write, a `SubDeviceMask` header, a GP control
    /// entry other than `NOP` and a family's extended base. ⊘ `false` (the default path until box
    /// step 1 shows the counts at 0 on each measured family): each is COUNTED in [`CeState::inca`]
    /// and handled exactly as before inc A — forwarded, pushed raw, skipped. The device sets it from
    /// `KF3_INCA_REFUSE=1` or `KF3_TSPACE=1` (T-mode refuses them in its own decoder regardless).
    pub strict: bool,
    /// ★ P1+P2 inc A, the count-only arm: what [`CeState::strict`] would have refused.
    pub inca: IncACounts,
}

/// ★ P1+P2 inc A, count-only (`docs/design/V3_P1P2_TSPACE.md` §8): per channel, what the by-name
/// refusals WOULD have refused while [`CeState::strict`] is off — each one handled exactly as before
/// inc A. Printed on the channel's `RETIRED` line and summed on the status line (`inca[…]`); box
/// step 1 must show every field 0 on each measured family before strict becomes the default.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IncACounts {
    /// Writes of a [`REFUSED_METHODS`] method — forwarded as written, as before.
    pub refused_methods: u64,
    /// `SubDeviceMask` headers — pushed raw, as before.
    pub subdevice_masks: u64,
    /// GP control entries other than `NOP` (and, on a family that defines it,
    /// `SET_PB_SEGMENT_EXTENDED_BASE`) — skipped as a `NOP`, as before.
    pub control_entries: u64,
    /// Segment entries a non-zero `SET_PB_SEGMENT_EXTENDED_BASE` would have rebased — read at
    /// address bits 39:0, as before.
    pub ext_base_unapplied: u64,
}

impl IncACounts {
    /// Every count, summed.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.refused_methods + self.subdevice_masks + self.control_entries + self.ext_base_unapplied
    }

    /// `refused_methods=… subdevice_masks=… control_entries=… ext_base_unapplied=…`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "refused_methods={} subdevice_masks={} control_entries={} ext_base_unapplied={}",
            self.refused_methods,
            self.subdevice_masks,
            self.control_entries,
            self.ext_base_unapplied
        )
    }
}

/// ★ v3-initrace (diagnostic record only): one side of a launch, as the guest named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// A PHYSICAL operand: `(target, address, bytes)`.
    Physical(Target, u64, u64),
    /// A VIRTUAL operand in the channel's own VA space: `(va, bytes)` — reached by the engine
    /// through the host space's placements (e.g. CeUtils' FB alias, `memmgrMemUtilsMapFbAlias`).
    Virtual(u64, u64),
}

/// ★ v3-initrace (diagnostic record only): one `LAUNCH_DMA` that moved data, as the guest wrote it
/// — each side's operand (`None`: that side is not read, e.g. a memset's source).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysLaunch {
    /// The `LAUNCH_DMA` word.
    pub launch: u32,
    /// The source, if read.
    pub src: Option<Operand>,
    /// The destination.
    pub dst: Option<Operand>,
}

/// The last [`PhysLaunches::CAP`] physical launches since the last take.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PhysLaunches {
    kept: [PhysLaunch; PhysLaunches::CAP],
    n: u8,
}

impl PhysLaunches {
    /// Launches kept between takes (older ones are dropped).
    pub const CAP: usize = 4;

    fn push(&mut self, l: PhysLaunch) {
        if usize::from(self.n) < Self::CAP {
            self.kept[usize::from(self.n)] = l;
            self.n += 1;
        } else {
            self.kept.copy_within(1.., 0);
            self.kept[Self::CAP - 1] = l;
        }
    }

    /// The launches recorded so far, oldest first; the record is emptied.
    pub fn take(&mut self) -> Vec<PhysLaunch> {
        let v = self.kept[..usize::from(self.n)].to_vec();
        self.n = 0;
        v
    }
}

/// `NV906F_SEMAPHOREA..D` / `NVC56F_SEMAPHOREA..D` — the legacy host semaphore
/// (`ogkm-580: cl906f.h:78-97`, `clc56f.h:76-83`; RM's CeUtils releases its PB-get index with it,
/// `channel_utils.c:737-750`).
const HOST_SEMAPHORE_A: u32 = 0x10;
const HOST_SEMAPHORE_B: u32 = 0x14;
const HOST_SEMAPHORE_C: u32 = 0x18;
const HOST_SEMAPHORE_D: u32 = 0x1c;
/// `NV906F_SEMAPHORED_OPERATION` 3:0 (`cl906f.h:85`; `clc56f.h` widens it to 4:0 with
/// `REDUCTION` = 0x10) — `_RELEASE` = 2 (`cl906f.h:87`).
const HOST_SEMAPHORE_D_OP_MASK: u32 = 0x1f;
const HOST_SEMAPHORE_D_OP_RELEASE: u32 = 2;

/// ★ v3-initrace: the semaphore registers a channel's methods last wrote (diagnostic record).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SemaRegs {
    /// CE `SET_SEMAPHORE_A` (address 48:32 / 56:32).
    pub ce_a: u32,
    /// CE `SET_SEMAPHORE_B` (address 31:0).
    pub ce_b: u32,
    /// CE `SET_SEMAPHORE_PAYLOAD`.
    pub ce_payload: u32,
    /// Host `SEMAPHOREA` — address 39:32.
    pub host_a: u32,
    /// Host `SEMAPHOREB` — address 31:2.
    pub host_b: u32,
    /// Host `SEMAPHOREC` — the payload.
    pub host_c: u32,
    /// Host `SEM_ADDR_LO` — address 31:2.
    pub sem_lo: u32,
    /// Host `SEM_ADDR_HI` — address 39:32.
    pub sem_hi: u32,
    /// Host `SEM_PAYLOAD_LO`.
    pub sem_payload: u32,
}

/// Which method released a semaphore.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseKind {
    /// A copy-engine `LAUNCH_DMA` with `SEMAPHORE_TYPE` one-word (`clc7b5.h:99`).
    #[default]
    CeOneWord,
    /// A copy-engine release with `SEMAPHORE_REDUCTION_ENABLE` (`clc7b5.h:147`, bit 19): the word
    /// becomes `op(old, payload)` (UVM's tracking semaphores INC with the payload as the wrap
    /// value), so the payload is NOT the value written — never judged against it.
    CeReduction,
    /// … four-word (with timestamp; the payload is the first word).
    CeFourWord,
    /// … conditional-interrupt semaphore.
    CeConditionalIntr,
    /// A host `SEMAPHORED` `RELEASE`.
    HostSemaphoreD,
    /// A host `SEM_EXECUTE` `RELEASE`.
    HostSemExecute,
}

/// ★ v3-initrace: one semaphore RELEASE a guest segment asked the engine for — the virtual
/// address in the channel's space and the 32-bit payload. Recorded as the rewriter forwards the
/// words unchanged; the completion probe (`KF3_COMPLETION_PROBE`) reads the word there after the
/// engine completed, through OUR placements, to say where the guest's completion landed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    /// The semaphore's virtual address in the channel's VA space.
    pub va: u64,
    /// The low 32 bits the engine writes.
    pub payload: u32,
    /// The method that asked.
    pub kind: ReleaseKind,
}

/// The last [`Releases::CAP`] releases since the last take (older ones are counted, not kept).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Releases {
    kept: [Release; Releases::CAP],
    n: u8,
    /// Releases dropped because more than [`Releases::CAP`] arrived between takes.
    pub dropped: u32,
}

impl Releases {
    /// Releases kept between takes.
    pub const CAP: usize = 4;

    fn push(&mut self, r: Release) {
        if usize::from(self.n) < Self::CAP {
            self.kept[usize::from(self.n)] = r;
            self.n += 1;
        } else {
            self.kept.copy_within(1.., 0);
            self.kept[Self::CAP - 1] = r;
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// The releases recorded so far, oldest first; the record is emptied.
    pub fn take(&mut self) -> Vec<Release> {
        let v = self.kept[..usize::from(self.n)].to_vec();
        self.n = 0;
        v
    }
}

/// `NVC7B5_LAUNCH_DMA_SEMAPHORE_REDUCTION_ENABLE` 19:19 (`ogkm-580: clc7b5.h:147`).
const LAUNCH_SEMAPHORE_REDUCTION_ENABLE: u32 = 1 << 19;

/// Record the semaphore methods `(sub, m, v)` sets — host methods below `0x100` on any
/// subchannel, CE methods on a hardware subchannel. Pure bookkeeping.
fn note_semaphore(st: &mut CeState, sub: u32, m: u32, v: u32) {
    let upper = upper_mask(st);
    let s = &mut st.sema;
    match m {
        HOST_SEMAPHORE_A => s.host_a = v,
        HOST_SEMAPHORE_B => s.host_b = v,
        HOST_SEMAPHORE_C => s.host_c = v,
        HOST_SEMAPHORE_D if v & HOST_SEMAPHORE_D_OP_MASK == HOST_SEMAPHORE_D_OP_RELEASE => {
            let va = (u64::from(s.host_a & 0xFF) << 32) | u64::from(s.host_b & !3);
            let r = Release {
                va,
                payload: s.host_c,
                kind: ReleaseKind::HostSemaphoreD,
            };
            st.releases.push(r);
        }
        kf_abi::submit::fifo::SEM_ADDR_LO => s.sem_lo = v,
        kf_abi::submit::fifo::SEM_ADDR_HI => s.sem_hi = v,
        kf_abi::submit::fifo::SEM_PAYLOAD_LO => s.sem_payload = v,
        kf_abi::submit::fifo::SEM_EXECUTE
            if v & kf_abi::submit::fifo::SEM_EXECUTE_OPERATION_MASK
                == kf_abi::submit::fifo::SEM_EXECUTE_OPERATION_RELEASE =>
        {
            let va = (u64::from(s.sem_hi & 0xFF) << 32) | u64::from(s.sem_lo & !3);
            let r = Release {
                va,
                payload: s.sem_payload,
                kind: ReleaseKind::HostSemExecute,
            };
            st.releases.push(r);
        }
        _ if sub > 4 || m < 0x100 => {}
        ce::SET_SEMAPHORE_A => s.ce_a = v,
        ce::SET_SEMAPHORE_B => s.ce_b = v,
        ce::SET_SEMAPHORE_PAYLOAD => s.ce_payload = v,
        ce::LAUNCH_DMA => {
            let kind = match (v & ce::LAUNCH_SEMAPHORE_TYPE_MASK) >> 3 {
                0 => return,
                _ if v & LAUNCH_SEMAPHORE_REDUCTION_ENABLE != 0 => ReleaseKind::CeReduction,
                1 => ReleaseKind::CeOneWord,
                2 => ReleaseKind::CeFourWord,
                _ => ReleaseKind::CeConditionalIntr,
            };
            let va = (u64::from(s.ce_a & upper) << 32) | u64::from(s.ce_b);
            let r = Release {
                va,
                payload: s.ce_payload,
                kind,
            };
            st.releases.push(r);
        }
        _ => {}
    }
}

/// One piece of rewritten work, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Method words to submit on our host channel.
    Words(Vec<u32>),
    /// The guest invalidated a VA space here: walk and reconcile before what follows runs.
    /// `pdb == None` is `PDB_ALL`.
    Invalidate {
        /// The named root (guest FB/phys address), or `None` for all spaces.
        pdb: Option<u64>,
    },
}

/// Why a segment could not be forwarded. Refused by name, never guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A header this codec cannot decode.
    BadHeader { at: usize, word: u32 },
    /// A method's arguments run past the segment.
    Truncated { at: usize },
    /// A physical operand in `PEERMEM`.
    PeerOperand,
    /// A physical operand no window covers contiguously.
    Untranslatable { target: Target, phys: u64, len: u64 },
    /// A physical operand in BLOCK-LINEAR layout: its footprint is a function of the block
    /// geometry, which a kernel scrub never uses — refused rather than bounded by a guess.
    BlockLinearPhysical,
    /// A launch whose byte extent overflows 64 bits.
    ExtentOverflow,
    /// `SET_OBJECT` of a class that is neither a copy engine nor `GP100_UVM_SW`: our host channel
    /// has no such object, and forwarding it would fault OUR channel.
    ForeignClass {
        /// The subchannel.
        subch: u32,
        /// The class named.
        class: u32,
    },
    /// A `MEM_OP_D` operation this route neither forwards nor serves (Hopper's `MMU_OPERATION`, or
    /// one no class header names).
    MemOp {
        /// `MEM_OP_D_OPERATION` (31:27).
        op: u32,
    },
    /// ★ P6b: a method at or above `0x100` on a software subchannel (5-7) no `SET_OBJECT` bound —
    /// on bare metal a software-method trap to RM; nothing on our host channel may run it.
    UnboundSubchannel {
        /// The subchannel.
        subch: u32,
        /// The method.
        method: u32,
    },
    /// A `GP100_UVM_SW` method with work behind it (`FAULT_CANCEL_*`, `CLEAR_FAULTED_*`): the fault
    /// plane that would serve it does not exist yet — refused rather than silently dropped.
    SwMethod {
        /// The method.
        method: u32,
    },
    /// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.2): a method refused BY NAME on every tier — it
    /// carries an address no rewriter authors, arms engine state that later writes to one, or
    /// belongs to a feature kayfabe does not support ([`refused_method`]). No stock CeUtils or UVM
    /// path emits any of them on a channel kayfabe runs (each row cites its only emitter).
    RefusedMethod {
        /// The subchannel.
        subch: u32,
        /// The method.
        method: u32,
        /// Its class-header name.
        name: &'static str,
    },
    /// ★ P1+P2 inc A (§3.7): a `SET`/`STORE`/`USE_SUBDEVICE_MASK` header. A single-GPU guest has
    /// no use for one, and a mask can stop later methods from executing — separating an
    /// authored address write from the trigger it belongs to. Forwarded raw until 2026-10-04.
    SubDeviceMask {
        /// The word index of the header.
        at: usize,
    },
    /// ★ P1+P2 inc C (T-mode, §3.6): a method no row of the tier's table classifies.
    Unclassified {
        /// The subchannel.
        subch: u32,
        /// The method.
        method: u32,
    },
    /// ★ T-mode: a copy-engine method before any `SET_OBJECT` bound a CE class.
    NoCeObject {
        /// The subchannel.
        subch: u32,
        /// The method.
        method: u32,
    },
    /// ★ T-mode (§3.2): a `LAUNCH_DMA` bit the tier's field table does not name, or a named field
    /// with a value the table refuses — never silently cleared.
    LaunchField {
        /// The word.
        word: u32,
        /// What was refused.
        what: &'static str,
    },
    /// ★ T-mode: a host `SEMAPHORED`/`SEM_EXECUTE` word with an unnamed bit or an undefined
    /// operation.
    HostSemOp {
        /// The method.
        method: u32,
        /// The word.
        word: u32,
    },
    /// ★ T-mode: an address-free method or register whose word carries bits beyond its fields.
    FieldValue {
        /// The method.
        method: u32,
        /// The word.
        word: u32,
        /// What was refused.
        what: &'static str,
    },
    /// ★ T-mode (§3.4): a BLOCK-LINEAR virtual operand — its footprint is a function of the block
    /// geometry, which `extents` does not model.
    BlockLinearVirtual,
    /// ★ T-mode (§3.4): a virtual operand or semaphore no placement row covers — at `at`.
    VirtualUnresolved {
        /// The operand's VA.
        va: u64,
        /// The first uncovered byte.
        at: u64,
    },
    /// ★ T-mode (§3.4): a row that resolves outside its window — into the firmware carve-out, or
    /// past the guest-RAM window.
    OutsideWindow {
        /// The VA.
        va: u64,
        /// Guest RAM (else the store).
        ram: bool,
        /// The backing offset.
        off: u64,
    },
    /// ★ T-mode: a semaphore that does not lie inside one row.
    SemaphoreSpansRows {
        /// Its VA.
        va: u64,
        /// Its bytes.
        bytes: u64,
    },
    /// ★ T-mode: a write, release or reduction through a row the guest mapped read-only.
    ReadOnlyRow {
        /// The VA.
        va: u64,
    },
    /// ★ T-mode: a reduction through a row the guest mapped with atomics disabled.
    AtomicDisabledRow {
        /// The VA.
        va: u64,
    },
    /// ★ T-mode (§2.2): a semaphore address a 40-bit form would truncate.
    Sem40 {
        /// The address.
        va: u64,
    },
    /// ★ T-mode (§3.4): a multi-line operand not contiguous over its whole pitch footprint.
    MultiLineDiscontiguous,
    /// ★ T-mode (§3.4): a row boundary inside one remap element.
    SplitInsideElement,
    /// ★ T-mode (§3.4): a launch that would need more than [`crate::tmode::MAX_PIECES`] pieces.
    TooManyPieces {
        /// The pieces it would need.
        pieces: usize,
    },
}

impl Refusal {
    /// The refusal's name, for counters (`TSHADOW would_refuse=[name:n …]`).
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Refusal::BadHeader { .. } => "bad_header",
            Refusal::Truncated { .. } => "truncated",
            Refusal::PeerOperand => "peer_operand",
            Refusal::Untranslatable { .. } => "untranslatable",
            Refusal::BlockLinearPhysical => "block_linear_physical",
            Refusal::ExtentOverflow => "extent_overflow",
            Refusal::ForeignClass { .. } => "foreign_class",
            Refusal::MemOp { .. } => "mem_op",
            Refusal::UnboundSubchannel { .. } => "unbound_subchannel",
            Refusal::SwMethod { .. } => "sw_method",
            Refusal::RefusedMethod { .. } => "refused_method",
            Refusal::SubDeviceMask { .. } => "subdevice_mask",
            Refusal::Unclassified { .. } => "unclassified",
            Refusal::NoCeObject { .. } => "no_ce_object",
            Refusal::LaunchField { .. } => "launch_field",
            Refusal::HostSemOp { .. } => "host_sem_op",
            Refusal::FieldValue { .. } => "field_value",
            Refusal::BlockLinearVirtual => "block_linear_virtual",
            Refusal::VirtualUnresolved { .. } => "virtual_unresolved",
            Refusal::OutsideWindow { .. } => "outside_window",
            Refusal::SemaphoreSpansRows { .. } => "semaphore_spans_rows",
            Refusal::ReadOnlyRow { .. } => "read_only_row",
            Refusal::AtomicDisabledRow { .. } => "atomic_disabled_row",
            Refusal::Sem40 { .. } => "sem40",
            Refusal::MultiLineDiscontiguous => "multi_line_discontiguous",
            Refusal::SplitInsideElement => "split_inside_element",
            Refusal::TooManyPieces { .. } => "too_many_pieces",
        }
    }
}

/// ★ P1+P2 inc A (`V3_P1P2_TSPACE.md` §3.2) — **the methods refused by name on every tier**:
/// `(first, last, name)`, host methods (below `0x100`) and copy-engine methods alike.
///
/// - host `CLEAR_FAULTED` (`0x84`, every host class from `NVC46F`): UVM pushes it only while it
///   services a non-replayable fault (`ogkm-580: kernel-open/nvidia-uvm/uvm_turing_host.c:73-92`),
///   and kayfabe delivers none;
/// - `PM_TRIGGER` / `PM_TRIGGER_END` (`clc7b5.h`: `0x140`, `0x1114`): performance-monitor triggers;
/// - `SET_MONITORED_FENCE_TYPE` and `_SIGNAL_ADDR_BASE_UPPER/LOWER` (`0x21C`–`0x224`): the fence
///   type arms a write to the SIGNAL_ADDR state at the next launch — an address no rewriter
///   authors;
/// - `SET_RENDER_ENABLE_A/B/C` (`0x254`–`0x25C`): A/B are an address, C selects a conditional mode
///   against whatever A/B the engine already holds;
/// - `SET_SECURE_COPY_MODE` (`0x500`) and the confidential-computing address methods
///   (`0x514`–`0x53C`, `ogkm-580: src/common/sdk/nvidia/inc/class/clc8b5.h`): stock pushes them only
///   under confidential computing (`ogkm-580: kernel-open/nvidia-uvm/uvm_hopper_ce.c:545`, `:588`),
///   which kayfabe does not support.
pub const REFUSED_METHODS: [(u32, u32, &str); 8] = [
    (HOST_CLEAR_FAULTED, HOST_CLEAR_FAULTED, "CLEAR_FAULTED"),
    (CE_PM_TRIGGER, CE_PM_TRIGGER, "PM_TRIGGER"),
    (CE_PM_TRIGGER_END, CE_PM_TRIGGER_END, "PM_TRIGGER_END"),
    (
        CE_SET_MONITORED_FENCE_TYPE,
        CE_SET_MONITORED_FENCE_TYPE,
        "SET_MONITORED_FENCE_TYPE",
    ),
    (
        CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_UPPER,
        CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_LOWER,
        "SET_MONITORED_FENCE_SIGNAL_ADDR_BASE",
    ),
    (
        CE_SET_RENDER_ENABLE_A,
        CE_SET_RENDER_ENABLE_C,
        "SET_RENDER_ENABLE",
    ),
    (
        CE_SET_SECURE_COPY_MODE,
        CE_SET_SECURE_COPY_MODE,
        "SET_SECURE_COPY_MODE",
    ),
    (
        CE_CC_ADDR_FIRST,
        CE_CC_ADDR_LAST,
        "confidential-computing address method",
    ),
];

/// `NVC56F_CLEAR_FAULTED` (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h`).
const HOST_CLEAR_FAULTED: u32 = 0x84;
/// `NVC7B5_PM_TRIGGER`.
const CE_PM_TRIGGER: u32 = 0x140;
/// `NVC7B5_PM_TRIGGER_END`.
const CE_PM_TRIGGER_END: u32 = 0x1114;
/// `NVC7B5_SET_MONITORED_FENCE_TYPE`.
const CE_SET_MONITORED_FENCE_TYPE: u32 = 0x21C;
/// `NVC7B5_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_UPPER`.
const CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_UPPER: u32 = 0x220;
/// `NVC7B5_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_LOWER`.
const CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_LOWER: u32 = 0x224;
/// `NVC7B5_SET_RENDER_ENABLE_A`.
const CE_SET_RENDER_ENABLE_A: u32 = 0x254;
/// `NVC7B5_SET_RENDER_ENABLE_C`.
const CE_SET_RENDER_ENABLE_C: u32 = 0x25C;
/// `NVC8B5_SET_SECURE_COPY_MODE`.
const CE_SET_SECURE_COPY_MODE: u32 = 0x500;
/// `NVC8B5_SET_DECRYPT_AUTH_TAG_COMPARE_ADDR_UPPER` — the first confidential-computing address.
const CE_CC_ADDR_FIRST: u32 = 0x514;
/// `NVC8B5_SET_ENCRYPT_IV_ADDR_LOWER` — the last.
const CE_CC_ADDR_LAST: u32 = 0x53C;

/// The name a method is refused under ([`REFUSED_METHODS`]), or `None`. Host methods (below
/// `0x100`) are matched on any subchannel, copy-engine methods on a hardware subchannel only — the
/// caller passes `ce = false` for a software subchannel, whose own refusal applies.
#[must_use]
pub fn refused_method(method: u32, ce: bool) -> Option<&'static str> {
    if method >= 0x100 && !ce {
        return None;
    }
    REFUSED_METHODS
        .iter()
        .find(|&&(lo, hi, _)| (lo..=hi).contains(&method))
        .map(|&(_, _, name)| name)
}

/// A CE class id? Supplied by the caller from the chip's class table.
pub type IsCeClass = fn(u32) -> bool;

/// ★ **Rewrite one pushbuffer segment.** `st` carries CE state across calls for this channel.
///
/// # Errors
/// [`Refusal`], by name.
pub fn rewrite(
    words: &[u32],
    is_ce: IsCeClass,
    st: &mut CeState,
    w: &dyn Window,
) -> Result<Vec<Piece>, Refusal> {
    rewrite_counted(words, is_ce, st, w, None)
}

/// [`rewrite`], with every header and method write counted into `census` when one is given
/// (the count-only instrument of `V3_P1P2_TSPACE.md` §3.6). The census sees a write BEFORE it is
/// handled, so a refused write is counted too.
///
/// # Errors
/// [`Refusal`], by name.
pub fn rewrite_counted(
    words: &[u32],
    is_ce: IsCeClass,
    st: &mut CeState,
    w: &dyn Window,
    mut census: Option<&mut Census>,
) -> Result<Vec<Piece>, Refusal> {
    let mut out: Vec<Piece> = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let hw = words[i];
        let Some(h) = method_header_decode(hw) else {
            return Err(Refusal::BadHeader { at: i, word: hw });
        };
        if let Some(c) = census.as_deref_mut() {
            c.form(form_tag(h.form));
        }
        i += 1;
        let (addrs_vals, consumed): (Vec<(u32, u32)>, usize) = match h.form {
            MethodForm::EndPbSegment => break,
            MethodForm::Immediate => (vec![(h.method, h.immd)], 0),
            MethodForm::SubDeviceMask => {
                // ★ P1+P2 inc A (§3.7): refused when strict; counted and pushed raw (as before
                // inc A) on the count-only default path.
                if st.strict {
                    return Err(Refusal::SubDeviceMask { at: i - 1 });
                }
                st.inca.subdevice_masks += 1;
                cur.push(hw);
                continue;
            }
            MethodForm::Incrementing | MethodForm::Legacy => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (
                    args.iter()
                        .enumerate()
                        .map(|(k, &v)| (h.method + 4 * k as u32, v))
                        .collect(),
                    n,
                )
            }
            MethodForm::NonIncrementing => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (args.iter().map(|&v| (h.method, v)).collect(), n)
            }
            MethodForm::IncrementOnce => {
                let n = h.arg_words;
                let args = words.get(i..i + n).ok_or(Refusal::Truncated { at: i })?;
                (
                    args.iter()
                        .enumerate()
                        .map(|(k, &v)| (if k == 0 { h.method } else { h.method + 4 }, v))
                        .collect(),
                    n,
                )
            }
        };
        i += consumed;
        let sub = h.subchannel;
        for (m, v) in addrs_vals {
            if let Some(c) = census.as_deref_mut() {
                count_write(c, st, sub, m, v);
            }
            // ★ v3-initrace: bookkeeping only, BEFORE the write is handled (a refused write
            // below records a release that never ran — harmless: the channel is then dead).
            if st.sw_subch & (1u8 << (sub & 7)) == 0 || m < 0x100 {
                note_semaphore(st, sub, m, v);
            }
            one_write(&mut out, &mut cur, is_ce, st, w, sub, m, v)?;
        }
    }
    if !cur.is_empty() {
        out.push(Piece::Words(cur));
    }
    Ok(out)
}

/// The census tag of a header form.
#[must_use]
pub const fn form_tag(f: MethodForm) -> &'static str {
    match f {
        MethodForm::Incrementing => "inc",
        MethodForm::NonIncrementing => "non",
        MethodForm::IncrementOnce => "one",
        MethodForm::Immediate => "imm",
        MethodForm::EndPbSegment => "end",
        MethodForm::Legacy => "legacy",
        MethodForm::SubDeviceMask => "sdm",
    }
}

/// Which part of the channel `(sub, m)` reaches, given the subchannel bindings in `st`.
#[must_use]
pub fn sub_kind(st: &CeState, sub: u32, m: u32) -> SubKind {
    if m < 0x100 {
        SubKind::Host
    } else if st.sw_subch & (1u8 << (sub & 7)) != 0 {
        SubKind::Sw
    } else if sub > 4 {
        SubKind::Unbound
    } else {
        SubKind::Ce
    }
}

fn count_write(c: &mut Census, st: &CeState, sub: u32, m: u32, v: u32) {
    let kind = sub_kind(st, sub, m);
    c.method(st.ce_class, kind, m);
    match (kind, m) {
        (SubKind::Ce, ce::LAUNCH_DMA) => c.launch(st.ce_class, v),
        (SubKind::Host, HOST_SEMAPHORE_D) => c.op(OpKind::SemaphoreD, v & HOST_SEMAPHORE_D_OP_MASK),
        (SubKind::Host, kf_abi::submit::fifo::SEM_EXECUTE) => c.op(
            OpKind::SemExecute,
            v & kf_abi::submit::fifo::SEM_EXECUTE_OPERATION_MASK,
        ),
        (SubKind::Host, MEM_OP_D) => c.op(OpKind::MemOpD, v >> 27),
        _ => {}
    }
}

fn emit(cur: &mut Vec<u32>, sub: u32, m: u32, v: u32) {
    // `method_header_inc` refuses only an out-of-range method or subchannel, and every method
    // here came out of a header that decoded, so both are in range.
    if let Some(h) = method_header_inc(sub, m, 1) {
        cur.push(h);
        cur.push(v);
    }
}

#[allow(clippy::too_many_arguments)]
fn one_write(
    out: &mut Vec<Piece>,
    cur: &mut Vec<u32>,
    is_ce: IsCeClass,
    st: &mut CeState,
    w: &dyn Window,
    sub: u32,
    m: u32,
    v: u32,
) -> Result<(), Refusal> {
    // ── host methods (any subchannel, below 0x100) ─────────────────────────────────────────
    let bit = 1u8 << (sub & 7);
    if m == 0 {
        // SET_OBJECT: which subchannels hold a CE, which hold the SW class; nothing else is ours.
        let class = v & 0xFFFF;
        st.ce_subch &= !bit;
        st.sw_subch &= !bit;
        if is_ce(class) {
            st.ce_subch |= bit;
            st.ce_class = class;
            emit(cur, sub, m, v);
        } else if class == GP100_UVM_SW {
            st.sw_subch |= bit; // consumed: no host object stands behind it
        } else {
            return Err(Refusal::ForeignClass { subch: sub, class });
        }
        return Ok(());
    }
    if st.sw_subch & bit != 0 && m >= 0x100 {
        // A SW subchannel's own methods (host methods below 0x100 still apply to the channel).
        return if m == SW_NO_OPERATION {
            Ok(())
        } else {
            Err(Refusal::SwMethod { method: m })
        };
    }
    // ★ P1+P2 inc A (§3.2): refused by name on every tier — a host method on any subchannel, a
    // copy-engine method on a hardware one (an unbound software subchannel refuses below) — when
    // strict; on the count-only default path counted, then handled exactly as before inc A.
    if let Some(name) = refused_method(m, sub <= 4) {
        if st.strict {
            return Err(Refusal::RefusedMethod {
                subch: sub,
                method: m,
                name,
            });
        }
        st.inca.refused_methods += 1;
    }
    // `MEM_OP_A..C` are operands of the `MEM_OP_D` that follows ("MEM_OP_D MUST be preceded by
    // MEM_OPs A-C", `clc56f.h`): held here, emitted with the D when its operation is forwarded.
    if m == MEM_OP_A {
        st.mem_op_a = v;
        return Ok(());
    }
    if m == MEM_OP_B {
        st.mem_op_b = v;
        return Ok(());
    }
    if m == MEM_OP_C {
        st.mem_op_c = v;
        return Ok(());
    }
    if m == MEM_OP_D {
        let op = v >> 27;
        if op == OP_MEMBAR {
            // ★ P6b (d): an ordering method, not a privileged one — forwarded as written.
            emit(cur, sub, MEM_OP_A, st.mem_op_a);
            emit(cur, sub, MEM_OP_B, st.mem_op_b);
            emit(cur, sub, MEM_OP_C, st.mem_op_c);
            emit(cur, sub, MEM_OP_D, v);
            return Ok(());
        }
        if op == OP_TLB_INVALIDATE || op == OP_TLB_INVALIDATE_TARGETED {
            if st.mem_op_a & MEM_OP_A_SYSMEMBAR_EN != 0 {
                // ★ P6b (d): the invalidate's SYSMEMBAR is kept as a forwarded MEMBAR ahead of it.
                emit(cur, sub, MEM_OP_A, 0);
                emit(cur, sub, MEM_OP_B, 0);
                emit(cur, sub, MEM_OP_C, MEMBAR_TYPE_SYS);
                emit(cur, sub, MEM_OP_D, OP_MEMBAR << 27);
            }
            if !cur.is_empty() {
                out.push(Piece::Words(std::mem::take(cur)));
            }
            let all = st.mem_op_c & 1 != 0;
            let lo = u64::from(st.mem_op_c & 0xFFFF_F000);
            let hi = u64::from(v & 0x07FF_FFFF) << 32;
            out.push(Piece::Invalidate {
                pdb: (!all).then_some(hi | lo),
            });
            return Ok(());
        }
        if OPS_L2.contains(&op) {
            // L2 maintenance: an unprivileged host method — forwarded with its operands, in order.
            emit(cur, sub, MEM_OP_A, st.mem_op_a);
            emit(cur, sub, MEM_OP_B, st.mem_op_b);
            emit(cur, sub, MEM_OP_C, st.mem_op_c);
            emit(cur, sub, MEM_OP_D, v);
            return Ok(());
        }
        if op == OP_ACCESS_COUNTER_CLR {
            // Served, not dropped: the device we present has no access counters to clear.
            return Ok(());
        }
        // ⊘ Everything else — Hopper's `MMU_OPERATION` (0xb, `VIDMEM_ACCESS_BIT_DUMP`,
        // `clc86f.h:121,139-141`; unused by UVM) and any operation no class header names — is
        // refused BY NAME rather than consumed silently.
        return Err(Refusal::MemOp { op });
    }
    // Host methods (below 0x100) apply to the channel whatever the subchannel: forwarded.
    if m < 0x100 {
        emit(cur, sub, m, v);
        return Ok(());
    }
    // ★★★★★ P6b: on a COPY-ENGINE channel every HARDWARE subchannel (0-4) reaches the CE — "HW
    // uses a fixed subchannel for CE" (`NVA06F_SUBCHANNEL_COPY_ENGINE` = 4, `cla06fsubch.h`;
    // `uvm_push_macros.h:84-85`) — whichever subchannel the SET_OBJECT named. nvidia-uvm binds the
    // CE on subchannel 0 (`uvm_maxwell_ce.c:31-36`, to verify the engine type) and pushes every CE
    // method on subchannel 4. `[measured p6b8]` keyed on the SET_OBJECT's subchannel, this
    // rewriter saw NONE of UVM's launches as CE launches and forwarded them VERBATIM — PHYSICAL
    // operands (UVM's page-table writes, `OFFSET_OUT` `0x201000`…) straight onto our host ring,
    // i.e. aimed at HOST physical memory — and the guest's own root stayed all zeros in the store.
    // ⇒ Every Translated channel is a CE channel (the plane births no other kind), so subchannels
    // 0-4 are the CE here, always; 5-7 are software subchannels and must be bound to be used.
    if sub > 4 {
        return Err(Refusal::UnboundSubchannel {
            subch: sub,
            method: m,
        });
    }
    // ── CE methods ─────────────────────────────────────────────────────────────────────────
    match m {
        ce::OFFSET_IN_UPPER => {
            st.off_in = (st.off_in & 0xFFFF_FFFF) | (u64::from(v & upper_mask(st)) << 32)
        }
        OFFSET_IN_LOWER => st.off_in = (st.off_in & !0xFFFF_FFFF) | u64::from(v),
        ce::OFFSET_OUT_UPPER => {
            st.off_out = (st.off_out & 0xFFFF_FFFF) | (u64::from(v & upper_mask(st)) << 32);
        }
        OFFSET_OUT_LOWER => st.off_out = (st.off_out & !0xFFFF_FFFF) | u64::from(v),
        ce::LINE_LENGTH_IN => st.line_len = v,
        LINE_COUNT => st.line_count = v,
        ce::SET_SRC_PHYS_MODE => st.src_mode = v,
        ce::SET_DST_PHYS_MODE => st.dst_mode = v,
        PITCH_IN => st.pitch_in = v,
        PITCH_OUT => st.pitch_out = v,
        ce::SET_REMAP_COMPONENTS => st.remap = v,
        ce::SET_REMAP_CONST_A => st.const_a = v,
        ce::LAUNCH_DMA => {
            let src_phys = v & ce::LAUNCH_SRC_PHYSICAL != 0;
            let dst_phys = v & ce::LAUNCH_DST_PHYSICAL != 0;
            // ★ v3-initrace: bookkeeping for the completion probe (the words are unchanged).
            if v & ce::LAUNCH_TRANSFER_MASK != ce::LAUNCH_TRANSFER_NONE
                && let Ok((src_len, dst_len)) = extents(st, v)
            {
                let side = |phys: bool, mode: u32, at: u64, n: u64| {
                    if phys {
                        Operand::Physical(Target::from_bits(mode), at, n)
                    } else {
                        Operand::Virtual(at, n)
                    }
                };
                let src = src_len.map(|n| side(src_phys, st.src_mode, st.off_in, n));
                let dst = Some(side(dst_phys, st.dst_mode, st.off_out, dst_len));
                st.launches.push(PhysLaunch {
                    launch: v,
                    src,
                    dst,
                });
            }
            if dst_phys && st.ce_class >= HOPPER_DMA_COPY_A && v & LAUNCH_MEMORY_SCRUB_ENABLE != 0 {
                return launch_scrub_translated(cur, st, w, sub, v);
            }
            if src_phys || dst_phys {
                return launch_translated(cur, st, w, sub, v, src_phys, dst_phys);
            }
        }
        _ => {}
    }
    emit(cur, sub, m, v);
    Ok(())
}

/// The bytes a launch touches on each side: `(source, destination)`, the source `None` when the
/// engine does not read it (no data transfer, or a remap whose every written component is a
/// constant — the scrub's fill). ★ Exact, never an under-estimate: the window check vets THIS
/// range, and a short one would pass a launch the engine then runs past the window's end.
fn extents(st: &CeState, v: u32) -> Result<(Option<u64>, u64), Refusal> {
    let remap = v & ce::LAUNCH_REMAP_ENABLE != 0;
    let comp = u64::from(ce::remap_component_bytes(st.remap));
    let n_dst = ce::remap_num_dst_components(st.remap);
    let n_src = u64::from(((st.remap >> REMAP_NUM_SRC_SHIFT) & 0x3) + 1);
    let (src_elem, dst_elem) = if remap {
        (comp * n_src, comp * u64::from(n_dst))
    } else {
        (1, 1)
    };
    let reads_src = v & ce::LAUNCH_TRANSFER_MASK != ce::LAUNCH_TRANSFER_NONE
        && (!remap
            || (0..n_dst).any(|c| ce::remap_dst_sel(st.remap, c) <= ce::REMAP_DST_SEL_SRC_MAX));
    let lines = if v & ce::LAUNCH_MULTI_LINE_ENABLE != 0 {
        u64::from(st.line_count)
    } else {
        1
    };
    let ext = |elem: u64, pitch: u32| -> Result<u64, Refusal> {
        let line = u64::from(st.line_len)
            .checked_mul(elem)
            .ok_or(Refusal::ExtentOverflow)?;
        let span = match lines {
            0 | 1 => line,
            n => u64::from(pitch)
                .checked_mul(n - 1)
                .and_then(|x| x.checked_add(line))
                .ok_or(Refusal::ExtentOverflow)?,
        };
        Ok(span.max(1))
    };
    let src = if reads_src {
        Some(ext(src_elem, st.pitch_in)?)
    } else {
        None
    };
    Ok((src, ext(dst_elem, st.pitch_out)?))
}

fn xlate(w: &dyn Window, mode: u32, phys: u64, len: u64) -> Result<u64, Refusal> {
    let t = Target::from_bits(mode);
    if t == Target::Peer {
        return Err(Refusal::PeerOperand);
    }
    w.translate(t, phys, len).ok_or(Refusal::Untranslatable {
        target: t,
        phys,
        len,
    })
}

/// ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md` §4): `OFFSET_{IN,OUT}_UPPER` is `16:0` through
/// `NVC7B5` (a 49-bit VA, `clc7b5.h:162,166`) and `24:0` from `HOPPER_DMA_COPY_A` on (57-bit,
/// `clc8b5.h:92,96`). `[measured GB203]` masking a Blackwell window VA to 17 bits sent the scrubber's
/// zero-fill to `0x1fffe_0005_0000` instead of `0x1ff_fffe_0005_0000` — Xid 31 FAULT_PDE on our ring.
const fn upper_mask(st: &CeState) -> u32 {
    if st.ce_class >= HOPPER_DMA_COPY_A {
        0x1FF_FFFF
    } else {
        0x1_FFFF
    }
}

fn put_offset(cur: &mut Vec<u32>, st: &CeState, sub: u32, upper: u32, va: u64) {
    emit(cur, sub, upper, ((va >> 32) as u32) & upper_mask(st));
    emit(cur, sub, upper + 4, (va & 0xFFFF_FFFF) as u32);
}

/// `HOPPER_DMA_COPY_A` — the first CE class with `LAUNCH_DMA_MEMORY_SCRUB_ENABLE` (`ogkm-580: clc8b5.h`).
const HOPPER_DMA_COPY_A: u32 = 0xC8B5;
/// `NVC8B5_LAUNCH_DMA_MEMORY_SCRUB_ENABLE` `23:23` (`clc8b5.h:84-86`).
const LAUNCH_MEMORY_SCRUB_ENABLE: u32 = 1 << 23;
/// `SET_REMAP_COMPONENTS` = `DST_X = CONST_A`, `COMPONENT_SIZE_ONE`, `NUM_DST_COMPONENTS_ONE` — RM's
/// own 1-byte fill map (`channel_utils.c:1029-1033`).
const REMAP_BYTE_FILL_FROM_CONST_A: u32 = ce::REMAP_DST_SEL_CONST_A;

/// ★★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md` §4) — **a Hopper+ FAST SCRUB on a physical
/// destination becomes the equivalent virtual zero-fill.**
///
/// `[measured GB203, bws3]` RM's PMA scrubber pushes `LAUNCH_DMA` with `MEMORY_SCRUB_ENABLE`
/// (`memmgrMemUtilsCheckMemoryFastScrubEnable_GH100`: class ≥ `HOPPER_DMA_COPY_A`, dst PHYSICAL
/// LOCAL_FB, 4 KiB aligned — `mem_utils_gm107.c:2055-2118`, `channel_utils.c:676-690`). Our generic
/// rewrite turned the destination VIRTUAL and kept the scrub bit, and the host CE raised
/// **Xid 71 (CE4 error)** on every launch: *"the fast scrubber only works with physical
/// addressing"* (`kernel-open/nvidia-uvm/uvm_hopper_ce.c:185-196`). Our host channel may not address
/// FB physically, so the operation is expressed as what it IS — zero the bytes — by the same
/// engine: remap-enabled, `DST_X = CONST_A = 0`, 1-byte components (with remap disabled a scrub's
/// element is one byte, so `LINE_LENGTH_IN` already counts bytes). The guest's remap registers
/// are restored after the launch, exactly as its offsets are.
fn launch_scrub_translated(
    cur: &mut Vec<u32>,
    st: &CeState,
    w: &dyn Window,
    sub: u32,
    v: u32,
) -> Result<(), Refusal> {
    if v & ce::LAUNCH_DST_PITCH == 0 {
        return Err(Refusal::BlockLinearPhysical);
    }
    // With remap DISABLED a scrub reads no source and its element is one byte.
    let (_, dst_len) = extents(st, v & !ce::LAUNCH_REMAP_ENABLE)?;
    let va = xlate(w, st.dst_mode, st.off_out, dst_len)?;
    emit(cur, sub, ce::SET_REMAP_CONST_A, 0);
    emit(
        cur,
        sub,
        ce::SET_REMAP_COMPONENTS,
        REMAP_BYTE_FILL_FROM_CONST_A,
    );
    put_offset(cur, st, sub, ce::OFFSET_OUT_UPPER, va);
    emit(
        cur,
        sub,
        ce::LAUNCH_DMA,
        (v & !(LAUNCH_MEMORY_SCRUB_ENABLE | ce::LAUNCH_SRC_PHYSICAL | ce::LAUNCH_DST_PHYSICAL))
            | ce::LAUNCH_REMAP_ENABLE,
    );
    put_offset(cur, st, sub, ce::OFFSET_OUT_UPPER, st.off_out);
    emit(cur, sub, ce::SET_REMAP_COMPONENTS, st.remap);
    emit(cur, sub, ce::SET_REMAP_CONST_A, st.const_a);
    Ok(())
}

fn launch_translated(
    cur: &mut Vec<u32>,
    st: &CeState,
    w: &dyn Window,
    sub: u32,
    v: u32,
    src_phys: bool,
    dst_phys: bool,
) -> Result<(), Refusal> {
    let (src_len, dst_len) = extents(st, v)?;
    let src_rewritten = src_phys && src_len.is_some();
    if src_rewritten && v & ce::LAUNCH_SRC_PITCH == 0 || dst_phys && v & ce::LAUNCH_DST_PITCH == 0 {
        return Err(Refusal::BlockLinearPhysical);
    }
    if let (true, Some(len)) = (src_phys, src_len) {
        let va = xlate(w, st.src_mode, st.off_in, len)?;
        put_offset(cur, st, sub, ce::OFFSET_IN_UPPER, va);
    }
    if dst_phys {
        let va = xlate(w, st.dst_mode, st.off_out, dst_len)?;
        put_offset(cur, st, sub, ce::OFFSET_OUT_UPPER, va);
    }
    emit(
        cur,
        sub,
        ce::LAUNCH_DMA,
        v & !(ce::LAUNCH_SRC_PHYSICAL | ce::LAUNCH_DST_PHYSICAL),
    );
    // Restore the guest's own offsets: a later VIRTUAL launch that does not rewrite them must
    // still read what the guest wrote, not our window address.
    if src_rewritten {
        put_offset(cur, st, sub, ce::OFFSET_IN_UPPER, st.off_in);
    }
    if dst_phys {
        put_offset(cur, st, sub, ce::OFFSET_OUT_UPPER, st.off_out);
    }
    Ok(())
}

/// ★ The rewriter's own method offsets and per-class rules, held to the class headers
/// (`kf_chip::hwref`, `docs/design/V3_HW_BOUNDARY_INVENTORY.md`): the MEM_OP operation codes it
/// forwards or splits on, the CE offsets it re-emits, the software class it consumes, and — the one
/// width that moved — `OFFSET_*_UPPER` per bound CE class.
#[cfg(test)]
mod hwref_check {
    use super::*;
    use kf_chip::hwref::expect::{class_range, class_val};

    #[test]
    fn the_rewriters_methods_are_the_class_headers() {
        for (ours, name) in [
            (MEM_OP_A, "NVC56F_MEM_OP_A"),
            (MEM_OP_B, "NVC56F_MEM_OP_B"),
            (MEM_OP_C, "NVC56F_MEM_OP_C"),
            (MEM_OP_D, "NVC56F_MEM_OP_D"),
            (OP_MEMBAR, "NVC56F_MEM_OP_D_OPERATION_MEMBAR"),
            (
                OP_TLB_INVALIDATE,
                "NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE",
            ),
            (
                OP_TLB_INVALIDATE_TARGETED,
                "NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED",
            ),
            (
                OP_ACCESS_COUNTER_CLR,
                "NVC56F_MEM_OP_D_OPERATION_ACCESS_COUNTER_CLR",
            ),
            (MEMBAR_TYPE_SYS, "NVC56F_MEM_OP_C_MEMBAR_TYPE_SYS_MEMBAR"),
            (LINE_COUNT, "NVC7B5_LINE_COUNT"),
            (OFFSET_IN_LOWER, "NVC7B5_OFFSET_IN_LOWER"),
            (OFFSET_OUT_LOWER, "NVC7B5_OFFSET_OUT_LOWER"),
            (PITCH_IN, "NVC7B5_PITCH_IN"),
            (PITCH_OUT, "NVC7B5_PITCH_OUT"),
            (GP100_UVM_SW, "GP100_UVM_SW"),
            (SW_NO_OPERATION, "NVC076_NO_OPERATION"),
            (HOPPER_DMA_COPY_A, "HOPPER_DMA_COPY_A"),
        ] {
            assert_eq!(u64::from(ours), class_val(name), "{name}");
        }
        assert_eq!(
            u64::from(MEM_OP_A_SYSMEMBAR_EN),
            1 << class_range("NVC56F_MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR").1
        );
        assert_eq!(
            u64::from(REMAP_NUM_SRC_SHIFT),
            class_range("NVC7B5_SET_REMAP_COMPONENTS_NUM_SRC_COMPONENTS").1
        );
        // The L2 operations forwarded verbatim: 0xd/0xe/0xf/0x10/0x15 from C56F, 0x11 (the
        // non-coherent sysmem invalidate) only C96F states.
        let l2 = [
            "NVC56F_MEM_OP_D_OPERATION_L2_PEERMEM_INVALIDATE",
            "NVC56F_MEM_OP_D_OPERATION_L2_SYSMEM_INVALIDATE",
            "NVC56F_MEM_OP_D_OPERATION_L2_CLEAN_COMPTAGS",
            "NVC56F_MEM_OP_D_OPERATION_L2_FLUSH_DIRTY",
            "NVC96F_MEM_OP_D_OPERATION_L2_SYSMEM_NCOH_INVALIDATE",
            "NVC56F_MEM_OP_D_OPERATION_L2_WAIT_FOR_SYS_PENDING_READS",
        ];
        assert_eq!(OPS_L2.map(u64::from).to_vec(), l2.map(class_val).to_vec());
        // The fast-scrub bit exists only from HOPPER_DMA_COPY_A; below it bit 23 is VPRMODE's.
        assert_eq!(
            u64::from(LAUNCH_MEMORY_SCRUB_ENABLE),
            1 << class_range("NVC8B5_LAUNCH_DMA_MEMORY_SCRUB_ENABLE").1
        );
        assert_eq!(class_range("NVC7B5_LAUNCH_DMA_VPRMODE"), (23, 22));
    }

    /// ★ P1+P2 inc A: the refused methods are the class headers' own offsets.
    #[test]
    fn the_refused_methods_are_the_class_headers() {
        for (ours, name) in [
            (HOST_CLEAR_FAULTED, "NVC56F_CLEAR_FAULTED"),
            (CE_PM_TRIGGER, "NVC7B5_PM_TRIGGER"),
            (CE_PM_TRIGGER_END, "NVC7B5_PM_TRIGGER_END"),
            (
                CE_SET_MONITORED_FENCE_TYPE,
                "NVC7B5_SET_MONITORED_FENCE_TYPE",
            ),
            (
                CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_UPPER,
                "NVC7B5_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_UPPER",
            ),
            (
                CE_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_LOWER,
                "NVC7B5_SET_MONITORED_FENCE_SIGNAL_ADDR_BASE_LOWER",
            ),
            (CE_SET_RENDER_ENABLE_A, "NVC7B5_SET_RENDER_ENABLE_A"),
            (CE_SET_RENDER_ENABLE_C, "NVC7B5_SET_RENDER_ENABLE_C"),
            (CE_SET_SECURE_COPY_MODE, "NVC8B5_SET_SECURE_COPY_MODE"),
            (
                CE_CC_ADDR_FIRST,
                "NVC8B5_SET_DECRYPT_AUTH_TAG_COMPARE_ADDR_UPPER",
            ),
            (CE_CC_ADDR_LAST, "NVC8B5_SET_ENCRYPT_IV_ADDR_LOWER"),
        ] {
            assert_eq!(u64::from(ours), class_val(name), "{name}");
        }
        // CLEAR_FAULTED is at the same offset in every host class that has one.
        for h in ["NVC46F", "NVC56F"] {
            assert_eq!(
                class_val(&format!("{h}_CLEAR_FAULTED")),
                u64::from(HOST_CLEAR_FAULTED)
            );
        }
    }

    #[test]
    fn the_offset_upper_mask_is_the_bound_classes_field() {
        for (class, header) in [
            (0xC3B5u32, "NVC3B5"),
            (0xC5B5, "NVC5B5"),
            (0xC6B5, "NVC6B5"),
            (0xC7B5, "NVC7B5"),
            (0xC8B5, "NVC8B5"),
            // C9B5/CAB5 state no OFFSET methods of their own: C8B5's layout (`uvm_hal.c:154-180`).
            (0xC9B5, "NVC8B5"),
            (0xCAB5, "NVC8B5"),
        ] {
            let st = CeState {
                ce_class: class,
                ..CeState::default()
            };
            let (hi, lo) = class_range(&format!("{header}_OFFSET_IN_UPPER_UPPER"));
            assert_eq!(
                u64::from(upper_mask(&st)),
                ((1u64 << (hi - lo + 1)) - 1) << lo,
                "{class:#x}"
            );
            assert_eq!(
                class_range(&format!("{header}_OFFSET_OUT_UPPER_UPPER")),
                (hi, lo),
                "{class:#x}"
            );
        }
    }
}
