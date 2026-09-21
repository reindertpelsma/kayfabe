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
    /// ⊘ Refused by name — see [`is_refused_time_register`].
    RefuseByName,
}

/// BAR0 is 16 MiB of 4 KiB pages.
pub const BAR0_PAGES: u32 = 4096;

/// The timer page, and why it is **not** in the read-trap set.
///
/// §5: *"On Turing and newer the kernel's clock is the pair in the **usermode** window — which is
/// already the read-only memslot over live host time — and the legacy page at the old offset is
/// touched only at boot and resume, never read at runtime."* ⇒ It is a **computed shadow**:
/// constants for the privilege mask and tick frequency.
pub const LEGACY_TIMER_PAGE: u32 = 0x9000 >> 12;

/// ⊘ *"…and the two time-register writes **refused by name**, so guest and host cannot drift onto
/// different timebases."*
///
/// ★ This is a refusal, not a trap: letting the guest **set** time would put it on a different
/// timebase from the host, and every later comparison between them would be silently wrong.
pub fn is_refused_time_register(bar0_offset: u32) -> bool {
    // PTIMER's two settable halves.
    matches!(bar0_offset, 0x9400 | 0x9410)
}

/// The written-down set.
#[derive(Debug, Default)]
pub struct ReadTrapSet {
    pages: Vec<ReadTrapPage>,
}

impl ReadTrapSet {
    pub fn new() -> ReadTrapSet {
        ReadTrapSet::default()
    }
    pub fn add(&mut self, page: u32, reason: ReadReason) -> &mut Self {
        self.pages.push(ReadTrapPage { page, reason });
        self
    }

    /// ★ THE DECISION. A page's policy is a function of the **phase**.
    pub fn policy(&self, page: u32, off: u32, phase: Phase) -> ReadPolicy {
        if is_refused_time_register(off) {
            return ReadPolicy::RefuseByName;
        }
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
