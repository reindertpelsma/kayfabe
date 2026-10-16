//! ★ 2026-10-16 — **the Windows USER-ring lifecycle: Scan, Pointer, Window**
//! (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`).
//!
//! The pure state machine the relay (`crates/kf-qemu/src/userpump.rs`), the exhaustive model
//! (`tests/user_lifecycle_model.rs`) and the property tests share. No I/O, no clock, no lock.
//!
//! Scan until the software object's binding is known; then Pointer (no pushbuffer byte is read) while no
//! registration is live; Window (scan again, with the SYNC_WAIT gate) while one is. See the design document
//! for the invariants I1..I7 and the tests that enforce them.

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
    /// The channel was torn down: do nothing at all (no read, no publication).
    Stop,
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
    /// PHASE 1b. GP_PUT values the cursor reached while the parser state was KNOWN (Scan), uncapped: the
    /// measurement of the anchor assumption P2' ("a driver does not end a submission inside a packet").
    pub put_boundaries_scan: u64,
    /// Of those, the ones with a packet OPEN at the boundary (the falsifier of P2'; nonzero anywhere = the anchor
    /// design must not be trusted for that workload).
    pub put_boundary_open_scan: u64,
    /// See above, in a Window.
    pub put_boundaries_window: u64,
    /// See above, in a Window.
    pub put_boundary_open_window: u64,
    /// Batch / credit markers authored in Pointer mode (at a GP_PUT boundary).
    pub pointer_markers: u64,
    /// A Pointer-mode marker that could not be placed (the cursor is not at an observed GP_PUT): the wake for that
    /// work is the bounded spin or the 50 ms backstop. Loud on purpose.
    pub pointer_carried_no_marker: u64,
    /// Backstop wakes of a Pointer-mode channel that found work (a timer woke it: counted loudly).
    pub pointer_backstop: u64,
    /// Of those, the ones beyond about one per second.
    pub pointer_backstop_loud: u64,
    /// Entries forwarded unscanned in Pointer mode AFTER the registry showed a registration, up to the anchor.
    pub anchor_forwarded: u64,
    /// Entries scanned after a source switch before the Window may end (detection aid for the cursor semantics).
    pub post_switch_scanned: u64,
    /// The new ring's USERD showed a nonzero GP_GET at the switch: reset-to-0 (production) is contradicted.
    pub switch_cursor_mismatch: u64,
    /// Pointer-mode entries forwarded whose software-subchannel binding could not be re-checked, counted when the
    /// stretch ends in a Fresh window.
    pub pointer_binding_unchecked: u64,
    /// The software object was found gone in Pointer mode (refused like the Scan path).
    pub pointer_software_gone: u64,
}

macro_rules! counts_since {
    ($a:expr, $b:expr, $($f:ident),*) => { Counts { $($f: $a.$f - $b.$f,)* window_entries_max: 0 } };
}
impl Counts {
    /// Field-wise `self - prev` (`window_entries_max` is a maximum, not a sum, and is reported as 0).
    #[must_use]
    pub fn since(&self, prev: &Counts) -> Counts {
        counts_since!(
            self,
            prev,
            entries_scan,
            entries_pointer,
            entries_window,
            to_pointer,
            to_window_exact,
            to_window_fresh,
            to_pointer_from_window,
            restarts,
            seeded_gates,
            segments_scan,
            segments_window,
            straddles_scan,
            straddles_window,
            window_fresh_start_parse_error,
            window_binding_changed,
            window_unscanned_long,
            pb_bytes_scan,
            pb_bytes_window,
            window_bar1_bytes,
            pointer_pb_read_violation,
            scan_no_software,
            after_teardown,
            put_boundaries_scan,
            put_boundary_open_scan,
            put_boundaries_window,
            put_boundary_open_window,
            pointer_markers,
            pointer_carried_no_marker,
            pointer_backstop,
            pointer_backstop_loud,
            anchor_forwarded,
            post_switch_scanned,
            switch_cursor_mismatch,
            pointer_binding_unchecked,
            pointer_software_gone
        )
    }
}

macro_rules! lifecycle_stats {
    ($($f:ident),*) => {
        /// VM-wide lifecycle counters (fixed atomics, no per-event allocation): the per-channel
        /// [`Counts`] merged by the relay, one bounded line at device stop.
        #[derive(Debug, Default)]
        pub struct LifecycleStats {
            $(
                #[doc = concat!("Sum over channels of `Counts::", stringify!($f), "`.")]
                pub $f: std::sync::atomic::AtomicU64,
            )*
            /// Longest Window of any channel, in entries (a maximum).
            pub window_entries_max: std::sync::atomic::AtomicU64,
        }
        impl LifecycleStats {
            /// Merge one channel's delta ([`Lifecycle::take_delta`]).
            pub fn add(&self, d: &Counts) {
                use std::sync::atomic::Ordering::Relaxed;
                $( if d.$f != 0 { self.$f.fetch_add(d.$f, Relaxed); } )*
            }
            /// Record a channel's longest Window.
            pub fn window_max(&self, entries: u64) {
                self.window_entries_max.fetch_max(entries, std::sync::atomic::Ordering::Relaxed);
            }
            /// Whether any channel ever left Scan or scanned anything.
            #[must_use]
            pub fn any(&self) -> bool {
                use std::sync::atomic::Ordering::Relaxed;
                self.entries_scan.load(Relaxed) + self.entries_pointer.load(Relaxed) + self.entries_window.load(Relaxed) > 0
            }
            /// One bounded line.
            #[must_use]
            pub fn summary(&self) -> String {
                use std::sync::atomic::Ordering::Relaxed;
                let mut s = String::new();
                $( s.push_str(&format!("{}={} ", stringify!($f), self.$f.load(Relaxed))); )*
                s.push_str(&format!("window_entries_max={}", self.window_entries_max.load(Relaxed)));
                s
            }
        }
    };
}
lifecycle_stats!(
    entries_scan,
    entries_pointer,
    entries_window,
    to_pointer,
    to_window_exact,
    to_window_fresh,
    to_pointer_from_window,
    restarts,
    seeded_gates,
    segments_scan,
    segments_window,
    straddles_scan,
    straddles_window,
    window_fresh_start_parse_error,
    window_binding_changed,
    window_unscanned_long,
    pb_bytes_scan,
    pb_bytes_window,
    window_bar1_bytes,
    pointer_pb_read_violation,
    scan_no_software,
    after_teardown,
    put_boundaries_scan,
    put_boundary_open_scan,
    put_boundaries_window,
    put_boundary_open_window,
    pointer_markers,
    pointer_carried_no_marker,
    pointer_backstop,
    pointer_backstop_loud,
    anchor_forwarded,
    post_switch_scanned,
    switch_cursor_mismatch,
    pointer_binding_unchecked,
    pointer_software_gone
);

/// What the relay observed at the entry it just loaded.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    /// The ONE loaded 8-byte value.
    pub raw: u64,
    /// Monotone count of guest entries relayed before this one.
    pub abs: u64,
    /// The registry epoch, loaded AFTER `raw`.
    pub epoch: u64,
    /// PHASE 1b. The absolute index of the `GP_PUT` value read in THIS turn (before `raw` and the epoch): the end of
    /// a guest submission, the anchor at which a Window may start.
    pub put_abs: u64,
    /// Subchannels (bit n) on which the scan has the software object bound; 0 outside Scan/Window.
    pub sw_mask: u8,
    /// The channel has a software object at all.
    pub have_software: bool,
    /// No gate is draining and no segment is in flight.
    pub quiescent: bool,
}

/// A deliberate defect of the machine, for the mutation checks (`tests/user_lifecycle_mutants.rs`).
#[cfg(feature = "lifecycle-mutants")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutant {
    /// I1: Pointer mode plans a scan.
    PointerScans,
    /// I3: a Fresh window never seeds its gate.
    NoSeed,
    /// I3: Pointer mode forgets the SYNC_WAIT it forwarded.
    ForgetSync,
    /// I5: a registration in Pointer mode moves the channel back to Scan.
    PointerToScan,
    /// A teardown does not stop the machine.
    ReviveAfterTeardown,
    /// I4: a Window ends as soon as the relay is quiescent, whatever the registry says.
    ExitIgnoresLive,
    /// P1: a Window keeps the binding it started with (a moved binding is never noticed).
    MaskFrozen,
    /// D1: a Window starts at the first entry that sees the registration, not at the anchor.
    AnchorIgnored,
    /// D3: Pointer mode places a marker wherever it is asked (inside a possibly open packet).
    PointerMarkersAnywhere,
    /// D3: Pointer mode never places a marker (the base behaviour).
    PointerNoMarkers,
}

#[cfg(feature = "lifecycle-mutants")]
impl Mutant {
    /// The mutant named by `KF_LIFECYCLE_MUTANT` (the mutation check script sets it).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("KF_LIFECYCLE_MUTANT").ok()?.as_str() {
            "pointer_scans" => Some(Self::PointerScans),
            "no_seed" => Some(Self::NoSeed),
            "forget_sync" => Some(Self::ForgetSync),
            "pointer_to_scan" => Some(Self::PointerToScan),
            "revive_after_teardown" => Some(Self::ReviveAfterTeardown),
            "exit_ignores_live" => Some(Self::ExitIgnoresLive),
            "mask_frozen" => Some(Self::MaskFrozen),
            "anchor_ignored" => Some(Self::AnchorIgnored),
            "pointer_markers_anywhere" => Some(Self::PointerMarkersAnywhere),
            "pointer_no_markers" => Some(Self::PointerNoMarkers),
            _ => None,
        }
    }
}

/// The pure lifecycle of one channel.
#[derive(Clone, Debug)]
pub struct Lifecycle {
    mode: Mode,
    start: Option<Start>,
    counts: Counts,
    reported: Counts,
    legacy: bool,
    live: u32,
    live_known: bool,
    seen_epoch: u64,
    outstanding: Option<u64>,
    need_restart: bool,
    need_seed: bool,
    mask: u8,
    window_entries: u64,
    window_segments: u64,
    last: Option<Transition>,
    no_software_counted: bool,
    /// PHASE 1b. Absolute index of the newest `GP_PUT` value read in a turn that showed no live registration: every
    /// entry below it was published before any later registration's reply (design section 1.1, anchor argument). A
    /// Window may start only at an index >= this one.
    safe_end: u64,
    /// The smallest `GP_PUT` index ahead of the cursor seen so far (a submission end the cursor has not reached).
    next_boundary: Option<u64>,
    /// `Some(relayed)` when the cursor sits at an observed `GP_PUT` boundary in this turn.
    here: Option<u64>,
    /// The last boundary counted (each boundary is counted once).
    counted: Option<u64>,
    /// Entries still to be scanned after a source switch before the Window may end.
    post_switch: u32,
    /// Entries forwarded unscanned in the current Pointer stretch.
    pointer_stretch: u64,
    #[cfg(feature = "lifecycle-mutants")]
    mutant: Option<Mutant>,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// How many segments at the start of a Fresh window count as the P2 measurement.
pub const FRESH_SEGMENTS: u64 = 4;

/// How many entries of the NEW ring are scanned after a source switch before the Window may end (a bounded
/// detection aid for the unmeasured cursor semantics; no correctness depends on the number).
pub const POST_SWITCH_SCAN: u32 = 8;

fn scan_plan(restart: bool, seed_gate: bool) -> Plan {
    Plan::Scan {
        restart,
        seed_gate,
        permit: ScanPermit(()),
    }
}

impl Lifecycle {
    /// A channel at birth: Scan.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mode: Mode::Scan,
            start: None,
            counts: Counts::default(),
            reported: Counts::default(),
            legacy: false,
            live: 0,
            live_known: false,
            seen_epoch: 0,
            outstanding: None,
            need_restart: false,
            need_seed: false,
            mask: 0,
            window_entries: 0,
            window_segments: 0,
            last: None,
            no_software_counted: false,
            safe_end: 0,
            next_boundary: None,
            here: None,
            counted: None,
            post_switch: 0,
            pointer_stretch: 0,
            #[cfg(feature = "lifecycle-mutants")]
            mutant: Mutant::from_env(),
        }
    }
    /// The base revision's behaviour: scan every entry for ever. For differential tests.
    #[must_use]
    pub fn always_scan() -> Self {
        Self {
            legacy: true,
            ..Self::new()
        }
    }
    /// A machine with a deliberate defect.
    #[cfg(feature = "lifecycle-mutants")]
    #[must_use]
    pub fn with_mutant(m: Mutant) -> Self {
        Self {
            mutant: Some(m),
            ..Self::new()
        }
    }
    /// Everything that decides behaviour, none of the counters (model checking). Indices behind the cursor
    /// (`relayed`) are equivalent and collapse, so the model's state space does not grow with history.
    #[must_use]
    pub fn state_key(&self, relayed: u64) -> String {
        let ahead = |v: u64| v.saturating_sub(relayed);
        format!(
            "{:?}|{:?}|{}|{}|{}|{:?}|{}|{}|{}|{}|{:?}|{}|{}",
            self.mode,
            self.start,
            self.live,
            self.live_known,
            self.seen_epoch,
            self.outstanding,
            self.need_restart,
            self.need_seed,
            self.mask,
            ahead(self.safe_end),
            self.next_boundary.map(ahead).filter(|d| *d > 0),
            u8::from(self.here == Some(relayed)),
            self.post_switch
        )
    }
    /// The current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }
    /// How the current Window began.
    #[must_use]
    pub fn start(&self) -> Option<Start> {
        if self.mode == Mode::Window {
            self.start
        } else {
            None
        }
    }
    /// The newest forwarded SYNC_WAIT entry of Pointer mode that no marker has proven passed.
    #[must_use]
    pub fn outstanding_sync(&self) -> Option<u64> {
        self.outstanding
    }
    /// The last live-registration count observed.
    #[must_use]
    pub fn live(&self) -> u32 {
        self.live
    }
    /// The transition the last call made, if any (taken).
    pub fn take_transition(&mut self) -> Option<Transition> {
        self.last.take()
    }

    fn go(&mut self, to: Mode, cause: Cause) {
        self.last = Some(Transition {
            from: self.mode,
            cause,
            to,
        });
        self.mode = to;
    }

    /// Decide what to do with the entry just loaded. `o.epoch` must have been loaded AFTER `o.raw`.
    /// `live` counts the live registrations of the channel's software object; it is called only when
    /// the epoch moved since the last call (or on the first call).
    pub fn entry(&mut self, o: &Obs, live: impl FnOnce() -> u32) -> Plan {
        if self.mode == Mode::Dead {
            self.counts.after_teardown += 1;
            return Plan::Stop;
        }
        if self.legacy {
            return scan_plan(false, false);
        }
        if !self.live_known || o.epoch != self.seen_epoch {
            self.live = live();
            self.seen_epoch = o.epoch;
            self.live_known = true;
        }
        match self.mode {
            Mode::Scan => {
                if o.sw_mask != 0 {
                    self.mask = o.sw_mask;
                    if self.live == 0 {
                        self.go(Mode::Pointer, Cause::SoftwareBound);
                        self.counts.to_pointer += 1;
                        self.pointer_stretch = 0;
                        // the scan is exact up to here: this turn's GP_PUT is the first anchor
                        self.safe_end = o.put_abs;
                    } else {
                        self.go(Mode::Window, Cause::SoftwareBoundRegistered);
                        self.start = Some(Start::Exact);
                        self.window_entries = 0;
                        self.window_segments = 0;
                        self.counts.to_window_exact += 1;
                    }
                } else if !o.have_software && !self.no_software_counted {
                    self.no_software_counted = true;
                    self.counts.scan_no_software += 1;
                }
            }
            Mode::Pointer => {
                if self.live == 0 {
                    // A turn that showed no live registration: its GP_PUT read happened before any later
                    // registration's reply, so everything below it is pre-reply (design section 1.1, anchor).
                    self.safe_end = self.safe_end.max(o.put_abs);
                } else {
                    #[cfg(feature = "lifecycle-mutants")]
                    if self.mutant == Some(Mutant::PointerToScan) {
                        self.go(Mode::Scan, Cause::Registration);
                        return scan_plan(false, false);
                    }
                    #[cfg(feature = "lifecycle-mutants")]
                    let at_anchor =
                        o.abs >= self.safe_end || self.mutant == Some(Mutant::AnchorIgnored);
                    #[cfg(not(feature = "lifecycle-mutants"))]
                    let at_anchor = o.abs >= self.safe_end;
                    if at_anchor {
                        self.go(Mode::Window, Cause::Registration);
                        self.start = Some(Start::Fresh);
                        self.need_restart = true;
                        self.need_seed = self.outstanding.is_some();
                        #[cfg(feature = "lifecycle-mutants")]
                        if self.mutant == Some(Mutant::NoSeed) {
                            self.need_seed = false;
                        }
                        self.window_entries = 0;
                        self.window_segments = 0;
                        self.counts.to_window_fresh += 1;
                        self.counts.pointer_binding_unchecked += self.pointer_stretch;
                    } else {
                        // Published before the reply, below the anchor: forwarded unscanned (a Window may not
                        // start inside a submission: its packet state is unknown in Pointer mode).
                        self.counts.anchor_forwarded += 1;
                    }
                }
            }
            Mode::Window => {
                #[cfg(feature = "lifecycle-mutants")]
                let drained = self.live == 0 || self.mutant == Some(Mutant::ExitIgnoresLive);
                #[cfg(not(feature = "lifecycle-mutants"))]
                let drained = self.live == 0;
                if drained && o.quiescent && self.post_switch == 0 {
                    self.go(Mode::Pointer, Cause::RegistryDrained);
                    self.start = None;
                    self.need_restart = false;
                    self.need_seed = false;
                    self.outstanding = None;
                    self.counts.to_pointer_from_window += 1;
                    self.safe_end = o.put_abs;
                    self.pointer_stretch = 0;
                }
            }
            Mode::Dead => {}
        }
        #[cfg(feature = "lifecycle-mutants")]
        let frozen = self.mutant == Some(Mutant::MaskFrozen);
        #[cfg(not(feature = "lifecycle-mutants"))]
        let frozen = false;
        if self.mode == Mode::Window && o.sw_mask != self.mask && !frozen {
            self.counts.window_binding_changed += 1;
            self.mask = o.sw_mask;
        }
        match self.mode {
            Mode::Pointer => {
                #[cfg(feature = "lifecycle-mutants")]
                if self.mutant == Some(Mutant::PointerScans) {
                    return scan_plan(false, false);
                }
                Plan::Forward
            }
            _ => scan_plan(self.need_restart, self.need_seed),
        }
    }

    /// The entry was published.
    pub fn forwarded(&mut self, abs: u64, raw: u64) {
        match self.mode {
            Mode::Scan => self.counts.entries_scan += 1,
            Mode::Pointer => {
                self.counts.entries_pointer += 1;
                self.pointer_stretch += 1;
                #[cfg(feature = "lifecycle-mutants")]
                if self.mutant == Some(Mutant::ForgetSync) {
                    return;
                }
                if raw & GP_SYNC_WAIT != 0 {
                    self.outstanding = Some(abs);
                }
            }
            Mode::Window => {
                self.counts.entries_window += 1;
                if self.post_switch > 0 {
                    self.post_switch -= 1;
                    self.counts.post_switch_scanned += 1;
                }
                self.window_entries += 1;
                self.counts.window_entries_max =
                    self.counts.window_entries_max.max(self.window_entries);
            }
            Mode::Dead => self.counts.after_teardown += 1,
        }
    }
    /// The relay applied the restart.
    pub fn restarted(&mut self) {
        if self.need_restart {
            self.need_restart = false;
            self.counts.restarts += 1;
        }
    }
    /// The seeded gate's witness was processed.
    pub fn gate_proven(&mut self) {
        if self.need_seed {
            self.need_seed = false;
            self.outstanding = None;
            self.counts.seeded_gates += 1;
        }
    }
    /// A segment was scanned and committed.
    pub fn segment_scanned(&mut self, ends_open: bool) {
        match self.mode {
            Mode::Scan => {
                self.counts.segments_scan += 1;
                self.counts.straddles_scan += u64::from(ends_open);
            }
            Mode::Window => {
                self.counts.segments_window += 1;
                self.counts.straddles_window += u64::from(ends_open);
                self.window_segments += 1;
            }
            Mode::Pointer | Mode::Dead => {}
        }
    }
    /// A scan refused a segment (call once per entry, on the first refusal).
    pub fn scan_refused(&mut self) {
        if self.mode == Mode::Window
            && self.start == Some(Start::Fresh)
            && self.window_segments < FRESH_SEGMENTS
        {
            self.counts.window_fresh_start_parse_error += 1;
        }
    }
    /// A pushbuffer was read (via a [`ScanPermit`]).
    pub fn note_pb_read(&mut self, _permit: &ScanPermit, bytes: usize, vram: bool) {
        let b = bytes as u64;
        match self.mode {
            Mode::Scan => self.counts.pb_bytes_scan += b,
            Mode::Window => {
                self.counts.pb_bytes_window += b;
                if vram {
                    self.counts.window_bar1_bytes += b;
                }
            }
            Mode::Pointer | Mode::Dead => {
                // a permit exists only for a scan plan; reaching here is a bug (the mutant PointerScans):
                // loud in a debug build, a counter in release (G1: `pushbuffer_bytes_read == 0` in Pointer)
                debug_assert!(
                    self.mode != Mode::Pointer,
                    "pushbuffer read in Pointer mode"
                );
                self.counts.pointer_pb_read_violation += 1;
            }
        }
    }
    /// A pushbuffer was read WITHOUT a permit, i.e. a bug; counted loudly.
    pub fn note_unpermitted_pb_read(&mut self, bytes: usize) {
        let _ = bytes;
        debug_assert!(
            self.mode != Mode::Pointer,
            "pushbuffer read in Pointer mode"
        );
        self.counts.pointer_pb_read_violation += 1;
    }
    /// A marker segment may be authored between GP entries now. Scan and Window know the parser state: a marker only
    /// when no packet is open. Pointer mode knows none; it authors a marker only where the cursor sits at a `GP_PUT`
    /// boundary the guest wrote (the end of a submission: assumption P2', measured by `put_boundary_open_*`), and
    /// counts (loudly) every wake it could not announce. `relayed` = the cursor's absolute index.
    pub fn marker_possible(&mut self, scan_carried: bool, relayed: u64) -> bool {
        match self.mode {
            Mode::Scan | Mode::Window => !scan_carried,
            Mode::Pointer => {
                #[cfg(feature = "lifecycle-mutants")]
                if self.mutant == Some(Mutant::PointerMarkersAnywhere) {
                    return true;
                }
                #[cfg(feature = "lifecycle-mutants")]
                if self.mutant == Some(Mutant::PointerNoMarkers) {
                    return false;
                }
                if self.here == Some(relayed) {
                    true
                } else {
                    self.counts.pointer_carried_no_marker += 1;
                    false
                }
            }
            Mode::Dead => false,
        }
    }
    /// A marker was authored.
    pub fn marker_published(&mut self) {
        if self.mode == Mode::Pointer {
            self.counts.pointer_markers += 1;
        }
    }
    /// The turn read `GP_PUT` while the cursor was at the absolute index `relayed`; `put_abs` is the index of the
    /// value read. Tracks the submission ends the cursor will reach, and measures (once per boundary) whether a
    /// packet is OPEN there when the parser state is known (`open`; `None` in Pointer mode): the falsifier of P2'.
    pub fn observe_put(&mut self, relayed: u64, put_abs: u64, open: Option<bool>) {
        let at = put_abs == relayed || self.next_boundary == Some(relayed);
        self.here = at.then_some(relayed);
        if at && self.counted != Some(relayed) {
            self.counted = Some(relayed);
            if let Some(open) = open {
                let o = u64::from(open);
                match self.mode {
                    Mode::Scan => {
                        self.counts.put_boundaries_scan += 1;
                        self.counts.put_boundary_open_scan += o;
                    }
                    Mode::Window => {
                        self.counts.put_boundaries_window += 1;
                        self.counts.put_boundary_open_window += o;
                    }
                    Mode::Pointer | Mode::Dead => {}
                }
            }
        }
        if put_abs > relayed && self.next_boundary.is_none_or(|b| b <= relayed) {
            self.next_boundary = Some(put_abs);
        }
    }
    /// The relay switched the channel's guest ring (ring growth). The new ring's first entries are scanned before a
    /// Window may end: a detection aid for the cursor semantics at the switch (design D5), never an edge of the
    /// table.
    pub fn source_switched(&mut self) {
        if self.mode == Mode::Window {
            self.post_switch = POST_SWITCH_SCAN;
        }
    }
    /// The new ring's USERD showed a nonzero `GP_GET` at the switch: reset-to-0 is contradicted (loud).
    pub fn switch_cursor_mismatch(&mut self) {
        self.counts.switch_cursor_mismatch += 1;
    }
    /// A Pointer-mode channel was woken by the 50 ms backstop and found work (`loud` = beyond about one per second).
    pub fn pointer_backstop(&mut self, loud: bool) {
        self.counts.pointer_backstop += 1;
        self.counts.pointer_backstop_loud += u64::from(loud);
    }
    /// The software object was found gone in Pointer mode.
    pub fn software_gone(&mut self) {
        self.counts.pointer_software_gone += 1;
    }
    /// The channel was freed.
    pub fn teardown(&mut self) {
        #[cfg(feature = "lifecycle-mutants")]
        if self.mutant == Some(Mutant::ReviveAfterTeardown) {
            return;
        }
        if self.mode != Mode::Dead {
            self.go(Mode::Dead, Cause::Teardown);
        }
    }
    /// Counters so far.
    #[must_use]
    pub fn counts(&self) -> &Counts {
        &self.counts
    }
    /// Counters since the last call (`window_entries_max` is a maximum, not a sum: read it from
    /// [`Lifecycle::counts`]).
    pub fn take_delta(&mut self) -> Counts {
        let d = self.counts.since(&self.reported);
        self.reported = self.counts;
        d
    }
}

/// The transition table (I5).
#[must_use]
pub fn next(mode: Mode, cause: Cause) -> Option<Mode> {
    match (mode, cause) {
        (Mode::Dead, _) => Some(Mode::Dead),
        (_, Cause::Teardown) => Some(Mode::Dead),
        (Mode::Scan, Cause::SoftwareBound) => Some(Mode::Pointer),
        (Mode::Scan, Cause::SoftwareBoundRegistered) => Some(Mode::Window),
        (Mode::Pointer, Cause::Registration) => Some(Mode::Window),
        (Mode::Window, Cause::RegistryDrained) => Some(Mode::Pointer),
        _ => None,
    }
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
            put_abs: abs + 1,
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
            Plan::Forward | Plan::Stop => panic!("expected a scan, got {plan:?}"),
        }
    }

    #[test]
    fn a_channel_is_scanned_until_its_software_object_is_bound_and_then_it_is_pointer() {
        let mut l = Lifecycle::new();
        assert_eq!(l.mode(), Mode::Scan);
        for i in 0..5 {
            assert!(matches!(
                l.entry(&obs(i, 0x1000, 0, 0), || 0),
                Plan::Scan { .. }
            ));
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
        assert_eq!(
            scan(p),
            (false, false),
            "no restart, no seed: the parser state is exact"
        );
        assert_eq!((l.mode(), l.start()), (Mode::Window, Some(Start::Exact)));
        assert_eq!(l.counts().to_window_exact, 1);
    }

    #[test]
    fn pointer_mode_forwards_and_never_asks_for_a_scan() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        for i in 1..1000 {
            assert_eq!(
                l.entry(&obs(i, 0x2000 + i, 0, B), || panic!("no epoch change")),
                Plan::Forward
            );
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
        assert_eq!(
            scan(p),
            (true, false),
            "restart once; nothing to seed without an unproven SYNC_WAIT"
        );
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
        assert_eq!(
            asked, 49,
            "the registry is consulted once per epoch change, never otherwise"
        );
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
        assert_eq!(
            l.outstanding_sync(),
            Some(3 * 32768),
            "the newest one, as a u64"
        );
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
            assert!(matches!(
                l.entry(&obs(i, 0x10, 1, 1 << 6), || panic!("epoch unchanged")),
                Plan::Scan { .. }
            ));
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
            assert!(matches!(
                l.entry(&obs(10 * round + 1, 0, e, B), || 1),
                Plan::Scan { restart: true, .. }
            ));
            l.restarted();
            assert_eq!(
                l.entry(&obs(10 * round + 2, 0, e + 1, B), || 0),
                Plan::Forward
            );
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
        assert_eq!(
            p,
            Plan::Stop,
            "a torn-down channel reads nothing and publishes nothing (review D6)"
        );
        assert_eq!(l.mode(), Mode::Dead);
        assert!(l.counts().after_teardown >= 1);
    }

    #[test]
    fn the_transition_table_is_exactly_the_documented_one() {
        use Cause::*;
        use Mode::*;
        assert_eq!(next(Scan, SoftwareBound), Some(Pointer));
        assert_eq!(next(Scan, SoftwareBoundRegistered), Some(Window));
        assert_eq!(
            next(Scan, Registration),
            None,
            "a Scan that has not bound yet keeps scanning anyway"
        );
        assert_eq!(next(Pointer, Registration), Some(Window));
        assert_eq!(next(Window, RegistryDrained), Some(Pointer));
        for m in [Scan, Pointer, Window, Dead] {
            assert_eq!(next(m, Teardown), Some(Dead));
        }
        // nothing else: in particular no way back to Scan, and nothing leaves Dead
        for m in [Scan, Pointer, Window, Dead] {
            for c in [
                SoftwareBound,
                SoftwareBoundRegistered,
                Registration,
                RegistryDrained,
                Teardown,
            ] {
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
        let Plan::Scan { permit, .. } = p else {
            panic!()
        };
        l.note_pb_read(&permit, 100, false);
        l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        let p = l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        let Plan::Scan { permit, .. } = p else {
            panic!()
        };
        l.note_pb_read(&permit, 40, true);
        l.note_pb_read(&permit, 8, false);
        let c = l.counts();
        assert_eq!(
            (c.pb_bytes_scan, c.pb_bytes_window, c.window_bar1_bytes),
            (100, 48, 40)
        );
    }

    #[test]
    fn segments_ending_in_an_open_packet_are_counted_per_mode_and_a_fresh_start_refusal_loudly() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, 0), || 0);
        l.segment_scanned(true);
        l.segment_scanned(false);
        assert_eq!(
            (l.counts().segments_scan, l.counts().straddles_scan),
            (2, 1)
        );
        // bind with a registration live: an EXACT window; a refusal there is not a fresh-start error
        l.entry(&obs(1, 0, 1, 1 << 6), || 1);
        l.scan_refused();
        assert_eq!(l.counts().window_fresh_start_parse_error, 0);
        l.segment_scanned(true);
        assert_eq!(
            (l.counts().segments_window, l.counts().straddles_window),
            (1, 1)
        );

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
        assert_eq!(
            l.counts().window_binding_changed,
            1,
            "5080 moved to another subchannel"
        );
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
    fn markers_need_a_known_parser_state_or_an_observed_gp_put_boundary() {
        let mut l = Lifecycle::new();
        assert!(l.marker_possible(false, 0));
        assert!(!l.marker_possible(true, 0), "a packet is open");
        l.entry(&obs(0, 0, 0, 1 << 6), || 0);
        // Pointer mode: no marker unless the cursor sits at a GP_PUT value the guest wrote
        assert!(
            !l.marker_possible(false, 1),
            "no boundary observed at the cursor"
        );
        assert_eq!(l.counts().pointer_carried_no_marker, 1, "loud");
        l.observe_put(1, 1, None);
        assert!(l.marker_possible(false, 1), "the cursor is at GP_PUT");
        l.observe_put(1, 5, None);
        assert!(
            !l.marker_possible(false, 1),
            "GP_PUT is ahead: mid-submission"
        );
        assert!(
            !l.marker_possible(false, 3),
            "an index that was never a GP_PUT"
        );
        l.observe_put(4, 5, None);
        assert!(
            !l.marker_possible(false, 4),
            "the remembered boundary is 5, not 4"
        );
        l.observe_put(5, 9, None);
        assert!(
            l.marker_possible(false, 5),
            "the remembered boundary was reached"
        );
        l.entry(&obs(1, 0, 1, B), || 1);
        assert!(l.marker_possible(false, 5) == l.marker_possible(false, 5));
    }

    /// PHASE 1b (D1): a Window starts at an anchor, a GP_PUT value read in a turn that showed no registration.
    #[test]
    fn a_window_starts_only_at_the_anchor_never_inside_the_submission_the_flip_was_seen_in() {
        let mut l = Lifecycle::new();
        let o = |abs: u64, put_abs: u64, epoch: u64| Obs {
            put_abs,
            ..obs(abs, 0x10, epoch, B)
        };
        // bind at entry 0, submission ends at 6 (put_abs = 6 for every turn of the submission)
        l.entry(&o(0, 6, 0), || 0);
        l.forwarded(0, 0x10);
        l.entry(&o(1, 6, 0), || 0);
        l.forwarded(1, 0x10);
        // the registration shows up at entry 2; the submission's end (6) is the anchor
        for abs in 2..6 {
            assert_eq!(
                l.entry(&o(abs, 6, 1), || 1),
                Plan::Forward,
                "entry {abs} is below the anchor"
            );
            l.forwarded(abs, 0x10);
            assert_eq!(l.mode(), Mode::Pointer);
        }
        assert_eq!(l.counts().anchor_forwarded, 4);
        let p = l.entry(&o(6, 9, 1), || 1);
        assert!(matches!(p, Plan::Scan { restart: true, .. }));
        assert_eq!((l.mode(), l.start()), (Mode::Window, Some(Start::Fresh)));
    }

    #[test]
    fn a_turn_that_showed_the_registration_never_moves_the_anchor() {
        let mut l = Lifecycle::new();
        let o = |abs: u64, put_abs: u64, epoch: u64| Obs {
            put_abs,
            ..obs(abs, 0x10, epoch, B)
        };
        l.entry(&o(0, 3, 0), || 0);
        l.forwarded(0, 0x10);
        // entry 1: the epoch moved; this turn's GP_PUT (9) was read AFTER the guest may have published post-reply
        // entries, so it is not the anchor: the last clean turn's (3) is
        assert_eq!(l.entry(&o(1, 9, 1), || 1), Plan::Forward);
        l.forwarded(1, 0x10);
        assert_eq!(l.entry(&o(2, 9, 1), || 1), Plan::Forward);
        l.forwarded(2, 0x10);
        assert!(matches!(
            l.entry(&o(3, 9, 1), || 1),
            Plan::Scan { restart: true, .. }
        ));
    }

    #[test]
    fn a_registration_removed_before_the_anchor_leaves_pointer_mode_alone() {
        let mut l = Lifecycle::new();
        let o = |abs: u64, put_abs: u64, epoch: u64| Obs {
            put_abs,
            ..obs(abs, 0x10, epoch, B)
        };
        l.entry(&o(0, 4, 0), || 0);
        assert_eq!(l.entry(&o(1, 4, 1), || 1), Plan::Forward);
        assert_eq!(l.entry(&o(2, 4, 2), || 0), Plan::Forward, "removed again");
        assert_eq!(l.entry(&o(3, 8, 2), || 0), Plan::Forward);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!(l.counts().to_window_fresh, 0);
    }

    /// P2' measurement: a packet open at an observed GP_PUT is counted once per boundary, per mode, uncapped.
    #[test]
    fn a_packet_open_at_a_gp_put_boundary_is_counted_once_per_boundary_in_scan_and_window() {
        let mut l = Lifecycle::new();
        l.observe_put(3, 3, Some(false));
        l.observe_put(3, 3, Some(false));
        l.observe_put(3, 7, Some(true));
        assert_eq!(
            (
                l.counts().put_boundaries_scan,
                l.counts().put_boundary_open_scan
            ),
            (1, 0)
        );
        l.observe_put(7, 7, Some(true));
        assert_eq!(
            (
                l.counts().put_boundaries_scan,
                l.counts().put_boundary_open_scan
            ),
            (2, 1)
        );
        // a boundary the cursor reaches after the guest moved GP_PUT on is still measured
        l.observe_put(8, 12, Some(false));
        l.observe_put(12, 12, Some(true));
        assert_eq!(
            (
                l.counts().put_boundaries_scan,
                l.counts().put_boundary_open_scan
            ),
            (3, 2)
        );
        l.entry(&obs(12, 0, 1, B), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.observe_put(13, 13, Some(true));
        assert_eq!(
            (
                l.counts().put_boundaries_window,
                l.counts().put_boundary_open_window
            ),
            (1, 1)
        );
    }

    /// D5: after a source switch the first entries of the new ring are scanned before the Window may end.
    #[test]
    fn after_a_source_switch_the_window_stays_open_for_a_bounded_number_of_entries() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, B), || 0);
        l.entry(&obs(1, 0, 1, B), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.forwarded(1, 0x10);
        l.source_switched();
        for i in 0..u64::from(POST_SWITCH_SCAN) {
            // the registry is empty and the relay quiescent, yet the Window holds
            assert!(
                matches!(l.entry(&obs(2 + i, 0, 2, B), || 0), Plan::Scan { .. }),
                "entry {i}"
            );
            assert_eq!(l.mode(), Mode::Window, "entry {i}");
            l.forwarded(2 + i, 0x10);
        }
        assert_eq!(l.counts().post_switch_scanned, u64::from(POST_SWITCH_SCAN));
        assert_eq!(l.entry(&obs(100, 0, 2, B), || 0), Plan::Forward);
        assert_eq!(l.mode(), Mode::Pointer);
    }

    #[test]
    fn a_source_switch_outside_a_window_changes_nothing() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, B), || 0);
        l.source_switched();
        assert_eq!(l.entry(&obs(1, 0, 0, B), || 0), Plan::Forward);
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

    /// I8 at the machine: a Window starts only at an anchor, and an anchor is only a `GP_PUT` value the worker has
    /// already read (never a later one, never an invented one). Random sequences of turns, registrations coming and
    /// going, ring laps.
    #[test]
    fn the_anchor_is_always_a_gp_put_value_already_read_and_a_window_never_starts_below_it() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..300 {
            let mut l = Lifecycle::new();
            let (mut abs, mut max_put, mut epoch, mut live) = (0u64, 0u64, 0u64, 0u32);
            let mut windows = 0;
            for _ in 0..400 {
                // the guest's GP_PUT is ahead of the cursor, sometimes by a lot
                let put_abs = abs + 1 + next() % 6;
                max_put = max_put.max(put_abs);
                if next() % 9 == 0 {
                    live = u32::from(next() % 2 == 0);
                    epoch += 1;
                }
                let o = Obs {
                    put_abs,
                    ..obs(abs, 0x10, epoch, B)
                };
                let was = l.mode();
                let plan = l.entry(&o, || live);
                assert!(
                    l.safe_end <= max_put,
                    "the anchor {} was never read (max {max_put})",
                    l.safe_end
                );
                if was == Mode::Pointer && l.mode() == Mode::Window {
                    windows += 1;
                    assert!(
                        abs >= l.safe_end,
                        "a Window started at {abs}, below the anchor {}",
                        l.safe_end
                    );
                    assert!(matches!(plan, Plan::Scan { restart: true, .. }));
                }
                l.forwarded(abs, 0x10);
                abs += 1;
                if l.mode() == Mode::Window {
                    l.restarted();
                }
            }
            let _ = windows;
        }
    }

    /// G4 / I6: nothing in the machine knows about time (OWNER_RULINGS section AE: no heuristics, no timers).
    #[test]
    fn the_machine_names_no_clock_no_sleep_and_no_poll() {
        let text = include_str!("user_lifecycle.rs");
        let prod = text.split("#[cfg(test)]\nmod tests").next().unwrap();
        for banned in [
            "Instant",
            "SystemTime",
            "Duration",
            "sleep",
            "poll(",
            "Poll",
            "tick",
            "timeout",
            "thread::",
        ] {
            assert!(
                !prod.contains(banned),
                "the lifecycle machine must not mention {banned}"
            );
        }
    }

    /// A7: a reused channel key gets a NEW machine; it knows nothing of the old life's epoch, mode or SYNC_WAIT.
    #[test]
    fn a_fresh_channel_on_a_reused_key_reads_the_registry_once_whatever_the_epoch_and_starts_in_scan()
     {
        let mut old = Lifecycle::new();
        old.entry(&obs(0, 0, 0, B), || 0);
        old.forwarded(0, SYNC | 1);
        old.entry(&obs(1, 0, 9, B), || 1);
        assert_eq!(old.mode(), Mode::Window);
        old.teardown();
        // the key is reused: a brand new machine, the registry epoch is far ahead
        let mut fresh = Lifecycle::new();
        assert_eq!(
            (fresh.mode(), fresh.outstanding_sync(), fresh.live()),
            (Mode::Scan, None, 0)
        );
        let mut asked = 0;
        for i in 0..10 {
            fresh.entry(&obs(i, 0, 1000, 0), || {
                asked += 1;
                0
            });
        }
        assert_eq!(asked, 1, "once, then only when the epoch moves");
        assert_eq!(fresh.mode(), Mode::Scan);
    }
}
