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

/// What the relay did to the world, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    HwWords,
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
        Ok((self.hw_get, 0))
    }
    fn write_guest_get(&mut self, get: u32) -> Turn<bool> {
        self.calls.push(Call::WriteGuestGet(get));
        Ok(true)
    }
    fn src_entries(&mut self) -> u32 {
        self.entries
    }
    fn max_entry_bytes(&mut self) -> u64 {
        kf_chan::user_storage::MAX_SOURCE_BYTES
    }
    fn gp_put(&mut self) -> Turn<Option<u32>> {
        self.calls.push(Call::GpPut);
        Ok(Some(self.put))
    }
    fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32> {
        self.calls.push(Call::Load(slot, hi));
        let v = self.ring[slot as usize];
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
    fn read_pb(&mut self, entry: u64, _seg: &GpSegment) -> Turn<(Vec<u32>, bool)> {
        self.calls.push(Call::ReadPb(entry));
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
    fn publish_entry(
        &mut self,
        kind: PubKind,
        fetch: &mut Fetch,
        generation: u64,
        guest_get: u32,
        entry: u64,
    ) -> Turn<()> {
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
    // Popped: nothing written back yet; ReadIndex (Change A): the read index of the prelude's first entry
    let want = if kf_chan::user_turn::guest_get_follows_pops() {
        0
    } else {
        1
    };
    assert_eq!(r.written_get, want, "what the guest sees before the turn");
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

// ------------------------------------------------------------------------------------------------
// ★ 2026-10-20 (simple relay, Change A): COPY BEFORE GET (hostile guest, TOCTOU)

/// Whatever the guest-visible `GP_GET` semantics (`user_turn::GUEST_GET`), the relay forwards a slot's value as it was BEFORE the
/// guest-visible `GP_GET` passed that slot: the guest overwrites every slot as soon as `GP_GET` passes it (legal: `GP_GET` says
/// the slot is free) and nothing it writes afterwards may reach the private ring. FALSIFIER: a published entry equal to a
/// post-GET overwrite, or the guest's `GP_GET` never reaching `GP_PUT`.
#[test]
fn copy_before_get_a_slot_overwritten_after_get_passed_it_is_never_forwarded() {
    let mut io = Mock::new(16);
    let mut r = pointer_relay(&mut io);
    let mut written: u32 = 2; // the guest GET after the prelude (slots 0 and 1 consumed)
    let originals: Vec<u64> = (2u8..12).map(|i| ge(i, Kind::Plain).raw()).collect();
    for (k, raw) in originals.iter().enumerate() {
        io.ring[2 + k] = *raw;
    }
    io.put = 12;
    let garbage = ge(0x77, Kind::Plain).raw();
    for _ in 0..200 {
        // the PBDMA pops everything published so far (the Popped semantics need it to free the guest)
        io.hw_get = (r.fetch.issued() % 16) as u32;
        turn(&mut r, &mut io).unwrap();
        let get = io
            .calls
            .iter()
            .rev()
            .find_map(|c| {
                if let Call::WriteGuestGet(g) = c {
                    Some(*g)
                } else {
                    None
                }
            })
            .unwrap_or(written);
        // the hostile guest reuses every slot the visible GET passed, at once
        let mut s = written;
        while s != get {
            io.ring[s as usize] = garbage;
            s = (s + 1) % 16;
        }
        written = get;
        if written == io.put && r.cursor == io.put {
            break;
        }
    }
    let forwarded: Vec<u64> = io
        .calls
        .iter()
        .filter_map(|c| {
            if let Call::Entry(_, e) = c {
                Some(*e)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(written, io.put, "the guest's GP_GET reached GP_PUT");
    assert!(
        !forwarded.contains(&garbage),
        "a post-GET overwrite was forwarded: {forwarded:x?}"
    );
    assert_eq!(
        forwarded, originals,
        "what was forwarded is exactly what the guest wrote before GET passed it"
    );
}
