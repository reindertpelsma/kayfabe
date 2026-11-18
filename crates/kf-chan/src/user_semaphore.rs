//! ★ 2026-11-18 — **Stage 1 of a Blackwell-capable private ring** (`docs/design/V3_BLACKWELL_USER_RING_ROUTE.md`
//! §2, option O1): the relay's progress from a SEMAPHORE-RELEASE marker on a family whose engine writes no
//! `USERD` word back (`kf_chip::Family::engine_writes_userd_gp_get`).
//!
//! GPU-free and not yet wired: no production path calls this module, and the Windows user-work route stays
//! declined on Blackwell (`kf-qemu` `userrelay::ring_relay_serves`) until a Blackwell box proves it. Families
//! that write `USERD` keep [`ProgressSource::EngineUserd`], whose marker cell is the existing
//! [`crate::user_storage::host_marker_words`] byte for byte.
//!
//! The marker cell on Blackwell is exactly the sequence UVM itself emits on that channel class
//! (`BLACKWELL_CHANNEL_GPFIFO_B` inherits `_A`, which inherits Hopper's host ops:
//! `ogkm-595 kernel-open/nvidia-uvm/uvm_hal.c:198-232`):
//! - the release, `uvm_hal_hopper_host_semaphore_release`, `uvm_hopper_host.c:31-52`:
//!   `NV_PUSH_5U(SEM_ADDR_LO, SEM_ADDR_HI, SEM_PAYLOAD_LO, SEM_PAYLOAD_HI 0, SEM_EXECUTE RELEASE | 32BIT |
//!   RELEASE_TIMESTAMP_DIS | RELEASE_WFI_DIS)`;
//! - the wake, Turing's `.interrupt`, `uvm_turing_host.c:79-81`: `NON_STALL_INTERRUPT 0`.
//!
//! `SET_REFERENCE` (`0x50`) is NOT used there: no public Hopper/Blackwell channel header defines it, and
//! UVM never emits it. Whether it is the Xid 32 METHOD of GB205 round B1n is INFERRED (design §2.1).
//!
//! **Hostile guest.** The semaphore word is GPU-writable and lives in the twin's own restricted address
//! space, which the guest's forwarded pushbuffers also execute in. A guest that learns its VA can write it.
//! Nothing but the relay's own markers legitimately writes it. So [`SemaphoreProgress::observe`] accepts only:
//! - `0` before the first marker was seen;
//! - the tagged value of an ISSUED marker, at or after the last one seen (monotonic, extended from 16 bits,
//!   outstanding markers bounded far below the extension's range).
//!
//! Anything else fails the channel closed (sticky). The derived credit can never exceed what was published,
//! and what is outstanding never exceeds the private ring's capacity. No value of the word reaches anything
//! but this channel's own private-ring bookkeeping.

use crate::user_gate::{MARK_TAG, Marks};
use std::collections::VecDeque;

/// Where the relay learns that its marker was processed (and so how far the ring was fetched).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressSource {
    /// The engine writes `USERD.GP_GET` and the marker's `SET_REFERENCE` writes `USERD.REFERENCE`.
    EngineUserd,
    /// The marker releases a 32-bit semaphore into a private word; fetch credit is derived from it.
    SemaphoreMarker,
}

impl ProgressSource {
    /// The family's source: derived from the family's own fact, never from a per-die table.
    #[must_use]
    pub const fn for_family(family: kf_chip::Family) -> Self {
        if family.engine_writes_userd_gp_get() {
            Self::EngineUserd
        } else {
            Self::SemaphoreMarker
        }
    }

    /// Bytes of one marker cell (a multiple of 16, as the private storage lays cells out).
    #[must_use]
    pub const fn cell_bytes(self) -> u64 {
        match self {
            Self::EngineUserd => 16,
            Self::SemaphoreMarker => 32,
        }
    }
}

/// `SEM_EXECUTE`: `OPERATION_RELEASE` (2:0 = 1), `PAYLOAD_SIZE_32BIT` (24:24 = 0), `RELEASE_TIMESTAMP_DIS`
/// (25:25 = 0), `RELEASE_WFI_DIS` (20:20 = 0) — `ogkm-595 clc86f.h:150-161`, `clca6f.h:42-49`.
pub const SEM_EXECUTE_RELEASE_32BIT_NO_WFI: u32 =
    kf_abi::submit::fifo::SEM_EXECUTE_OPERATION_RELEASE;

/// The highest semaphore VA: `SEM_ADDR_HI` `OFFSET` is 24:0 (`clca6f.h:38-39`), so 57 bits.
pub const SEM_VA_LIMIT: u64 = 1 << 57;

/// Pure: the words of one marker cell for `source`. `sem_va` is the private semaphore word's GPU VA
/// (ignored by [`ProgressSource::EngineUserd`]).
///
/// # Errors
/// `USER_SEM_VA` for a semaphore VA that is zero, not 4-byte aligned (`SEM_ADDR_LO` `OFFSET` 31:2) or beyond
/// [`SEM_VA_LIMIT`]; a method that does not encode.
pub fn marker_cell(
    source: ProgressSource,
    value: u32,
    sem_va: u64,
) -> Result<Vec<u32>, &'static str> {
    use kf_abi::submit::{fifo, method_header_inc};
    match source {
        ProgressSource::EngineUserd => {
            crate::user_storage::host_marker_words(value).map(|w| w.to_vec())
        }
        ProgressSource::SemaphoreMarker => {
            if sem_va == 0 || sem_va & 3 != 0 || sem_va >= SEM_VA_LIMIT {
                return Err("USER_SEM_VA");
            }
            let sem = method_header_inc(0, fifo::SEM_ADDR_LO, 5).ok_or("USER_SEM_FORMAT")?;
            let nsi =
                method_header_inc(0, fifo::NON_STALL_INTERRUPT, 1).ok_or("USER_SEM_FORMAT")?;
            #[allow(clippy::cast_possible_truncation)]
            let (lo, hi) = (sem_va as u32, (sem_va >> 32) as u32);
            Ok(vec![
                sem,
                lo,
                hi,
                value,
                0,
                SEM_EXECUTE_RELEASE_32BIT_NO_WFI,
                nsi,
                0,
            ])
        }
    }
}

/// Why a channel's semaphore progress failed closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemFail {
    /// A non-zero word without the marker tag, or `0` after a marker was seen.
    Untagged,
    /// A tagged word naming a marker never issued, or one before the last seen (a regression).
    NotIssuedOrRegressed,
}

/// The relay's view after an observation: what `Io::hw_words` returns on a family with no `USERD` write-back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HwWords {
    /// Derived private `GP_GET`: the ring position just after the last processed marker, modulo capacity.
    pub get: u32,
    /// The tagged `REFERENCE` of the last processed marker ([`Marks::value`]), `0` before the first.
    pub reference: u32,
}

/// Fetch credit and marker proof derived from the private semaphore word, for one private ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaphoreProgress {
    entries: u32,
    /// Private entries published (absolute), markers included.
    published: u64,
    /// Private entries known consumed (absolute): the end of the last processed marker.
    consumed: u64,
    issued: u64,
    seen: u64,
    /// `(marker count, absolute end position)` of markers not yet seen, oldest first.
    outstanding: VecDeque<(u64, u64)>,
    failed: Option<SemFail>,
}

impl SemaphoreProgress {
    /// A ring of `entries` slots (a power of two, at least 2, at most 0x8000 so the 16-bit count extension
    /// in [`Marks::value`] stays unambiguous).
    ///
    /// # Errors
    /// `USER_SEM_RING` for any other capacity.
    pub fn new(entries: u32) -> Result<Self, &'static str> {
        if entries < 2 || !entries.is_power_of_two() || entries > 0x8000 {
            return Err("USER_SEM_RING");
        }
        Ok(Self {
            entries,
            published: 0,
            consumed: 0,
            issued: 0,
            seen: 0,
            outstanding: VecDeque::new(),
            failed: None,
        })
    }

    /// Slots free for publication: one slot always stays empty (`GET == PUT` means empty).
    #[must_use]
    pub fn free(&self) -> u64 {
        u64::from(self.entries) - 1 - (self.published - self.consumed)
    }

    /// Record `n` published entries (guest descriptors, barriers or controls).
    ///
    /// # Errors
    /// `USER_SEM_FULL` when they do not fit (nothing recorded); `USER_SEM_FAILED` after a fail-closed.
    pub fn publish_entries(&mut self, n: u64) -> Result<(), &'static str> {
        self.live()?;
        if n > self.free() {
            return Err("USER_SEM_FULL");
        }
        self.published += n;
        Ok(())
    }

    /// Record one marker entry: returns `(its count, the semaphore value it releases)`.
    ///
    /// # Errors
    /// `USER_SEM_FULL` with no slot free; `USER_SEM_FAILED` after a fail-closed.
    pub fn publish_marker(&mut self) -> Result<(u64, u32), &'static str> {
        self.live()?;
        if self.free() == 0 {
            return Err("USER_SEM_FULL");
        }
        self.published += 1;
        self.issued += 1;
        self.outstanding.push_back((self.issued, self.published));
        Ok((self.issued, Marks::value(self.issued)))
    }

    /// Read the private semaphore `word`: the derived hardware words, or the channel fails closed.
    ///
    /// # Errors
    /// [`SemFail`] (sticky: every later call answers the first failure).
    pub fn observe(&mut self, word: u32) -> Result<HwWords, SemFail> {
        if let Some(f) = self.failed {
            return Err(f);
        }
        if word == 0 && self.seen == 0 {
            return Ok(self.words());
        }
        if word & 0xFFFF_0000 != MARK_TAG {
            return Err(self.fail(SemFail::Untagged));
        }
        // Outstanding markers never exceed `entries` (<= 0x8000), so a regressed 16-bit count wraps to a delta
        // larger than anything outstanding and is refused here.
        let delta = u64::from((word as u16).wrapping_sub(self.seen as u16));
        if delta > self.issued - self.seen {
            return Err(self.fail(SemFail::NotIssuedOrRegressed));
        }
        self.seen += delta;
        while let Some(&(count, end)) = self.outstanding.front() {
            if count > self.seen {
                break;
            }
            self.consumed = end;
            self.outstanding.pop_front();
        }
        Ok(self.words())
    }

    /// Markers processed so far.
    #[must_use]
    pub const fn seen(&self) -> u64 {
        self.seen
    }

    /// The fail-closed reason, if any.
    #[must_use]
    pub const fn failed(&self) -> Option<SemFail> {
        self.failed
    }

    fn words(&self) -> HwWords {
        #[allow(clippy::cast_possible_truncation)]
        let get = (self.consumed % u64::from(self.entries)) as u32;
        HwWords {
            get,
            reference: if self.seen == 0 {
                0
            } else {
                Marks::value(self.seen)
            },
        }
    }

    fn fail(&mut self, f: SemFail) -> SemFail {
        self.failed = Some(f);
        f
    }

    fn live(&self) -> Result<(), &'static str> {
        if self.failed.is_some() {
            Err("USER_SEM_FAILED")
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::fifo;

    fn method_of(header: u32) -> u32 {
        kf_abi::submit::method_header_decode(header)
            .expect("a method header")
            .method
    }

    /// Every family that writes `USERD` back keeps today's marker cell, byte for byte.
    #[test]
    fn userd_families_keep_todays_marker_cell_byte_for_byte() {
        for family in kf_chip::Family::ALL {
            if family == kf_chip::Family::Blackwell {
                continue;
            }
            let source = ProgressSource::for_family(family);
            assert_eq!(source, ProgressSource::EngineUserd, "{family:?}");
            assert_eq!(source.cell_bytes(), 16);
            for value in [0, 1, Marks::value(1), Marks::value(0xFFFF), u32::MAX] {
                let want = crate::user_storage::host_marker_words(value)
                    .unwrap()
                    .to_vec();
                assert_eq!(marker_cell(source, value, 0x1234_5000).unwrap(), want);
                assert_eq!(
                    marker_cell(source, value, 0).unwrap(),
                    want,
                    "the VA is ignored"
                );
            }
        }
    }

    #[test]
    fn blackwell_takes_the_semaphore_source() {
        assert_eq!(
            ProgressSource::for_family(kf_chip::Family::Blackwell),
            ProgressSource::SemaphoreMarker
        );
        assert_eq!(ProgressSource::SemaphoreMarker.cell_bytes(), 32);
    }

    /// The Blackwell cell is UVM's own release + interrupt: SEM_ADDR_LO x5 (incrementing), then
    /// NON_STALL_INTERRUPT; no SET_REFERENCE anywhere.
    #[test]
    fn the_semaphore_cell_is_uvms_release_then_interrupt() {
        let va = 0x0001_2345_6789_abc4_u64;
        let cell = marker_cell(ProgressSource::SemaphoreMarker, Marks::value(7), va).unwrap();
        assert_eq!(cell.len() * 4, 32);
        let h = kf_abi::submit::method_header_decode(cell[0]).unwrap();
        assert_eq!((h.method, h.arg_words), (fifo::SEM_ADDR_LO, 5));
        assert_eq!(h.form, kf_abi::submit::MethodForm::Incrementing);
        assert_eq!(
            cell[1], 0x6789_abc4,
            "SEM_ADDR_LO: the low word, bits 1:0 zero"
        );
        assert_eq!(cell[2], 0x0001_2345, "SEM_ADDR_HI: the high word");
        assert_eq!(cell[3], Marks::value(7), "SEM_PAYLOAD_LO: the marker value");
        assert_eq!(cell[4], 0, "SEM_PAYLOAD_HI");
        assert_eq!(
            cell[5], 1,
            "SEM_EXECUTE: RELEASE, 32-bit, no timestamp, no WFI"
        );
        assert_eq!(method_of(cell[6]), fifo::NON_STALL_INTERRUPT);
        assert_eq!(cell[7], 0);
        assert!(
            [cell[0], cell[6]]
                .into_iter()
                .filter_map(kf_abi::submit::method_header_decode)
                .all(|h| h.method != fifo::SET_REFERENCE_C56F)
        );
    }

    #[test]
    fn the_semaphore_va_is_bounded() {
        for bad in [0, 2, 0x1001, SEM_VA_LIMIT, 0xFFFF_FFFF_FFFF_FFFC] {
            assert_eq!(
                marker_cell(ProgressSource::SemaphoreMarker, 1, bad),
                Err("USER_SEM_VA"),
                "{bad:#x}"
            );
        }
        assert!(marker_cell(ProgressSource::SemaphoreMarker, 1, SEM_VA_LIMIT - 4).is_ok());
    }

    #[test]
    fn the_ring_capacity_is_bounded() {
        for bad in [0, 1, 3, 0x1_0000] {
            assert_eq!(SemaphoreProgress::new(bad), Err("USER_SEM_RING"));
        }
        assert!(SemaphoreProgress::new(0x8000).is_ok());
    }

    /// The credit is the end of the last processed marker, and only that.
    #[test]
    fn credit_follows_processed_markers_only() {
        let mut p = SemaphoreProgress::new(16).unwrap();
        assert_eq!(
            p.observe(0),
            Ok(HwWords {
                get: 0,
                reference: 0
            })
        );
        p.publish_entries(2).unwrap();
        let (n1, v1) = p.publish_marker().unwrap();
        p.publish_entries(4).unwrap();
        let (n2, v2) = p.publish_marker().unwrap();
        assert_eq!((n1, n2), (1, 2));
        assert_eq!(
            p.observe(0),
            Ok(HwWords {
                get: 0,
                reference: 0
            }),
            "nothing processed yet"
        );
        assert_eq!(
            p.observe(v1),
            Ok(HwWords {
                get: 3,
                reference: v1
            })
        );
        assert_eq!(
            p.observe(v1),
            Ok(HwWords {
                get: 3,
                reference: v1
            }),
            "a repeat is no progress"
        );
        assert_eq!(
            p.observe(v2),
            Ok(HwWords {
                get: 8,
                reference: v2
            })
        );
        assert_eq!(p.seen(), 2);
    }

    /// Outstanding work never exceeds capacity - 1; the credit never passes what was published.
    #[test]
    fn publication_is_bounded_by_the_derived_credit() {
        let mut p = SemaphoreProgress::new(8).unwrap();
        p.publish_entries(6).unwrap();
        let (_, v) = p.publish_marker().unwrap();
        assert_eq!(p.free(), 0);
        assert_eq!(p.publish_entries(1), Err("USER_SEM_FULL"));
        assert_eq!(p.publish_marker(), Err("USER_SEM_FULL"));
        assert_eq!(p.observe(v).unwrap().get, 7);
        assert_eq!(p.free(), 7);
        // Wraps modulo capacity.
        p.publish_entries(6).unwrap();
        let (_, v) = p.publish_marker().unwrap();
        assert_eq!(p.observe(v).unwrap().get, 14 % 8);
    }

    /// A forged untagged word fails the channel closed, sticky, and stops publication.
    #[test]
    fn an_untagged_word_fails_closed() {
        let mut p = SemaphoreProgress::new(16).unwrap();
        p.publish_marker().unwrap();
        assert_eq!(p.observe(0xdead_beef), Err(SemFail::Untagged));
        assert_eq!(p.observe(Marks::value(1)), Err(SemFail::Untagged), "sticky");
        assert_eq!(p.publish_entries(1), Err("USER_SEM_FAILED"));
        assert_eq!(p.publish_marker(), Err("USER_SEM_FAILED"));
        assert_eq!(p.failed(), Some(SemFail::Untagged));
    }

    /// Zero after a marker was seen is a regression to "nothing processed": refused.
    #[test]
    fn zero_after_progress_fails_closed() {
        let mut p = SemaphoreProgress::new(16).unwrap();
        let (_, v) = p.publish_marker().unwrap();
        p.observe(v).unwrap();
        assert_eq!(p.observe(0), Err(SemFail::Untagged));
    }

    /// A tagged value for a marker never issued (forged ahead) cannot grant credit.
    #[test]
    fn a_marker_not_issued_fails_closed() {
        let mut p = SemaphoreProgress::new(16).unwrap();
        p.publish_entries(3).unwrap();
        p.publish_marker().unwrap();
        assert_eq!(
            p.observe(Marks::value(2)),
            Err(SemFail::NotIssuedOrRegressed)
        );
        let mut p = SemaphoreProgress::new(16).unwrap();
        assert_eq!(
            p.observe(Marks::value(1)),
            Err(SemFail::NotIssuedOrRegressed),
            "none issued"
        );
    }

    /// A tagged value older than the last seen marker (a regression) fails closed.
    #[test]
    fn a_regressing_marker_fails_closed() {
        let mut p = SemaphoreProgress::new(16).unwrap();
        let (_, v1) = p.publish_marker().unwrap();
        let (_, v2) = p.publish_marker().unwrap();
        p.observe(v2).unwrap();
        assert_eq!(p.observe(v1), Err(SemFail::NotIssuedOrRegressed));
        assert_eq!(p.observe(v2), Err(SemFail::NotIssuedOrRegressed), "sticky");
    }

    /// One slot always stays empty: a ring of 2 holds one entry.
    #[test]
    fn a_full_ring_keeps_one_slot_empty() {
        let mut p = SemaphoreProgress::new(2).unwrap();
        assert_eq!(p.free(), 1);
        p.publish_entries(1).unwrap();
        assert_eq!(p.publish_marker(), Err("USER_SEM_FULL"));
        assert_eq!(p.publish_entries(1), Err("USER_SEM_FULL"));
    }

    /// Each cell is a well-formed method stream that fills exactly its cell: every header's arguments are
    /// inside the cell, and nothing is left over.
    #[test]
    fn the_cells_are_well_formed_method_streams() {
        for source in [ProgressSource::EngineUserd, ProgressSource::SemaphoreMarker] {
            let cell = marker_cell(source, Marks::value(3), 0x10_0000).unwrap();
            assert_eq!(cell.len() as u64 * 4, source.cell_bytes(), "{source:?}");
            let mut i = 0;
            let mut methods = Vec::new();
            while i < cell.len() {
                let h = kf_abi::submit::method_header_decode(cell[i]).expect("a header");
                methods.push((h.method, h.arg_words));
                i += 1 + h.arg_words;
            }
            assert_eq!(
                i,
                cell.len(),
                "{source:?}: the last header's arguments end the cell"
            );
            let want = match source {
                ProgressSource::EngineUserd => {
                    vec![
                        (fifo::SET_REFERENCE_C56F, 1),
                        (fifo::NON_STALL_INTERRUPT, 1),
                    ]
                }
                ProgressSource::SemaphoreMarker => {
                    vec![(fifo::SEM_ADDR_LO, 5), (fifo::NON_STALL_INTERRUPT, 1)]
                }
            };
            assert_eq!(methods, want, "{source:?}");
        }
    }

    /// The 16-bit count extends across its wrap for a long-lived ring.
    #[test]
    fn the_count_extends_across_the_16_bit_wrap() {
        let mut p = SemaphoreProgress::new(4).unwrap();
        for i in 1..=0x1_0005_u64 {
            let (n, v) = p.publish_marker().unwrap();
            assert_eq!(n, i);
            let w = p.observe(v).unwrap();
            assert_eq!(w.reference, v);
            assert_eq!(p.seen(), i);
        }
    }
}
