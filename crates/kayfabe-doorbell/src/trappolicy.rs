//! Where a trap may be installed at all — the structural rule, above the classifier.
//!
//! ## ⊘⊘⊘ MOST OF THIS WAS ALREADY IN v3. I DID NOT IMPLEMENT IT.
//!
//! `[owner, 2026-09-21]` *"**remember** the no traps for bar1/2 (except doorbell in bar1), and the
//! no read trap everywhere, and write trap allowed in bar0 (not in pramin, but is allowed only if
//! doorbell is mapped in bar1 and then only that page)."* — the word is **remember**, not decide.
//! `THE_DESIGN.md` already said it, in two places:
//!
//! | already stated | where |
//! |---|---|
//! | *"**BAR1 is never trapped**, with exactly one exception: the page the guest maps the usermode object at"* | §5.7 |
//! | BAR2 is *"**mapped, not trapped and not served**"* | §6.5 |
//!
//! ⇒ `trap.rs` took `Class` as a **caller-supplied input** and never derived it from
//! `(bar, offset)`. A review had already flagged exactly that — *"the three-way classifier is not
//! in this crate; nothing derives it"* — and it was left. ⚠ **The rule was not missing from the
//! design; it was missing from the code**, and this file is the implementation catching up.
//!
//! ⊘ **Why the distinction is worth writing down:** filing an unimplemented rule as a fresh
//! decision makes the history say *"the design evolved"* when it actually says *"the code
//! lagged."* The first invites no fix; the second is a defect with an owner.
//!
//! ★ **One part genuinely IS a change**, and only one: §5's **524-page read-trap allowlist** is
//! superseded by *"no read trap everywhere"*. That supersession is folded into §5 itself, above
//! the text it corrects.
//!
//! ## The rule, as a table
//!
//! | region | read | write |
//! |---|---|---|
//! | **BAR0**, except PRAMIN | ⊘ never trapped | ★ **trap allowed** — this is the privileged arm |
//! | **BAR0 / PRAMIN** | ⊘ never | ⊘ **never.** It is a bring-up aperture, not a running path |
//! | **BAR1**, except the doorbell page | ⊘ never | ⊘ **never** |
//! | **BAR1 / the doorbell page** | ⊘ never | ★ allowed **iff** the doorbell is mapped in BAR1 (Hopper+), and **only that one page** |
//! | **BAR2** | ⊘ never | ⊘ **never** |
//!
//! ## ⊘⊘⊘ "NO READ TRAP ANYWHERE" SUPERSEDES §5's READ-TRAP ALLOWLIST
//!
//! `THE_DESIGN.md` §5 says *"roughly **524 of 4096** BAR0 pages must still read-trap — the
//! framebuffer window resolves through a latch, and the firmware-boot pages are state-machine
//! state."* ⚠ **The owner's rule removes that set entirely**, and it is the stronger position:
//!
//! ★ §5 already measured what a read trap costs — *"**99 %** of the C artifact's 1 000–3 000 exits
//! per token were READS of a single firmware debug register … that one page cost a **2.5× loss on
//! LLM decode**."* A read-trap allowlist is a standing invitation to that defect, and it only has
//! to be wrong about **one** page to pay it.
//!
//! ⇒ **The latch case does not need a read exit.** A latch is *set by a write*, and writes are
//! trapped — so the value a read must return can be **computed into the shadow at write time**.
//! §5 already uses exactly this for the timer page (*"a **computed shadow**: constants for the
//! privilege mask and tick frequency"*). Generalising it removes the last reason to exit on a read.
//!
//! ⚠ **What this costs, stated honestly:** every latch-resolved register now needs its shadow
//! recomputed on the write that moves it, and a register whose value changes for a reason we do
//! **not** see (a real hardware counter) cannot be served this way at all. ⊘ That is a real
//! constraint on what we may emulate — and it is better as a design boundary than as a read exit
//! nobody notices until a parity run.
//!
//! ⇒ [`crate::readtrap`] is retained **only** as the phase-scoped record of which pages are
//! *computed* rather than plain — it no longer authorises an exit. See [`may_trap_read`].

use crate::vmm::Bar;

/// BAR0's PRAMIN window. ⊘ §PRAMIN is a bring-up aperture, not a running path: it is how the
/// driver reaches instance memory before the real mappings exist, and trapping it would put a
/// boot-time loop through the privileged ring.
pub const PRAMIN_BASE: u64 = 0x0070_0000;
pub const PRAMIN_LEN: u64 = 0x0010_0000;

/// Where the doorbell lives. ⊘ §5: *"generated per die/arch; **Hopper+ maps it over BAR1**"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorbellPlacement {
    /// Pre-Hopper: the doorbell is a BAR0 register, so BAR1 is never trapped at all.
    Bar0 { offset: u64 },
    /// Hopper+: one 64 KiB page of BAR1. ★ The **only** page of BAR1 that may ever trap.
    Bar1 { page_base: u64 },
}

/// ★★★ May a WRITE at `(bar, offset)` be trapped?
///
/// ⊘ This is a **structural** gate, not a policy: it runs above the three-way classifier, so a
/// classifier bug cannot install a trap the design forbids. The classifier decides *what a trapped
/// write does*; this decides *whether one may exist*.
pub fn may_trap_write(bar: Bar, offset: u64, doorbell: DoorbellPlacement) -> bool {
    match bar.0 {
        0 => {
            // ⊘ PRAMIN is carved out of BAR0's otherwise-trappable space.
            !(PRAMIN_BASE..PRAMIN_BASE + PRAMIN_LEN).contains(&offset)
        }
        1 => match doorbell {
            // ⊘ The doorbell is elsewhere ⇒ BAR1 carries no trap at all.
            DoorbellPlacement::Bar0 { .. } => false,
            // ★ Exactly one page, and only because the doorbell is in it.
            DoorbellPlacement::Bar1 { page_base } => {
                (page_base..page_base + 0x1_0000).contains(&offset)
            }
        },
        // ⊘ BAR2 is mapped, never trapped and never served — §6.2. Trapping it would put a
        // page-table fill through the privileged ring and overflow it on one large map.
        _ => false,
    }
}

/// ★ May a READ at `(bar, offset)` be trapped? ⊘ **Never, anywhere.**
///
/// The function exists so the answer is a *call site* rather than an absence — a reviewer can see
/// that reads were considered and refused, and a future change has somewhere to be argued.
pub fn may_trap_read(_bar: Bar, _offset: u64) -> bool {
    false
}

/// How a read must be satisfied instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSource {
    /// Plain DRAM the guest reads directly. No exit, no code.
    Shadow,
    /// ⊘ A value that resolves through a latch ⇒ the shadow is **recomputed on the trapped write
    /// that moves the latch**, never read-exited.
    ComputedShadow,
}
