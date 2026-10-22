//! ★ 2026-10-14 — **when the USER-ring relay may READ a guest GP entry's pushbuffer**
//! (`docs/design/V3_USER_RING_SCAN_RACE.md`).
//!
//! Public `dev_pbdma.ref` (`NV_PPBDMA_GP_ENTRY1_SYNC`): with `SYNC_WAIT`, Host does not begin
//! fetching the entry's pushbuffer segment until the PRECEDING segment has been processed — and
//! for a `LENGTH == 0` entry, "no further GP entries will be processed until the wait finishes".
//! A guest may therefore publish entries behind a `SYNC_WAIT` whose pushbuffers it writes only
//! later, gated by a semaphore acquire in the preceding segment. Measured on Win11 + 580.88
//! (`traces/user_ring_scan_20261014/`): every scan refusal and every stale read sat behind a
//! `SYNC_WAIT` NOP the GPU had not yet reached; the relay had scanned bytes the GPU never fetches
//! (all-zero 88-word segments, filled milliseconds later), refused them, or sized a stale data word
//! as a packet header and lost its place for every later segment.
//!
//! [`SyncGate`] is the hardware's own rule, authored on the relay's private ring: before reading
//! anything at or after a guest `SYNC_WAIT` entry, publish a barrier (`SYNC_WAIT` NOP) and a marker
//! segment, and read on only once the marker has been PROCESSED -- the barrier's wait, i.e. the
//! preceding segment's processing, is then complete. That is the moment the hardware itself would
//! fetch.
//!
//! ★ 2026-10-14 (MEASURED, bare metal RTX 4070 / 595.91.07, `kf-syncwait-stress --families
//! barrier`): the proof is `USERD.REFERENCE` written by a `SET_REFERENCE` method in the marker,
//! NOT `GP_GET`. With an ACQUIRE stalling entry 0 and a SYNC_WAIT barrier behind it, `GP_GET`
//! already covered every published entry, the segment behind the barrier had not executed and
//! `REFERENCE` was still 0 (it became the marker's value when the acquire was satisfied). `GP_GET`
//! counts entries the PBDMA has POPPED from the GPFIFO, which runs ahead of processing; the gate the
//! earlier revision built on "`GP_GET` past the witness" let the relay scan pushbuffers the guest had
//! not yet written (49 of 60 late-bytes plans refused: the scan met the poison). [`Marks`] is that
//! proof's bookkeeping.
//!
//! A SYNC_WAIT entry reached while a method packet is OPEN across it cannot be gated: no witness may
//! enter an open packet (the original Xid-32 PBENTRY), and a control entry has no CPU-visible effect.
//! The hardware does not delay such a segment either (public `dev_pbdma.ref`, `GP_ENTRY1_SYNC`: once
//! the split packet's header is read the preceding segment counts as processed), so the guest's bytes
//! are there when it publishes; the relay reads it at once and counts it (`carried_ungated`).
//!
//! [`Grace`] is defence in depth: a scan refusal of a segment that was never published is re-read and
//! rescanned a bounded NUMBER of times ([`REFUSE_RETRIES`]) before it is final. Measured after the gate:
//! no stale reads at all; the retry exists so a visibility window nobody has measured yet costs a few
//! re-reads, not a dead channel and an adapter reset. It counts re-reads and never consults a clock
//! (2026-10-14, owner: correctness never depends on timing). Neither grants anything; a refusal after
//! the retries is exactly the refusal it was before, and hardware USER privilege enforces isolation.
//!
//! There is deliberately NO `GP_PUT` poll here any more: see [`SyncGate`] and
//! `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` §2.2 (the old `Poll` rested on an overread
//! measurement) -- a doorbell makes every `GP_PUT` write before it visible, and everything else the
//! relay waits for is announced by an event (`kf_chan::user_loop`).

/// How many times the same unpublished segment is re-read and rescanned after its first refusal
/// before the refusal is final. A COUNT of re-reads, never a duration.
pub const REFUSE_RETRIES: u32 = 3;

/// What the relay does with the guest entry at its cursor ([`SyncGate::entry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Read and relay it now.
    Proceed,
    /// Author a barrier + marker on the private ring first, then call [`SyncGate::published`].
    PublishBarrier,
    /// The marker has not been processed yet: read nothing, come back later.
    Wait,
}

/// ★ 2026-10-14 (Win11 Minecraft in-world Xid 32 PBENTRY) — **which witness follows the barrier.**
///
/// Public `dev_pbdma.ref` (`NV_PPBDMA_GP_ENTRY1_SYNC`): "in the case where the final PB entry of
/// the preceding PB segment is a method header representing a PB compressed method sequence of
/// nonzero length — that is, the compressed method sequence is split across PB segments with all
/// of its method data entries in the PB segment for which SYNC_WAIT is set — then Host is
/// considered to have finished processing the preceding PB segment once that method header is
/// read." A packet may therefore cross INTO a guest `SYNC_WAIT` entry, and the PBDMA's parse state
/// (the open packet and the arguments it still expects) carries across every GP entry in between.
/// A marker segment (words) is safe only between packets: with a packet open it is fetched as that
/// packet's next ARGUMENTS, every later guest word shifts, and the packet's last data word is parsed
/// as a header (`NV_PPBDMA_INTR_0_PBENTRY` when it does not decode, a wrong method when it does).
/// A control entry has no byte but no CPU-visible effect either, so it proves nothing: with a packet
/// open there is NO witness, and the entry is not gated ([`Witness::None`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Witness {
    /// A private marker segment `[SET_REFERENCE tag|n, NON_STALL_INTERRUPT, 0]`: processing it
    /// writes `USERD.REFERENCE` ([`Marks`]) and raises the non-stall interrupt the worker's epoll
    /// watches -- only at a packet boundary.
    Marker,
    /// No witness exists (a packet is open across the gated entry): the entry cannot be gated.
    None,
}

/// The witness to author after the barrier, given whether the relay's scan (which has sized every
/// packet of every segment published before the gated entry) still carries an open packet.
#[must_use]
pub const fn witness(carried: bool) -> Witness {
    if carried {
        Witness::None
    } else {
        Witness::Marker
    }
}

/// The tag in the high half of every marker's `REFERENCE` value. The guest may itself execute
/// `SET_REFERENCE` on the private channel (its segments are forwarded unchanged), so only a tagged
/// value counts as a marker; an untagged one is ignored. A guest that forges the tag can only make
/// the relay read ITS OWN late bytes early (pushbuffer integrity is not a security boundary for USER
/// channels, OWNER_RULINGS 2026-10-13).
pub const MARK_TAG: u32 = 0x4B46_0000;

/// Bookkeeping of the relay's reference-bearing markers: `issued` published, `seen` observed
/// processed. The 16-bit count in the value is extended against `seen` (at most `0x7FFF` markers are
/// outstanding: the private ring holds 511 entries).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    issued: u64,
    seen: u64,
}

impl Marks {
    /// Markers published so far.
    #[must_use]
    pub const fn issued(&self) -> u64 {
        self.issued
    }
    /// Markers observed processed.
    #[must_use]
    pub const fn seen(&self) -> u64 {
        self.seen
    }
    /// The `REFERENCE` value marker number `count` writes.
    #[must_use]
    pub const fn value(count: u64) -> u32 {
        MARK_TAG | (count & 0xFFFF) as u32
    }
    /// Reserve the next marker before it is published: `(its count, the value it writes)`.
    ///
    /// # Errors
    /// Too many markers outstanding to extend the 16-bit count unambiguously.
    pub fn issue(&mut self) -> Result<(u64, u32), &'static str> {
        if self.issued - self.seen >= 0x7FFF {
            return Err("USER_MARKS_OUTSTANDING");
        }
        self.issued += 1;
        Ok((self.issued, Self::value(self.issued)))
    }
    /// The absolute count of markers processed that a `REFERENCE` word implies, WITHOUT recording it.
    /// An untagged word (the guest's own `SET_REFERENCE`, or none yet) or one that names a marker
    /// not yet issued implies no progress.
    #[must_use]
    pub fn peek(&self, word: u32) -> u64 {
        if word & 0xFFFF_0000 != MARK_TAG {
            return self.seen;
        }
        let delta = u64::from((word as u16).wrapping_sub(self.seen as u16));
        if delta <= self.issued - self.seen {
            self.seen + delta
        } else {
            self.seen
        }
    }
    /// Record what [`Marks::peek`] implies; returns the count processed.
    pub fn observe(&mut self, word: u32) -> u64 {
        self.seen = self.peek(word);
        self.seen
    }
}

/// Per-channel `SYNC_WAIT` state ([`Gate`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncGate {
    /// Marker count ([`Marks::issued`] after the gate's marker) that must be PROCESSED before reading on.
    witness: Option<u64>,
    /// The guest entry `(ring generation, cursor)` whose wait is already proven.
    cleared: Option<(u64, u32)>,
}

impl SyncGate {
    /// The guest entry `raw` sits at `(generation, cursor)`; `processed` is the number of gate markers
    /// the PBDMA has PROCESSED ([`Marks::seen`]) -- never a `GP_GET` count. Entries without
    /// `SYNC_WAIT` (bit 63, `GP_ENTRY1_SYNC`) always proceed.
    pub fn entry(&mut self, raw: u64, generation: u64, cursor: u32, processed: u64) -> Gate {
        // ★ A proven wait belongs to ONE guest entry (public `dev_pbdma.ref`, `GP_ENTRY1_SYNC`: the
        // hardware waits every time the ring reaches a SYNC_WAIT entry). Once the relay is looking at
        // another `(generation, cursor)` the proof is spent: a ring lap later the same slot holds a NEW
        // entry, and a stale marker would wave it through unread-barrier (the model's lap scenario).
        if self.cleared.is_some_and(|k| k != (generation, cursor)) {
            self.cleared = None;
        }
        if raw >> 63 & 1 == 0 || self.cleared == Some((generation, cursor)) {
            return Gate::Proceed;
        }
        match self.witness {
            None => Gate::PublishBarrier,
            Some(w) if processed >= w => {
                self.witness = None;
                self.cleared = Some((generation, cursor));
                Gate::Proceed
            }
            Some(_) => Gate::Wait,
        }
    }

    /// The barrier and marker are on the private ring; `marks_issued` is [`Marks::issued`] after the
    /// gate's marker.
    pub fn published(&mut self, marks_issued: u64) {
        self.witness = Some(marks_issued);
    }

    /// Whether a witness is outstanding (the caller keeps its worker ticking).
    pub fn waiting(&self) -> bool {
        self.witness.is_some()
    }
}

/// Bounded retry of a scan refusal ([`REFUSE_RETRIES`] re-reads, no clock).
#[derive(Debug, Clone, Copy, Default)]
pub struct Grace {
    /// `(generation, cursor, refusals so far)` of the entry being retried.
    first: Option<(u64, u32, u32)>,
}

impl Grace {
    /// A scan of the unpublished entry `(generation, cursor)` refused: `true` = re-read and rescan
    /// (the caller re-turns at once), `false` = the refusal is final.
    pub fn refused(&mut self, generation: u64, cursor: u32) -> bool {
        match &mut self.first {
            Some((g, c, n)) if *g == generation && *c == cursor => {
                *n += 1;
                *n <= REFUSE_RETRIES
            }
            _ => {
                self.first = Some((generation, cursor, 1));
                true
            }
        }
    }
    /// The entry scanned clean: forget the refusal.
    pub fn clear(&mut self) {
        self.first = None;
    }
    /// Whether a refusal is being retried.
    pub fn retrying(&self) -> bool {
        self.first.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_scan::{Fetch, Refusal, Scan};

    const SYNC_NOP: u64 = 1 << 63 | 1 << 41;
    fn seg(va: u64, words: u64, sync: bool) -> u64 {
        kf_abi::submit::gp_entry(va, words * 4).unwrap() | if sync { 1 << 63 } else { 0 }
    }

    /// ★ The measured failure, as an engine replay. Guest ring (Win11, run 2990 `ringtrace5`,
    /// entries 447-453): `[SYNC NOP, seg A, seg B, SYNC NOP, seg C, seg D]`; segment D's bytes are
    /// written by the guest only after the GPU passes the semaphore acquire in segment C. A relay
    /// that reads D when the guest's PUT names it scans zeros-then-tail and refuses `Legacy`; the
    /// gate makes it wait until the hardware would fetch D.
    #[test]
    fn a_segment_behind_an_unreached_sync_wait_is_not_read_until_the_witness_runs() {
        // D as the GPU will fetch it, and D as the relay read it early (front lines not yet written).
        let filled = {
            let mut w = vec![
                0x8000_0800u32,
                0x8011_0810,
                0xa002_0815,
                0,
                0x0425_2900,
                0x800e_0817,
                0x8016_0813,
                0xa007_0045,
            ];
            w.extend([
                0x340,
                3,
                0x58c0_0000,
                0x71cc_0301,
                3,
                0x18c0_0000,
                0x318c_0300,
            ]);
            w.extend([0x8000_0e6a, 0xe000_0000]);
            w
        };
        let mut early = filled.clone();
        early[..13].fill(0);
        // The old relay read D at once: its scan refuses the tail's data word as a header.
        assert_eq!(
            Scan::default().segment(&early, |_| false),
            Err(Refusal::Legacy)
        );
        assert_eq!(Scan::default().segment(&filled, |_| false), Ok(None));

        // The gated relay over the same ring.
        let ring = [
            SYNC_NOP,
            seg(0x1000, 8, false),
            seg(0x2000, 8, false),
            SYNC_NOP,
        ];
        let mut fetch = Fetch::new(64).unwrap();
        let mut gate = SyncGate::default();
        let mut cursor = 0u32;
        let mut reads_of_d_before_witness = 0;
        let mut witness_fetched = false;
        let mut step = 0;
        while cursor < ring.len() as u32 + 1 && step < 100 {
            step += 1;
            let raw = ring
                .get(cursor as usize)
                .copied()
                .unwrap_or(seg(0x4000, 16, false));
            match gate.entry(raw, 1, cursor, fetch.fetched()) {
                Gate::PublishBarrier => {
                    witness_fetched = false;
                    fetch.publish(None).unwrap();
                    fetch.publish(None).unwrap();
                    gate.published(fetch.issued());
                }
                Gate::Wait => {
                    // the GPU is still in the segment before the barrier (its acquire); release
                    // it after a few polls, as the guest's semaphore release would
                    if step % 3 == 0 {
                        let get = (fetch.issued() & 63) as u32;
                        fetch.observe(get).unwrap();
                        witness_fetched = true;
                    }
                }
                Gate::Proceed => {
                    if cursor == ring.len() as u32 && !witness_fetched {
                        reads_of_d_before_witness += 1;
                    }
                    fetch.publish(Some((1, cursor + 1))).unwrap();
                    cursor += 1;
                }
            }
        }
        assert_eq!(cursor, ring.len() as u32 + 1, "every entry relayed");
        assert_eq!(
            reads_of_d_before_witness, 0,
            "D read before the hardware would fetch it"
        );
    }

    /// A model of what the PBDMA generates from a sequence of pushbuffer segments (one per GP
    /// entry; a control entry contributes none): packets are parsed ACROSS segment boundaries, as
    /// the public `dev_pbdma.ref` describes. `Err(i)` = word `i` of the concatenated stream was
    /// expected to be a header and does not decode (`NV_PPBDMA_INTR_0_PBENTRY`).
    fn pbdma_methods(segments: &[Vec<u32>]) -> Result<Vec<(u32, u32, u32)>, usize> {
        use kf_abi::submit::{MethodForm, method_header_decode};
        let words: Vec<u32> = segments.iter().flatten().copied().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let h = method_header_decode(words[i]).ok_or(i)?;
            i += 1;
            match h.form {
                MethodForm::Immediate => out.push((h.subchannel, h.method, h.immd)),
                MethodForm::EndPbSegment | MethodForm::SubDeviceMask => {}
                _ => {
                    for k in 0..h.arg_words {
                        let Some(&v) = words.get(i) else {
                            return Ok(out); // still open at the end of what was published
                        };
                        let m = match h.form {
                            MethodForm::NonIncrementing => h.method,
                            MethodForm::IncrementOnce => h.method + if k > 0 { 4 } else { 0 },
                            _ => h.method + 4 * k as u32,
                        };
                        out.push((h.subchannel, m, v));
                        i += 1;
                    }
                }
            }
        }
        Ok(out)
    }

    /// The relay over a guest ring of `(SYNC_WAIT, words)` segments: each segment is scanned and
    /// published as written; before a SYNC_WAIT segment the gate authors a barrier (no bytes) and
    /// the witness `pick` chooses ([`Witness::None`] = no gate at all). Returns the private ring's
    /// segments, in GP order.
    fn relay(ring: &[(bool, Vec<u32>)], pick: impl Fn(bool) -> Witness) -> Vec<Vec<u32>> {
        let mut scan = Scan::default();
        let mut fetch = Fetch::new(64).unwrap();
        let mut gate = SyncGate::default();
        let mut marks = Marks::default();
        let mut private = Vec::new();
        let mut cursor = 0u32;
        let mut guard = 0;
        while (cursor as usize) < ring.len() && guard < 100 {
            guard += 1;
            let (sync, words) = &ring[cursor as usize];
            let raw = seg(0x1000 * u64::from(cursor + 1), words.len() as u64, *sync);
            // a SYNC_WAIT entry reached with a packet open has no witness: it is not gated
            let ungated = *sync && pick(!scan.at_boundary()) == Witness::None;
            let verdict = if ungated {
                Gate::Proceed
            } else {
                gate.entry(raw, 1, cursor, marks.seen())
            };
            match verdict {
                Gate::PublishBarrier => {
                    fetch.publish(None).unwrap();
                    private.push(Vec::new()); // the barrier: a control entry
                    fetch.publish(None).unwrap();
                    let (_, value) = marks.issue().unwrap();
                    // the marker segment: [SET_REFERENCE value, NON_STALL_INTERRUPT 0]
                    private.push(marker(value));
                    gate.published(marks.issued());
                }
                Gate::Wait => {
                    // the hardware processes the marker: REFERENCE takes its value
                    let get = (fetch.issued() & 63) as u32;
                    fetch.observe(get).unwrap();
                    marks.observe(Marks::value(marks.issued()));
                }
                Gate::Proceed => {
                    assert_eq!(scan.segment(words, |_| false), Ok(None));
                    fetch.publish(Some((1, cursor + 1))).unwrap();
                    private.push(words.clone());
                    cursor += 1;
                }
            }
        }
        private
    }

    /// The batch/gate marker segment as `Storage::write_host_marker` authors it.
    fn marker(value: u32) -> Vec<u32> {
        let h = |m| kf_abi::submit::method_header_inc(0, m, 1).unwrap();
        vec![
            h(kf_abi::submit::fifo::SET_REFERENCE_C56F),
            value,
            h(kf_abi::submit::fifo::NON_STALL_INTERRUPT),
            0,
        ]
    }

    /// ★ The Minecraft in-world defect's mechanism, as a model test. Guest ring: segment A ends
    /// inside an I2M-style inline upload (`INC 1:0x1b0 x3`, one argument present — the measured
    /// shape, `carried_out=IncrementOnce sub1 m0x1b0 1/1057`), the next entry B carries SYNC_WAIT and
    /// the remaining arguments (legal: `dev_pbdma.ref`, `GP_ENTRY1_SYNC`). The relay's private ring,
    /// parsed as the PBDMA parses it, must generate EXACTLY the guest's methods. With the byte
    /// witness (the relay before this fix) the zero word becomes an argument and B's last data
    /// word is parsed as a header: PBENTRY.
    #[test]
    fn a_packet_carried_into_a_sync_wait_entry_is_not_gated_and_gets_no_marker() {
        let h = |sub, m, n| kf_abi::submit::method_header_inc(sub, m, n).unwrap();
        let ring = vec![
            (false, vec![h(0, 0x100, 1), 0, h(1, 0x1b0, 3), 0x11]),
            // 0xC000_1234 is DATA here; as a header it is SEC_OP RESERVED6 (does not decode).
            (true, vec![0x22, 0xC000_1234, h(0, 0x100, 1), 7]),
            (false, vec![h(0, 0x100, 1), 8]),
        ];
        let guest: Vec<Vec<u32>> = ring.iter().map(|(_, w)| w.clone()).collect();
        let expected = pbdma_methods(&guest).expect("the guest's own stream decodes");
        assert_eq!(expected.len(), 6);

        // The relay as fixed: the open packet means no witness and no gate -- the private ring is the
        // guest's segments, byte for byte.
        assert_eq!(witness(true), Witness::None);
        let fixed = relay(&ring, witness);
        assert_eq!(fixed, guest);
        assert_eq!(pbdma_methods(&fixed), Ok(expected.clone()));

        // A marker placed anyway (the relay before the fix put a witness word there whatever the parse
        // state): its words become the packet's arguments and B's last data word is parsed as a header.
        let before = relay(&ring, |_| Witness::Marker);
        assert_ne!(pbdma_methods(&before), Ok(expected));

        // At a packet boundary the marker gates and generates the guest's methods plus its own two
        // (SET_REFERENCE, NON_STALL_INTERRUPT).
        let boundary = vec![
            (false, vec![h(0, 0x100, 1), 0]),
            (true, vec![h(1, 0x1b0, 2), 0x22, 0x33]),
        ];
        let guest: Vec<Vec<u32>> = boundary.iter().map(|(_, w)| w.clone()).collect();
        let expected = pbdma_methods(&guest).unwrap();
        assert_eq!(witness(false), Witness::Marker);
        let got = pbdma_methods(&relay(&boundary, witness)).unwrap();
        let own = |m: &(u32, u32, u32)| {
            m.0 == 0
                && (m.1 == kf_abi::submit::fifo::SET_REFERENCE_C56F
                    || m.1 == kf_abi::submit::fifo::NON_STALL_INTERRUPT)
        };
        assert_eq!(got.iter().filter(|m| own(m)).count(), 2);
        assert_eq!(
            got.into_iter().filter(|m| !own(m)).collect::<Vec<_>>(),
            expected
        );
    }

    /// The marker words are a tagged reference: only tagged values count, extended against `seen`.
    #[test]
    fn marks_count_only_tagged_references_and_extend_the_sixteen_bit_count() {
        let mut m = Marks::default();
        assert_eq!(m.observe(0), 0, "no marker issued, no progress");
        let (c1, v1) = m.issue().unwrap();
        let (c2, v2) = m.issue().unwrap();
        assert_eq!((c1, c2, v1, v2), (1, 2, MARK_TAG | 1, MARK_TAG | 2));
        // the guest's own SET_REFERENCE, whatever its value, proves nothing
        for word in [0u32, 1, 2, 0x1000, 0xdead_beef, 0x4B47_0001] {
            assert_eq!(m.peek(word), 0, "untagged {word:#x}");
        }
        assert_eq!(m.peek(v1), 1);
        assert_eq!(m.observe(v2), 2);
        // a value naming a marker not issued yet implies nothing
        assert_eq!(m.peek(MARK_TAG | 9), 2);
        // the 16-bit count wraps without confusion
        let mut m = Marks::default();
        for _ in 0..70_000 {
            let (c, v) = m.issue().unwrap();
            assert_eq!(m.peek(v), c);
            m.observe(v);
        }
        assert_eq!(m.seen(), 70_000);
    }

    #[test]
    fn entries_without_sync_wait_always_proceed() {
        let mut g = SyncGate::default();
        for c in 0..8 {
            assert_eq!(g.entry(seg(0x1000, 4, false), 1, c, 0), Gate::Proceed);
        }
        assert!(!g.waiting());
    }

    #[test]
    fn sync_wait_publishes_one_barrier_then_waits_for_its_witness_then_proceeds_once() {
        let mut g = SyncGate::default();
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 10), Gate::PublishBarrier);
        g.published(12);
        assert!(g.waiting());
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 11), Gate::Wait);
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 12), Gate::Proceed);
        // the same entry again (a retried turn) needs no second barrier
        assert_eq!(g.entry(SYNC_NOP, 1, 5, 12), Gate::Proceed);
        // a SYNC_WAIT segment at the next cursor is gated afresh
        assert_eq!(
            g.entry(seg(0x1000, 4, true), 1, 6, 12),
            Gate::PublishBarrier
        );
        // and a new ring generation at the same cursor too
        g.published(14);
        assert_eq!(g.entry(seg(0x1000, 4, true), 1, 6, 14), Gate::Proceed);
        assert_eq!(g.entry(SYNC_NOP, 2, 5, 14), Gate::PublishBarrier);
    }

    #[test]
    fn grace_retries_the_same_entry_a_bounded_number_of_times_then_is_final() {
        let mut g = Grace::default();
        for _ in 0..REFUSE_RETRIES {
            assert!(g.refused(1, 5));
        }
        assert!(!g.refused(1, 5));
        // another entry, or another ring generation, starts its own retries
        assert!(g.refused(1, 6));
        assert!(g.refused(2, 6));
        g.clear();
        assert!(!g.retrying());
    }
}
