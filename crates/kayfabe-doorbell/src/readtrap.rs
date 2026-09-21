//! ⊘⊘⊘ **THIS MODULE NO LONGER AUTHORISES A READ EXIT.** Owner ruling w823, folded into
//! `THE_DESIGN.md` §5: **there is no read trap anywhere.** See [`crate::trappolicy::may_trap_read`],
//! which returns `false` unconditionally.
//!
//! ★ What survives, and why the module is kept rather than deleted: the **phase** distinction is
//! still real. A page whose value resolves through a latch needs its shadow **recomputed on the
//! trapped write that moves the latch**, and a boot-state page stops needing even that once boot
//! completes. ⇒ [`ReadPolicy::Trap`] now means *"this shadow is COMPUTED, not plain"* — never
//! *"exit to us"*. ⚠ The variant keeps its name only because renaming it across the tree is a
//! larger change than the ruling needs; the meaning is the one stated here.
//!
//! The read-trap allowlist — §5, and it is a **parity** hazard before it is a correctness one.
//!
//! ## *"Only writes trap"* is false for about 524 of 4096 BAR0 pages
//!
//! §5: reads are otherwise served from ordinary DRAM the guest reads directly, *"with no exit and
//! no code — because a vCPU inside an MMIO exit is not preemptible, and driver init polls some
//! registers thousands of times."* But the framebuffer window resolves through a latch, and the
//! firmware-boot pages are state-machine state. ⇒ **The read-trap set is an allowlist, written
//! down, exactly like the write list.**
//!
//! ## ⊘⊘⊘ AND IT IS SCOPED TO A PHASE, NOT FOREVER — this is the expensive part
//!
//! §5, measured on the C artifact:
//!
//! > *"**99 % of its 1 000–3 000 exits per token were READS of a single firmware debug register**
//! > on a page this design keeps read-trapped as 'boot state'. That one page cost a **2.5× loss on
//! > LLM decode**."*
//!
//! ⇒ **A page is read-trapped for a PHASE.** The runtime-polled words on the firmware pages are
//! shadowed in DRAM; the trap applies during boot and is **dropped when boot completes**. A
//! permanent read-trap on a page the guest polls at runtime is not a small inefficiency — it is a
//! multiple on the product's headline number.
//!
//! ## ⊘⊘⊘ THE TIMER REFUSAL WAS ON THE WRONG PATH (fable w824, HIGH 2)
//!
//! This file used to answer `RefuseByName` for a **READ** of `0x9400`/`0x9410`, and had no write
//! arm at all. Both halves were wrong, measured from ogkm:
//!
//! * Turing+ kernel RM **never reads** `NV_PTIMER_TIME_0/1`. `tmrReadTimeLoReg_TU102` /
//!   `tmrReadTimeHiReg_TU102` (`timer_tu102.c:135-165`) read `NV_VIRTUAL_FUNCTION_TIME_0/1` —
//!   BAR0 `0x30080`/`0x30084` (`turing/tu102/dev_vm.h:224,226`) — and that dispatch covers every
//!   non-Tegra chip (`g_objtmr_nvoc.c:499-503`). A "refused read" of a readable register is not
//!   even expressible: a load has to return *something*.
//! * It **writes** them — `NV_PTIMER_TIME_1` then `NV_PTIMER_TIME_0` (`timer_gv100.c:71-72`,
//!   *"Writing TIME_0 is the trigger"*) — once at boot (`kernel_gsp.c:5039`) and at resume
//!   (`gpu_suspend.c:236`), gated on `NV_PTIMER_TIME_PRIV_LEVEL_MASK` bit 4
//!   (`volta/gv100/dev_timer.h:28-31`, tested at `timer_gv100.c:56`).
//!
//! ⇒ The refusal lives on the **write** path now ([`TimerRegs::is_refused_write`], consulted by
//! `TrapPath::privileged`), and the registers are **per timer HAL**, grouped the way
//! `g_objtmr_nvoc.c:420-445` groups them. See [`TimerRegs`] for the decision and its reason.

/// Which phase the device is in. ⊘ The allowlist is a function of this, which is the whole point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Firmware boot: falcon/RISC-V bring-up, WPR2, the msgq handshake.
    Boot,
    /// After `GSP_INIT_DONE`. ★ Most boot read-traps are dropped here.
    Runtime,
}

/// Why a page reads through us at all. ⊘ Naming the reason is what lets a page be **dropped** from
/// the set later: *"boot state"* is droppable at runtime, *"a latch"* never is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadReason {
    /// The value the guest must see is computed from a latch it wrote elsewhere — e.g. the
    /// framebuffer window. ⊘ **Never droppable:** the value does not exist in DRAM to be read.
    ResolvesThroughLatch,
    /// Firmware state-machine state during bring-up. ★ **Droppable at `Runtime`** — the words the
    /// guest polls afterwards are shadowed in DRAM.
    BootStateMachine,
}

/// One entry of the written-down set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadTrapPage {
    pub page: u32,
    pub reason: ReadReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPolicy {
    /// Serve from the DRAM shadow. No exit, no code.
    FromShadow,
    /// Exit to us.
    Trap(ReadReason),
}

/// BAR0 on the GA106 bench: 16 MiB of 4 KiB pages. ⊘ A **named default**, not a fact about GPUs.
///
/// `[fable w824, LOW 5]` §50 level 1 for the real value is `NV_ESC_CARD_INFO.reg_size`
/// (`kernel-open/common/inc/nv-ioctl.h:63`, filled from `nv->regs->size` at `nv.c:2384`, dispatched
/// at `nv.c:2593` under `NV_CTL_DEVICE_ONLY` — no admin check), an unprivileged host ioctl. Until
/// the host binding exists, [`ReadTrapSet::with_bar0_bytes`] takes the value and this is what
/// [`ReadTrapSet::new`] passes it.
pub const GA106_BAR0_BYTES: u32 = 16 << 20;
pub const GA106_BAR0_PAGES: u32 = GA106_BAR0_BYTES >> 12;
/// ⊘ Kept as the GA106 default under its old name so existing bounds tests still say what they
/// said; new code should name the die or take the value from `CARD_INFO`.
pub const BAR0_PAGES: u32 = GA106_BAR0_PAGES;

/// The legacy timer page, and why it is **not** in the read-trap set.
///
/// §5: *"On Turing and newer the kernel's clock is the pair in the **usermode** window — which is
/// already the read-only memslot over live host time — and the legacy page at the old offset is
/// touched only at boot and resume, never read at runtime."* ⇒ It is a **computed shadow**:
/// constants for the privilege mask and tick frequency.
pub const LEGACY_TIMER_PAGE: u32 = 0x9000 >> 12;

/// Where Turing+ kernel RM actually READS time from: `NV_VIRTUAL_FUNCTION_TIME_0/1`.
/// §50 level 2: `turing/tu102/dev_vm.h:224,226` (`R--4R`), read at `timer_tu102.c:142,159`; the
/// same offsets in `ampere/ga100/dev_vm.h:127,129` and `blackwell/gb100/dev_vm.h:616,618`.
pub const VF_TIME_0: u32 = 0x30080;
pub const VF_TIME_1: u32 = 0x30084;

/// The three ways ogkm sets GPU time, grouped exactly as `g_objtmr_nvoc.c:420-445` dispatches
/// `tmrSetCurrentTime` — by HAL, not by die.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerHal {
    /// `tmrSetCurrentTime_GV100`: TU102–TU117, GA100–GA107, AD102–AD107 — **Turing, Ampere, Ada**.
    Gv100,
    /// `tmrSetCurrentTime_GH100`: GH100 and every **discrete Blackwell** (GB100/GB102/GB110/
    /// GB112, GB202–GB207, GR100/GR102) — the dispatch's `else` arm.
    Gh100,
    /// `tmrSetCurrentTime_GB10B`: GB10B, GB20B, GB20C — the **integrated** Blackwell parts.
    /// ⊘ Not in the finding; found while reading the dispatch. It writes NO register at all.
    Gb10b,
}

/// One HAL's time-setting registers: what it writes, and the privilege mask it tests first.
///
/// ## ★ THE DECISION: the write is REFUSED BY NAME and the PLM says "level 0 may write"
///
/// Two options were on the table; this is the second, and why:
///
/// 1. *Serve `0x9430` with `WRITE_PROTECTION_LEVEL0 = DISABLE` so ogkm takes the else branch.*
///    ⊘ That branch is `NV_PRINTF(LEVEL_ERROR, "Write to PTIMER attempted even though Level 0 PLM
///    is disabled")` + **`NV_ASSERT(0)`** + `NV_ERR_PRIV_SEC_VIOLATION` (`timer_gv100.c:77-81`),
///    on every boot and every resume — a guest-visible assertion about hardware that is not true
///    of the GA106 being emulated: the caller waits for GFW boot precisely *"to avoid PLM
///    collisions"* (`kernel_gsp.c:5034-5037`), i.e. level 0 IS writable by then on real silicon.
/// 2. *Accept the write as `Plain` with a time offset.* ⊘ An offset cannot be honoured: the
///    guest READS time from [`VF_TIME_0`]/[`VF_TIME_1`], which §5 serves as a read-only memslot
///    over **live host time with no exit** — there is no code on that path to add an offset in.
///    Accepting the write would make the guest believe it owns a timebase it can never read.
///
/// ⇒ **The write is dropped by name** (`Action::RefusedByName`, never queued, never applied to
/// the host, shadow untouched) and the PLM shadow answers `LEVEL0 = ENABLE`, so ogkm's `if`
/// branch runs, returns `NV_OK`, and the guest stays on the host's timebase — which is §5's
/// stated intent: *"guest and host cannot drift onto different timebases."* Nothing is lost: the
/// guest writes wall-clock nanoseconds, the host RM already wrote its own wall-clock nanoseconds
/// into the same counter at host driver load, and `tmrSetCurrentTime_GV100` never reads back.
///
/// ⚠ PLM shadow provenance: §50 level 3 (fabricated iff guest userspace doesn't read it). Kernel
/// RM tests exactly one bit (`GPU_FLD_TEST_DRF_DEF(_WRITE_PROTECTION_LEVEL0, _ENABLE)`,
/// `timer_gv100.c:56`); no userspace window covers `0x9430`; the other fields are answered zero
/// because nothing has measured them and a zero is at least not a claim.
///
/// ⚠ GH100 HAL, stated as a gap: `tmrSetCurrentTime_GH100` first READS
/// `NV_PGC6_SCI_SEC_TIMER_TIME_0/1` (`timer_gh100.c:73-78`) — a running counter — and asserts it
/// is below wall-clock ns. A static shadow of zero satisfies the hi-lo-hi loop and the assert,
/// but it is not the counter; serving it live is unbuilt. The two OFFSET writes are refused for
/// the same reason as the GV100 pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerRegs {
    pub hal: TimerHal,
    /// BAR0 offsets whose WRITE would move the GPU's timebase. Refused by name.
    pub refused_writes: &'static [u32],
    /// The privilege-level mask ogkm tests before writing, if this HAL has one.
    pub priv_level_mask: Option<u32>,
}

/// `NV_PTIMER_TIME_PRIV_LEVEL_MASK_WRITE_PROTECTION_LEVEL0 4:4`, `_ENABLE 1`
/// (`volta/gv100/dev_timer.h:29-30`). The one bit ogkm tests.
pub const PLM_WRITE_PROTECTION_LEVEL0_ENABLE: u32 = 1 << 4;

/// **Turing, Ampere, Ada.** §50 level 2: `timer_gv100.c` includes
/// `published/volta/gv100/dev_timer.h` (`:31`) for all three families — `NV_PTIMER_TIME_0 0x9400`
/// (`:26`), `NV_PTIMER_TIME_1 0x9410` (`:27`), `NV_PTIMER_TIME_PRIV_LEVEL_MASK 0x9430` (`:28`).
pub const TIMER_GV100: TimerRegs = TimerRegs {
    hal: TimerHal::Gv100,
    refused_writes: &[0x9400, 0x9410],
    priv_level_mask: Some(0x9430),
};

/// **Hopper and discrete Blackwell.** §50 level 2: `timer_gh100.c` includes
/// `published/hopper/gh100/dev_gc6_island.h` — `NV_PGC6_SCI_SYS_TIMER_OFFSET_0 0x118df4` (`:35`),
/// `_OFFSET_1 0x118df8` (`:41`), written at `timer_gh100.c:89-91` with `_UPDATE_TRIGGER` in `_0`.
/// (`NV_PGC6_SCI_SEC_TIMER_TIME_0/1 0x118f54/0x118f58`, `:27,31`, are the READ side.) ⊘ No PLM:
/// the GH100 body has no privilege test.
pub const TIMER_GH100: TimerRegs = TimerRegs {
    hal: TimerHal::Gh100,
    refused_writes: &[0x118df4, 0x118df8],
    priv_level_mask: None,
};

/// **Integrated Blackwell (GB10B/GB20B/GB20C).** `tmrSetCurrentTime_GB10B` (`timer_gb10b.c:46-73`)
/// reads the VF time pair and keeps `sysTimerOffsetNs` in software — **no register is written**.
pub const TIMER_GB10B: TimerRegs = TimerRegs {
    hal: TimerHal::Gb10b,
    refused_writes: &[],
    priv_level_mask: None,
};

/// Which HAL a family's **discrete** parts take. ⊘ Per large family, from the dispatch masks in
/// `g_objtmr_nvoc.c:429-445`. The integrated Blackwell SoC parts take [`TIMER_GB10B`]; a caller
/// that knows it has one selects that directly rather than this file growing a per-die axis.
pub fn timer_regs_for(f: crate::classgen::Family) -> TimerRegs {
    use crate::classgen::Family::*;
    match f {
        Turing | Ampere | Ada => TIMER_GV100,
        Hopper | Blackwell => TIMER_GH100,
    }
}

impl TimerRegs {
    /// ★ THE WRITE-SIDE DECISION. `true` ⇒ the trap returns `Action::RefusedByName`.
    #[inline]
    pub fn is_refused_write(&self, bar0_offset: u32) -> bool {
        self.refused_writes.contains(&bar0_offset)
    }

    /// What the PLM shadow must hold so ogkm takes its success branch: `(offset, value)`.
    pub fn plm_shadow(&self) -> Option<(u32, u32)> {
        self.priv_level_mask.map(|off| (off, PLM_WRITE_PROTECTION_LEVEL0_ENABLE))
    }
}

/// The written-down set.
#[derive(Debug)]
pub struct ReadTrapSet {
    pages: Vec<ReadTrapPage>,
    bar0_pages: u32,
}

impl Default for ReadTrapSet {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadTrapSet {
    /// The GA106 default. See [`GA106_BAR0_BYTES`] for where the real value comes from.
    pub fn new() -> ReadTrapSet {
        Self::with_bar0_bytes(GA106_BAR0_BYTES)
    }

    /// ⊘ `bar0_bytes` is the value `NV_ESC_CARD_INFO.reg_size` reports for this device.
    pub fn with_bar0_bytes(bar0_bytes: u32) -> ReadTrapSet {
        ReadTrapSet { pages: Vec::new(), bar0_pages: bar0_bytes >> 12 }
    }

    #[inline]
    pub fn bar0_pages(&self) -> u32 {
        self.bar0_pages
    }

    /// ⚠ Panics if `page` lies outside BAR0. The set is built at device construction, never on
    /// the vCPU path, and an entry naming a page the BAR does not have is a programming error that
    /// must not become a silently-ignored line — *put the check where the bound lives.*
    pub fn add(&mut self, page: u32, reason: ReadReason) -> &mut Self {
        assert!(
            page < self.bar0_pages,
            "read-trap page {page:#x} is outside BAR0 ({} pages)",
            self.bar0_pages
        );
        self.pages.push(ReadTrapPage { page, reason });
        self
    }

    /// ★ THE DECISION. A page's policy is a function of the **phase**.
    pub fn policy(&self, page: u32, off: u32, phase: Phase) -> ReadPolicy {
        let _ = off;
        match self.pages.iter().find(|p| p.page == page) {
            None => ReadPolicy::FromShadow,
            Some(p) => match (p.reason, phase) {
                // ⊘ A latch never resolves from DRAM, in any phase.
                (ReadReason::ResolvesThroughLatch, _) => ReadPolicy::Trap(p.reason),
                (ReadReason::BootStateMachine, Phase::Boot) => ReadPolicy::Trap(p.reason),
                // ★★★ DROPPED. This single arm is the 2.5× on LLM decode.
                (ReadReason::BootStateMachine, Phase::Runtime) => ReadPolicy::FromShadow,
            },
        }
    }

    /// ⚠ §5 puts a number on the set: *"roughly 524 of 4096"*. A set that has silently grown
    /// toward "trap everything" has given up the property the whole design rests on, so the size
    /// is asserted rather than assumed.
    pub fn trapped_at(&self, phase: Phase) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(self.policy(p.page, 0, phase), ReadPolicy::Trap(_)))
            .count()
    }
}
