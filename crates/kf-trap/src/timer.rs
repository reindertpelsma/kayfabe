//! Per-family timer registers and the writes we refuse — §50 level 2, from ogkm's timer HALs.
//!
//! ## ⊘⊘⊘ THIS IS NOT A READ-TRAP SET. That module was DELETED, not emptied.
//!
//! `[owner, 2026-09-21]` *"read traps cost code implementation, and then we get that rot back in
//! v3 while my idea was to get it removed. It's useless to implement trap code for something
//! that's intended to not trap — **mapping code is very different from trapping code**."*
//!
//! ★★★ **An argument about MECHANISM, not configuration** — which is why the allowlist had to be
//! removed rather than set empty. A read trap needs a trapped region registration, a read handler,
//! a decode dispatch and a per-register model lookup. Serving from DRAM needs a memslot and a page
//! fill. ⇒ They share no code, so keeping the first "in case" is not a cheap option — it is a
//! second implementation.
//!
//! ⚠ **And once the mechanism exists it becomes the default**, because it is the easy way to
//! answer a register nobody has modelled. `[measured w582, the old tree]` **161 422** BAR0 reads
//! reached the handler; only **138** were unclaimed — the rest were *claimed by a decode arm and
//! trapped anyway*, while *"Goal 2 is zero read traps"* stayed open and no instrument could say
//! which arm was responsible. ⇒ v3 has no read-trap path to reach for.
//!
//! ⊘ I had built one: `readtrap.rs`, with an allowlist, a `Phase`, `ReadPolicy::Trap` and a
//! `trap_read` entry point. **That was the rot returning under a new name.** What survives here is
//! the part that was never about reads: the per-family timer facts, which govern a **write**
//! refusal.

/// (`kernel-open/common/inc/nv-ioctl.h:63`, filled from `nv->regs->size` at `nv.c:2384`, dispatched
/// at `nv.c:2593` under `NV_CTL_DEVICE_ONLY` — no admin check), an unprivileged host ioctl. Until
/// the host binding exists, [`ReadTrapSet::with_bar0_bytes`] takes the value and this is what
/// [`ReadTrapSet::new`] passes it.
pub const GA106_BAR0_BYTES: u32 = 16 << 20;
pub const GA106_BAR0_PAGES: u32 = GA106_BAR0_BYTES >> 12;
/// ⊘ Kept as the GA106 default under its old name so existing bounds tests still say what they
/// said; new code should name the die or take the value from `CARD_INFO`.
pub const BAR0_PAGES: u32 = GA106_BAR0_PAGES;



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
pub fn timer_regs_for(f: kf_chip::Family) -> TimerRegs {
    use kf_chip::Family::*;
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
