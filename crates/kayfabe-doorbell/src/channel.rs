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

use crate::token::Route;

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
