//! The vCPU trap path — §5, and it is where §4's **inner** privilege boundary is enforced.
//!
//! ## Only writes trap
//!
//! §5: reads are served from ordinary DRAM the guest reads directly, *"with no exit and no code —
//! because a vCPU inside an MMIO exit is not preemptible, and driver init polls some registers
//! thousands of times."*
//!
//! ## ★★★ The classifier is THREE-WAY, and collapsing it to two is a vulnerability
//!
//! | arm | action |
//! |---|---|
//! | **the doorbell** (4 bytes) | route-dependent; see [`TrapPath::doorbell`] |
//! | **userspace-mappable, not the doorbell** | ⊘ **return, doing nothing** |
//! | **privileged** (guest root only) | shadow-write if readable, append to the ring, bump |
//!
//! §5: *"The middle arm is the one the threat model needs. The doorbell page is 64 KiB and the
//! doorbell is four bytes of it; every other offset on it is guest-userspace-writable, and without
//! this arm an unprivileged process pushes unbounded garbage onto the plane reserved for guest
//! root."*
//!
//! ⇒ The adversary here is **not** the guest kernel. It is a sandboxed process inside the guest,
//! attacking **its own guest's root** (§47). Every decision below is made with that reader in mind.
//!
//! ## What the trap may NOT do (§41, §48, §48.2)
//!
//! No blocking, no allocation, no lock, no unbounded spin, no syscall on the common path. A vCPU
//! in an MMIO exit is not preemptible, so *"spend the latency budget behind the trap, never in
//! it"*. Everything here is a CAS, a store, or a bounded loop over a fixed array.

use crate::bitmap::RungBitmap;
use crate::ring::{PrivRing, Push, RegWrite};
use crate::shadow::WriteSemantics;
use crate::timer::TimerRegs;
use crate::token::{Route, TokenWord};
use crate::wake::{WakeWord, Wake};

/// Where an address falls. Generated per die/arch — §5: *"generated per die/arch; Hopper+ maps it
/// over BAR1"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The doorbell itself. `index.of_doorbell(val)` names the token.
    Doorbell,
    /// On a page guest userspace can map, but not the doorbell. ⊘ **The arm the threat model
    /// needs.**
    UserspaceMappable,
    /// Guest root only.
    Privileged { readable: bool, semantics: WriteSemantics },
}

/// What the caller must do after the trap returns. ⊘ The trap itself performs no syscall; it
/// *says* whether one is owed, so the whole path stays testable with no OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing at all. ★ The overwhelmingly common case, and it costs no exit beyond the write.
    None,
    /// Write the host doorbell **inline, on this vCPU**. No queue, no wake, no lock (§5).
    RingHostInline { host_token: u32 },
    /// Work was published; wake one worker via the worker eventfd.
    WakeWorker,
    /// A privileged write was queued; wake the **drainer** — ⊘ a *different* word and a
    /// *different* eventfd (§5.3), because sharing one with wake-exactly-one lets a register
    /// write wake a worker instead of the drainer.
    WakeDrainer,
    /// ⊘ The privileged ring was full. The device is poisoned; raise a guest-visible fault.
    PoisonDevice,
    /// ⊘ A privileged write REFUSED BY NAME: not queued, not applied to the host, shadow
    /// untouched. Today that is the time-setting registers ([`crate::timer::TimerRegs`]):
    /// §5, *"so guest and host cannot drift onto different timebases."* ★ Distinct from `None`
    /// so the caller can count it — this arm is guest-root-only, so a counter here is not an
    /// adversary-controlled metric the way one on the doorbell arm would be.
    RefusedByName,
}

/// The trap path's immutable wiring.
pub struct TrapPath<'a> {
    pub tokens: &'a [TokenWord],
    pub bits: &'a RungBitmap,
    /// §5.3: two words, signalled by the two arms of the classifier, *"which are disjoint by
    /// construction"*.
    pub worker_wake: &'a WakeWord,
    pub drainer_wake: &'a WakeWord,
    pub ring: &'a PrivRing,
    /// ★ 2026-09-26: doorbell value → table index, per family (`crate::tokenindex`).
    pub index: crate::tokenindex::TokenIndex,
    /// The time-setting registers this device's timer HAL writes — refused by name on the
    /// privileged arm. `[fable w824, HIGH 2]`: the refusal used to sit on the READ path.
    pub timer: TimerRegs,
}

impl TrapPath<'_> {
    // ⊘ There is deliberately NO read path on this type. §5 (superseded w823): no read traps
    // anywhere. A `read()` here "for the hard cases" is exactly how the old tree reached
    // 161 422 read exits — the mechanism gets used because it exists.

    /// ★ THE WHOLE TRAP. Mirrors §5's pseudocode arm for arm.
    pub fn write(&self, class: Class, bar: u8, off: u32, val: u64, width: u8) -> Action {
        match class {
            Class::Doorbell => self.doorbell(val),
            // ⊘⊘⊘ **DO NOTHING.** Not "validate and reject", not "count and drop" — return.
            // Anything that touches shared state here is reachable by an unprivileged guest
            // process at whatever rate it likes.
            Class::UserspaceMappable => Action::None,
            Class::Privileged { readable, semantics } => {
                self.privileged(readable, semantics, bar, off, val, width)
            }
        }
    }

    /// The doorbell arm.
    fn doorbell(&self, val: u64) -> Action {
        // ⊘ MASK, never validate (§5.1). Masking is what the hardware does with undecoded bits,
        // and it removes an error path an adversary could aim at.
        // ★ 2026-09-26: per family (`crate::tokenindex`) — `& mask` through Hopper, runlist|chid on
        // Blackwell; a value naming no slot does nothing.
        let Some(tok) = self.index.of_doorbell(val as u32) else {
            return Action::None;
        };
        let Some(w) = self.tokens.get(tok as usize) else {
            return Action::None;
        };
        let t = w.load();
        match t.route {
            // ★ INLINE, on this vCPU. §5: "No queue, no wake, no lock." The owner's ruling
            // (2026-09-13): "passthrough doorbells are inline in vcpu, no queue, no worker."
            // ⚠ This is the one sanctioned inline store — it is a single MMIO write to a mapped
            // window, not work.
            Route::Passthrough => Action::RingHostInline { host_token: t.host_token },

            // ⊘⊘⊘ **RETURN DOING NOTHING AT ALL — no bit, no bump, no wake.**
            //
            // §5 states the attack: *"Bumping the sequence for an unowned token lets an
            // unprivileged process keep every worker spinning: workers register as polling only
            // if the sequence is unchanged, so a token nobody owns, rung in a loop, prevents them
            // ever parking."*
            //
            // ⇒ A counter here would be a metric an adversary controls, and a *log line* here
            // would be far worse. This arm is deliberately empty.
            Route::Unknown => Action::None,

            Route::Translated | Route::Emulated => {
                // §5.2: stamp and set RUNG in one CAS. Publishes only on IDLE → RUNG; every other
                // transition already has a bit outstanding or is owned by a worker who re-checks.
                if w.ring(self.ring.applied_seq()) {
                    // ⊘ bit THEN summary, inside publish(). Never reachable in the wrong order.
                    self.bits.publish(tok);
                    match self.worker_wake.bump() {
                        Wake::SignalOne => Action::WakeWorker,
                        Wake::NoOne => Action::None,
                    }
                } else {
                    // Already pending. ★ This is the cheap path a spinning guest thread takes,
                    // and it costs one CAS and no wake — which is what makes a tight ring loop
                    // survivable rather than a denial of service.
                    Action::None
                }
            }
        }
    }

    /// The privileged arm — guest root only.
    fn privileged(
        &self,
        readable: bool,
        semantics: WriteSemantics,
        bar: u8,
        off: u32,
        val: u64,
        width: u8,
    ) -> Action {
        // ⊘ §5.5: the shadow is written **synchronously, in the trap, before anything is queued**,
        // because reads come from that page. The sharp case is interrupt masking: the ISR writes
        // the mask then reads pending, and a deferred mask delivers an interrupt the guest
        // already masked.
        //
        // ⚠ The caller owns the shadow store (it holds the page); we report that it is owed by
        // ordering it FIRST here and returning only afterwards.
        let _ = (readable, semantics);

        // ⊘ THE TIMEBASE REFUSAL, on the WRITE path where ogkm actually touches these registers
        // (`timer_gv100.c:71-72`, boot + resume). Never queued, never applied, shadow untouched;
        // the PLM shadow says "level 0 may write" so ogkm's success branch runs and it never
        // asserts. Why not an offset: the guest READS time from a memslot with no exit. See
        // `timer::TimerRegs`.
        if bar == 0 && self.timer.is_refused_write(off) {
            return Action::RefusedByName;
        }

        // ⊘ §5.4: a data port must NEVER enter the ring — Turing/GA100 firmware load is
        // 16 000–65 000 back-to-back writes and would overflow any ring that exists. It is a
        // synchronous store into the falcon image shadow, done by the caller.
        if semantics == WriteSemantics::DataPort {
            return Action::None;
        }

        match self.ring.push(RegWrite { bar, offset: off, value: val, width }) {
            Push::Queued(_) => match self.drainer_wake.bump() {
                Wake::SignalOne => Action::WakeDrainer,
                Wake::NoOne => Action::None,
            },
            // ⊘ §5.4: full ⇒ poison, never wait. A guest-visible fault, because dropping one
            // write silently leaves state a later, differently-privileged guest process inherits.
            Push::Poisoned => Action::PoisonDevice,
            // Already poisoned: dropped by name, and the device is already faulted.
            Push::Dropped => Action::None,
        }
    }
}
