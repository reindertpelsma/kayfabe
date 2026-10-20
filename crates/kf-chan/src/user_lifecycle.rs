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

/// ★ 2026-10-16 (PHASE 1c, second review N2). The measurement-class switch of the lifecycle, read ONCE from the
/// environment variable [`LifecycleMode::ENV`] when a relay is created (never per turn, never required to boot; an
/// unknown value is reported loudly and means [`LifecycleMode::Default`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LifecycleMode {
    /// The lifecycle fully on: Scan, Pointer (with markers at observed `GP_PUT` boundaries), anchored Windows.
    #[default]
    Default,
    /// The machine never leaves Scan: the base revision's behaviour (every entry's pushbuffer is read and scanned) with
    /// ALL 1b counters on, so `put_boundary_open_scan` measures assumption P2' on the whole run, not only on the setup
    /// stretch before the first `SET_OBJECT`.
    AlwaysScan,
    /// Pointer mode places no markers (the wake is the bounded spin / the counted backstop); anchored Windows stay.
    NoPointerMarkers,
    /// A Window starts at the first entry that sees the registration (PHASE 1 behaviour), not at the anchor.
    NoAnchoredWindows,
}

impl LifecycleMode {
    /// The environment variable.
    pub const ENV: &'static str = "KF3_LIFECYCLE_MODE";
    /// Every mode, for tests and the documentation.
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::AlwaysScan,
        Self::NoPointerMarkers,
        Self::NoAnchoredWindows,
    ];
    /// The value of [`Self::ENV`] that selects this mode.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AlwaysScan => "always_scan",
            Self::NoPointerMarkers => "no_pointer_markers",
            Self::NoAnchoredWindows => "no_anchored_windows",
        }
    }
    /// The mode a value names (`None` for an unknown name).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == value)
    }
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
    /// PHASE 1c. A `GP_PUT` read while the guest ring was FULL (`put + 1` equals the `GP_GET` last written back): a
    /// driver that fills its ring may have cut a packet to get space, so that value is never an anchor.
    pub anchor_refused_ring_full: u64,
    /// PHASE 1c. A Window that started at an entry ABOVE the newest anchor (the anchors in between were refused, see
    /// above): its first boundary is not a known `GP_PUT` boundary. Must be 0 in an ordinary run.
    pub anchor_stale_start: u64,
    /// PHASE 1c (owner request, precondition P1). SET_OBJECTs of the 5080 software class the scan saw, EVERY one (not only
    /// the first), over the channel's scanned life (the whole run in `always_scan`; Pointer mode scans nothing).
    pub set_object_5080_total: u64,
    /// SET_OBJECTs on a subchannel that held the 5080 software object and bound a different class (a rebind).
    pub set_object_5080_rebinds: u64,
    /// PHASE 1c. This channel was the first in the VM to see a packet open at a `GP_PUT` boundary and switched Pointer
    /// markers off VM-wide (summed over the VM it is 0 or 1).
    pub pointer_markers_latched_off: u64,
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
            pointer_software_gone,
            anchor_refused_ring_full,
            anchor_stale_start,
            pointer_markers_latched_off,
            set_object_5080_total,
            set_object_5080_rebinds
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
            /// PHASE 1c (P1 measurement): the most 5080 SET_OBJECTs any ONE channel's scan saw (a maximum).
            pub set_object_5080_max_per_channel: std::sync::atomic::AtomicU64,
            /// The VM-wide switch of Pointer-mode markers (PHASE 1c): every channel's [`Lifecycle`] reads and sets it.
            pub latch: std::sync::Arc<MarkerLatch>,
            /// ★ 2026-10-20 `KF3_DIAG_USER_STALL`'s cumulative counters (empty, and absent from the line, when off).
            pub stall: crate::user_stall::StallStats,
        }
        impl LifecycleStats {
            /// Merge one channel's delta ([`Lifecycle::take_delta`]).
            pub fn add(&self, d: &Counts) {
                use std::sync::atomic::Ordering::Relaxed;
                $( if d.$f != 0 { self.$f.fetch_add(d.$f, Relaxed); } )*
            }
            /// Record a channel's total of scanned 5080 SET_OBJECTs (kept as the maximum over channels).
            pub fn set_object_max(&self, total: u64) {
                self.set_object_5080_max_per_channel
                    .fetch_max(total, std::sync::atomic::Ordering::Relaxed);
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
                s.push_str(&format!(
                    "set_object_5080_max_per_channel={} ",
                    self.set_object_5080_max_per_channel.load(Relaxed)
                ));
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
    pointer_software_gone,
    anchor_refused_ring_full,
    anchor_stale_start,
    pointer_markers_latched_off,
    set_object_5080_total,
    set_object_5080_rebinds
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
    /// PHASE 1c. The guest ring was FULL when that `GP_PUT` was read (`put + 1` equals the `GP_GET` last written
    /// back): the value is never an anchor.
    pub put_full: bool,
    /// Subchannels (bit n) on which the scan has the software object bound; 0 outside Scan/Window.
    pub sw_mask: u8,
    /// The channel has a software object at all.
    pub have_software: bool,
    /// No gate is draining and no segment is in flight.
    pub quiescent: bool,
}

/// ★ PHASE 1c (second review N1/N2). The VM-wide switch of Pointer-mode markers. A Pointer marker is safe only where
/// the guest's `GP_PUT` is a packet boundary (assumption P2'); the first channel that SEES a packet open at a
/// `GP_PUT` boundary (Scan or Window: the parser state is known) falsifies P2' for the guest, and markers (a
/// performance feature, never a correctness one) are switched off for every channel of the VM. Anchored Windows
/// stay: they have their own counters. One-way: it is never cleared (a guest that did it once may do it again).
#[derive(Debug, Default)]
pub struct MarkerLatch(std::sync::atomic::AtomicBool);

impl MarkerLatch {
    /// Whether Pointer markers are switched off.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Switch them off. Returns `true` for the call that did it (exactly one caller ever gets `true`).
    pub fn set(&self) -> bool {
        !self.0.swap(true, std::sync::atomic::Ordering::AcqRel)
    }
}

/// How many `USER_RING_PUT_BOUNDARY_OPEN` notices a channel reports outside a Window (inside a Window every one is).
pub const OPEN_NOTICES: u64 = 8;
/// The most notices a machine keeps between two [`Lifecycle::take_notices`] calls (one turn produces at most three).
const NOTICE_CAP: usize = 16;

/// The first segment of a Fresh window, as the `USER_RING_WINDOW_FRESH` notice reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshFirst {
    /// Scanned and ended on a packet boundary.
    Closed,
    /// Scanned and ended inside an open packet.
    EndsOpen,
    /// The scan refused it (counted as `window_fresh_start_parse_error`).
    Refused,
    /// The Window ended before any segment was scanned.
    None,
}

/// What the owner should say aloud (the machine has no I/O). Plain data; `take_notices` drains them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// `USER_RING_PUT_BOUNDARY_OPEN`: the cursor reached a `GP_PUT` value with a packet OPEN (P2' is false here).
    PutBoundaryOpen {
        /// The mode that measured it (Scan or Window).
        mode: Mode,
        /// The cursor's absolute index.
        relayed: u64,
        /// The absolute index of the `GP_PUT` read.
        put_abs: u64,
        /// The n-th such boundary of this channel.
        nth: u64,
    },
    /// `USER_RING_POINTER_MARKERS_LATCHED_OFF`: this channel was the first in the VM; Pointer markers are off VM-wide.
    MarkersLatchedOff {
        /// The mode that measured it.
        mode: Mode,
        /// The cursor's absolute index.
        relayed: u64,
        /// The absolute index of the `GP_PUT` read.
        put_abs: u64,
    },
    /// `USER_RING_WINDOW_FRESH`: a Window started after Pointer mode.
    WindowFresh {
        /// The absolute index of the first Window entry (the cursor at the flip).
        cursor: u64,
        /// The anchor in force at the flip (the newest `GP_PUT` read in a turn that showed no registration).
        anchor: u64,
        /// Entries forwarded unscanned between the turn that showed the registration and the anchor.
        anchor_forwarded: u64,
        /// The gate was seeded from a recorded SYNC_WAIT.
        seeded: bool,
        /// The Window started above the anchor (anchors were refused in between).
        stale: bool,
        /// What the first segment did.
        first: FreshFirst,
    },
}

/// Facts of a Fresh window kept until its first segment is known.
#[derive(Clone, Copy, Debug)]
struct FreshInfo {
    cursor: u64,
    anchor: u64,
    anchor_forwarded: u64,
    seeded: bool,
    stale: bool,
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
    kind: LifecycleMode,
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
    /// PHASE 1c. This channel saw a packet open at a `GP_PUT` boundary: its Pointer markers are off (also when no
    /// shared [`MarkerLatch`] is attached, e.g. in the model).
    latched: bool,
    /// The VM-wide switch (see [`MarkerLatch`]); `None` for a stand-alone machine.
    shared: Option<std::sync::Arc<MarkerLatch>>,
    /// Entries forwarded unscanned since the registration was first seen (the flip's `anchor_forwarded`).
    flip_forwarded: u64,
    /// The Fresh window whose first segment has not been scanned yet.
    fresh: Option<FreshInfo>,
    /// `USER_RING_PUT_BOUNDARY_OPEN` notices queued so far for this channel.
    open_notices: u64,
    notices: Vec<Notice>,
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
            kind: LifecycleMode::Default,
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
            latched: false,
            shared: None,
            flip_forwarded: 0,
            fresh: None,
            open_notices: 0,
            notices: Vec::new(),
            #[cfg(feature = "lifecycle-mutants")]
            mutant: Mutant::from_env(),
        }
    }
    /// The base revision's behaviour: scan every entry for ever. For differential tests.
    #[must_use]
    pub fn always_scan() -> Self {
        Self::new().with_mode(LifecycleMode::AlwaysScan)
    }
    /// The same machine under a measurement-class mode ([`LifecycleMode`]).
    #[must_use]
    pub fn with_mode(self, kind: LifecycleMode) -> Self {
        Self { kind, ..self }
    }
    /// The mode this machine runs under.
    #[must_use]
    pub fn lifecycle_mode(&self) -> LifecycleMode {
        self.kind
    }
    /// Attach the VM-wide marker switch (the pump gives every channel the VM's).
    #[must_use]
    pub fn with_latch(self, latch: std::sync::Arc<MarkerLatch>) -> Self {
        Self {
            shared: Some(latch),
            ..self
        }
    }
    /// Pointer markers are off for this channel: the mode says so, or a packet was seen open at a `GP_PUT` boundary
    /// (here or on any channel of the VM).
    #[must_use]
    pub fn markers_off(&self) -> bool {
        self.kind == LifecycleMode::NoPointerMarkers
            || self.latched
            || self.shared.as_ref().is_some_and(|l| l.is_set())
    }
    /// What the owner should say aloud since the last call (drained).
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }
    fn notice(&mut self, n: Notice) {
        if self.notices.len() < NOTICE_CAP {
            self.notices.push(n);
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
            "{:?}|{:?}|{}|{}|{}|{:?}|{}|{}|{}|{}|{:?}|{}|{}|{:?}|{}",
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
            self.post_switch,
            self.kind,
            u8::from(self.latched)
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

    /// This turn read `GP_PUT` in a turn that showed no live registration: it is the newest anchor, unless the guest ring
    /// was full when it was read (a driver that fills its ring may have cut a packet to get space).
    fn anchor(&mut self, o: &Obs) {
        if o.put_full {
            self.counts.anchor_refused_ring_full += 1;
        } else {
            self.safe_end = o.put_abs;
        }
    }

    /// The first segment of a Fresh window is known (or the Window ended without one): queue its notice, once.
    fn fresh_done(&mut self, first: FreshFirst) {
        if let Some(f) = self.fresh.take() {
            self.notice(Notice::WindowFresh {
                cursor: f.cursor,
                anchor: f.anchor,
                anchor_forwarded: f.anchor_forwarded,
                seeded: f.seeded,
                stale: f.stale,
                first,
            });
        }
    }

    /// Decide what to do with the entry just loaded. `o.epoch` must have been loaded AFTER `o.raw`.
    /// `live` counts the live registrations of the channel's software object; it is called only when
    /// the epoch moved since the last call (or on the first call).
    pub fn entry(&mut self, o: &Obs, live: impl FnOnce() -> u32) -> Plan {
        if self.mode == Mode::Dead {
            self.counts.after_teardown += 1;
            return Plan::Stop;
        }
        if self.kind == LifecycleMode::AlwaysScan {
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
                        self.anchor(o);
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
                    if !o.put_full {
                        self.safe_end = self.safe_end.max(o.put_abs);
                    } else {
                        self.counts.anchor_refused_ring_full += 1;
                    }
                } else {
                    #[cfg(feature = "lifecycle-mutants")]
                    if self.mutant == Some(Mutant::PointerToScan) {
                        self.go(Mode::Scan, Cause::Registration);
                        return scan_plan(false, false);
                    }
                    // `NoAnchoredWindows` (measurement class): the PHASE 1 start, at the entry that sees the registration
                    let unanchored = self.kind == LifecycleMode::NoAnchoredWindows;
                    #[cfg(feature = "lifecycle-mutants")]
                    let at_anchor = o.abs >= self.safe_end
                        || unanchored
                        || self.mutant == Some(Mutant::AnchorIgnored);
                    #[cfg(not(feature = "lifecycle-mutants"))]
                    let at_anchor = o.abs >= self.safe_end || unanchored;
                    if at_anchor {
                        // above the newest anchor: the anchors in between were refused (ring full), so this boundary
                        // is not a known GP_PUT boundary. Reported, never silent.
                        let stale = o.abs > self.safe_end && !unanchored;
                        self.counts.anchor_stale_start += u64::from(stale);
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
                        self.fresh = Some(FreshInfo {
                            cursor: o.abs,
                            anchor: self.safe_end,
                            anchor_forwarded: self.flip_forwarded,
                            seeded: self.need_seed,
                            stale,
                        });
                        self.flip_forwarded = 0;
                    } else {
                        // Published before the reply, below the anchor: forwarded unscanned (a Window may not
                        // start inside a submission: its packet state is unknown in Pointer mode).
                        self.counts.anchor_forwarded += 1;
                        self.flip_forwarded += 1;
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
                    self.anchor(o);
                    self.pointer_stretch = 0;
                    self.flip_forwarded = 0;
                    self.fresh_done(FreshFirst::None);
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
            Mode::Scan => {
                self.counts.entries_scan += 1;
                // `AlwaysScan` never opens a Window, so the entries after a source switch are counted here
                if self.post_switch > 0 {
                    self.post_switch -= 1;
                    self.counts.post_switch_scanned += 1;
                }
            }
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
                self.fresh_done(if ends_open {
                    FreshFirst::EndsOpen
                } else {
                    FreshFirst::Closed
                });
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
            self.fresh_done(FreshFirst::Refused);
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
                if !self.markers_off() && self.here == Some(relayed) {
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
                if open && matches!(self.mode, Mode::Scan | Mode::Window) {
                    self.packet_open_at_put(relayed, put_abs);
                }
            }
        }
        if put_abs > relayed && self.next_boundary.is_none_or(|b| b <= relayed) {
            self.next_boundary = Some(put_abs);
        }
    }

    /// A packet is OPEN at a `GP_PUT` boundary the parser state is known for: assumption P2' is false for this guest.
    /// Say so (the first few per channel, every one inside a Window) and switch Pointer markers off VM-wide.
    fn packet_open_at_put(&mut self, relayed: u64, put_abs: u64) {
        self.open_notices += 1;
        if self.open_notices <= OPEN_NOTICES || self.mode == Mode::Window {
            self.notice(Notice::PutBoundaryOpen {
                mode: self.mode,
                relayed,
                put_abs,
                nth: self.open_notices,
            });
        }
        let first_here = !self.latched;
        self.latched = true;
        if self.shared.as_ref().map_or(first_here, |l| l.set()) {
            self.counts.pointer_markers_latched_off += 1;
            self.notice(Notice::MarkersLatchedOff {
                mode: self.mode,
                relayed,
                put_abs,
            });
        }
    }
    /// The relay switched the channel's guest ring (ring growth). The new ring's first entries are scanned before a
    /// Window may end: a detection aid for the cursor semantics at the switch (design D5), never an edge of the
    /// table.
    pub fn source_switched(&mut self) {
        // PHASE 1c (review 2, N4): `next_boundary`, `here` and `counted` are absolute indices of the OLD ring's `GP_PUT`
        // values. They name nothing of the new ring: a Pointer marker believed "at a GP_PUT boundary" there would land
        // in the middle of a guest submission.
        self.next_boundary = None;
        self.here = None;
        self.counted = None;
        if self.mode == Mode::Window || self.kind == LifecycleMode::AlwaysScan {
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
    /// The scan's cumulative SET_OBJECT tally (absolute values; the delta mechanism turns them into increments).
    pub fn sync_set_objects(&mut self, software_binds: u64, rebinds: u64) {
        self.counts.set_object_5080_total = software_binds;
        self.counts.set_object_5080_rebinds = rebinds;
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
            put_full: false,
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
        // a registration opens a Window: the parser state is known again, so a marker needs no packet open
        l.entry(&obs(1, 0, 1, B), || 1);
        assert_eq!(l.mode(), Mode::Window);
        assert!(l.marker_possible(false, 5), "Window: no packet open");
        assert!(!l.marker_possible(true, 5), "Window: a packet is open");
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

    /// PHASE 1c (review 2, N2): the mode names are the documented ones; an unknown name is not a mode.
    #[test]
    fn the_measurement_mode_names_parse_exactly() {
        for m in LifecycleMode::ALL {
            assert_eq!(LifecycleMode::parse(m.name()), Some(m));
        }
        assert_eq!(
            LifecycleMode::parse("default"),
            Some(LifecycleMode::Default)
        );
        assert_eq!(
            LifecycleMode::parse("always_scan"),
            Some(LifecycleMode::AlwaysScan)
        );
        assert_eq!(
            LifecycleMode::parse("no_pointer_markers"),
            Some(LifecycleMode::NoPointerMarkers)
        );
        assert_eq!(
            LifecycleMode::parse("no_anchored_windows"),
            Some(LifecycleMode::NoAnchoredWindows)
        );
        for bad in ["", "Default", "always-scan", "scan", " default"] {
            assert_eq!(LifecycleMode::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Lifecycle::new().lifecycle_mode(), LifecycleMode::Default);
    }

    /// `always_scan`: the machine never leaves Scan, plans a scan for every entry, and every 1b counter keeps running
    /// for the whole run (the P2' falsifier included).
    #[test]
    fn always_scan_never_leaves_scan_and_measures_the_boundaries_of_the_whole_run() {
        let mut d = Lifecycle::new();
        let mut a = Lifecycle::new().with_mode(LifecycleMode::AlwaysScan);
        for l in [&mut d, &mut a] {
            // the software object is bound at entry 0
            l.entry(&obs(0, 0x10, 0, B), || 0);
            l.forwarded(0, 0x10);
        }
        assert_eq!(d.mode(), Mode::Pointer);
        assert_eq!(a.mode(), Mode::Scan, "never leaves Scan");
        for abs in 1..6u64 {
            assert_eq!(d.entry(&obs(abs, 0x10, 0, B), || 0), Plan::Forward);
            assert!(
                matches!(a.entry(&obs(abs, 0x10, 0, B), || 0), Plan::Scan { .. }),
                "entry {abs} is scanned"
            );
            d.forwarded(abs, 0x10);
            a.forwarded(abs, 0x10);
        }
        // a registration changes nothing in always_scan
        assert!(matches!(
            a.entry(&obs(6, 0x10, 1, B), || 1),
            Plan::Scan { restart: false, .. }
        ));
        assert_eq!((a.mode(), a.counts().to_window_fresh), (Mode::Scan, 0));
        // GP_PUT boundaries with a packet open, long after the binding: counted only by always_scan
        a.observe_put(9, 9, Some(true));
        d.observe_put(9, 9, None); // Pointer mode knows no parser state
        assert_eq!(
            (
                a.counts().put_boundaries_scan,
                a.counts().put_boundary_open_scan
            ),
            (1, 1)
        );
        assert_eq!(
            (
                d.counts().put_boundaries_scan,
                d.counts().put_boundary_open_scan
            ),
            (0, 0),
            "the default mode cannot see a boundary after Scan ended: the reason for the mode"
        );
        // a source switch is still bookkept (8 entries are counted after it)
        a.source_switched();
        for abs in 7..20u64 {
            a.entry(&obs(abs, 0x10, 1, B), || 1);
            a.forwarded(abs, 0x10);
        }
        assert_eq!(a.counts().post_switch_scanned, u64::from(POST_SWITCH_SCAN));
        assert_eq!(a.counts().entries_scan, 1 + 5 + 13);
    }

    /// `no_pointer_markers`: a marker is never placed in Pointer mode, even at an observed `GP_PUT` boundary; Scan and
    /// Window keep their markers; the wake it cannot announce is counted (loud).
    #[test]
    fn no_pointer_markers_places_none_in_pointer_mode_but_keeps_the_rest() {
        for mode in [LifecycleMode::Default, LifecycleMode::NoPointerMarkers] {
            let mut l = Lifecycle::new().with_mode(mode);
            assert!(
                l.marker_possible(false, 0),
                "Scan: a marker when no packet is open"
            );
            l.entry(&obs(0, 0, 0, 1 << 6), || 0);
            l.observe_put(1, 1, None);
            let possible = l.marker_possible(false, 1);
            assert_eq!(possible, mode == LifecycleMode::Default, "{mode:?}");
            assert_eq!(
                l.counts().pointer_carried_no_marker,
                u64::from(mode == LifecycleMode::NoPointerMarkers),
                "{mode:?}: the wake that has no marker is counted"
            );
            // a Window still places markers (parser state known)
            l.entry(&obs(1, 0, 1, B), || 1);
            assert_eq!(l.mode(), Mode::Window);
            assert!(l.marker_possible(false, 1), "{mode:?}: Window");
        }
    }

    /// `no_anchored_windows`: a Window starts at the entry that sees the registration (PHASE 1), not at the anchor.
    #[test]
    fn no_anchored_windows_starts_the_window_at_the_entry_that_sees_the_registration() {
        for mode in [LifecycleMode::Default, LifecycleMode::NoAnchoredWindows] {
            let mut l = Lifecycle::new().with_mode(mode);
            let o = |abs: u64, put_abs: u64, epoch: u64| Obs {
                put_abs,
                ..obs(abs, 0x10, epoch, B)
            };
            l.entry(&o(0, 6, 0), || 0);
            l.forwarded(0, 0x10);
            let plan = l.entry(&o(1, 6, 1), || 1);
            if mode == LifecycleMode::NoAnchoredWindows {
                assert!(matches!(plan, Plan::Scan { restart: true, .. }));
                assert_eq!(l.mode(), Mode::Window);
                assert_eq!(l.counts().anchor_forwarded, 0);
            } else {
                assert_eq!(plan, Plan::Forward, "below the anchor");
                assert_eq!(l.mode(), Mode::Pointer);
                assert_eq!(l.counts().anchor_forwarded, 1);
            }
        }
    }

    /// PHASE 1c item 3 (review 2, mutant A): leaving a Window re-anchors Pointer mode at the exit turn's `GP_PUT`. A
    /// registration seen at the very next entry (no clean Pointer turn in between) must not open the second Window
    /// inside the submission the exit turn belongs to.
    #[test]
    fn the_second_window_of_a_channel_is_anchored_at_the_gp_put_of_the_turn_that_left_the_first() {
        let mut l = Lifecycle::new();
        let o = |abs: u64, put_abs: u64, epoch: u64| Obs {
            put_abs,
            ..obs(abs, 0x10, epoch, B)
        };
        l.entry(&o(0, 1, 0), || 0); // bound: Pointer, first anchor 1
        l.forwarded(0, 0x10);
        for abs in 1..3u64 {
            l.entry(&o(abs, abs + 1, 1), || 1); // first registration
        }
        assert_eq!(l.mode(), Mode::Window);
        l.forwarded(2, 0x10);
        // the registration is gone, the relay is quiescent: the exit turn reads GP_PUT = 9 (a submission 3..9)
        assert_eq!(l.entry(&o(3, 9, 2), || 0), Plan::Forward);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!(
            l.safe_end, 9,
            "the exit re-anchors at the exit turn's GP_PUT"
        );
        l.forwarded(3, 0x10);
        // a second registration at the very next entry, inside that submission
        for abs in 4..9u64 {
            assert_eq!(
                l.entry(&o(abs, 9, 3), || 1),
                Plan::Forward,
                "entry {abs} is below the anchor: forwarded unscanned"
            );
            l.forwarded(abs, 0x10);
            assert_eq!(l.mode(), Mode::Pointer, "entry {abs}");
        }
        assert!(matches!(
            l.entry(&o(9, 12, 3), || 1),
            Plan::Scan { restart: true, .. }
        ));
        assert_eq!((l.mode(), l.counts().to_window_fresh), (Mode::Window, 2));
    }

    /// PHASE 1c item 4 (review 2, N4). `next_boundary` and `here` are absolute indices of the OLD ring's `GP_PUT`
    /// values; after a source switch they name nothing of the new ring and must not make a Pointer marker "safe".
    #[test]
    fn a_remembered_gp_put_boundary_of_the_old_ring_is_forgotten_at_a_source_switch() {
        let mut l = Lifecycle::new();
        l.entry(&obs(0, 0, 0, B), || 0);
        l.entry(&obs(1, 0, 1, B), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.forwarded(1, 0x10);
        // the OLD ring's GP_PUT is far ahead of the trigger (30)
        l.observe_put(2, 30, Some(false));
        assert_eq!(l.next_boundary, Some(30));
        l.source_switched();
        assert_eq!(l.next_boundary, None, "the old ring's boundary is gone");
        assert_eq!(l.here, None);
        // out of the Window, the cursor reaches index 30 of the NEW ring, where no GP_PUT was ever 30
        for abs in 3..14u64 {
            l.entry(&obs(abs, 0x10, 2, B), || 0); // the post-switch scan holds the Window for 8 entries, then it ends
            l.forwarded(abs, 0x10);
        }
        assert_eq!(l.mode(), Mode::Pointer);
        l.observe_put(30, 35, None);
        assert!(
            !l.marker_possible(false, 30),
            "30 was never a GP_PUT of the new ring: no marker there"
        );
        assert_eq!(l.counts().pointer_carried_no_marker, 1);
        // a boundary of the NEW ring is still honoured
        l.observe_put(31, 31, None);
        assert!(l.marker_possible(false, 31));
    }

    /// The boundary already counted at this index belongs to the old ring too: the new ring's boundary at the same
    /// index is a different one and is measured.
    #[test]
    fn a_boundary_counted_before_the_switch_does_not_hide_the_new_rings_boundary_at_the_same_index()
    {
        let mut l = Lifecycle::new();
        l.observe_put(4, 4, Some(false));
        assert_eq!(l.counts().put_boundaries_scan, 1);
        l.source_switched();
        l.observe_put(4, 4, Some(true));
        assert_eq!(
            (
                l.counts().put_boundaries_scan,
                l.counts().put_boundary_open_scan
            ),
            (2, 1)
        );
    }

    // ---- PHASE 1c item 2: the ring-full anchor guard, the P2' latch, the notices

    fn full(abs: u64, put_abs: u64, epoch: u64) -> Obs {
        Obs {
            put_abs,
            put_full: true,
            ..obs(abs, 0x10, epoch, B)
        }
    }
    fn at(abs: u64, put_abs: u64, epoch: u64) -> Obs {
        Obs {
            put_abs,
            ..obs(abs, 0x10, epoch, B)
        }
    }

    /// A GP_PUT read while the guest ring is full is never an anchor; the refusal is counted; a Window that then starts
    /// above the newest anchor says so (`anchor_stale_start`), it is not silent.
    #[test]
    fn a_gp_put_read_while_the_guest_ring_is_full_is_never_an_anchor() {
        let mut l = Lifecycle::new();
        l.entry(&at(0, 3, 0), || 0); // Scan -> Pointer, anchor 3
        l.forwarded(0, 0x10);
        for abs in 1..3u64 {
            l.entry(&at(abs, 3, 0), || 0);
            l.forwarded(abs, 0x10);
        }
        assert_eq!(l.safe_end, 3);
        // the cursor reaches 3; the guest published up to 9 with a FULL ring: not an anchor
        l.entry(&full(3, 9, 0), || 0);
        l.forwarded(3, 0x10);
        l.entry(&full(4, 9, 0), || 0);
        l.forwarded(4, 0x10);
        assert_eq!(
            l.safe_end, 3,
            "the full ring's GP_PUT did not move the anchor"
        );
        assert_eq!(l.counts().anchor_refused_ring_full, 2);
        // a non-full read moves it again
        l.entry(&at(5, 11, 0), || 0);
        l.forwarded(5, 0x10);
        assert_eq!((l.safe_end, l.counts().anchor_refused_ring_full), (11, 2));
        // refused anchors, then a registration: the Window starts ABOVE the anchor, loudly
        let mut l = Lifecycle::new();
        l.entry(&at(0, 3, 0), || 0);
        l.forwarded(0, 0x10);
        for abs in 1..6u64 {
            l.entry(&full(abs, 9, 0), || 0);
            l.forwarded(abs, 0x10);
        }
        assert_eq!(l.safe_end, 3);
        assert!(matches!(
            l.entry(&full(6, 9, 1), || 1),
            Plan::Scan { restart: true, .. }
        ));
        assert_eq!((l.mode(), l.counts().anchor_stale_start), (Mode::Window, 1));
        assert_eq!(l.counts().anchor_forwarded, 0);
        // the anchored start (ordinary run) is not stale
        let mut l = Lifecycle::new();
        l.entry(&at(0, 6, 0), || 0);
        l.forwarded(0, 0x10);
        for abs in 1..6u64 {
            l.entry(&at(abs, 6, 1), || 1);
            l.forwarded(abs, 0x10);
        }
        l.entry(&at(6, 9, 1), || 1);
        assert_eq!((l.mode(), l.counts().anchor_stale_start), (Mode::Window, 0));
        // the PHASE 1 start is unanchored by definition and not reported as stale
        let mut l = Lifecycle::new().with_mode(LifecycleMode::NoAnchoredWindows);
        l.entry(&at(0, 3, 0), || 0);
        l.forwarded(0, 0x10);
        l.entry(&full(1, 9, 1), || 1);
        assert_eq!((l.mode(), l.counts().anchor_stale_start), (Mode::Window, 0));
    }

    /// The first anchor (Scan -> Pointer) and the one after a Window follow the same rule.
    #[test]
    fn the_first_anchor_and_the_anchor_after_a_window_also_refuse_a_full_ring() {
        let mut l = Lifecycle::new();
        l.entry(&full(0, 4, 0), || 0);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!((l.safe_end, l.counts().anchor_refused_ring_full), (0, 1));
        let mut l = Lifecycle::new();
        l.entry(&at(0, 4, 0), || 0);
        l.forwarded(0, 0x10);
        l.entry(&at(1, 4, 1), || 1);
        assert_eq!(l.mode(), Mode::Pointer, "below the anchor");
        for abs in 1..4u64 {
            l.entry(&at(abs, 4, 1), || 1);
            l.forwarded(abs, 0x10);
        }
        l.entry(&at(4, 8, 1), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.forwarded(4, 0x10);
        // the registration is gone; the exit turn's GP_PUT was read with a full ring
        l.entry(&full(5, 13, 2), || 0);
        assert_eq!(l.mode(), Mode::Pointer);
        assert_eq!(l.safe_end, 4, "the exit kept the old anchor");
        assert_eq!(l.counts().anchor_refused_ring_full, 1);
        l.entry(&at(6, 13, 2), || 0);
        assert_eq!(l.safe_end, 13);
    }

    fn boundary_open(l: &mut Lifecycle, at_abs: u64) {
        l.observe_put(at_abs, at_abs, Some(true));
    }

    /// The first packet seen open at a GP_PUT boundary (parser state known) switches Pointer markers off, loudly,
    /// counted once; a closed boundary does not; Scan and Window keep their markers.
    #[test]
    fn the_first_packet_open_at_a_gp_put_boundary_switches_pointer_markers_off() {
        let mut l = Lifecycle::new();
        l.observe_put(2, 2, Some(false));
        assert!(!l.markers_off(), "a closed boundary latches nothing");
        assert!(l.take_notices().is_empty());
        boundary_open(&mut l, 5);
        assert!(l.markers_off());
        assert_eq!(l.counts().pointer_markers_latched_off, 1);
        let n = l.take_notices();
        assert_eq!(
            n,
            vec![
                Notice::PutBoundaryOpen {
                    mode: Mode::Scan,
                    relayed: 5,
                    put_abs: 5,
                    nth: 1
                },
                Notice::MarkersLatchedOff {
                    mode: Mode::Scan,
                    relayed: 5,
                    put_abs: 5
                }
            ]
        );
        assert!(
            l.marker_possible(false, 5),
            "Scan still places markers (parser state known)"
        );
        // Pointer mode at a real GP_PUT boundary: no marker, the unannounced wake is counted
        l.entry(&at(6, 9, 0), || 0);
        assert_eq!(l.mode(), Mode::Pointer);
        l.observe_put(7, 7, None);
        assert!(!l.marker_possible(false, 7), "latched off");
        assert_eq!(l.counts().pointer_carried_no_marker, 1);
        // a second open boundary adds a notice for the boundary but latches nothing new
        let mut w = Lifecycle::new();
        boundary_open(&mut w, 1);
        w.take_notices();
        boundary_open(&mut w, 2);
        assert_eq!(w.counts().pointer_markers_latched_off, 1, "once");
        assert_eq!(w.counts().put_boundary_open_scan, 2);
        assert!(matches!(
            w.take_notices().as_slice(),
            [Notice::PutBoundaryOpen { nth: 2, .. }]
        ));
    }

    /// VM-wide: the first channel latches, every other channel's Pointer markers go off at once without its having seen
    /// anything, and the VM counts the latch once.
    #[test]
    fn the_latch_is_vm_wide_and_counted_once() {
        let latch = std::sync::Arc::new(MarkerLatch::default());
        // `a` and `d` are still scanning (parser state known); `b` is in Pointer mode
        let mut a = Lifecycle::new().with_latch(latch.clone());
        let mut d = Lifecycle::new().with_latch(latch.clone());
        let mut b = Lifecycle::new().with_latch(latch.clone());
        let mut c = Lifecycle::new().with_latch(std::sync::Arc::new(MarkerLatch::default()));
        for l in [&mut b, &mut c] {
            l.entry(&at(0, 3, 0), || 0);
            assert_eq!(l.mode(), Mode::Pointer);
            l.observe_put(1, 1, None);
            assert!(l.marker_possible(false, 1));
        }
        boundary_open(&mut a, 4);
        assert!(
            !b.marker_possible(false, 1),
            "b never saw an open packet itself"
        );
        assert_eq!(b.counts().pointer_carried_no_marker, 1);
        boundary_open(&mut d, 4); // d also sees one: joins, does not count a second latch
        assert_eq!(
            a.counts().pointer_markers_latched_off + d.counts().pointer_markers_latched_off,
            1
        );
        assert!(
            d.take_notices()
                .iter()
                .all(|n| !matches!(n, Notice::MarkersLatchedOff { .. }))
        );
        assert!(d.markers_off());
        assert!(
            c.marker_possible(false, 1),
            "another VM's machine is unaffected"
        );
        assert!(latch.is_set());
        assert!(!latch.set(), "set() is true for exactly one caller");
    }

    /// The first OPEN_NOTICES boundaries of a channel are reported; inside a Window every one is.
    #[test]
    fn put_boundary_open_notices_are_bounded_outside_a_window_and_unbounded_inside_one() {
        let mut l = Lifecycle::new();
        for i in 0..20u64 {
            boundary_open(&mut l, 100 + i);
        }
        let n = l.take_notices();
        let opens = n
            .iter()
            .filter(|n| matches!(n, Notice::PutBoundaryOpen { .. }))
            .count();
        assert_eq!(opens as u64, OPEN_NOTICES);
        assert_eq!(
            l.counts().put_boundary_open_scan,
            20,
            "the counter is uncapped"
        );
        // a Window: every occurrence
        let mut w = Lifecycle::new();
        for i in 0..OPEN_NOTICES + 2 {
            boundary_open(&mut w, 100 + i);
        }
        w.take_notices();
        w.entry(&at(200, 0, 1), || 1);
        assert_eq!(w.mode(), Mode::Window);
        for i in 0..5u64 {
            boundary_open(&mut w, 300 + i);
        }
        let n = w.take_notices();
        assert_eq!(
            n.iter()
                .filter(|n| matches!(
                    n,
                    Notice::PutBoundaryOpen {
                        mode: Mode::Window,
                        ..
                    }
                ))
                .count(),
            5
        );
    }

    fn fresh_window(first: Option<(bool, bool)>, seeded: bool) -> Vec<Notice> {
        // Pointer mode with an unproven SYNC_WAIT (seeded) or not, a registration, the anchor, the window
        let mut l = Lifecycle::new();
        l.entry(&at(0, 3, 0), || 0);
        l.forwarded(0, if seeded { SYNC | 1 } else { 1 });
        for abs in 1..3u64 {
            l.entry(&at(abs, 3, 1), || 1);
            l.forwarded(abs, 1);
        }
        l.entry(&at(3, 5, 1), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.restarted();
        match first {
            Some((open, false)) => l.segment_scanned(open),
            Some((_, true)) => l.scan_refused(),
            None => {
                l.forwarded(3, 1);
                l.entry(&at(4, 5, 2), || 0);
                assert_eq!(l.mode(), Mode::Pointer);
            }
        }
        l.take_notices()
    }

    #[test]
    fn window_fresh_reports_the_flip_and_the_result_of_its_first_segment_once() {
        let flip = |first, seeded| Notice::WindowFresh {
            cursor: 3,
            anchor: 3,
            anchor_forwarded: 2,
            seeded,
            stale: false,
            first,
        };
        assert_eq!(
            fresh_window(Some((false, false)), false),
            vec![flip(FreshFirst::Closed, false)]
        );
        assert_eq!(
            fresh_window(Some((true, false)), true),
            vec![flip(FreshFirst::EndsOpen, true)]
        );
        assert_eq!(
            fresh_window(Some((false, true)), false),
            vec![flip(FreshFirst::Refused, false)]
        );
        assert_eq!(
            fresh_window(None, false),
            vec![flip(FreshFirst::None, false)]
        );
        // a second segment adds nothing
        let mut l = Lifecycle::new();
        l.entry(&at(0, 1, 0), || 0);
        l.entry(&at(1, 2, 1), || 1);
        assert_eq!(l.mode(), Mode::Window);
        l.segment_scanned(false);
        l.segment_scanned(true);
        l.scan_refused();
        assert_eq!(l.take_notices().len(), 1);
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

    /// 2026-10-20: the stall diagnostic's counters ride the existing lifecycle counters line, and only when counted.
    #[test]
    fn the_stall_diagnostic_counters_join_the_summary_line_only_when_counted() {
        use std::sync::atomic::Ordering::Relaxed;
        let stats = LifecycleStats::default();
        assert!(!stats.summary().contains("diag_user_stall"), "{}", stats.summary());
        let before = stats.summary();
        stats.stall.conditions.fetch_add(2, Relaxed);
        stats.stall.acquires.fetch_add(5, Relaxed);
        let line = stats.summary();
        assert!(line.starts_with(&before), "the existing fields keep their place: {line}");
        assert!(line.contains("diag_user_stall_conditions=0x2"), "{line}");
        assert!(line.contains("diag_user_stall_acquires=0x5"), "{line}");
        assert!(!line.contains('\n'));
    }
}
