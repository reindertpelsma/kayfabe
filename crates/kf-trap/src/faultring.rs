//! ★★★ **The guest's replayable fault buffer, as the registers show it** — `MMU_FAULT_BUFFER_GET`
//! and `PUT` of the replayable buffer, and the one rule that derives the interrupt from them.
//! (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.2, §3.3.)
//!
//! ## Who writes what
//!
//! - **`GET` is the guest's.** Its nvidia-uvm writes it over BAR0 after consuming entries — twice
//!   per update, the second time with the write-1-to-clear `GETPTR_CORRUPTED`/`OVERFLOW` bits
//!   (`ogkm-580: uvm_volta_fault_buffer.c` `uvm_hal_volta_fault_buffer_write_get`). The vCPU
//!   applies it HERE, synchronously and lock-free ([`FaultRing::write_get`]).
//! - **`PUT` is ours.** The delivery thread advances it after an entry is fully written
//!   ([`FaultRing::publish_put`]). A guest write to `PUT` is ignored (the register is `R--4A`).
//!
//! ## The level, and why it cannot be lost
//!
//! Hardware asserts the replayable interrupt while `GET != PUT`, re-evaluated when `PUT` advances
//! and when `GET` is written — the guest's re-arm sequence ends with a `GET` write *"to force the
//! re-evaluation of the interrupt condition"* (`uvm_turing_fault_buffer.c`
//! `uvm_hal_turing_clear_replayable_faults`). So both evaluation points here do
//! `store(own) → SeqCst → load(other)`: of two racing sides at least one sees the other's store, so
//! a non-empty buffer always ends with a [`Eval::Pending`] from one of them. Two is a duplicate
//! latch, which the interrupt tree absorbs (`the_interrupt_arming_model.md`: over-report, never
//! under-report).
//!
//! ⊘ No lock, no allocation, no syscall: the vCPU path is atomics only (`THE_CONSTRAINTS.md` 4).

use core::sync::atomic::{AtomicU32, Ordering};

/// What an evaluation point concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eval {
    /// `GET == PUT` (or no buffer): nothing pending.
    Empty,
    /// `GET != PUT`: latch the replayable vector.
    Pending,
}

/// Which of the two registers a BAR0 offset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    /// `MMU_FAULT_BUFFER_GET(replayable)`.
    Get,
    /// `MMU_FAULT_BUFFER_PUT(replayable)`.
    Put,
}

/// ★ The two registers of the replayable buffer and its capacity.
#[derive(Debug)]
pub struct FaultRing {
    get_off: u64,
    put_off: u64,
    ptr_mask: u32,
    /// Entries in the registered buffer (`size / 32`); 0 = none registered.
    entries: AtomicU32,
    get: AtomicU32,
    put: AtomicU32,
}

impl FaultRing {
    /// The ring whose `GET`/`PUT` sit at BAR0 `get_off`/`put_off` (absolute), `PTR` masked by
    /// `ptr_mask` — all three resolved per family (`kf_chip::fault::FaultConsts`).
    #[must_use]
    pub const fn new(get_off: u64, put_off: u64, ptr_mask: u32) -> FaultRing {
        FaultRing {
            get_off,
            put_off,
            ptr_mask,
            entries: AtomicU32::new(0),
            get: AtomicU32::new(0),
            put: AtomicU32::new(0),
        }
    }

    /// Which register `off` is, if either.
    #[must_use]
    pub fn decode(&self, off: u64) -> Option<Reg> {
        if off == self.get_off {
            Some(Reg::Get)
        } else if off == self.put_off {
            Some(Reg::Put)
        } else {
            None
        }
    }

    /// `GET`'s BAR0 offset.
    #[must_use]
    pub const fn get_off(&self) -> u64 {
        self.get_off
    }

    /// `PUT`'s BAR0 offset.
    #[must_use]
    pub const fn put_off(&self) -> u64 {
        self.put_off
    }

    /// A buffer of `entries` was registered (or replaced): both cursors restart at 0. ⊘ An
    /// `entries` that does not fit `PTR` is clamped to what the register can count.
    pub fn register(&self, entries: u32) {
        let e = entries.min(self.ptr_mask.saturating_add(1));
        self.entries.store(0, Ordering::SeqCst);
        self.get.store(0, Ordering::SeqCst);
        self.put.store(0, Ordering::SeqCst);
        self.entries.store(e, Ordering::SeqCst);
    }

    /// No buffer: nothing may be delivered, nothing is pending.
    pub fn unregister(&self) {
        self.entries.store(0, Ordering::SeqCst);
    }

    /// Entries in the registered buffer (0 = none).
    #[must_use]
    pub fn entries(&self) -> u32 {
        self.entries.load(Ordering::SeqCst)
    }

    /// The guest's `GET` as last written (masked to `PTR`).
    #[must_use]
    pub fn get(&self) -> u32 {
        self.get.load(Ordering::SeqCst)
    }

    /// Our `PUT`.
    #[must_use]
    pub fn put(&self) -> u32 {
        self.put.load(Ordering::SeqCst)
    }

    /// ★ **vCPU**: the guest wrote `GET`. Returns what the read-back shows (the `PTR` field only —
    /// the high bits are write-1-to-clear of flags this device never sets) and the evaluation.
    pub fn write_get(&self, value: u32) -> (u32, Eval) {
        let g = value & self.ptr_mask;
        self.get.store(g, Ordering::SeqCst);
        (g, self.evaluate())
    }

    /// ★ **Delivery thread**: `PUT` moves to `new_put` (the entries before it are fully written
    /// and `VALID`). Returns the evaluation.
    pub fn publish_put(&self, new_put: u32) -> Eval {
        self.put.store(new_put & self.ptr_mask, Ordering::SeqCst);
        self.evaluate()
    }

    /// `GET != PUT` with a buffer registered.
    #[must_use]
    pub fn evaluate(&self) -> Eval {
        let (g, p) = (
            self.get.load(Ordering::SeqCst),
            self.put.load(Ordering::SeqCst),
        );
        if self.entries.load(Ordering::SeqCst) != 0 && g != p {
            Eval::Pending
        } else {
            Eval::Empty
        }
    }

    /// How many entries may be written at `PUT` now without overwriting one the guest has not
    /// consumed: `entries − 1 − used` (one slot stays empty so full ≠ empty). ⊘ A `GET` outside
    /// the ring (a hostile or corrupt write) means NO room — delivery stalls, only for this guest,
    /// bounded by the host's own timeout.
    #[must_use]
    pub fn room(&self) -> u32 {
        let e = self.entries.load(Ordering::SeqCst);
        let (g, p) = (
            self.get.load(Ordering::SeqCst),
            self.put.load(Ordering::SeqCst),
        );
        if e == 0 || g >= e || p >= e {
            return 0;
        }
        let used = (p + e - g) % e;
        e - 1 - used
    }

    /// The index after `i`, wrapping at the ring's size (`None` with no buffer).
    #[must_use]
    pub fn next(&self, i: u32) -> Option<u32> {
        let e = self.entries.load(Ordering::SeqCst);
        (e != 0).then(|| (i + 1) % e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const PRIV: u64 = 0xB8_0000;

    fn ring() -> FaultRing {
        FaultRing::new(PRIV + 0x3028, PRIV + 0x302c, 0xF_FFFF)
    }

    #[test]
    fn it_decodes_only_its_two_registers() {
        let r = ring();
        assert_eq!(r.decode(PRIV + 0x3028), Some(Reg::Get));
        assert_eq!(r.decode(PRIV + 0x302c), Some(Reg::Put));
        assert_eq!(
            r.decode(PRIV + 0x3008),
            None,
            "GET(0) is the other buffer's"
        );
        assert_eq!(r.decode(PRIV + 0x3070), None);
    }

    #[test]
    fn unregistered_is_never_pending_and_has_no_room() {
        let r = ring();
        assert_eq!(r.publish_put(3), Eval::Empty);
        assert_eq!(r.room(), 0);
        assert_eq!(r.next(0), None);
    }

    /// Capacity: one slot stays empty; wrap at the ring size.
    #[test]
    fn capacity_and_wrap() {
        let r = ring();
        r.register(4);
        assert_eq!(r.room(), 3);
        assert_eq!(r.publish_put(3), Eval::Pending);
        assert_eq!(r.room(), 0, "full: put + 1 == get (mod 4)");
        assert_eq!(r.write_get(2).1, Eval::Pending, "one still unconsumed");
        assert_eq!(r.room(), 2);
        assert_eq!(r.next(3), Some(0), "wraps");
        assert_eq!(r.write_get(3).1, Eval::Empty, "caught up");
        assert_eq!(r.room(), 3);
    }

    /// The guest's second `GET` write carries the W1C bits: the read-back is the pointer only.
    #[test]
    fn the_clear_bits_do_not_stick() {
        let r = ring();
        r.register(8);
        r.publish_put(5);
        let (shown, ev) = r.write_get(5 | (1 << 30) | (1 << 31));
        assert_eq!(shown, 5);
        assert_eq!(ev, Eval::Empty);
        assert_eq!(r.get(), 5);
    }

    /// ⊘ A hostile `GET` outside the ring stalls delivery and still evaluates honestly.
    #[test]
    fn a_hostile_get_stalls_only_delivery() {
        let r = ring();
        r.register(8);
        r.publish_put(2);
        assert_eq!(r.write_get(100).1, Eval::Pending, "100 != 2");
        assert_eq!(r.room(), 0, "no delivery against a GET outside the ring");
    }

    /// A re-registration restarts both cursors.
    #[test]
    fn a_reregistration_restarts() {
        let r = ring();
        r.register(8);
        r.publish_put(5);
        r.register(16);
        assert_eq!((r.get(), r.put(), r.entries()), (0, 0, 16));
        assert_eq!(r.evaluate(), Eval::Empty);
    }

    /// ★ The Dekker property on real threads: a guest thread consuming and a delivery thread
    /// producing, each evaluating after its own store. Whenever the final state is non-empty, at
    /// least one of them must have reported `Pending` after the LAST state change — here: after
    /// every round, if `get != put` then someone saw it. Run many rounds.
    #[test]
    fn a_racing_get_and_put_never_lose_the_level() {
        for round in 0..2000u32 {
            let r = Arc::new(ring());
            r.register(1024);
            r.publish_put(round % 7);
            let a = Arc::clone(&r);
            let b = Arc::clone(&r);
            let target_put = (round % 7) + 1 + (round % 3);
            let target_get = round % 5;
            let t1 = std::thread::spawn(move || a.publish_put(target_put));
            let t2 = std::thread::spawn(move || b.write_get(target_get).1);
            let (e1, e2) = (t1.join().unwrap(), t2.join().unwrap());
            if target_get != target_put {
                assert!(
                    e1 == Eval::Pending || e2 == Eval::Pending,
                    "round {round}: lost the level (get {target_get} put {target_put})"
                );
            }
        }
    }
}
