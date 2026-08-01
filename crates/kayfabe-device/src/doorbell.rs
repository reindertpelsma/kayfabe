//! ★★★ **The usermode doorbell aperture — the transport, and the port it hands to**
//! (`docs/design/execution_plane_increments.md` increment **E2**).
//!
//! A guest rings a channel by storing its **work-submit token** into one 32-bit register:
//! `NV_VIRTUAL_FUNCTION_DOORBELL`, which RM writes with
//! `GPU_VREG_WR32(pGpu, NV_VIRTUAL_FUNCTION_DOORBELL, workSubmitToken)`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c:162`,
//! `kfifoUpdateUsermodeDoorbell_GA100`). Guest **userspace** rings the same register
//! through the 64 KiB usermode mapping `kfifoConstructUsermodeMemdescs_GV100` hands it,
//! which is the same physical offset in base-address register 0. So there is exactly one
//! offset, and both rings arrive here.
//!
//! ## Why the register plane cannot answer it, and what crosses instead
//!
//! Answering a doorbell means routing a token to a channel, gating its working set and
//! ringing a **host** token on that proc's isolate — `kayfabe_rt::SharedDevice::doorbell`,
//! four crates up the lattice. This crate is Axis-B: it knows *where the register is* and
//! must never learn what a channel is. So what crosses is a **port**: the composition root
//! installs a [`DoorbellPort`], and until it does, the aperture answers a **named
//! refusal** ([`RefusingDoorbell`]) exactly as [`crate::RefusingRam`] and
//! [`crate::RefusingFb`] do for their seams.
//!
//! ⊘ **There is deliberately no "accepted and dropped" answer.** [`DoorbellReport`] is an
//! enum with two arms — the core served it, or the core refused it **by name** — so
//! "counted as a doorbell, went nowhere, looked fine" is unrepresentable. That shape is
//! the one `#146` had to remove from the framebuffer write path after a boot spent a day
//! being diagnosed as something else.
//!
//! ## ⚠ The port is called with NO plane lock held, and that is a requirement
//!
//! `SharedDevice::doorbell` takes the core's ranked locks and can **block** on the isolate
//! pool's backpressure gate. Calling it under [`crate::RegPlane`]'s FSM mutex would put a
//! guest-controlled wait underneath a lock every other vCPU needs to read a register, so
//! the classification in `RegPlane::write` runs **before** that mutex is taken and the
//! port lives outside it (see [`crate::RegPlane::set_doorbell`]).

extern crate alloc;

use alloc::string::String;

use kayfabe_trace::FaultTag;

/// What the core did with one doorbell.
///
/// ★★ **An enum and not a struct with two `Option`s.** A struct could hold neither, or
/// both, and both of those states would be a report a reader could take for success. The
/// enum makes the two answers the only two answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorbellReport {
    /// The core **served** it: a `kayfabe_fwd::DoorbellOutcome` came back.
    ///
    /// The fields are the routing facts the port chose to surface — deliberately small,
    /// because this crate must not learn the core's vocabulary. `proc`/`chan` are the
    /// core's own ids as plain integers; a shell prints them and nothing branches on them.
    Served {
        /// ★ The **guest's** token — the value it stored. Carried on both arms so a report
        /// says which ring it is about without a second field beside it; a log line that
        /// had to pair a report with a token from elsewhere is one that can pair the wrong
        /// two under concurrency.
        token: u64,
        /// The proc the token routed to.
        proc: u32,
        /// The channel it routed to.
        chan: u32,
        /// The **host** token that was rung — not the guest's, which is the whole point of
        /// the translation.
        host_token: u64,
        /// True if this dispatch is what scheduled the channel (its first submission).
        scheduled_now: bool,
    },
    /// The core **refused** it, by name.
    ///
    /// ★ A `kind` and a sentence, which is increment **E1**'s standard applied one seam
    /// over: a check keyed on a word is satisfied by writing the word, and *"the token did
    /// not decode"* and *"the token decoded to a channel nobody allocated"* are different
    /// diagnoses with different fixes.
    Refused {
        /// The guest's token — see [`DoorbellReport::Served::token`].
        token: u64,
        /// Why.
        refusal: DoorbellRefused,
    },
}

/// A doorbell the core would not serve — the fault's stable **kind**, and one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoorbellRefused {
    /// The refusal's stable name, from the fault type's own `Faulted::fault_tag`.
    ///
    /// ★ Not a `String`: the tag comes out of an exhaustive `match` in the crate that owns
    /// the fault enum, so a new fault variant fails that build until it is named. A tag
    /// this crate formatted itself would drift silently instead.
    pub kind: FaultTag,
    /// The refusal, whole — the fault's own payload, formatted. Carries the token, the
    /// decoded vChid or whatever else the variant holds.
    pub why: String,
}

impl DoorbellReport {
    /// Whether the core served this doorbell.
    #[must_use]
    pub fn is_served(&self) -> bool {
        matches!(self, DoorbellReport::Served { .. })
    }

    /// The refusal, if this is one.
    #[must_use]
    pub fn refusal(&self) -> Option<&DoorbellRefused> {
        match self {
            DoorbellReport::Refused { refusal, .. } => Some(refusal),
            DoorbellReport::Served { .. } => None,
        }
    }

    /// The **guest's** token this report is about — present on both arms.
    #[must_use]
    pub fn token(&self) -> u64 {
        match self {
            DoorbellReport::Served { token, .. } | DoorbellReport::Refused { token, .. } => *token,
        }
    }
}

/// ★★★ **The seam a guest doorbell crosses on its way to the core.**
///
/// One method, because a doorbell is one fact: *the guest stored this token*. Everything
/// else — which channel, whose address space, which host token — is the core's to decide
/// and this crate's to stay ignorant of.
///
/// `Send + Sync` because the register plane is driven from every vCPU thread at once.
pub trait DoorbellPort: Send + Sync + core::fmt::Debug {
    /// Ring `token`. Called with **no** register-plane lock held; see the module docs.
    fn ring(&self, token: u64) -> DoorbellReport;
}

/// ★★ **Where the doorbell sits inside the usermode window** — `0x90`, and it is the same
/// `0x90` on every generation from Volta to Blackwell.
///
/// `NV_VIRTUAL_FUNCTION_DOORBELL` is `0x30090` and `DRF_BASE(NV_VIRTUAL_FUNCTION)` is
/// `0x30000` (`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_vm.h:131`,
/// `.../turing/tu102/dev_vm.h:27-28`), so the register is `0x90` bytes into the window
/// whose base the chip **advertises**. The same `0x30090` appears unchanged in
/// `.../blackwell/gb100/dev_vm.h:620` and `.../blackwell/gb202/dev_vm.h:27`, and
/// `kayfabe_arch`'s class table records the class-side twin (`NVC361_NOTIFY_CHANNEL_PENDING
/// = 0x90`, inherited by every `USERMODE` class over the same span).
///
/// `[src]`, not `[measured]`: four headers of the vendor's own tree agreeing across four
/// generations, which is as close as a register offset gets without a bench write.
pub const USERMODE_DOORBELL_OFF: u64 = 0x90;

/// ★★★ **Where this chip's doorbell is, DERIVED from the base the chip advertises** —
/// or `None` if the chip names no usermode register group.
///
/// # Why derived and not a [`crate::ChipProfile`] row
///
/// Because the two numbers are one fact, and a row would let them disagree. The base comes
/// out of [`kayfabe_abi::chipinfo::reg_base::USERMODE`] — the **same table** the emulated
/// GSP answers `gpuGetRegBaseOffset_FWCLIENT` from, i.e. the number this device *told the
/// guest driver* to map its 64 KiB usermode window at. A second, independent row stating
/// where we decode the doorbell could drift from the one we advertised, and the symptom
/// would be a guest storing a token into an offset this port answers with a defaulted zero
/// — a ring that vanished, with a healthy-looking boot around it.
///
/// ⊘ `None` is a **refusal to classify**, not a default offset. A chip that names no
/// usermode group has told the driver it has no such window; inventing one here would
/// decode an address the guest was never given.
#[must_use]
pub fn doorbell_reg(chip: &crate::ChipProfile) -> Option<u64> {
    chip.chip_info
        .reg_bases
        .iter()
        .find(|r| r.index == kayfabe_abi::chipinfo::reg_base::USERMODE)
        .map(|r| u64::from(r.offset) + USERMODE_DOORBELL_OFF)
}

/// The kind [`RefusingDoorbell`] answers with.
///
/// ★ Its own tag rather than a borrowed one from `kayfabe-fwd`: *"this build never
/// installed a forwarding plane"* is a fact about the **composition root**, not a routing
/// fault, and a shell that reported it as `FwdFault::…` would send an operator to read the
/// wrong crate.
pub const NO_DOORBELL_PORT_KIND: FaultTag = FaultTag("Device::NoDoorbellPort");

/// Why [`RefusingDoorbell`] refuses.
pub const NO_DOORBELL_PORT: &str = "the register plane has no doorbell port installed; the shell never called \
     set_doorbell, so a guest work-submit token has nowhere to go — this device counted \
     the ring and forwarded NOTHING, and says so rather than answering the guest as if it \
     had";

/// The port a plane is built with: every doorbell **refused, by name**.
///
/// ⊘ The refusal is the decision, not a placeholder. A default that silently swallowed the
/// token would make "the forwarding plane is absent" and "the forwarding plane is present
/// and lost the ring" the same observation, which is exactly the class of defect increment
/// E1 exists to abolish one seam over.
#[derive(Debug, Clone, Copy, Default)]
pub struct RefusingDoorbell;

impl DoorbellPort for RefusingDoorbell {
    fn ring(&self, token: u64) -> DoorbellReport {
        DoorbellReport::Refused {
            token,
            refusal: DoorbellRefused {
                kind: NO_DOORBELL_PORT_KIND,
                why: String::from(NO_DOORBELL_PORT),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_port_refuses_by_name_and_never_serves() {
        let r = RefusingDoorbell.ring(0x1234);
        assert!(!r.is_served());
        assert_eq!(r.token(), 0x1234);
        let why = r.refusal().expect("the default port refuses");
        assert_eq!(why.kind, NO_DOORBELL_PORT_KIND);
        // ★ The sentence must name the CALL a shell forgot to make, not merely say "no".
        assert!(why.why.contains("set_doorbell"), "{}", why.why);
        assert!(why.why.contains("forwarded NOTHING"), "{}", why.why);
    }

    #[test]
    fn a_report_is_exactly_one_of_the_two_answers() {
        let served = DoorbellReport::Served {
            token: 7,
            proc: 1,
            chan: 2,
            host_token: 3,
            scheduled_now: true,
        };
        assert!(served.is_served());
        assert!(served.refusal().is_none());
        assert_eq!(served.token(), 7);
        // ★ Token ZERO, deliberately: it is a legal work-submit token, so a report about it
        // must still be a report — this is the case a sentinel-valued design gets wrong.
        let refused = RefusingDoorbell.ring(0);
        assert!(!refused.is_served());
        assert!(refused.refusal().is_some());
        assert_eq!(refused.token(), 0);
    }
}
