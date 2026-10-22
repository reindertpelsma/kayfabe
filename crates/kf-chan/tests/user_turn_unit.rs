//! ★ 2026-10-16 (PHASE 1b) — **unit tests of the production per-entry turn** (`kf_chan::user_turn::Relay`) with a
//! small programmable [`Io`]: the order of the loads, what is read and what is not, what is published, and what each
//! refusal does. A second angle on the invariants the interleaving model, the property tests and the thread stress
//! check by exploration (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md` sections 0a and 3): every test here is
//! deterministic and reads one mechanism.
#![allow(missing_docs, dead_code, clippy::too_many_lines)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::{Marks, SyncGate};
use kf_chan::user_lifecycle::{Lifecycle, Mode};
use kf_chan::user_loop::Need;
use kf_chan::user_scan::{Fetch, Scan};
use kf_chan::user_turn::{
    Ev, Fail, GpSegment, Io, Phase, PubKind, Relay, Staged, Step, SwView, Turn,
};
use std::collections::HashMap;
use support::{Ge, Kind, ge, sw};
#[path = "fixtures/run3440.rs"]
mod measured;

/// What the relay did to the world, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    HwWords,
    ScanBound,
    WriteGuestGet(u32),
    GpPut,
    Load(u32, bool),
    Epoch,
    Live,
    Software,
    ReadPb(u64),
    Stage,
    Entry(PubKind, u64),
    Gate,
    Marker,
}

struct Mock {
    entries: u32,
    ring: Vec<u64>,
    put: u32,
    epoch: u64,
    live: u32,
    gone: bool,
    pb: HashMap<u64, Vec<u32>>,
    calls: Vec<Call>,
    events: Vec<Ev>,
    next_scan: Scan,
    /// The private ring's `GP_GET` the mock reports (the PBDMA's pops).
    hw_get: u32,
    hw_reference: u32,
    rewrite_on_validate: Option<u64>,
    rewrite_after_lo: Option<u64>,
    tested: u64,
    wide: Option<(u64, Vec<u32>)>,
    part_reads: Vec<(u64, u64)>,
    read_retry_once: Option<u64>,
    publish_retry_once: bool,
    walk_blocked: bool,
    scan_bound: u64,
    fail_stream_allocation: bool,
}

impl Mock {
    fn new(entries: u32) -> Self {
        Self {
            entries,
            ring: vec![0; entries as usize],
            put: 0,
            epoch: 0,
            live: 0,
            gone: false,
            pb: HashMap::new(),
            calls: Vec::new(),
            events: Vec::new(),
            next_scan: Scan::default(),
            hw_get: 0,
            hw_reference: 0,
            rewrite_on_validate: None,
            rewrite_after_lo: None,
            tested: 0,
            wide: None,
            part_reads: Vec::new(),
            read_retry_once: None,
            publish_retry_once: false,
            walk_blocked: false,
            scan_bound: kf_chan::user_storage::MAX_SOURCE_BYTES,
            fail_stream_allocation: false,
        }
    }
    /// The guest publishes `g` at `slot` (its pushbuffer words are known to the store).
    fn put_entry(&mut self, slot: u32, g: &Ge) {
        self.ring[slot as usize] = g.raw();
        self.pb.insert(g.raw(), g.words());
    }
    fn count(&self, c: &Call) -> usize {
        self.calls.iter().filter(|x| *x == c).count()
    }
    fn gates(&self) -> usize {
        self.calls.iter().filter(|c| **c == Call::Gate).count()
    }
}

impl Io for Mock {
    fn hw_words(&mut self) -> Turn<(u32, u32)> {
        self.calls.push(Call::HwWords);
        Ok((self.hw_get, self.hw_reference))
    }
    fn write_guest_get(&mut self, get: u32) -> Turn<bool> {
        self.calls.push(Call::WriteGuestGet(get));
        Ok(true)
    }
    fn src_entries(&mut self) -> u32 {
        self.entries
    }
    fn max_scan_bytes(&mut self) -> u64 {
        self.calls.push(Call::ScanBound);
        self.scan_bound
    }
    fn walk_busy(&mut self) -> bool {
        self.walk_blocked
    }
    fn gp_put(&mut self) -> Turn<Option<u32>> {
        self.calls.push(Call::GpPut);
        Ok(Some(self.put))
    }
    fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32> {
        self.calls.push(Call::Load(slot, hi));
        let v = self.ring[slot as usize];
        if !hi && let Some(replacement) = self.rewrite_after_lo.take() {
            self.ring[slot as usize] = replacement;
        }
        Ok(if hi { (v >> 32) as u32 } else { v as u32 })
    }
    fn registry_epoch(&mut self) -> u64 {
        self.calls.push(Call::Epoch);
        self.epoch
    }
    fn registry_live(&mut self) -> u32 {
        self.calls.push(Call::Live);
        self.live
    }
    fn software(&mut self, _refresh: bool) -> Turn<SwView> {
        self.calls.push(Call::Software);
        Ok(SwView {
            have: true,
            gone: self.gone,
        })
    }
    fn sw_matches(&self, value: u32) -> bool {
        sw(value)
    }
    fn read_pb(&mut self, entry: u64, seg: &GpSegment) -> Turn<(Vec<u32>, bool)> {
        if self.fail_stream_allocation {
            return Err("USER_RING_STREAM_ALLOC".into());
        }
        self.calls.push(Call::ReadPb(entry));
        if let Some((base, words)) = &self.wide
            && seg.gpu_va >= *base
            && seg.gpu_va < *base + words.len() as u64 * 4
        {
            assert!(seg.len_bytes <= self.scan_bound, "oversized snapshot");
            if self.read_retry_once == Some(seg.gpu_va) {
                self.read_retry_once = None;
                return Err(Fail::Retry);
            }
            let start = ((seg.gpu_va - base) / 4) as usize;
            let count = seg.len_bytes as usize / 4;
            self.part_reads.push((seg.gpu_va, seg.len_bytes));
            return Ok((words[start..start + count].to_vec(), false));
        }
        Ok((
            self.pb
                .get(&(entry & !support::SYNC))
                .cloned()
                .unwrap_or_default(),
            false,
        ))
    }
    fn stage(&mut self, scan: &Scan, words: &[u32]) -> Turn<Staged> {
        self.calls.push(Call::Stage);
        let mut next = scan.clone();
        match next.segment(words, sw) {
            Err(e) => Err(Fail::Refused(format!("USER_RING_SCAN {e:?}"))),
            Ok(t) => {
                self.next_scan = next;
                Ok(Staged { rare: t.is_some() })
            }
        }
    }
    fn validated(&mut self, entry: u64) {
        self.tested = entry;
        if let Some(replacement) = self.rewrite_on_validate.take() {
            self.ring[2] = replacement;
        }
    }
    fn publish_entry(
        &mut self,
        kind: PubKind,
        fetch: &mut Fetch,
        generation: u64,
        guest_get: u32,
        entry: u64,
    ) -> Turn<()> {
        if self.publish_retry_once {
            self.publish_retry_once = false;
            return Err(Fail::Retry);
        }
        fetch
            .publish(Some((generation, guest_get)))
            .map_err(str::to_owned)?;
        self.calls.push(Call::Entry(kind, entry));
        Ok(())
    }
    fn publish_gate(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<u64> {
        fetch.publish(None).map_err(str::to_owned)?;
        fetch.publish(None).map_err(str::to_owned)?;
        let (n, _) = marks.issue().map_err(str::to_owned)?;
        self.calls.push(Call::Gate);
        Ok(n)
    }
    fn publish_marker(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<()> {
        fetch.publish(None).map_err(str::to_owned)?;
        marks.issue().map_err(str::to_owned)?;
        self.calls.push(Call::Marker);
        Ok(())
    }
    fn event(&mut self, e: Ev) {
        self.events.push(e);
    }
}

fn relay() -> Relay {
    Relay::new(16, Lifecycle::new(), Scan::for_subdevice(1).unwrap()).unwrap()
}

/// One turn, standing in for the pump's segment machinery after a scanned segment (non-rare path).
fn turn(r: &mut Relay, io: &mut Mock) -> Result<(), Fail> {
    r.claim();
    let mut guard = 0;
    while !r.idle() {
        guard += 1;
        assert!(guard < 64, "a turn must be bounded");
        match r.step(io)? {
            Step::Scanned { rare } => {
                assert!(!rare, "no trigger in these tests");
                r.fetch
                    .publish(Some((r.generation, r.next_get)))
                    .map_err(str::to_owned)?;
                r.scan = io.next_scan.clone();
                let open = r.scan.carried().is_some();
                r.life.segment_scanned(open);
                r.cursor = r.next_get;
                r.note_relayed(r.raw);
                r.unnotified = true;
                r.again = true;
            }
            Step::Segment | Step::Shadow => panic!("not used here"),
            Step::More | Step::Idle { .. } => {}
        }
    }
    Ok(())
}

/// A relay that has scanned a SET_OBJECT and is in Pointer mode with its cursor at `1` (entry 0 = SetObj).
fn pointer_relay(io: &mut Mock) -> Relay {
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::SetObj));
    io.put = 1;
    turn(&mut r, io).unwrap();
    // the SET_OBJECT is now committed; the next entry is the first Pointer entry
    io.put_entry(1, &ge(1, Kind::Plain));
    io.put = 2;
    turn(&mut r, io).unwrap();
    assert_eq!(r.life.mode(), Mode::Pointer);
    io.calls.clear();
    io.events.clear();
    r
}

// ------------------------------------------------------------------------------------------------

#[test]
fn a_turn_loads_gp_put_once_then_the_entry_at_the_cursor_then_the_epoch_in_that_order() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.put_entry(2, &ge(2, Kind::Plain));
    io.put_entry(3, &ge(3, Kind::Plain));
    io.put = 4;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    let order: Vec<&Call> = io
        .calls
        .iter()
        .filter(|c| matches!(c, Call::GpPut | Call::Load(..) | Call::Epoch))
        .collect();
    assert_eq!(
        order,
        vec![
            &Call::GpPut,
            &Call::Load(2, false),
            &Call::Load(2, true),
            &Call::Epoch
        ],
        "GP_PUT, then the entry's two dwords at the cursor, then the registry epoch (I8, design 1.1)"
    );
    assert_eq!(io.count(&Call::GpPut), 1, "GP_PUT is loaded once per turn");
}

#[test]
fn a_turn_with_gp_put_at_the_cursor_loads_no_ring_entry_and_no_pushbuffer() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    // the slot AT GP_PUT holds a valid-looking entry that must never be read
    io.put_entry(2, &ge(77, Kind::Plain));
    io.put = 2;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(
        !io.calls
            .iter()
            .any(|c| matches!(c, Call::Load(..) | Call::ReadPb(_) | Call::Entry(..))),
        "nothing at or beyond GP_PUT is read or published: {:?}",
        io.calls
    );
    assert_eq!(r.cursor, 2);
}

#[test]
fn the_entry_is_stored_exactly_as_loaded_with_two_dword_loads_per_entry() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let g = ge(9, Kind::Plain);
    io.put_entry(2, &g);
    io.put = 3;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        io.calls
            .iter()
            .filter(|c| matches!(c, Call::Load(..)))
            .count(),
        2,
        "ONE load of the 8-byte entry (two dwords), never a second fetch for the store"
    );
    assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, g.raw())));
}

#[test]
fn a_torn_down_machine_loads_nothing_after_the_plan_and_publishes_nothing() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    r.life.teardown();
    io.put_entry(2, &ge(2, Kind::Plain));
    io.put = 3;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(
        !io.calls
            .iter()
            .any(|c| matches!(c, Call::Entry(..) | Call::ReadPb(_) | Call::Gate))
    );
    assert_eq!(r.cursor, 2, "the cursor does not move");
}

#[test]
fn pointer_mode_refuses_a_freed_software_object_and_publishes_nothing_for_it() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.gone = true;
    io.put_entry(2, &ge(2, Kind::Plain));
    io.put = 3;
    let e = turn(&mut r, &mut io).unwrap_err();
    assert_eq!(e, Fail::Refused("USER_RING_SOFTWARE_OBJECT_GONE".into()));
    assert_eq!(r.life.counts().pointer_software_gone, 1);
    assert!(r.dead);
    assert!(!io.calls.iter().any(|c| matches!(c, Call::Entry(..))));
}

#[test]
fn an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    // reserved bit 8 of the high dword
    io.ring[2] = ge(2, Kind::Plain).raw() | (1 << 40);
    io.put = 3;
    let e = turn(&mut r, &mut io).unwrap_err();
    assert!(
        matches!(&e, Fail::Refused(w) if w.starts_with("USER_RING_GP_FORMAT")),
        "{e:?}"
    );
    assert_eq!(r.refused_entry, 1);
    assert!(!io.calls.iter().any(|c| matches!(c, Call::Entry(..))));
}

#[test]
fn a_gp_put_outside_the_ring_is_refused_before_any_entry_is_read() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.put = 16;
    io.calls.clear();
    let e = turn(&mut r, &mut io).unwrap_err();
    assert_eq!(e, Fail::Refused("USER_RING_PUT_BOUND".into()));
    assert!(!io.calls.iter().any(|c| matches!(c, Call::Load(..))));
}

#[test]
fn the_binding_is_known_only_after_the_datum_not_after_the_set_object_header() {
    let mut io = Mock::new(16);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::SetObjHead));
    io.put_entry(1, &ge(1, Kind::SetObjTail));
    io.put_entry(2, &ge(2, Kind::Plain));
    io.put = 3;
    turn(&mut r, &mut io).unwrap(); // the header: a packet is open, nothing is bound
    turn(&mut r, &mut io).unwrap(); // the datum: bound after this segment
    assert_eq!(
        r.life.mode(),
        Mode::Scan,
        "bound only once the next entry is planned"
    );
    turn(&mut r, &mut io).unwrap(); // the next entry sees the binding: Pointer
    assert_eq!(r.life.mode(), Mode::Pointer);
    // and the header alone never counted: re-run with only the header scanned
    let mut io = Mock::new(16);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::SetObjHead));
    io.put_entry(1, &ge(2, Kind::Plain));
    io.put = 2;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Scan);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        r.life.mode(),
        Mode::Scan,
        "a SET_OBJECT header whose datum is unscanned is not a binding"
    );
}

#[test]
fn a_sync_wait_entry_behind_an_open_packet_is_read_at_once_and_counted_without_a_gate() {
    let mut io = Mock::new(16);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::OpenHead));
    io.put_entry(1, &ge(1, Kind::OpenTail).sync());
    io.put = 2;
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.gates(), 0, "no witness fits inside a packet");
    assert!(
        io.events
            .iter()
            .any(|e| matches!(e, Ev::CarriedUngated { .. }))
    );
    assert!(
        io.calls
            .contains(&Call::ReadPb(ge(1, Kind::OpenTail).sync().raw()))
    );
}

/// A Pointer relay with a SYNC_WAIT entry forwarded and a registration that appears on the next entry.
fn flipped_relay(io: &mut Mock, sync_before: bool) -> Relay {
    let mut r = pointer_relay(io);
    if sync_before {
        io.put_entry(2, &ge(2, Kind::Plain).sync());
        io.put = 3;
        turn(&mut r, io).unwrap();
    }
    let at = r.cursor;
    io.put_entry(at, &ge(3, Kind::Plain));
    io.put = at + 1;
    io.epoch = 1;
    io.live = 1;
    io.calls.clear();
    r
}

#[test]
fn a_window_that_follows_a_forwarded_sync_wait_gates_its_first_entry_before_reading_it() {
    let mut io = Mock::new(16);
    let mut r = flipped_relay(&mut io, true);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Window);
    assert_eq!(io.gates(), 1, "the seeded gate: barrier + marker");
    assert!(
        !io.calls.iter().any(|c| matches!(c, Call::ReadPb(_))),
        "nothing is read before the marker is processed"
    );
    assert!(matches!(r.need, Need::Marker { .. }));
}

#[test]
fn a_window_with_no_forwarded_sync_wait_behind_it_reads_its_first_entry_at_once() {
    let mut io = Mock::new(16);
    let mut r = flipped_relay(&mut io, false);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Window);
    assert_eq!(io.gates(), 0);
    assert!(io.calls.iter().any(|c| matches!(c, Call::ReadPb(_))));
}

#[test]
fn a_fresh_window_resets_the_gate_so_a_proof_from_an_earlier_lap_is_gone() {
    let mut io = Mock::new(16);
    let mut r = flipped_relay(&mut io, false);
    // a stale proof: this slot was gated and proven in an earlier life of the ring
    let mut stale = SyncGate::default();
    let raw = ge(3, Kind::Plain).sync().raw();
    assert!(matches!(
        stale.entry(raw, r.generation, r.cursor, 0),
        kf_chan::user_gate::Gate::PublishBarrier
    ));
    stale.published(1);
    assert!(matches!(
        stale.entry(raw, r.generation, r.cursor, 1),
        kf_chan::user_gate::Gate::Proceed
    ));
    r.sync = stale;
    // stop right after the plan (the restart is applied there, before the gate runs)
    r.claim();
    while r.phase != Phase::Gate {
        r.step(&mut io).unwrap();
    }
    assert_eq!(
        format!("{:?}", r.sync),
        format!("{:?}", SyncGate::default()),
        "the Fresh restart forgot the stale proof"
    );
    assert_eq!(r.life.counts().restarts, 1);
}

#[test]
fn the_fresh_window_restarts_the_parser_but_keeps_bindings_and_starts_at_the_anchor_not_before() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    // one submission of three entries: GP_PUT = 5 is read in the first turn; the registration shows up on the second
    for (i, slot) in (2..5).enumerate() {
        io.put_entry(slot, &ge(10 + i as u8, Kind::Plain));
    }
    io.put = 5;
    turn(&mut r, &mut io).unwrap(); // clean: anchor = 5
    io.epoch = 1;
    io.live = 1;
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(
        r.life.mode(),
        Mode::Pointer,
        "entries below the anchor stay unscanned"
    );
    assert_eq!(r.life.counts().anchor_forwarded, 2);
    // the guest publishes after the reply
    io.put_entry(5, &ge(20, Kind::Plain));
    io.put = 6;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Window);
    assert_eq!(r.life.counts().restarts, 1);
}

#[test]
fn a_pointer_marker_is_authored_where_the_cursor_reached_gp_put_and_not_mid_submission() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    // a three-entry submission: the marker comes at the batch end only
    for slot in 2..5 {
        io.put_entry(slot, &ge(30 + slot as u8, Kind::Plain));
    }
    io.put = 5;
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.count(&Call::Marker), 0, "mid-submission: no marker");
    turn(&mut r, &mut io).unwrap(); // GP_PUT == cursor
    assert_eq!(io.count(&Call::Marker), 1, "at the batch end");
    assert!(matches!(r.need, Need::Marker { .. }));
    assert_eq!(r.life.counts().pointer_markers, 1);
}

#[test]
fn a_wake_pointer_mode_cannot_announce_is_counted_loudly_not_hidden() {
    let mut io = Mock::new(64);
    let mut r = pointer_relay(&mut io);
    // one long submission larger than the private ring: the credit wait happens mid-submission, away from any
    // observed GP_PUT
    for slot in 2..22 {
        io.put_entry(slot, &ge(40 + slot as u8, Kind::Plain));
    }
    io.put = 22;
    for _ in 0..22 {
        turn(&mut r, &mut io).unwrap();
        if matches!(r.need, Need::Popped { .. }) {
            break;
        }
    }
    assert!(
        matches!(r.need, Need::Popped { .. }),
        "no marker: the spin / backstop must wake it"
    );
    assert!(r.life.counts().pointer_carried_no_marker >= 1);
    assert_eq!(io.count(&Call::Marker), 0);
}

#[test]
fn a_fresh_window_forgets_a_packet_the_scan_left_open_before_pointer_mode() {
    use kf_abi::submit::gp_entry;
    let mut io = Mock::new(16);
    let mut r = relay();
    // entry 0 binds the software object AND ends inside an open packet
    let words = ge(0, Kind::SetObjOpen).words();
    let raw0 = gp_entry(support::seg_va(0), words.len() as u64 * 4).unwrap();
    io.ring[0] = raw0;
    io.pb.insert(raw0, words);
    io.put = 1;
    turn(&mut r, &mut io).unwrap();
    assert!(r.scan.carried().is_some(), "the scan left a packet open");
    // Pointer mode forwards the packet's tail unscanned
    io.put_entry(1, &ge(1, Kind::OpenTail));
    io.put = 2;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Pointer);
    assert!(
        r.scan.carried().is_some(),
        "Pointer mode does not touch the parser"
    );
    // a registration: the Window starts at the anchor (GP_PUT = 3) with a restarted parser
    io.put_entry(2, &ge(2, Kind::Plain));
    io.put = 3;
    io.epoch = 1;
    io.live = 1;
    r.claim();
    while r.phase != Phase::Gate {
        r.step(&mut io).unwrap();
    }
    assert!(
        r.scan.carried().is_none(),
        "the Fresh restart forgot the packet left open before Pointer mode"
    );
    assert_eq!(r.life.mode(), Mode::Window);
}

// ---- PHASE 1c item 2: the ring-full anchor guard at the real turn, and the P2' notices

#[test]
fn the_guest_ring_is_full_when_gp_put_leaves_one_slot_before_the_gp_get_the_guest_sees() {
    let mut io = Mock::new(8);
    let mut r = pointer_relay(&mut io);
    assert_eq!(r.written_get, 0, "nothing written back yet");
    // the PBDMA popped both forwarded entries: the next turn writes GP_GET = 2 back
    io.hw_get = 2;
    // the guest filled slots 2..=7 and 0: GP_PUT = 1, so put + 1 == 2 == the GP_GET it sees
    for slot in [2u32, 3, 4, 5, 6, 7, 0] {
        io.put_entry(slot, &ge(10 + slot as u8, Kind::Plain));
    }
    io.put = 1;
    let before = r.life.counts().anchor_refused_ring_full;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.written_get, 2, "the write-back is what the guest sees");
    assert!(io.calls.contains(&Call::WriteGuestGet(2)));
    assert!(
        r.put_full,
        "GP_PUT = 1 leaves exactly one free slot before GP_GET = 2"
    );
    assert_eq!(
        r.life.counts().anchor_refused_ring_full,
        before + 1,
        "the Pointer turn refused the anchor"
    );
    // an ordinary GP_PUT (not one slot before the GET) is an anchor again
    io.put = 5;
    turn(&mut r, &mut io).unwrap();
    assert!(!r.put_full);
    assert_eq!(r.life.counts().anchor_refused_ring_full, before + 1);
}

#[test]
fn a_fresh_relay_sees_a_ring_filled_to_the_last_slot_as_full() {
    let mut io = Mock::new(8);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::SetObj));
    io.put = 7; // the guest wrote 7 entries before any write-back: GP_GET it sees is 0
    for slot in 1..7u32 {
        io.put_entry(slot, &ge(slot as u8, Kind::Plain));
    }
    turn(&mut r, &mut io).unwrap();
    assert!(
        r.put_full,
        "(7 + 1) mod 8 == 0 == the GP_GET the guest sees"
    );
}

#[test]
fn a_packet_open_at_gp_put_in_scan_mode_is_reported_with_the_packet_and_switches_markers_off() {
    let mut io = Mock::new(16);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::OpenHead));
    io.put = 1;
    turn(&mut r, &mut io).unwrap(); // scans the head; the packet stays open
    assert!(r.scan.carried().is_some());
    assert!(
        r.life.take_notices().is_empty(),
        "no GP_PUT boundary reached yet"
    );
    turn(&mut r, &mut io).unwrap(); // GP_PUT == cursor with the packet open
    let (h, n) = r.put_packet.expect("the packet open at GP_PUT");
    assert_eq!((h.subchannel, h.method, n), (1, 0x1b0, 1));
    assert_eq!(r.life.counts().put_boundary_open_scan, 1);
    assert_eq!(r.life.counts().pointer_markers_latched_off, 1);
    assert!(r.life.markers_off());
    use kf_chan::user_lifecycle::Notice;
    let notices = r.life.take_notices();
    assert!(matches!(
        notices[0],
        Notice::PutBoundaryOpen {
            mode: Mode::Scan,
            ..
        }
    ));
    assert!(matches!(notices[1], Notice::MarkersLatchedOff { .. }));
    assert!(
        !io.calls.contains(&Call::Marker),
        "no marker inside the open packet (Scan mode already refused it)"
    );
}

// ---- PHASE 1c item 3 (review 2, mutant W): the ring switch is production code, not pump code

/// Drive a relay into a Window (a registration seen, the anchor reached).
fn window_relay(io: &mut Mock) -> Relay {
    let mut r = pointer_relay(io);
    io.epoch = 1;
    io.live = 1;
    for slot in 2..8u32 {
        io.put_entry(slot, &ge(20 + slot as u8, Kind::Plain));
        io.put = slot + 1;
        turn(&mut r, io).unwrap();
        if r.life.mode() == Mode::Window {
            return r;
        }
    }
    panic!(
        "the registration never opened a Window: {:?}",
        r.life.mode()
    );
}

#[test]
fn switch_source_installs_the_new_ring_and_keeps_the_window_open_over_its_first_entries() {
    let mut io = Mock::new(16);
    let mut r = window_relay(&mut io);
    r.written_get = 7;
    r.guest_get = Some((r.generation, 5));
    let gen_before = r.generation;
    io.live = 0;
    io.epoch = 2; // the trigger consumed the registration
    r.switch_source(gen_before + 1);
    assert_eq!(r.generation, gen_before + 1);
    assert_eq!(r.cursor, 0, "the new ring starts at slot 0");
    assert_eq!(
        r.written_get, 0,
        "the guest's GET in the new USERD is taken to be 0"
    );
    assert_eq!(r.guest_get, None, "no write-back of the old ring is owed");
    assert!(!r.grace.retrying());
    // the registry is empty and the relay quiescent, yet the Window holds for POST_SWITCH_SCAN entries of the new ring
    let n = u64::from(kf_chan::user_lifecycle::POST_SWITCH_SCAN);
    for slot in 0..n as u32 {
        io.put_entry(slot, &ge(40 + slot as u8, Kind::Plain));
        io.put = slot + 1;
        turn(&mut r, &mut io).unwrap();
        assert_eq!(r.life.mode(), Mode::Window, "entry {slot} of the new ring");
    }
    assert_eq!(r.life.counts().post_switch_scanned, n);
    io.put_entry(n as u32, &ge(60, Kind::Plain));
    io.put = n as u32 + 1;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Pointer, "then it may end");
}

#[test]
fn a_switch_outside_a_window_resets_the_cursor_but_scans_nothing_extra() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    r.switch_source(2);
    assert_eq!((r.generation, r.cursor, r.written_get), (2, 0, 0));
    io.put_entry(0, &ge(70, Kind::Plain));
    io.put = 1;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Pointer);
    assert!(!io.calls.iter().any(|c| matches!(c, Call::ReadPb(_))));
    assert_eq!(r.life.counts().post_switch_scanned, 0);
}

#[test]
fn a_nonzero_guest_get_in_the_new_userd_contradicts_the_reset_and_is_what_the_guest_sees() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    r.switch_source(2);
    assert!(!r.new_ring_get(0), "GP_GET 0 agrees with reset-to-0");
    assert_eq!(r.life.counts().switch_cursor_mismatch, 0);
    assert!(
        r.new_ring_get(5),
        "a nonzero GP_GET is reported to the owner"
    );
    assert_eq!(r.life.counts().switch_cursor_mismatch, 1);
    assert_eq!(r.written_get, 5);
}

#[test]
fn the_switch_forgets_the_old_rings_gp_put_boundaries_so_no_marker_lands_at_a_stale_index() {
    // Pointer mode; the OLD ring's GP_PUT is at absolute index 14 (remembered as the next boundary). The ring is switched.
    // On the NEW ring the guest's GP_PUT is far ahead (40 entries): when the private ring runs out of credit at absolute
    // index 14 (a credit stall in the middle of a submission) no marker may be authored there -- 14 was a GP_PUT of the
    // old ring only.
    let mut io = Mock::new(64);
    let mut r = pointer_relay(&mut io);
    for slot in 2..14u32 {
        io.put_entry(slot, &ge(30 + slot as u8, Kind::Plain));
    }
    io.put = 14;
    turn(&mut r, &mut io).unwrap(); // forwards one entry; the GP_PUT (absolute 14) is remembered
    r.switch_source(2);
    for slot in 0..40u32 {
        io.put_entry(slot, &ge(60 + slot as u8, Kind::Plain));
    }
    io.put = 40;
    for _ in 0..40 {
        turn(&mut r, &mut io).unwrap();
    }
    assert_eq!(
        r.relayed, 14,
        "the private ring ran out of credit at the old ring's boundary index"
    );
    assert!(
        matches!(r.need, Need::Popped { .. }),
        "the stall waits for pops: {:?}",
        r.need
    );
    assert_eq!(io.count(&Call::Marker), 0, "no marker at a stale boundary");
    assert!(
        r.life.counts().pointer_carried_no_marker > 0,
        "the unannounced wake is counted instead"
    );
}

// Decision Phase 1: hardware format bounds Pointer; copies alone use the scan budget.
#[test]
fn pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer() {
    for raw in measured::RUN3440 {
        let mut io = Mock::new(16);
        let mut r = pointer_relay(&mut io);
        io.ring[2] = raw;
        io.put = 3;
        turn(&mut r, &mut io).unwrap();
        assert_eq!(r.cursor, 3);
        assert!(!r.dead);
        assert_eq!(r.refused_entry, 0);
        assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, raw)));
        assert_eq!(io.tested, raw);
        assert!(!io.calls.iter().any(|c| matches!(
            c,
            Call::ReadPb(_) | Call::Stage | Call::Gate | Call::ScanBound
        )));
        assert_eq!(r.life.counts().pointer_pb_read_violation, 0);
        assert!(format!("{:?}", r.life.counts()).contains("pointer_large_entries: 1"));
    }
}

#[test]
fn a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let raw = measured::RUN3440[0];
    assert_eq!(((raw >> 42) & 0x1fffff) * 4, 1_026_596);
    io.ring[2] = raw;
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.count(&Call::Marker), 1);
    assert!(
        !io.calls.contains(&Call::WriteGuestGet(3)),
        "no receipt before native fetch"
    );
    io.hw_get = r.fetch.issued() as u32 & 15;
    io.hw_reference = Marks::value(r.marks.issued());
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.marks.seen(), r.marks.issued());
    assert!(io.calls.contains(&Call::WriteGuestGet(3)));
    assert_eq!(r.written_get, 3);
    assert!(!r.dead);
    assert!(format!("{:?}", r.life.counts()).contains("pointer_entry_max_bytes: 1026596"));
}

#[test]
fn pointer_forwards_the_largest_representable_descriptor() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let raw = kf_abi::submit::gp_entry(0x4000, 8_388_604).unwrap() | (1 << 41) | (1 << 63);
    assert!(kf_abi::submit::gp_entry(0x4000, 8_388_608).is_none());
    io.ring[2] = raw;
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, raw)));
    assert!(
        !io.calls
            .iter()
            .any(|c| matches!(c, Call::ReadPb(_) | Call::ScanBound))
    );
    assert_eq!(r.life.outstanding_sync(), Some(2));
    assert!(format!("{:?}", r.life.counts()).contains("pointer_large_entries: 1"));
    assert!(format!("{:?}", r.life.counts()).contains("pointer_entry_max_bytes: 8388604"));
}

#[test]
fn a_large_captured_descriptor_is_not_reloaded_after_guest_mutation() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let raw = measured::RUN3440[0];
    io.ring[2] = raw;
    io.rewrite_on_validate = Some(u64::MAX);
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.ring[2], u64::MAX);
    assert_eq!(io.tested, raw);
    assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, raw)));
    assert!(!io.calls.contains(&Call::Entry(PubKind::Pointer, u64::MAX)));
}

#[test]
fn pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit() {
    let good = kf_abi::submit::gp_entry(0x4000, 4).unwrap();
    let bads = [
        good | 1,
        good | 2,
        good | (1 << 40),
        1 << 32,
        2 << 32,
        kf_abi::submit::gp_entry((1 << 40) - 4, 4).unwrap(),
    ];
    for bad in bads {
        let mut io = Mock::new(16);
        let mut r = pointer_relay(&mut io);
        let issued = r.fetch.issued();
        io.ring[2] = bad;
        io.put = 3;
        let e = turn(&mut r, &mut io).unwrap_err();
        assert!(
            matches!(e, Fail::Refused(ref why) if why.starts_with("USER_RING_GP_FORMAT Entry")),
            "{e:?}"
        );
        assert!(r.dead);
        assert_eq!(r.fetch.issued(), issued);
        assert_eq!(r.cursor, 2);
    }
}

#[test]
fn pointer_zero_length_nops_preserve_sync_level_operand_without_pb_or_budget() {
    for raw in [0, 0xffff_ffff, (1 << 63) | (1 << 41) | 7] {
        let mut io = Mock::new(16);
        let mut r = pointer_relay(&mut io);
        io.ring[2] = raw;
        io.put = 3;
        turn(&mut r, &mut io).unwrap();
        assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, raw)));
        assert!(
            !io.calls
                .iter()
                .any(|c| matches!(c, Call::ReadPb(_) | Call::ScanBound | Call::Gate))
        );
    }
}

#[test]
fn a_ring_of_32768_maximum_pointer_descriptors_has_constant_work_and_no_pb_reads() {
    let mut io = Mock::new(32768);
    let mut r = pointer_relay(&mut io);
    let raw = kf_abi::submit::gp_entry(0x4000, 8_388_604).unwrap();
    for _ in 0..32768 {
        let slot = r.cursor;
        io.ring[slot as usize] = raw;
        io.put = (slot + 1) & 32767;
        io.calls.clear();
        io.hw_get = r.fetch.issued() as u32 & 15;
        io.hw_reference = Marks::value(r.marks.issued());
        turn(&mut r, &mut io).unwrap();
        assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, raw)));
        turn(&mut r, &mut io).unwrap();
        assert!(io.calls.len() <= 20, "{:?}", io.calls);
        assert!(
            !io.calls
                .iter()
                .any(|c| matches!(c, Call::ReadPb(_) | Call::ScanBound))
        );
        assert!(!r.dead);
    }
}

#[test]
fn a_large_sync_wait_pointer_entry_seeds_the_next_fresh_window_gate() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.ring[2] = measured::RUN3440[0] | (1 << 63);
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.outstanding_sync(), Some(2));
    io.put_entry(3, &ge(3, Kind::Plain));
    io.put = 4;
    io.epoch += 1;
    io.live = 1;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.life.mode(), Mode::Window);
    assert_eq!(io.gates(), 1);
    assert!(!io.calls.iter().any(|c| matches!(c, Call::ReadPb(_))));
}

#[test]
fn large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum() {
    let stats = kf_chan::user_lifecycle::LifecycleStats::default();
    for len in [8_388_604, 1_026_596] {
        let mut io = Mock::new(16);
        let mut r = pointer_relay(&mut io);
        io.ring[2] = kf_abi::submit::gp_entry(0x4000, len).unwrap();
        io.put = 3;
        turn(&mut r, &mut io).unwrap();
        stats.add(&r.life.take_delta());
        stats.add(&r.life.take_delta());
    }
    let summary = stats.summary();
    assert!(summary.contains("pointer_large_entries=2 "), "{summary}");
    assert!(
        summary.contains("pointer_entry_max_bytes=8388604 "),
        "{summary}"
    );
}

#[test]
fn small_and_control_pointer_entries_do_not_count_as_large() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.ring[2] = (1 << 63) | (1 << 41);
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    let stats = kf_chan::user_lifecycle::LifecycleStats::default();
    stats.add(&r.life.take_delta());
    let summary = stats.summary();
    assert!(summary.contains("pointer_large_entries=0 "), "{summary}");
    assert!(summary.contains("pointer_entry_max_bytes=8 "), "{summary}");
}

#[test]
fn small_descriptor_action_transcript() {
    for always_scan in [false, true] {
        let mut io = Mock::new(16);
        let life = if always_scan {
            Lifecycle::always_scan()
        } else {
            Lifecycle::new()
        };
        let mut r = Relay::new(16, life, Scan::for_subdevice(1).unwrap()).unwrap();
        for i in 0..8 {
            io.put_entry(
                i,
                &ge(i as u8, if i == 2 { Kind::SetObj } else { Kind::Plain }),
            );
        }
        io.put = 8;
        for _ in 0..16 {
            io.hw_get = r.fetch.issued() as u32 & 15;
            io.hw_reference = Marks::value(r.marks.issued());
            turn(&mut r, &mut io).unwrap();
        }
        assert!(!r.dead);
        assert_eq!(r.cursor, 8);
        assert_eq!(r.written_get, 8);
        let actions: Vec<_> = io.calls.iter().filter(|c| **c != Call::ScanBound).collect();
        println!("SMALL_ACTIONS {always_scan}: {actions:?}");
    }
}

#[test]
fn a_large_length_changed_between_dword_loads_stores_exactly_the_validated_pair() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let first = ge(1, Kind::Plain).raw();
    let second = measured::RUN3440[0];
    io.ring[2] = first;
    io.rewrite_after_lo = Some(second);
    io.put = 3;
    let captured = (first & 0xffff_ffff) | (second & !0xffff_ffff);
    turn(&mut r, &mut io).unwrap();
    assert_ne!(captured, second);
    assert_eq!(io.tested, captured);
    assert!(io.calls.contains(&Call::Entry(PubKind::Pointer, captured)));
    assert!(!io.calls.iter().any(|c| matches!(c, Call::ReadPb(_))));
}

#[test]
fn pointer_maximum_is_not_summed_on_repeated_delta_flushes() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    io.ring[2] = measured::RUN3440[0];
    io.put = 3;
    turn(&mut r, &mut io).unwrap();
    let stats = kf_chan::user_lifecycle::LifecycleStats::default();
    for _ in 0..3 {
        stats.add(&r.life.take_delta());
        let summary = stats.summary();
        assert!(summary.contains("pointer_large_entries=1 "), "{summary}");
        assert!(
            summary.contains("pointer_entry_max_bytes=1026596 "),
            "{summary}"
        );
    }
}

// Phase 2 RED: all calls below exercise the existing production Relay API.
const STREAM_VA: u64 = 0x0002_3110_102c;
const COPY_BOUND: u64 = kf_chan::user_storage::MAX_SOURCE_BYTES;

fn wide_words(bytes: u64) -> Vec<u32> {
    let mut words = vec![0; bytes as usize / 4];
    words[0] = kf_abi::submit::method_header_inc(0, 0, 1).unwrap();
    words[1] = 0xc7c0;
    words
}

fn install_wide(io: &mut Mock, r: &Relay, words: Vec<u32>, sync: bool) -> u64 {
    let raw = kf_abi::submit::gp_entry(STREAM_VA, words.len() as u64 * 4).unwrap()
        | (1 << 41)
        | if sync { 1 << 63 } else { 0 };
    io.ring[r.cursor as usize] = raw;
    io.put = (r.cursor + 1) & (io.entries - 1);
    io.wide = Some((STREAM_VA, words));
    io.calls.clear();
    raw
}

fn stream_window(io: &mut Mock) -> Relay {
    let r = pointer_relay(io);
    io.live = 1;
    io.epoch += 1;
    r
}

fn native_words(io: &Mock) -> Vec<u64> {
    io.calls
        .iter()
        .filter_map(|c| match c {
            Call::Entry(_, raw) => Some(*raw),
            _ => None,
        })
        .collect()
}

#[test]
fn stream_1026596_window_uses_two_turns_and_commits_only_on_intact_publication() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    let words = wide_words(1_026_596);
    let mut expected = before.clone();
    assert_eq!(expected.segment(&words, sw), Ok(None));
    let raw = install_wide(&mut io, &r, words, false);
    assert_eq!(raw, measured::RUN3440[0]);
    let issued = r.fetch.issued();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.part_reads, vec![(STREAM_VA, COPY_BOUND)]);
    assert_eq!(r.scan, before);
    assert_eq!(r.cursor, 2);
    assert_eq!(r.fetch.issued(), issued);
    assert!(r.again);
    assert_eq!(r.need, Need::Again);
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(
        !io.calls.contains(&Call::GpPut),
        "continuation must not re-plan"
    );
    assert_eq!(
        io.part_reads,
        vec![(STREAM_VA, COPY_BOUND), (STREAM_VA + COPY_BOUND, 109_092)]
    );
    assert_eq!(native_words(&io), vec![raw]);
    assert_eq!(r.scan, expected);
    assert_eq!(r.cursor, 3);
    assert_eq!(r.fetch.issued(), issued + 1);
    assert!(!r.dead);
}

#[test]
fn stream_end_in_first_part_never_reads_trigger_shaped_later_part() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let mut words = wide_words(1_026_596);
    words[2] = 7 << 29;
    let cut = COPY_BOUND as usize / 4;
    words[cut] = kf_abi::submit::method_header_inc(6, 0x200, 1).unwrap();
    words[cut + 1] = 27;
    let mut expected = r.scan.fresh_resume();
    expected.segment(&words, sw).unwrap();
    let raw = install_wide(&mut io, &r, words, false);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.part_reads, vec![(STREAM_VA, COPY_BOUND)]);
    assert_eq!(native_words(&io), vec![raw]);
    assert_eq!(r.scan, expected);
    assert!(!r.dead);
}

#[test]
fn stream_trigger_in_second_part_refuses_staged_span_without_publication_or_action() {
    stream_trigger_budget(false);
}
#[test]
fn stream_software_header_at_part_end_is_a_trigger_not_a_cross_gp_packet_error() {
    stream_trigger_budget(true);
}
fn stream_trigger_budget(across_cut: bool) {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    let mut words = wide_words(1_026_596);
    let cut = COPY_BOUND as usize / 4;
    let header = if across_cut { cut - 1 } else { cut + 3 };
    words[header] = kf_abi::submit::method_header_inc(6, 0x200, 1).unwrap();
    words[header + 1] = 27;
    let mut whole = before.clone();
    assert_eq!(whole.segment(&words, sw).unwrap().unwrap().start, header);
    install_wide(&mut io, &r, words, false);
    let issued = r.fetch.issued();
    turn(&mut r, &mut io).unwrap();
    let e = turn(&mut r, &mut io).unwrap_err();
    assert!(
        matches!(e, Fail::Refused(ref why) if why.starts_with("USER_RING_STAGED_SPAN_BUDGET ") && why.contains(&format!("trigger_at={header}"))),
        "{e:?}"
    );
    assert!(r.dead);
    assert_eq!(r.scan, before);
    assert_eq!(r.fetch.issued(), issued);
    assert!(native_words(&io).is_empty());
    assert!(!io.calls.contains(&Call::Stage));
    assert!(format!("{:?}", r.life.counts()).contains("refused_staged_span_budget: 1"));
}

#[test]
fn stream_carries_packet_from_previous_descriptor_and_leaves_open_tail_for_next() {
    let mut io = Mock::new(16);
    let mut r = relay();
    io.put_entry(0, &ge(0, Kind::OpenHead));
    io.put = 1;
    turn(&mut r, &mut io).unwrap();
    let before = r.scan.clone();
    let mut words = vec![0; 1_026_596 / 4];
    words[0] = 0x22;
    words[1] = 0x33;
    *words.last_mut().unwrap() = kf_abi::submit::method_header_inc(1, 0x100, 3).unwrap();
    let mut whole = before.clone();
    whole.segment(&words, sw).unwrap();
    let raw = install_wide(&mut io, &r, words, false);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.scan, before);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.scan, whole);
    assert!(r.scan.carried().is_some());
    assert_eq!(native_words(&io), vec![raw]);
    let next = ge(5, Kind::Plain);
    io.put_entry(r.cursor, &next);
    io.put = r.cursor + 1;
    whole.segment(&next.words(), sw).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.scan, whole);
}

#[test]
fn stream_publication_retry_keeps_scan_uncommitted_and_owes_a_fresh_attempt() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    io.publish_retry_once = true;
    assert_eq!(turn(&mut r, &mut io), Err(Fail::Retry));
    assert_eq!(r.scan, before);
    assert!(native_words(&io).is_empty());
    assert!(r.again);
    assert_eq!(r.need, Need::Again);
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(io.calls.contains(&Call::GpPut));
    assert_eq!(io.part_reads.last(), Some(&(STREAM_VA, COPY_BOUND)));
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.cursor, 3);
}

#[test]
fn stream_read_retry_drops_prior_parts_and_restarts_from_first_part() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    io.read_retry_once = Some(STREAM_VA + COPY_BOUND);
    assert_eq!(turn(&mut r, &mut io), Err(Fail::Retry));
    assert_eq!(r.scan, before);
    assert!(r.again);
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(io.calls.contains(&Call::GpPut));
    assert_eq!(
        io.part_reads,
        vec![(STREAM_VA, COPY_BOUND), (STREAM_VA, COPY_BOUND)]
    );
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.cursor, 3);
}

#[test]
fn stream_maximum_descriptor_reads_at_most_ten_bounded_parts_one_per_turn() {
    let mut io = Mock::new(16);
    let mut r = relay();
    let raw = install_wide(&mut io, &r, wide_words(8_388_604), false);
    for part in 0..10 {
        io.calls.clear();
        let n = io.part_reads.len();
        turn(&mut r, &mut io).unwrap();
        assert_eq!(io.part_reads.len(), n + 1);
        if part != 0 {
            assert!(!io.calls.contains(&Call::GpPut));
        }
        if part != 9 {
            assert!(native_words(&io).is_empty());
            assert!(r.again);
            assert_eq!(r.need, Need::Again);
        }
    }
    assert_eq!(native_words(&io), vec![raw]);
    assert_eq!(io.part_reads.iter().map(|(_, n)| n).sum::<u64>(), 8_388_604);
    assert!(io.part_reads.iter().all(|(_, n)| *n <= COPY_BOUND));
    assert_eq!(r.cursor, 1);
}

#[test]
fn stream_scan_error_retries_from_part_zero_with_original_uncommitted_state() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    let mut words = wide_words(1_026_596);
    let cut = COPY_BOUND as usize / 4;
    words[0] = kf_abi::submit::method_header_inc(7, 0, 1).unwrap();
    words[1] = support::SW;
    words[cut] = 0x0004_0001; // legacy packet with arguments: existing named refusal
    install_wide(&mut io, &r, words, false);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.scan, before);
    turn(&mut r, &mut io).unwrap();
    assert!(!r.dead);
    assert!(r.again);
    assert_eq!(r.scan, before);
    io.wide.as_mut().unwrap().1[cut] = 0;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(io.calls.contains(&Call::GpPut));
    assert_eq!(io.part_reads.last(), Some(&(STREAM_VA, COPY_BOUND)));
    let mut expected = before.clone();
    expected.segment(&io.wide.as_ref().unwrap().1, sw).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.scan, expected);
    assert_eq!(r.cursor, 3);
}

#[test]
fn stream_sync_wait_reads_no_part_until_marker_is_processed_even_if_gp_get_advanced() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let raw = install_wide(&mut io, &r, wide_words(1_026_596), true);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.gates(), 1);
    assert!(io.part_reads.is_empty());
    io.hw_get = r.fetch.issued() as u32 & 15;
    turn(&mut r, &mut io).unwrap();
    assert!(io.part_reads.is_empty());
    io.hw_reference = Marks::value(r.marks.issued());
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.part_reads.len(), 1);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(native_words(&io), vec![raw]);
}

#[test]
fn stream_seeded_fresh_window_gates_before_first_part() {
    let mut io = Mock::new(16);
    let mut r = flipped_relay(&mut io, true);
    let raw = install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.gates(), 1);
    assert!(io.part_reads.is_empty());
    io.hw_get = r.fetch.issued() as u32 & 15;
    io.hw_reference = Marks::value(r.marks.issued());
    turn(&mut r, &mut io).unwrap();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(native_words(&io), vec![raw]);
}

#[test]
fn stream_retire_drops_unpublished_state() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    r.retire();
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(native_words(&io).is_empty());
    assert_eq!(r.scan, before);
    assert_eq!(io.part_reads.len(), 1);
}

#[test]
fn stream_source_switch_drops_unpublished_old_parts() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    r.switch_source(2);
    io.put = 0;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(native_words(&io).is_empty());
    assert_eq!(r.scan, before);
    assert_eq!(io.part_reads.len(), 1);
    assert_eq!(r.generation, 2);
}

#[test]
fn stream_receipt_reservation_failure_refuses_before_read_or_publication() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    io.fail_stream_allocation = true;
    let e = turn(&mut r, &mut io).unwrap_err();
    assert!(
        matches!(e,Fail::Refused(ref why) if why=="USER_RING_STREAM_ALLOC"),
        "{e:?}"
    );
    assert!(r.dead);
    assert!(io.part_reads.is_empty());
    assert!(native_words(&io).is_empty());
}

#[test]
fn stream_walk_busy_pauses_reads_without_losing_the_descriptor() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let raw = install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    io.walk_blocked = true;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert_eq!(io.part_reads.len(), 1);
    assert!(native_words(&io).is_empty());
    io.walk_blocked = false;
    io.calls.clear();
    turn(&mut r, &mut io).unwrap();
    assert!(!io.calls.contains(&Call::GpPut));
    assert_eq!(native_words(&io), vec![raw]);
}

#[test]
fn stream_walk_pause_preserves_uncommitted_parser_state() {
    let mut io = Mock::new(16);
    let mut r = stream_window(&mut io);
    let before = r.scan.fresh_resume();
    install_wide(&mut io, &r, wide_words(1_026_596), false);
    turn(&mut r, &mut io).unwrap();
    io.walk_blocked = true;
    for _ in 0..3 {
        turn(&mut r, &mut io).unwrap();
        assert_eq!(r.scan, before);
        assert_eq!(io.part_reads.len(), 1);
    }
    io.walk_blocked = false;
    turn(&mut r, &mut io).unwrap();
    assert_eq!(r.cursor, 3);
}

#[test]
fn stream_random_packet_forms_masks_triggers_and_end_match_whole_descriptor_scan() {
    use kf_abi::submit::method_header_inc as h;
    for seed in 0..300u32 {
        let mut io = Mock::new(16);
        let mut r = stream_window(&mut io);
        io.scan_bound = 32 + u64::from(seed % 3) * 16;
        let before = r.scan.fresh_resume();
        let mut words = vec![h(0, 0, 1).unwrap(), 0xc7c0];
        for i in 0..5u32 {
            let form = [1, 3, 5, 4][((seed + i) % 4) as usize];
            let n = (seed + i) % 4 + 1;
            words.push((h(1, 0x100, n).unwrap() & !(7 << 29)) | (form << 29));
            if form != 4 {
                words.extend((0..n).map(|k| seed ^ k));
            }
        }
        words.extend([0x0002_0010, 0x0003_0000]);
        if seed % 4 == 0 {
            words.push(7 << 29);
        }
        if seed % 3 == 0 {
            words.push(0x0001_0000);
        } // inactive mask
        if seed % 5 == 0 {
            words.extend([h(6, 0x200, 1).unwrap(), 27]);
        }
        words.resize(words.len().max(io.scan_bound as usize / 4 + 2), 0);
        assert!(words.len() as u64 * 4 <= 10 * io.scan_bound);
        let mut expected = before.clone();
        let trigger = expected.segment(&words, sw).unwrap();
        let raw = install_wide(&mut io, &r, words, false);
        let mut done = false;
        for _ in 0..10 {
            let n = io.part_reads.len();
            io.calls.clear();
            match turn(&mut r, &mut io) {
                Err(Fail::Refused(why)) => {
                    assert!(trigger.is_some(), "seed {seed}: {why}");
                    assert!(
                        why.starts_with("USER_RING_STAGED_SPAN_BUDGET "),
                        "seed {seed}: {why}"
                    );
                    assert_eq!(r.scan, before);
                    assert!(native_words(&io).is_empty());
                    done = true;
                    break;
                }
                Err(e) => panic!("seed {seed}: {e:?}"),
                Ok(()) => {
                    assert_eq!(io.part_reads.len(), n + 1);
                    if r.cursor == 3 {
                        assert!(trigger.is_none());
                        assert_eq!(native_words(&io), vec![raw]);
                        assert_eq!(r.scan, expected);
                        done = true;
                        break;
                    }
                    assert_eq!(r.scan, before);
                }
            }
        }
        assert!(done, "seed {seed}");
    }
}

// Ring characterization: these assertions run unchanged against master, Phase 1 and Phase 2.
fn wrap_relay(entries: u32, cursor: u32, pointer: bool) -> (Relay, Mock) {
    let mut io = Mock::new(entries);
    let mut scan = Scan::for_subdevice(1).unwrap();
    if pointer {
        scan.segment(&ge(0, Kind::SetObj).words(), sw).unwrap();
    }
    let life = if pointer { Lifecycle::new() } else { Lifecycle::always_scan() };
    let mut r = Relay::new(16, life, scan).unwrap();
    r.cursor = cursor;
    r.written_get = cursor;
    r.relayed = u64::from(cursor);
    for slot in 0..entries {
        io.ring[slot as usize] = u64::from(slot) << 2; // format-legal, distinct control words
    }
    (r, io)
}

fn wrap_drain(entries: u32, start: u32, put: u32, pointer: bool, cached: bool) {
    let (mut r, mut io) = wrap_relay(entries, start, pointer);
    r = r.with_lazy(if cached { kf_chan::user_lazy::LazyConfig::ON } else { kf_chan::user_lazy::LazyConfig::OFF });
    io.put = put;
    let mut expected = Vec::new();
    let mut slot = start;
    while slot != put {
        expected.push(u64::from(slot) << 2);
        slot = if slot + 1 == entries { 0 } else { slot + 1 };
    }
    for _ in 0..expected.len() + 3 {
        turn(&mut r, &mut io).unwrap();
        io.hw_get = (r.fetch.issued() & 15) as u32;
    }
    let copied: Vec<u64> = io.calls.iter().filter_map(|c| match c {
        Call::Entry(_, raw) => Some(*raw), _ => None,
    }).collect();
    assert_eq!(copied, expected, "n={entries} start={start} put={put} pointer={pointer} cache={cached}");
    assert_eq!(r.cursor, put);
    assert_eq!(r.written_get, put, "write-back wraps with the native fetch receipts");
    assert_eq!(r.relayed, u64::from(start) + expected.len() as u64);
    assert!(!r.dead);
}

#[test]
fn wrap_characterization_every_small_ring_range_copies_modulo_in_order() {
    for entries in [4, 8, 16] {
        for start in 0..entries {
            for put in 0..entries {
                wrap_drain(entries, start, put, false, false);
                wrap_drain(entries, start, put, true, true);
            }
        }
    }
}

#[test]
fn wrap_characterization_ring_end_ranges_and_maximum_admitted_ring() {
    for entries in [32, 32768, 1 << 20] {
        for (start, put) in [(entries - 2, 2), (entries - 1, 0), (0, 1), (entries - 1, entries - 1)] {
            wrap_drain(entries, start, put, true, false);
            wrap_drain(entries, start, put, true, true);
        }
    }
}

#[test]
fn wrap_characterization_full_means_one_reserved_slot_even_when_put_is_below_get() {
    for start in 0..16 {
        let (mut r, mut io) = wrap_relay(16, start, true);
        io.put = if start == 0 { 15 } else { start - 1 };
        turn(&mut r, &mut io).unwrap();
        assert!(r.put_full);
        assert_eq!(r.cursor, (start + 1) % 16);
        wrap_drain(16, start, io.put, true, true);
    }
}

#[test]
fn wrap_characterization_equal_indices_are_empty_even_after_a_complete_lap() {
    for start in [0, 1, 15] {
        let (mut r, mut io) = wrap_relay(16, start, true);
        io.put = start;
        // GP_PUT has no wrap/phase bit. A producer writing all sixteen slots
        // would present this same value; it is not a second legal full encoding.
        r.relayed += 16;
        turn(&mut r, &mut io).unwrap();
        assert_eq!(r.cursor, start);
        assert!(!r.put_full);
        assert!(!io.calls.iter().any(|c| matches!(c, Call::Load(..) | Call::Entry(..) | Call::ReadPb(..))));
    }
}

#[test]
fn wrap_characterization_u32_put_edges_refuse_before_a_slot_load() {
    for put in [16, 17, 1 << 31, u32::MAX - 1, u32::MAX] {
        let (mut r, mut io) = wrap_relay(16, 15, true);
        io.put = put;
        assert_eq!(turn(&mut r, &mut io), Err(Fail::Refused("USER_RING_PUT_BOUND".into())));
        assert!(r.dead);
        assert!(!io.calls.iter().any(|c| matches!(c, Call::Load(..) | Call::Entry(..) | Call::ReadPb(..))));
    }
}

#[test]
fn wrap_characterization_native_get_wrap_and_unissued_get_edges_preserve_receipts() {
    let mut f = Fetch::new(16).unwrap();
    for round in 0..20u32 {
        let mut expected = Vec::new();
        for at in 0..15u32 {
            let receipt = (1, round * 15 + at);
            f.publish(Some(receipt)).unwrap();
            expected.push(receipt);
        }
        assert_eq!(f.available(), 0);
        assert!(f.publish(Some((1, 999))).is_err());
        let get = (f.issued() & 15) as u32;
        assert_eq!(f.observe(get).unwrap(), expected);
        assert_eq!(f.available(), 15);
        assert!(f.observe(get).unwrap().is_empty());
        let old = f.fetched();
        for invalid in [16, 1 << 31, u32::MAX] {
            assert_eq!(f.peek(invalid), Err("USER_FETCH_GET"));
            assert_eq!(f.observe(invalid), Err("USER_FETCH_GET"));
            assert_eq!(f.fetched(), old);
        }
        assert_eq!(f.observe((get + 1) % 16), Err("USER_FETCH_UNISSUED_GET"));
        assert_eq!(f.fetched(), old);
    }
}
