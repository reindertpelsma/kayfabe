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
use kf_chan::user_stream::Source;
use kf_chan::user_turn::{
    Ev, Fail, GpSegment, Io, Phase, PubKind, Relay, Staged, Step, SwView, Turn,
};
use std::collections::HashMap;
use support::{Ge, Kind, SW, ge, sw};

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
        Ok((0, 0))
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
