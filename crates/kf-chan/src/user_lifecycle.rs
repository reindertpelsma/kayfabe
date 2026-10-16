//! ★ 2026-10-16 — **the Windows USER-ring lifecycle: Scan, Pointer, Window**
//! (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`).
//!
//! The pure state machine the relay (`crates/kf-qemu/src/userpump.rs`), the exhaustive model
//! (`tests/user_lifecycle_model.rs`) and the property tests share. No I/O, no clock, no lock.
//!
//! STUB (tests-first commit): every entry plans a scan, i.e. the behaviour of the base revision.

/// What the relay does with the guest's pushbuffers of a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// From birth until the software object's binding is known.
    Scan,
    /// Copy GP entries as they are; no pushbuffer byte is read.
    Pointer,
    /// A registration is live: scan again until it is gone.
    Window,
    /// Freed. Absorbs every later event.
    Dead,
}

/// How a Window began.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Start {
    /// Continues a Scan that never stopped: the parser state is exact.
    Exact,
    /// Starts after Pointer mode: the parser state is restarted (assumption P2).
    Fresh,
}

/// The only way to read a pushbuffer: obtained from a [`Plan::Scan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScanPermit(());

/// What to do with the entry just loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Publish the loaded value as it is; read nothing.
    Forward,
    /// Scan the pushbuffer before forwarding.
    Scan {
        /// Restart the parser state (and the gate) first, once.
        restart: bool,
        /// Gate this entry as if it were a SYNC_WAIT entry (an unproven one precedes the Window).
        seed_gate: bool,
        /// The right to read.
        permit: ScanPermit,
    },
}

/// Why the mode changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The scan committed a SET_OBJECT binding the software object (no registration is live).
    SoftwareBound,
    /// The same, with a registration already live: the scan simply continues as a Window.
    SoftwareBoundRegistered,
    /// The registry shows a live registration.
    Registration,
    /// The registry shows none and the relay is quiescent.
    RegistryDrained,
    /// The channel was freed.
    Teardown,
}

/// One observed transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    /// Before.
    pub from: Mode,
    /// Why.
    pub cause: Cause,
    /// After.
    pub to: Mode,
}

/// Per-channel counters. All plain integers; merged into the VM-wide atomics by the relay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Entries forwarded per mode.
    pub entries_scan: u64,
    /// See above.
    pub entries_pointer: u64,
    /// See above.
    pub entries_window: u64,
    /// Scan -> Pointer.
    pub to_pointer: u64,
    /// Scan -> Window.
    pub to_window_exact: u64,
    /// Pointer -> Window.
    pub to_window_fresh: u64,
    /// Window -> Pointer.
    pub to_pointer_from_window: u64,
    /// Fresh restarts applied.
    pub restarts: u64,
    /// Gates seeded from an unproven Pointer-mode SYNC_WAIT and proven.
    pub seeded_gates: u64,
    /// Longest Window, in entries.
    pub window_entries_max: u64,
    /// Segments scanned per mode.
    pub segments_scan: u64,
    /// See above.
    pub segments_window: u64,
    /// Segments that END inside an open packet, per mode (the P2 measurement).
    pub straddles_scan: u64,
    /// See above.
    pub straddles_window: u64,
    /// A scan refusal in the first segments of a Fresh window.
    pub window_fresh_start_parse_error: u64,
    /// The software binding a Window relies on changed (P1 falsifier).
    pub window_binding_changed: u64,
    /// Entries a Window forwarded without scanning (0 by construction).
    pub window_unscanned_long: u64,
    /// Pushbuffer bytes read, per mode.
    pub pb_bytes_scan: u64,
    /// See above.
    pub pb_bytes_window: u64,
    /// Of the Window's, the bytes read from video memory.
    pub window_bar1_bytes: u64,
    /// A pushbuffer read in Pointer mode (must stay 0).
    pub pointer_pb_read_violation: u64,
    /// Channels that never had a software object to bind.
    pub scan_no_software: u64,
    /// Events after Teardown.
    pub after_teardown: u64,
}

macro_rules! counts_since {
    ($a:expr, $b:expr, $($f:ident),*) => { Counts { $($f: $a.$f - $b.$f,)* window_entries_max: 0 } };
}
impl Counts {
    /// Field-wise `self - prev` (`window_entries_max` is a maximum, not a sum, and is reported as 0).
    #[must_use]
    pub fn since(&self, prev: &Counts) -> Counts {
        counts_since!(
            self, prev, entries_scan, entries_pointer, entries_window, to_pointer, to_window_exact,
            to_window_fresh, to_pointer_from_window, restarts, seeded_gates, segments_scan,
            segments_window, straddles_scan, straddles_window, window_fresh_start_parse_error,
            window_binding_changed, window_unscanned_long, pb_bytes_scan, pb_bytes_window,
            window_bar1_bytes, pointer_pb_read_violation, scan_no_software, after_teardown
        )
    }
}

/// What the relay observed at the entry it just loaded.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    /// The ONE loaded 8-byte value.
    pub raw: u64,
    /// Monotone count of guest entries relayed before this one.
    pub abs: u64,
    /// The registry epoch, loaded AFTER `raw`.
    pub epoch: u64,
    /// Subchannels (bit n) on which the scan has the software object bound; 0 outside Scan/Window.
    pub sw_mask: u8,
    /// The channel has a software object at all.
    pub have_software: bool,
    /// No gate is draining and no segment is in flight.
    pub quiescent: bool,
}

/// The pure lifecycle of one channel.
#[derive(Clone, Debug)]
pub struct Lifecycle {
    mode: Mode,
    counts: Counts,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Lifecycle {
    /// A channel at birth: Scan.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mode: Mode::Scan,
            counts: Counts::default(),
        }
    }
    /// The base revision's behaviour: scan every entry for ever. For differential tests.
    /// STUB (tests-first commit).
    #[must_use]
    pub fn always_scan() -> Self {
        Self::new()
    }
    /// Everything that decides behaviour, none of the counters (model checking).
    /// STUB (tests-first commit).
    #[must_use]
    pub fn state_key(&self) -> String {
        String::new()
    }
    /// The current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }
    /// How the current Window began.
    #[must_use]
    pub fn start(&self) -> Option<Start> {
        None
    }
    /// The newest forwarded SYNC_WAIT entry of Pointer mode that no marker has proven passed.
    #[must_use]
    pub fn outstanding_sync(&self) -> Option<u64> {
        None
    }
    /// The last live-registration count observed.
    #[must_use]
    pub fn live(&self) -> u32 {
        0
    }
    /// The transition the last call made, if any (taken).
    pub fn take_transition(&mut self) -> Option<Transition> {
        None
    }
    /// Decide what to do with the entry just loaded.
    pub fn entry(&mut self, _o: &Obs, _live: impl FnOnce() -> u32) -> Plan {
        Plan::Scan {
            restart: false,
            seed_gate: false,
            permit: ScanPermit(()),
        }
    }
    /// The entry was published.
    pub fn forwarded(&mut self, _abs: u64, _raw: u64) {}
    /// The relay applied the restart.
    pub fn restarted(&mut self) {}
    /// The seeded gate's witness was processed.
    pub fn gate_proven(&mut self) {}
    /// A segment was scanned and committed.
    pub fn segment_scanned(&mut self, _ends_open: bool) {}
    /// A scan refused a segment (first time for that entry).
    pub fn scan_refused(&mut self) {}
    /// A pushbuffer was read (via a [`ScanPermit`]).
    pub fn note_pb_read(&mut self, _permit: &ScanPermit, _bytes: usize, _vram: bool) {}
    /// A pushbuffer was read WITHOUT a permit, i.e. a bug; counted loudly.
    pub fn note_unpermitted_pb_read(&mut self, _bytes: usize) {}
    /// A marker segment may be authored between GP entries now.
    #[must_use]
    pub fn marker_possible(&self, _scan_carried: bool) -> bool {
        !_scan_carried
    }
    /// The channel was freed.
    pub fn teardown(&mut self) {}
    /// Counters so far.
    #[must_use]
    pub fn counts(&self) -> &Counts {
        &self.counts
    }
    /// Counters since the last call.
    pub fn take_delta(&mut self) -> Counts {
        Counts::default()
    }
}

/// The transition table (I5). `live` = live registrations, `bound` = the software object is bound.
#[must_use]
pub fn next(_mode: Mode, _cause: Cause) -> Option<Mode> {
    None
}

/// The sync-wait bit of a GP entry (bit 31 of the high dword).
pub const GP_SYNC_WAIT: u64 = 1 << 63;

#[cfg(test)]
mod tests {
    use super::*;

    const SYNC: u64 = GP_SYNC_WAIT;
    const B: u8 = 1 << 6;
    fn obs(abs: u64, raw: u64, epoch: u64, sw_mask: u8) -> Obs {
        Obs {
            raw,
            abs,
            epoch,
            sw_mask,
            have_software: true,
            quiescent: true,
        }
    }
    fn scan(plan: Plan) -> (bool, bool) {
        match plan {
            Plan::Scan {
                restart, seed_gate, ..
            } => (restart, seed_gate),
            Plan::Forward => panic!("expected a scan, got Forward"),
        }
    }

    #[test]
    fn a_channel_is_scanned_until_its_software_object_is_bound_and_then_it_is_pointer() {
        let mut l = Lifecycle::new();
        assert_eq!(l.mode(), Mode::Scan);
        for i in 0..5 {
            assert!(matches!(l.entry(&obs(i, 0x1000, 0, 0), || 0), Plan::Scan { .. }));
            l.forwarded(i, 0x1000);
        }
        assert_eq!(l.mode(), Mode::Scan);
        assert_eq!(l.entry(&obs(5, 0x1000, 0, 1 << 6), || 0), Plan::Forward);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!(
            l.take_transition(),
            Some(Transition {
                from: Mode::Scan,
                cause: Cause::SoftwareBound,
                to: Mode::Pointer
            })
        );
        assert_eq!(l.counts().to_pointer, 1);
    }

    #[test]
    fn a_registration_live_when_the_binding_is_seen_continues_the_scan_as_an_exact_window() {
        let mut l = Lifecycle::new();
        let p = l.entry(&obs(0, 0x1000, 3, 1 << 6), || 2);
        assert_eq!(scan(p), (false, false), "no restart, no seed: the parser state is exact");
        assert_eq!((l.mode(), l.start()), (Mode::Window, Some(Start::Exact)));
        assert_eq!(l.counts().to_window_exact, 1);
    }

    #[test]
    fn pointer_mode_forwards_and_never_asks_for_a_scan() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        for i in 1..1000 {
            assert_eq!(l.entry(&obs(i, 0x2000 + i, 0, B), || panic!("no epoch change")), Plan::Forward);
            l.forwarded(i, 0x2000 + i);
        }
        assert_eq!(l.counts().entries_pointer, 999);
    }

    #[test]
    fn a_new_epoch_with_a_live_registration_opens_a_fresh_window_at_that_entry() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        assert_eq!(l.entry(&obs(1, 0x10, 0, B), || 0), Plan::Forward);
        l.forwarded(1, 0x10);
        let p = l.entry(&obs(2, 0x20, 1, B), || 1);
        assert_eq!(scan(p), (true, false), "restart once; nothing to seed without an unproven SYNC_WAIT");
        assert_eq!((l.mode(), l.start()), (Mode::Window, Some(Start::Fresh)));
        assert_eq!(l.counts().to_window_fresh, 1);
        assert_eq!(l.live(), 1);
    }

    #[test]
    fn an_epoch_change_without_a_live_registration_of_this_object_changes_nothing() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        let mut asked = 0;
        for e in 1..50u64 {
            assert_eq!(
                l.entry(&obs(e, e, e, B), || {
                    asked += 1;
                    0
                }),
                Plan::Forward
            );
        }
        assert_eq!(asked, 49, "the registry is consulted once per epoch change, never otherwise");
        assert_eq!(l.mode(), Mode::Pointer);
    }

    #[test]
    fn the_restart_is_requested_until_applied_and_the_seed_until_the_witness_is_proven() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.entry(&obs(1, SYNC | 0x10, 0, B), || 0);
        l.forwarded(1, SYNC | 0x10);
        assert_eq!(l.outstanding_sync(), Some(1));
        let p = l.entry(&obs(2, 0x20, 1, B), || 1);
        assert_eq!(scan(p), (true, true));
        // the same entry presented again (a retried turn): the restart stays until applied
        assert_eq!(scan(l.entry(&obs(2, 0x20, 1, B), || 1)), (true, true));
        l.restarted();
        assert_eq!(scan(l.entry(&obs(2, 0x20, 1, B), || 1)), (false, true));
        l.gate_proven();
        assert_eq!(scan(l.entry(&obs(2, 0x20, 1, B), || 1)), (false, false));
        assert_eq!(l.outstanding_sync(), None);
        assert_eq!((l.counts().restarts, l.counts().seeded_gates), (1, 1));
    }

    #[test]
    fn the_newest_pointer_sync_wait_is_recorded_by_monotone_index_across_ring_laps() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.entry(&obs(1, 0x10, 0, B), || 0);
        l.forwarded(1, SYNC | 0x10);
        // many ring laps of ordinary entries later the record is the same absolute index
        for i in 2..(3 * 32768u64) {
            l.entry(&obs(i, 0x30, 0, B), || 0);
            l.forwarded(i, 0x30);
        }
        assert_eq!(l.outstanding_sync(), Some(1));
        l.entry(&obs(3 * 32768, SYNC | 0x40, 0, B), || 0);
        l.forwarded(3 * 32768, SYNC | 0x40);
        assert_eq!(l.outstanding_sync(), Some(3 * 32768), "the newest one, as a u64");
    }

    #[test]
    fn a_window_ends_only_when_the_registry_is_empty_and_the_relay_is_quiescent() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 5, 1 << 6), || 1);
        assert_eq!(l.mode(), Mode::Window);
        // a gate is draining: even an empty registry cannot end the window
        let mut o = obs(1, 0x10, 6, 1 << 6);
        o.quiescent = false;
        assert!(matches!(l.entry(&o, || 0), Plan::Scan { .. }));
        assert_eq!(l.mode(), Mode::Window);
        o.quiescent = true;
        assert_eq!(l.entry(&o, || 0), Plan::Forward);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!(l.counts().to_pointer_from_window, 1);
        assert_eq!(l.outstanding_sync(), None);
    }

    #[test]
    fn a_window_whose_trigger_never_comes_never_ends_and_no_time_is_involved() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 1, 1 << 6), || 1);
        for i in 1..200_000u64 {
            assert!(matches!(l.entry(&obs(i, 0x10, 1, 1 << 6), || panic!("epoch unchanged")), Plan::Scan { .. }));
            l.forwarded(i, 0x10);
        }
        assert_eq!(l.mode(), Mode::Window);
        assert_eq!(l.counts().entries_window, 199_999);
        assert_eq!(l.counts().window_entries_max, 199_999);
    }

    #[test]
    fn two_registrations_back_to_back_keep_one_window_open_until_both_are_gone() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.entry(&obs(1, 0, 1, B), || 1);
        l.entry(&obs(2, 0, 2, B), || 2);
        assert_eq!((l.mode(), l.counts().to_window_fresh), (Mode::Window, 1));
        l.entry(&obs(3, 0, 3, B), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.entry(&obs(4, 0, 4, B), || 0);
        assert_eq!(l.mode(), Mode::Pointer);
    }

    #[test]
    fn a_new_registration_after_a_window_repeats_the_cycle() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        for round in 0..100u64 {
            let e = round * 2 + 1;
            assert!(matches!(l.entry(&obs(10 * round + 1, 0, e, B), || 1), Plan::Scan { restart: true, .. }));
            l.restarted();
            assert_eq!(l.entry(&obs(10 * round + 2, 0, e + 1, B), || 0), Plan::Forward);
        }
        assert_eq!(l.counts().to_window_fresh, 100);
        assert_eq!(l.counts().to_pointer_from_window, 100);
    }

    #[test]
    fn teardown_is_terminal_and_a_late_event_cannot_revive_the_channel() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.teardown();
        assert_eq!(l.mode(), Mode::Dead);
        let p = l.entry(&obs(1, 0, 9, B), || 1);
        assert!(matches!(p, Plan::Scan { .. }), "the safe plan");
        assert_eq!(l.mode(), Mode::Dead);
        assert!(l.counts().after_teardown >= 1);
    }

    #[test]
    fn the_transition_table_is_exactly_the_documented_one() {
        use Cause::*;
        use Mode::*;
        assert_eq!(next(Scan, SoftwareBound), Some(Pointer));
        assert_eq!(next(Scan, SoftwareBoundRegistered), Some(Window));
        assert_eq!(next(Scan, Registration), None, "a Scan that has not bound yet keeps scanning anyway");
        assert_eq!(next(Pointer, Registration), Some(Window));
        assert_eq!(next(Window, RegistryDrained), Some(Pointer));
        for m in [Scan, Pointer, Window, Dead] {
            assert_eq!(next(m, Teardown), Some(Dead));
        }
        // nothing else: in particular no way back to Scan, and nothing leaves Dead
        for m in [Scan, Pointer, Window, Dead] {
            for c in [SoftwareBound, SoftwareBoundRegistered, Registration, RegistryDrained, Teardown] {
                if let Some(to) = next(m, c) {
                    assert_ne!(to, Scan, "{m:?} {c:?}");
                    if m == Dead {
                        assert_eq!(to, Dead);
                    }
                }
            }
        }
        assert_eq!(next(Pointer, SoftwareBound), None);
        assert_eq!(next(Pointer, SoftwareBoundRegistered), None);
        assert_eq!(next(Window, SoftwareBoundRegistered), None);
        assert_eq!(next(Window, SoftwareBound), None);
        assert_eq!(next(Pointer, RegistryDrained), None);
        assert_eq!(next(Scan, RegistryDrained), None);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn a_pushbuffer_read_in_pointer_mode_is_counted_loudly() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.note_unpermitted_pb_read(64);
        assert_eq!(l.counts().pointer_pb_read_violation, 1);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "pushbuffer read in Pointer mode")]
    fn a_pushbuffer_read_in_pointer_mode_trips_the_debug_assertion() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.note_unpermitted_pb_read(64);
    }

    #[test]
    fn pushbuffer_reads_are_counted_per_mode_with_the_window_bar1_share() {
        let mut l = Lifecycle::new();
        let p = l.entry(&obs(0, 0, 0, 0), || 0);
        let Plan::Scan { permit, .. } = p else { panic!() };
        l.note_pb_read(&permit, 100, false);
        l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        let p = l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        let Plan::Scan { permit, .. } = p else { panic!() };
        l.note_pb_read(&permit, 40, true);
        l.note_pb_read(&permit, 8, false);
        let c = l.counts();
        assert_eq!((c.pb_bytes_scan, c.pb_bytes_window, c.window_bar1_bytes), (100, 48, 40));
    }

    #[test]
    fn segments_ending_in_an_open_packet_are_counted_per_mode_and_a_fresh_start_refusal_loudly() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 0), || 0);
        l.segment_scanned(true);
        l.segment_scanned(false);
        assert_eq!((l.counts().segments_scan, l.counts().straddles_scan), (2, 1));
        // bind with a registration live: an EXACT window; a refusal there is not a fresh-start error
        l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        l.scan_refused();
        assert_eq!(l.counts().window_fresh_start_parse_error, 0);
        l.segment_scanned(true);
        assert_eq!((l.counts().segments_window, l.counts().straddles_window), (1, 1));

        let mut f = Lifecycle::new();
        f.entry(&obs(0, 0, 0, 1 << 6), || 0);
        f.entry(&obs(1, 0, 1, B), || 1);
        f.scan_refused();
        assert_eq!(f.counts().window_fresh_start_parse_error, 1);
        for _ in 0..4 {
            f.segment_scanned(false);
        }
        f.scan_refused();
        assert_eq!(
            f.counts().window_fresh_start_parse_error,
            1,
            "only the first segments of a fresh window are the P2 measurement"
        );
    }

    #[test]
    fn a_changed_software_binding_inside_a_window_is_counted() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        l.entry(&obs(2, 0, 1, 1 << 6), || 1);
        assert_eq!(l.counts().window_binding_changed, 0);
        l.entry(&obs(3, 0, 1, 1 << 7), || 1);
        assert_eq!(l.counts().window_binding_changed, 1, "5080 moved to another subchannel");
        l.entry(&obs(4, 0, 1, 0), || 1);
        assert_eq!(l.counts().window_binding_changed, 2, "5080 unbound");
    }

    #[test]
    fn a_channel_without_a_software_object_scans_forever_and_is_counted_once() {
        let mut l = Lifecycle::new();
        for i in 0..1000 {
            let mut o = obs(i, 0, 0, 0);
            o.have_software = false;
            assert!(matches!(l.entry(&o, || 0), Plan::Scan { .. }));
        }
        assert_eq!((l.mode(), l.counts().scan_no_software), (Mode::Scan, 1));
    }

    #[test]
    fn markers_need_a_mode_that_knows_the_parser_state() {
        let mut l = Lifecycle::new();
        assert!(l.marker_possible(false));
        assert!(!l.marker_possible(true), "a packet is open");
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        assert!(!l.marker_possible(false), "Pointer mode has no parser state");
        l.entry(&obs(1, 0, 1, B), || 1);
        assert!(l.marker_possible(false));
    }

    #[test]
    fn deltas_are_what_changed_since_the_last_take() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        l.forwarded(0, 1);
        let a = l.take_delta();
        assert_eq!(a.to_pointer, 1);
        l.entry(&obs(1, 0, 0, B), || 0);
        l.forwarded(1, 1);
        let b = l.take_delta();
        assert_eq!((b.to_pointer, b.entries_pointer), (0, 1));
    }
}
