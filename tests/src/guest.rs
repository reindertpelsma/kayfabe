//! ★ The guest side of the harness: a real x86-32 image, and the [`Device`] a real MMIO
//! exit is dispatched into.
//!
//! `l1_os_shell.md` §10 stage M2-d. Everything else in this crate scripts the guest's
//! *protocol*; this module scripts the guest's *instructions*, because the two things a
//! vCPU makes observable (§14.8 F7) can only be observed by a guest that actually runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kayfabe_arch::ids::GpuId;
use kayfabe_rt::device::SharedDevice;
use kayfabe_vmm::{BarId, CoreEvent, Device, Vmm};

/// The doorbell register's offset inside BAR0, in this harness's layout.
pub const DOORBELL_OFF: u64 = 0x100;
/// How far apart consecutive lanes' doorbell registers sit. One register per lane is what
/// lets several vCPUs be distinguished at the exit without several code images.
pub const DOORBELL_STRIDE: u64 = 0x10;
/// A BAR0 offset that is deliberately **not** a doorbell: read it and the device answers
/// with the offset itself, so a wrong-offset dispatch is visible in the value.
pub const ECHO_OFF: u64 = 0x40;

/// What a 32-bit guest load observes when its address reached **nothing**: the exit
/// dispatch completes an unclaimed load with all-ones, as a real machine does, rather than
/// with zero — because zero is a plausible register value and all-ones is the universal
/// "nobody answered".
pub const UNBACKED_SENTINEL: u32 = u32::MAX;

/// ★ The guest image: a 32-bit loop that loads from `probe` and stores what it found
/// through `ebx`, forever.
///
/// ```text
///   0: A1 <imm32>   mov eax, [probe]   ; served from RAM with no exit — or an MMIO load
///   5: 89 03        mov [ebx], eax     ; ALWAYS an MMIO store: `ebx` is a trapped address
///   7: EB F7        jmp  -9            ; back to 0
/// ```
///
/// Two properties earn it its place:
///
/// - **The value is the proof.** What arrives at the store *is* what the memslot behind
///   `probe` holds, so an offset computed wrongly delivers a different number rather than
///   passing silently (§14.8 F7(a)).
/// - **Every iteration traps.** The vCPU is never more than one instruction from returning
///   to us, so a teardown that wants the guest stopped never waits on a clock.
///
/// The probe address is an immediate rather than a constant, because a hard-coded `0x2000`
/// is only page-aligned on a 4 KiB host — the same reason no literal `4096` appears in the
/// raw crate's alignment path.
#[must_use]
pub fn probe_loop_image(probe: u32) -> [u8; 9] {
    let a = probe.to_le_bytes();
    [0xA1, a[0], a[1], a[2], a[3], 0x89, 0x03, 0xEB, 0xF7]
}

/// One doorbell lane: which GPU it targets, the 32-bit cookie the guest must present, and
/// the channel token that cookie stands for.
///
/// ## ★ Why a cookie and not the token itself
///
/// A channel token is 64 bits and the guest image is 32-bit, so a `mov [ebx], eax` cannot
/// carry one. Splitting it into two stores would make the doorbell a two-instruction
/// protocol with a tearing window — a worse thing to model than the problem it solves.
///
/// The cookie is **stronger** than passing the token would have been, which is why it is
/// not a workaround. The value the guest presents is the value we placed in its probe page,
/// so the device can assert that the bytes made the round trip **exactly**: a memslot whose
/// offset arithmetic was wrong delivers a different cookie and is counted as
/// [`DeviceTally::cookie_mismatch`], rather than delivering a plausible-looking token that
/// happens to route somewhere. §14.8 F7(a) asked for "where a memslot points" to be
/// observable; a mismatch counter observes it by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lane {
    /// The GPU this lane's doorbell targets.
    pub gpu: GpuId,
    /// The 32-bit value the guest must present, i.e. what we put in its probe page.
    pub cookie: u32,
    /// The channel token that cookie stands for.
    pub token: u64,
}

/// What the device saw.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeviceTally {
    /// Doorbell writes that rang a real channel through the core.
    pub rang: u64,
    /// Doorbell writes the core refused (a token naming nothing — the MISS = FAULT path).
    pub refused: u64,
    /// ★ Doorbell writes whose cookie was **not** the one we placed in that lane's probe
    /// page **and not the unbacked sentinel**. Nonzero means the bytes the guest read were
    /// not the bytes we wrote — a memslot pointing somewhere other than where the
    /// arithmetic said.
    pub cookie_mismatch: u64,
    /// ★ Doorbell writes carrying [`UNBACKED_SENTINEL`]: the guest's load found no memslot,
    /// took an unclaimed MMIO exit, and was completed with the all-ones value a real
    /// machine returns when nothing answers.
    ///
    /// Counted **separately** from a mismatch, and that separation is the point: "the guest
    /// read the wrong memory" and "the guest read memory that is no longer there" are
    /// different facts, and only the first is a bug. Merging them would make a teardown
    /// look like an addressing error — or, far worse, let an addressing error hide inside
    /// an expected teardown.
    pub unbacked_cookie: u64,
    /// Writes to a BAR offset that is not a doorbell.
    pub stray_writes: u64,
    /// Reads served.
    pub reads: u64,
    /// Deferred/lock-fault/isolate events delivered.
    pub events: u64,
}

/// ★ A real [`Device`], so a real MMIO exit reaches the real core.
///
/// This is the piece that makes the vCPU an *end-to-end* instrument rather than a memory
/// tester: the guest stores a channel token into a trapped BAR register, the exit is
/// classified by the adapter, and the store lands in [`SharedDevice::doorbell`] under the
/// core's own ranked locks — the same entry point every other test in this suite calls
/// directly.
///
/// `&self` throughout, which is the port's own shape (`kayfabe_vmm::Device`'s rustdoc: a
/// `&mut self` MMIO entry point *"forces whole-device exclusivity per call"* and would make
/// the per-`Proc` sharding unreachable). Several vCPU threads enter it at once and the
/// tallies are atomics for exactly that reason.
pub struct DoorbellDevice {
    dev: Arc<SharedDevice>,
    lanes: Vec<Lane>,
    rang: AtomicU64,
    refused: AtomicU64,
    cookie_mismatch: AtomicU64,
    unbacked_cookie: AtomicU64,
    stray_writes: AtomicU64,
    reads: AtomicU64,
    events: AtomicU64,
}

impl DoorbellDevice {
    /// Route lane `i`'s doorbell (BAR0 offset `DOORBELL_OFF + i * DOORBELL_STRIDE`) at
    /// `lanes[i]`.
    #[must_use]
    pub fn new(dev: Arc<SharedDevice>, lanes: Vec<Lane>) -> Self {
        DoorbellDevice {
            dev,
            lanes,
            rang: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            cookie_mismatch: AtomicU64::new(0),
            unbacked_cookie: AtomicU64::new(0),
            stray_writes: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            events: AtomicU64::new(0),
        }
    }

    /// The guest-physical address lane `i`'s doorbell sits at, given BAR0's base.
    #[must_use]
    pub fn lane_gpa(bar0_base: u64, lane: usize) -> u64 {
        bar0_base + DOORBELL_OFF + (lane as u64) * DOORBELL_STRIDE
    }

    /// What the device saw.
    #[must_use]
    pub fn tally(&self) -> DeviceTally {
        DeviceTally {
            rang: self.rang.load(Ordering::SeqCst),
            refused: self.refused.load(Ordering::SeqCst),
            cookie_mismatch: self.cookie_mismatch.load(Ordering::SeqCst),
            unbacked_cookie: self.unbacked_cookie.load(Ordering::SeqCst),
            stray_writes: self.stray_writes.load(Ordering::SeqCst),
            reads: self.reads.load(Ordering::SeqCst),
            events: self.events.load(Ordering::SeqCst),
        }
    }
}

impl core::fmt::Debug for DoorbellDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // `SharedDevice` has no `Debug` and should not grow one for a test helper's sake.
        f.debug_struct("DoorbellDevice")
            .field("lanes", &self.lanes)
            .field("tally", &self.tally())
            .finish_non_exhaustive()
    }
}

impl Device for DoorbellDevice {
    fn mmio_read(&self, _vmm: &mut dyn Vmm, _bar: BarId, off: u64, _size: u8) -> u64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        // The offset itself, so a dispatch that computed the wrong one is visible in the
        // value the guest goes on to store rather than being indistinguishable from zero.
        off
    }

    fn mmio_write(&self, _vmm: &mut dyn Vmm, bar: BarId, off: u64, _size: u8, val: u64) {
        let lane = off
            .checked_sub(DOORBELL_OFF)
            .filter(|d| d % DOORBELL_STRIDE == 0)
            .map(|d| (d / DOORBELL_STRIDE) as usize)
            .filter(|l| *l < self.lanes.len());
        let (Some(lane), BarId::Bar0) = (lane, bar) else {
            self.stray_writes.fetch_add(1, Ordering::SeqCst);
            return;
        };
        let l = self.lanes[lane];
        // ★ The round trip, asserted at the device: the guest presents what we placed in
        // its probe page, or this lane's memslot is not where we think it is.
        #[allow(clippy::cast_possible_truncation)]
        let presented = val as u32;
        if presented != l.cookie {
            if presented == UNBACKED_SENTINEL {
                // The guest's probe page is gone: its load took an unclaimed exit and was
                // completed with all-ones. Expected during a teardown, and never a
                // doorbell.
                self.unbacked_cookie.fetch_add(1, Ordering::SeqCst);
            } else {
                self.cookie_mismatch.fetch_add(1, Ordering::SeqCst);
            }
            return;
        }
        // ★ The whole point: a guest store, arriving at the core's own gated ring path.
        match self.dev.doorbell(l.gpu, l.token, &[]) {
            Ok(_) => self.rang.fetch_add(1, Ordering::SeqCst),
            // A token that names nothing is the core's MISS = FAULT answer, not an error
            // to surface into the guest: a real doorbell write has no return value.
            Err(_) => self.refused.fetch_add(1, Ordering::SeqCst),
        };
    }

    fn event(&self, _vmm: &mut dyn Vmm, _ev: CoreEvent) {
        self.events.fetch_add(1, Ordering::SeqCst);
    }
}
