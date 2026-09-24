//! Channels — §7. The routing decision, and the one refusal that is a security boundary.
//!
//! A channel is a pushbuffer, a GPFIFO ring of 8-byte entries, and a small block holding the ring
//! cursors. ★ *"The consumer cursor lives in memory mapped to the owning process and nobody else —
//! which is why hardware can treat a hostile ring as harmless, and why we must too."*
//!
//! | kind | contract |
//! |---|---|
//! | **Passthrough** | ring and return; **may** run on the vCPU. ⊘ We never read a byte of the ring |
//! | **Translated** | every operand rewritten into our VA space, submitted on our host channel. **The GPU moves the bytes** |
//! | **Emulated** | we implement the function |

use kf_trap::token::Route;

/// Who owns the channel inside the guest. ⊘ This is **not** cosmetic: it selects the failure
/// policy in [`Submission::decide`], and getting it wrong is a guest-wide denial of service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// A guest **kernel** channel — the scrubber, UVM's own channels.
    Kernel,
    /// An ordinary unprivileged guest userspace channel.
    User,
}

/// §7: *"Birth is at allocation, never lazy."*
///
/// ⊘ **The reason is measured, not stylistic:** *"RM zeroes a caller-supplied cursor block at
/// allocation, so adopting at first doorbell wipes the cursor that just rang."* ⇒ A lazy birth
/// destroys the very producer cursor whose ring triggered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Birth {
    AtAllocation,
}

/// What to do with a submission whose operands we could not translate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Submit it.
    Submit,
    /// ⊘⊘⊘ **Refuse the submission and poison the device.** Root-visible and honest.
    RefuseAndPoison,
    /// Fault the channel — permitted **only** for a user channel, where the blast radius is the
    /// process that asked.
    FaultChannel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submission {
    pub owner: Owner,
    pub route: Route,
    pub all_operands_translatable: bool,
}

impl Submission {
    /// ★★★ THE DECISION, and the kernel arm is a security boundary rather than a correctness one.
    ///
    /// §7, stated as an absolute:
    ///
    /// > *"An untranslatable operand must never become a deliberate fault on a kernel channel. The
    /// > unified-memory driver treats **any** channel error as **globally fatal** — one fault kills
    /// > CUDA for **every process in the guest** until the driver reloads. ⇒ A design that forwards
    /// > a translation miss as a sentinel fault hands unprivileged guest userspace a way to kill
    /// > the whole guest's GPU stack."*
    ///
    /// ⇒ This is §4's **inner** boundary again: the attacker is an unprivileged guest process, the
    /// victim is every other process in that guest, and the weapon is a *deliberate* fault on a
    /// channel it does not own but can influence.
    pub fn decide(self) -> Disposition {
        if self.all_operands_translatable {
            return Disposition::Submit;
        }
        match self.owner {
            // ⊘ Refuse. Never fault — see above.
            Owner::Kernel => Disposition::RefuseAndPoison,
            // A user channel's fault costs the process that asked for it, which is the correct
            // blast radius and is what hardware would do anyway.
            Owner::User => Disposition::FaultChannel,
        }
    }

    /// §7: *"Kernel channels are TRANSLATED, not emulated. The scrubber and UVM's own channels do
    /// real work on real memory; running them on our CPU is how a guest process reads another's
    /// freed pages."*
    ///
    /// ⊘ This is the rule the w823 measurement found violated seven arms' worth on the old
    /// architecture (`forwarded=0, emulated>0`): the work ran on the CPU, and the content ledger
    /// could not tell because the guest's window and the host object are one memory.
    pub fn kernel_channels_are_never_emulated(owner: Owner, route: Route) -> bool {
        !(owner == Owner::Kernel && route == Route::Emulated)
    }
}

/// §7: *"Sizing and decoding are different jobs."*
///
/// > *"We SIZE every pushbuffer method form so the stream never desynchronises, and DECODE only
/// > what we model. An undefined form decodes to nothing rather than to a guess."*
///
/// ⚠ *"And a method is not a unit of meaning — a copy is five method runs and the launch carries
/// no operands, so only a stateful walk over a run produces a fact."*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded {
    /// We model this form and read these operand words.
    Modelled { operand_words: u8 },
    /// ⊘ We can SIZE it — so the stream stays in sync — but we do not model it. It decodes to
    /// **nothing**, never to a guess.
    SizedOnly,
}

/// A method form's size must always be known, even when its meaning is not.
///
/// ⊘ Returning `None` from a *sizer* would desynchronise the stream, after which every later
/// method is garbage — so sizing is total by construction and only decoding is partial.
pub fn size_is_total(form_known_to_model: bool, words: u8) -> (u8, Decoded) {
    (
        words,
        if form_known_to_model { Decoded::Modelled { operand_words: words } } else { Decoded::SizedOnly },
    )
}

// ---- §46: what the CPU may move, and it is almost nothing --------------------------------------

/// ★★★ **The largest number of bytes the CPU may ever move on a guest's behalf.**
///
/// `[owner w824]` *"confirm that CPU copies are dead and dead from gpu vidmem … anything larger
/// than some trivial integer size from mmio reads then"*.
///
/// ⊘ **Eight bytes: the widest single MMIO access.** A register read or a 64-bit doorbell word is
/// a value we author; anything wider is *data*, and data is the GPU's job (§46). ⇒ The bound is
/// not a performance tuning knob — it is the line between **answering a register** and
/// **doing the engine's work**.
pub const CPU_MOVE_MAX_BYTES: u64 = 8;

/// Why a CPU-side move was refused. ⊘ Refused **by name**, so `refused=0` means nothing was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuMoveRefusal {
    /// ⊘⊘⊘ **§46.** *"The CeUtils scrub genuinely clears memory; kernel CE genuinely moves
    /// bytes."* A scrub whose bytes our CPU wrote is not a scrub that happened on the GPU, and
    /// **the guest cannot tell until something depends on the GPU having done it.**
    ///
    /// ⚠ `[measured w797]` turning the CPU executor off took the 30-arm suite from **18 PASS to
    /// 15** — every one of the six arms that "regressed" had been passing **because our CPU was
    /// doing the GPU's work.** ⇒ The CPU executor is faster to make green, and that is precisely
    /// the hazard: it buys passing arms with a lie the ledger cannot see.
    TooLarge { want: u64, max: u64 },
    /// ★ The operand names **real card vidmem**. Unreachable by construction today — the executor
    /// that could move bytes holds neither the emulated framebuffer nor guest RAM — but named so
    /// that a future executor which *could* reach it is refused rather than silently permitted.
    NamesRealVidmem,
}

impl CpuMoveRefusal {
    pub fn name(&self) -> &'static str {
        match self {
            CpuMoveRefusal::TooLarge { .. } => "cpu_move_too_large",
            CpuMoveRefusal::NamesRealVidmem => "cpu_move_names_real_vidmem",
        }
    }
}

/// ★★★ May the CPU move `len` bytes itself?
///
/// ⊘ **`real_vidmem` is a parameter and not an assertion.** Today no CPU executor can address card
/// vidmem — `CeExecutor::Ours` names only fabricated space, and the isolate holding the real host
/// RM handles refuses it before any ring store. That is a property of *which process holds what*,
/// and process layout is exactly the kind of thing a refactor changes quietly. ⇒ The predicate
/// takes the fact as an input so the answer stays correct if the layout stops being.
pub fn may_cpu_move(len: u64, names_real_vidmem: bool) -> Result<(), CpuMoveRefusal> {
    if names_real_vidmem {
        return Err(CpuMoveRefusal::NamesRealVidmem);
    }
    if len > CPU_MOVE_MAX_BYTES {
        return Err(CpuMoveRefusal::TooLarge { want: len, max: CPU_MOVE_MAX_BYTES });
    }
    Ok(())
}
