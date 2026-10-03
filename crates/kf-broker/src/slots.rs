// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The frame ring** — the display's copy targets, shared by the display worker, QEMU's
//! console and the broker relay through ONE atomic word (`docs/design/V3_DISPLAY.md` §8.3).
//!
//! | bits  | field |
//! |-------|-------|
//! | 0-3   | console front (the frame the console shows) |
//! | 4-7   | console ready (the newest frame the console has not taken) |
//! | 8-11  | broker ready (the newest frame the relay has not taken) |
//! | 12-16 | broker-held mask (frames the broker may still be reading) |
//!
//! ⊘ **One word, not two.** A console word and a broker word read separately race: the worker
//! reads the held set (slot R absent), the relay moves R from ready to held, the worker then
//! reads ready (empty) — R looks free and is overwritten while the broker shows it. With one
//! word every transition is a CAS of the whole occupancy, so the worker's one load is a
//! consistent snapshot.
//!
//! Who moves what:
//! - **worker**: [`FrameRing::fill_target`] (a slot named nowhere in the word), then
//!   [`FrameRing::publish`] — console ready and broker ready both become that slot, dropping an
//!   older ready one nobody took;
//! - **console** (main loop): [`FrameRing::take_console`] — ready becomes front;
//! - **relay** (main loop): [`FrameRing::take_broker`] — broker ready joins the held mask, only
//!   while fewer than [`HELD_CAP`] are held; [`FrameRing::release_held`] on `RELEASE` or a
//!   reclaim; [`FrameRing::requeue`] / [`FrameRing::clear_held`] on disconnect.
//!
//! **The cap of 2 held frames keeps the console fed.** Both ready fields only ever name the LAST
//! published slot, so at most four slots are occupied (front, last published, two held) and with
//! [`BROKER_SLOTS`] = 5 a fill target always exists; [`FrameRing::fill_target`] returning `None`
//! is a counted invariant violation, never a fallback.
//!
//! Slot descriptors ([`SlotFds`], the geometry) are written only while a slot is free — named
//! nowhere in the word, so only the worker can reach it — and read only by a thread that holds
//! the slot through the word. ⊘ **Descriptors are never closed while the ring lives**: each slot
//! keeps every generation of its backing in a `OnceLock` (at most [`GENERATIONS`]), so a stale
//! read can never name a recycled descriptor number (a host-RM fd, the guest-RAM memfd) that
//! would then be sent to a process in the user's session.

use kf_linux_raw::{RawError, SharedRam, fd_inode};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The most slots a ring has (the held mask is 5 bits).
pub const MAX_SLOTS: usize = 5;
/// Slots with the broker on: front, last published, two held, one fill target.
pub const BROKER_SLOTS: usize = 5;
/// Slots with the broker off: today's triple buffer.
pub const CONSOLE_SLOTS: usize = 3;
/// The most frames the broker may hold at once.
pub const HELD_CAP: u32 = 2;
/// Backing generations per slot: the 1080p size, then the largest — a slot grows at most once.
pub const GENERATIONS: usize = 2;
/// "No slot" in a 4-bit field.
pub const NO_SLOT: u32 = 0xF;

const fn front(s: u32) -> u32 {
    s & 0xF
}
const fn console_ready(s: u32) -> u32 {
    (s >> 4) & 0xF
}
const fn broker_ready(s: u32) -> u32 {
    (s >> 8) & 0xF
}
const fn held(s: u32) -> u32 {
    (s >> 12) & 0x1F
}
const fn pack(front: u32, console_ready: u32, broker_ready: u32, held: u32) -> u32 {
    (front & 0xF)
        | ((console_ready & 0xF) << 4)
        | ((broker_ready & 0xF) << 8)
        | ((held & 0x1F) << 12)
}

/// A frame's description as the worker published it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameGeom {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// `DRM_FORMAT_*` of the bytes (the console's pixman format is derived from it by the VMM).
    pub fourcc: u32,
    /// The worker's frame counter.
    pub serial: u64,
}

/// What [`FrameRing::take_broker`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// The slot now held by the broker.
    Taken(usize),
    /// No new frame since the last take.
    Empty,
    /// A frame is ready but [`HELD_CAP`] frames are already held.
    Full,
}

/// ★ One generation of a slot's broker-visible backing: the sealed memfd the frame lives in,
/// and (when `/dev/udmabuf` could be opened) a dma-buf over the same pages, with the identity
/// the broker will name each by (`st_ino`, `nvkvm_broker.c:1487`).
#[derive(Debug)]
pub struct SlotFds {
    memfd: SharedRam,
    memfd_id: u64,
    dmabuf: Option<OwnedFd>,
    dmabuf_id: Option<u64>,
}

impl SlotFds {
    /// Describe a backing, reading each descriptor's identity.
    ///
    /// # Errors
    /// The `fstat` failure; [`RawError::Unsupported`] when the two descriptors share an inode
    /// number (a RELEASE could not tell them apart — the caller recreates the backing).
    pub fn new(memfd: SharedRam, dmabuf: Option<OwnedFd>) -> Result<SlotFds, RawError> {
        let memfd_id = fd_inode(memfd.as_backing_fd())?;
        let dmabuf_id = match &dmabuf {
            Some(d) => Some(fd_inode(d.as_fd())?),
            None => None,
        };
        if dmabuf_id == Some(memfd_id) {
            return Err(RawError::Unsupported {
                what: "a frame whose memfd and dma-buf share an inode number",
                detail: "a broker RELEASE names a buffer by inode; recreate the backing",
            });
        }
        Ok(SlotFds {
            memfd,
            memfd_id,
            dmabuf,
            dmabuf_id,
        })
    }

    /// The memfd (the `F_SHM` rung).
    #[must_use]
    pub fn memfd(&self) -> BorrowedFd<'_> {
        self.memfd.as_backing_fd()
    }

    /// The memfd's identity.
    #[must_use]
    pub fn memfd_id(&self) -> u64 {
        self.memfd_id
    }

    /// The dma-buf, if one was made (the dma-buf rungs).
    #[must_use]
    pub fn dmabuf(&self) -> Option<BorrowedFd<'_>> {
        self.dmabuf.as_ref().map(AsFd::as_fd)
    }

    /// The dma-buf's identity.
    #[must_use]
    pub fn dmabuf_id(&self) -> Option<u64> {
        self.dmabuf_id
    }

    /// The backing's length in bytes (frozen by the memfd's seals).
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.memfd.len_bytes()
    }

    fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        core::iter::once(self.memfd_id).chain(self.dmabuf_id)
    }
}

/// Why [`FrameRing::install`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallRefusal {
    /// The slot is not free (named in the occupancy word): its descriptors may be in use.
    NotFree,
    /// Every generation of the slot is used.
    NoGeneration,
    /// An identity collides with one another slot (or generation) already carries.
    IdCollision(u64),
    /// No such slot.
    NoSlot,
}

/// One slot's published description.
#[derive(Debug, Default)]
struct SlotMeta {
    width: AtomicU32,
    height: AtomicU32,
    stride: AtomicU32,
    fourcc: AtomicU32,
    serial: AtomicU64,
    gens: [OnceLock<SlotFds>; GENERATIONS],
    /// How many generations are installed; the newest is the current one.
    installed: AtomicU32,
}

/// ★ The ring. See the module docs.
#[derive(Debug)]
pub struct FrameRing {
    state: AtomicU32,
    count: usize,
    broker: bool,
    meta: [SlotMeta; MAX_SLOTS],
}

impl FrameRing {
    /// A ring of `count` slots (clamped to 1..=[`MAX_SLOTS`]); `broker` = publishes also make the
    /// frame broker-ready.
    #[must_use]
    pub fn new(count: usize, broker: bool) -> FrameRing {
        FrameRing {
            state: AtomicU32::new(pack(NO_SLOT, NO_SLOT, NO_SLOT, 0)),
            count: count.clamp(1, MAX_SLOTS),
            broker,
            meta: core::array::from_fn(|_| SlotMeta::default()),
        }
    }

    /// Slots in this ring.
    #[must_use]
    pub fn slots(&self) -> usize {
        self.count
    }

    /// Whether publishes feed the broker.
    #[must_use]
    pub fn feeds_broker(&self) -> bool {
        self.broker
    }

    fn occupied(s: u32) -> u32 {
        let mut m = held(s);
        for f in [front(s), console_ready(s), broker_ready(s)] {
            if f != NO_SLOT {
                m |= 1 << f;
            }
        }
        m
    }

    /// ★ **Worker**: a slot named nowhere in the word, other than `inflight` (the worker's own
    /// copy target). `None` violates the cap argument above and is the caller's counted fault.
    #[must_use]
    pub fn fill_target(&self, inflight: Option<usize>) -> Option<usize> {
        let busy = Self::occupied(self.state.load(Ordering::Acquire));
        (0..self.count).find(|&i| busy & (1 << i) == 0 && Some(i) != inflight)
    }

    /// Whether `slot` is named anywhere in the word.
    #[must_use]
    pub fn is_free(&self, slot: usize) -> bool {
        slot < self.count && Self::occupied(self.state.load(Ordering::Acquire)) & (1 << slot) == 0
    }

    /// **Worker**: describe a free slot's next frame (before [`FrameRing::publish`]).
    pub fn describe(&self, slot: usize, g: FrameGeom) {
        let Some(m) = self.meta.get(slot) else { return };
        m.width.store(g.width, Ordering::Release);
        m.height.store(g.height, Ordering::Release);
        m.stride.store(g.stride, Ordering::Release);
        m.fourcc.store(g.fourcc, Ordering::Release);
        m.serial.store(g.serial, Ordering::Release);
    }

    /// ★ **Worker**: install a new backing generation for a FREE slot. The previous one is
    /// retired, never dropped (see the module docs).
    ///
    /// # Errors
    /// [`InstallRefusal`]; the descriptors in `fds` are handed back so the caller can drop
    /// them (they were never visible to another thread) and recreate.
    pub fn install(&self, slot: usize, fds: SlotFds) -> Result<(), (InstallRefusal, SlotFds)> {
        let Some(m) = self.meta.get(slot).filter(|_| slot < self.count) else {
            return Err((InstallRefusal::NoSlot, fds));
        };
        if !self.is_free(slot) {
            return Err((InstallRefusal::NotFree, fds));
        }
        let clash = self.meta[..self.count]
            .iter()
            .flat_map(|other| other.gens.iter().filter_map(OnceLock::get))
            .find_map(|g| fds.ids().find(|id| g.ids().any(|o| o == *id)));
        if let Some(id) = clash {
            return Err((InstallRefusal::IdCollision(id), fds));
        }
        let n = m.installed.load(Ordering::Acquire) as usize;
        let Some(cell) = m.gens.get(n) else {
            return Err((InstallRefusal::NoGeneration, fds));
        };
        if let Err(fds) = cell.set(fds) {
            return Err((InstallRefusal::NoGeneration, fds));
        }
        m.installed.store(n as u32 + 1, Ordering::Release);
        Ok(())
    }

    /// The current backing of `slot` — read only by the slot's holder (the worker for a free
    /// slot, the relay for a held one).
    #[must_use]
    pub fn fds(&self, slot: usize) -> Option<&SlotFds> {
        let m = self.meta.get(slot)?;
        let n = m.installed.load(Ordering::Acquire) as usize;
        n.checked_sub(1)
            .and_then(|i| m.gens.get(i))
            .and_then(OnceLock::get)
    }

    /// The published description of `slot`.
    #[must_use]
    pub fn geometry(&self, slot: usize) -> FrameGeom {
        self.meta
            .get(slot)
            .map_or_else(FrameGeom::default, |m| FrameGeom {
                width: m.width.load(Ordering::Acquire),
                height: m.height.load(Ordering::Acquire),
                stride: m.stride.load(Ordering::Acquire),
                fourcc: m.fourcc.load(Ordering::Acquire),
                serial: m.serial.load(Ordering::Acquire),
            })
    }

    fn update(&self, f: impl Fn(u32) -> Option<u32>) -> Option<(u32, u32)> {
        let mut s = self.state.load(Ordering::Acquire);
        loop {
            let n = f(s)?;
            match self
                .state
                .compare_exchange(s, n, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some((s, n)),
                Err(now) => s = now,
            }
        }
    }

    /// ★ **Worker**: make `slot` the newest frame for the console (and the broker). An older
    /// ready frame nobody took is dropped — latest wins.
    pub fn publish(&self, slot: usize) {
        let j = slot as u32;
        let broker = self.broker;
        let _ = self.update(|s| {
            Some(pack(
                front(s),
                j,
                if broker { j } else { broker_ready(s) },
                held(s),
            ))
        });
    }

    /// ★ **Console**: the newest frame — the ready one becomes the front — or the front again
    /// when nothing is new; `None` before the first.
    #[must_use]
    pub fn take_console(&self) -> Option<usize> {
        match self.update(|s| {
            (console_ready(s) != NO_SLOT)
                .then(|| pack(console_ready(s), NO_SLOT, broker_ready(s), held(s)))
        }) {
            Some((_, n)) => Some(front(n) as usize),
            None => {
                let f = front(self.state.load(Ordering::Acquire));
                (f != NO_SLOT).then_some(f as usize)
            }
        }
    }

    /// ★ **Relay**: take the broker-ready frame into the held set, only while fewer than
    /// [`HELD_CAP`] are held.
    #[must_use]
    pub fn take_broker(&self) -> Take {
        let s = self.state.load(Ordering::Acquire);
        if broker_ready(s) == NO_SLOT {
            return Take::Empty;
        }
        match self.update(|s| {
            let r = broker_ready(s);
            (r != NO_SLOT && held(s).count_ones() < HELD_CAP)
                .then(|| pack(front(s), console_ready(s), NO_SLOT, held(s) | (1 << r)))
        }) {
            Some((was, _)) => Take::Taken(broker_ready(was) as usize),
            None if broker_ready(self.state.load(Ordering::Acquire)) == NO_SLOT => Take::Empty,
            None => Take::Full,
        }
    }

    /// The broker-ready slot, if any (a peek).
    #[must_use]
    pub fn broker_ready(&self) -> Option<usize> {
        let r = broker_ready(self.state.load(Ordering::Acquire));
        (r != NO_SLOT).then_some(r as usize)
    }

    /// The held mask.
    #[must_use]
    pub fn held_mask(&self) -> u32 {
        held(self.state.load(Ordering::Acquire))
    }

    /// ★ **Relay**: the broker no longer reads `slot` (`RELEASE`, or a reclaim). Returns whether
    /// it was held.
    pub fn release_held(&self, slot: usize) -> bool {
        let b = 1u32 << slot;
        self.update(|s| {
            (held(s) & b != 0)
                .then(|| pack(front(s), console_ready(s), broker_ready(s), held(s) & !b))
        })
        .is_some()
    }

    /// **Relay**, on disconnect: move held `slot` back to broker-ready when nothing newer is
    /// there, so the next connection replays it. Returns whether it moved.
    pub fn requeue(&self, slot: usize) -> bool {
        let b = 1u32 << slot;
        self.update(|s| {
            (held(s) & b != 0 && broker_ready(s) == NO_SLOT)
                .then(|| pack(front(s), console_ready(s), slot as u32, held(s) & !b))
        })
        .is_some()
    }

    /// **Relay**, on disconnect: no frame is held any more. Returns the mask that was held.
    pub fn clear_held(&self) -> u32 {
        self.update(|s| {
            (held(s) != 0).then(|| pack(front(s), console_ready(s), broker_ready(s), 0))
        })
        .map_or(0, |(was, _)| held(was))
    }

    /// The raw word, for tests and diagnostics.
    #[must_use]
    pub fn word(&self) -> u32 {
        self.state.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn fds(tag: &std::ffi::CStr) -> SlotFds {
        SlotFds::new(SharedRam::create_named(tag, 4096).expect("memfd"), None).expect("ids")
    }

    #[test]
    fn the_word_packs_four_fields_and_starts_empty() {
        let r = FrameRing::new(BROKER_SLOTS, true);
        assert_eq!(r.word(), pack(NO_SLOT, NO_SLOT, NO_SLOT, 0));
        let w = pack(1, 2, 3, 0b10101);
        assert_eq!(
            (front(w), console_ready(w), broker_ready(w), held(w)),
            (1, 2, 3, 0b10101)
        );
        assert_eq!(r.take_console(), None);
        assert_eq!(r.take_broker(), Take::Empty);
    }

    /// ★ Never fills a slot that is held, ready (either kind) or the console's front.
    #[test]
    fn the_fill_target_is_never_held_ready_or_shown() {
        let r = FrameRing::new(BROKER_SLOTS, true);
        let a = r.fill_target(None).unwrap();
        r.publish(a);
        assert_eq!(r.take_console(), Some(a));
        assert_eq!(
            r.take_broker(),
            Take::Taken(a),
            "front AND held: one slot, two readers"
        );
        let b = r.fill_target(None).unwrap();
        assert_ne!(b, a);
        r.publish(b);
        assert_eq!(r.take_broker(), Take::Taken(b));
        let c = r.fill_target(None).unwrap();
        r.publish(c);
        let d = r.fill_target(Some(c)).unwrap();
        for x in [a, b, c] {
            assert_ne!(d, x);
        }
        let busy = FrameRing::occupied(r.word());
        assert_eq!(busy & (1 << d), 0);
    }

    /// ★ The cap of 2 held frames, and with five slots a fill target ALWAYS exists whatever the
    /// console and the broker do (one copy in flight at a time: the target IS the in-flight one).
    #[test]
    fn with_five_slots_a_fill_target_always_exists_and_the_broker_holds_at_most_two() {
        let r = FrameRing::new(BROKER_SLOTS, true);
        for step in 0..500u32 {
            let t = r.fill_target(None).expect("a fill target must exist");
            r.publish(t);
            if step % 3 == 0 {
                let _ = r.take_console();
            }
            match r.take_broker() {
                Take::Taken(_) | Take::Empty => {}
                Take::Full => assert_eq!(r.held_mask().count_ones(), HELD_CAP),
            }
            assert!(r.held_mask().count_ones() <= HELD_CAP);
            // the worst case: front + last published + 2 held = 4 occupied, one target left
            assert!(
                r.fill_target(None).is_some(),
                "front + last + 2 held + target = 5"
            );
            if step % 7 == 0 {
                // the broker releases its oldest
                let m = r.held_mask();
                if m != 0 {
                    assert!(r.release_held(m.trailing_zeros() as usize));
                }
            }
        }
    }

    #[test]
    fn three_slots_without_the_broker_is_todays_triple_buffer() {
        let r = FrameRing::new(CONSOLE_SLOTS, false);
        let a = r.fill_target(None).unwrap();
        r.publish(a);
        assert_eq!(r.broker_ready(), None, "the broker is off");
        assert_eq!(r.take_broker(), Take::Empty);
        assert_eq!(r.take_console(), Some(a));
        for _ in 0..50 {
            let t = r.fill_target(None).expect("front + ready + one");
            r.publish(t);
            let t2 = r.fill_target(None).expect("three slots");
            r.publish(t2);
        }
        assert_eq!(r.slots(), 3);
    }

    #[test]
    fn requeue_and_clear_on_disconnect() {
        let r = FrameRing::new(BROKER_SLOTS, true);
        let a = r.fill_target(None).unwrap();
        r.publish(a);
        assert_eq!(r.take_broker(), Take::Taken(a));
        let b = r.fill_target(None).unwrap();
        r.publish(b);
        assert_eq!(r.take_broker(), Take::Taken(b));
        assert!(r.requeue(b), "the latest commit becomes the replay frame");
        assert_eq!(r.broker_ready(), Some(b));
        assert!(
            !r.requeue(a),
            "only one replay frame: broker-ready is taken"
        );
        assert_eq!(r.clear_held(), 1 << a);
        assert_eq!(r.held_mask(), 0);
        assert!(!r.release_held(a), "an unheld slot releases nothing");
    }

    /// ★ Distinct identities across every slot and generation; at most two generations; only
    /// while free; never dropped (the old generation stays readable).
    #[test]
    fn installing_backings_checks_freedom_generations_and_identity() {
        let r = FrameRing::new(BROKER_SLOTS, true);
        r.install(0, fds(c"kfb-gen-a")).expect("first");
        let first = r.fds(0).unwrap().memfd_id();
        r.install(0, fds(c"kfb-gen-b")).expect("second (growth)");
        assert_ne!(r.fds(0).unwrap().memfd_id(), first, "the newest is current");
        let (e, _) = r.install(0, fds(c"kfb-gen-c")).unwrap_err();
        assert_eq!(e, InstallRefusal::NoGeneration);
        // a dup of a live memfd has the SAME inode: refused as a collision
        let dup = SharedRam::create_named(c"kfb-dup", 4096).expect("memfd");
        let other = SlotFds::new(dup, None).unwrap();
        let id = other.memfd_id();
        r.install(1, other).expect("slot 1");
        let again = SlotFds {
            memfd: SharedRam::create_named(c"kfb-x", 4096).unwrap(),
            memfd_id: id,
            dmabuf: None,
            dmabuf_id: None,
        };
        let (e, _) = r.install(2, again).unwrap_err();
        assert_eq!(e, InstallRefusal::IdCollision(id));
        // not while held
        r.publish(3);
        let (e, _) = r.install(3, fds(c"kfb-held")).unwrap_err();
        assert_eq!(e, InstallRefusal::NotFree);
        assert_eq!(
            r.install(7, fds(c"kfb-none")).unwrap_err().0,
            InstallRefusal::NoSlot
        );
    }

    /// ★ The TOCTOU case, interleaved for real: a worker picking targets and publishing races a
    /// relay taking and releasing and a console taking. Each reader marks what it holds BEFORE
    /// the worker could see it free again; the worker must never pick a marked slot. With two
    /// separately-read words this fails within a few thousand rounds.
    #[test]
    fn the_worker_never_picks_a_slot_another_thread_holds() {
        let r = Arc::new(FrameRing::new(BROKER_SLOTS, true));
        let owned = Arc::new(AtomicU32::new(0)); // slots the relay or the console holds
        let stop = Arc::new(AtomicBool::new(false));
        let relay = {
            let (r, owned, stop) = (r.clone(), owned.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut mine: Vec<usize> = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    if let Take::Taken(j) = r.take_broker() {
                        owned.fetch_or(1 << (8 + j), Ordering::SeqCst);
                        mine.push(j);
                    }
                    if mine.len() == 2 {
                        let j = mine.remove(0);
                        owned.fetch_and(!(1 << (8 + j)), Ordering::SeqCst);
                        assert!(r.release_held(j));
                    }
                }
            })
        };
        let console = {
            let (r, owned, stop) = (r.clone(), owned.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut shown: Option<usize> = None;
                while !stop.load(Ordering::Relaxed) {
                    // the console marks its next front before taking it: clear the old mark only
                    // after the take moved the front away from it
                    let next = r.take_console();
                    if next != shown {
                        if let Some(n) = next {
                            owned.fetch_or(1 << n, Ordering::SeqCst);
                        }
                        if let Some(o) = shown {
                            owned.fetch_and(!(1 << o), Ordering::SeqCst);
                        }
                        shown = next;
                    }
                }
            })
        };
        for _ in 0..200_000 {
            let t = r.fill_target(None).expect("a target");
            // the relay's mark is set after its CAS, so a slot it took before our load is
            // either still marked or already released — never free-and-marked
            let o = owned.load(Ordering::SeqCst);
            assert_eq!(
                o & (1 << (8 + t)),
                0,
                "the worker picked slot {t} while the broker held it"
            );
            r.publish(t);
        }
        stop.store(true, Ordering::Relaxed);
        relay.join().unwrap();
        console.join().unwrap();
    }
}
